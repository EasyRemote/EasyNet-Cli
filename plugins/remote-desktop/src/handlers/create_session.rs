// EasyNet CLI — remote desktop create-session handler
// ===================================================

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_CREATE_SESSION;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{now_ms, RemoteDesktopSession};
use crate::daemon::plugins::remote_desktop::session_creation::RemoteDesktopSessionCreationWorkflow;
use crate::daemon::plugins::remote_desktop::session_lifecycle::prune_inactive_sessions;
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
use crate::daemon::plugins::remote_desktop::view::serialize_session_with_token;

/// Handle `remote_desktop.create_session`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args)?;
    preflight_session_insert(&plugin, workflow.session_id())?;
    let target_binding_verifier = plugin.target_binding_verifier();
    let workflow = workflow
        .consume_consent(&plugin.consent_registry(), &env)?
        .resolve_target_with_verifier(target_binding_verifier.as_ref())?;
    insert_created_session(plugin, workflow)
}

fn insert_created_session(
    plugin: Arc<RemoteDesktopPlugin>,
    workflow: RemoteDesktopSessionCreationWorkflow,
) -> anyhow::Result<Value> {
    let session_id = workflow.session_id().to_string();
    let session = RemoteDesktopSession::new(workflow.into_session_init()?);
    let recovery_snapshot = RemoteDesktopRecoverySnapshot::from_session(&session)?;
    let now = now_ms();
    let (
        pruned_recovery_snapshots,
        watchdog_session_id,
        tracker_session_id,
        lease_expires_at_ms,
        view,
    ) = plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<_> {
            let pruned_recovery_snapshots = prune_inactive_sessions(&plugin, sessions, now);
            ensure_session_insertable(plugin.config().max_sessions(), sessions, &session_id)?;
            let watchdog_session_id = session_id.clone();
            let tracker_session_id = session_id.clone();
            let lease_expires_at_ms = session.lease_expires_at_ms();
            let view = serialize_session_with_token(&session);
            sessions.insert(session_id, session);
            Ok((
                pruned_recovery_snapshots,
                watchdog_session_id,
                tracker_session_id,
                lease_expires_at_ms,
                view,
            ))
        })?;
    if let Err(err) = persist_recovery_snapshots(&plugin, &pruned_recovery_snapshots) {
        remove_inserted_session(&plugin, &tracker_session_id);
        return Err(err);
    }
    if let Err(err) = RemoteDesktopPlugin::schedule_session_lease(
        &plugin,
        watchdog_session_id.clone(),
        lease_expires_at_ms,
    ) {
        remove_inserted_session(&plugin, &tracker_session_id);
        return Err(err);
    }
    if let Err(err) = RemoteDesktopPlugin::track_session_target(&plugin, tracker_session_id.clone())
    {
        plugin.cancel_session_lease(&watchdog_session_id);
        remove_inserted_session(&plugin, &tracker_session_id);
        return Err(err);
    }
    if let Err(err) = plugin.persist_recovery_snapshot(&recovery_snapshot) {
        plugin.cancel_session_lease(&watchdog_session_id);
        plugin.cancel_session_target_tracking(&tracker_session_id);
        remove_inserted_session(&plugin, &tracker_session_id);
        return Err(err);
    }
    Ok(view)
}

fn persist_recovery_snapshots(
    plugin: &RemoteDesktopPlugin,
    snapshots: &[RemoteDesktopRecoverySnapshot],
) -> anyhow::Result<()> {
    for snapshot in snapshots {
        plugin.persist_recovery_snapshot(snapshot)?;
    }
    Ok(())
}

fn remove_inserted_session(plugin: &RemoteDesktopPlugin, session_id: &str) {
    plugin.session_store().with_sessions(|sessions| {
        sessions.remove(session_id);
    });
}

fn preflight_session_insert(
    plugin: &Arc<RemoteDesktopPlugin>,
    session_id: &str,
) -> anyhow::Result<()> {
    let now = now_ms();
    let (recovery_snapshots, insertable) = plugin.session_store().with_sessions(|sessions| {
        let recovery_snapshots = prune_inactive_sessions(plugin, sessions, now);
        let insertable =
            ensure_session_insertable(plugin.config().max_sessions(), sessions, session_id);
        (recovery_snapshots, insertable)
    });
    persist_recovery_snapshots(plugin, &recovery_snapshots)?;
    insertable
}

fn ensure_session_insertable(
    max_sessions: usize,
    sessions: &HashMap<String, RemoteDesktopSession>,
    session_id: &str,
) -> anyhow::Result<()> {
    if sessions.contains_key(session_id) {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_CREATE_SESSION,
            detail: format!("session_id {session_id:?} already exists"),
        }
        .into());
    }
    let active_sessions = sessions
        .values()
        .filter(|session| !session.is_terminal())
        .count();
    if active_sessions >= max_sessions {
        return Err(RemoteDesktopError::SessionStoreFull {
            ability: ABILITY_CREATE_SESSION,
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::persistence::{
        resources,
        resources::{ResourceBinding, ResourceType, ResourceUpsert, ResourcesFile},
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, test_lock, test_plugin, with_consent_ticket,
        with_input_control_consent_ticket,
    };

    #[test]
    fn create_session_requires_subject() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let err = handle(
            Arc::clone(&plugin),
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/alice",
                "easynet:///r/acme/user/alice",
            ),
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("subject_required"));
    }

    #[test]
    fn create_session_rejects_subject_in_args() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let err = handle(
            Arc::clone(&plugin),
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/alice",
                "easynet:///r/acme/resource/01",
            ),
            json!({"subject": "bad"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("subject_in_args"));
    }

    #[test]
    fn create_session_requires_local_user_consent_receipt() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-no-consent-display");
        resources::save(&file).unwrap();
        let env = EnvelopeContext::for_test("easynet:///r/acme/user/alice", ura);
        let args = with_consent_ticket(&plugin, &env, json!({"session_id": "rd-no-consent"}));
        let err = handle(Arc::clone(&plugin), env, args).unwrap_err();
        assert!(err.to_string().contains("consent_receipt_required"));
    }

    #[test]
    fn create_session_returns_target_binding_and_scope_audit() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-binding-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let response = handle(
            Arc::clone(&plugin),
            env.clone(),
            with_consent_ticket(
                &plugin,
                &env,
                json!({"session_id": "rd-binding-view", "mode": "view_only"}),
            ),
        )
        .unwrap();

        assert_eq!(response["target_binding"]["subject_ura"], json!(ura));
        assert_eq!(response["target_binding"]["target_kind"], json!("display"));
        assert_eq!(
            response["target_binding"]["capture_scope"],
            json!("DisplaySurface")
        );
        assert_eq!(response["scope_audit"]["scope_widened"], json!(false));
        assert_eq!(
            response["scope_audit"]["display_fallback_used"],
            json!(false)
        );
        assert_eq!(
            response["latest_target_diagnostic"]["status"],
            json!("resolved")
        );
    }

    #[test]
    fn create_session_persists_recovery_snapshot() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-recovery-create-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);

        handle(
            Arc::clone(&plugin),
            env.clone(),
            with_consent_ticket(
                &plugin,
                &env,
                json!({"session_id": "rd-recovery-create", "mode": "view_only"}),
            ),
        )
        .expect("create_session persists an active recovery snapshot");

        let snapshot = plugin
            .recovery_store()
            .load("rd-recovery-create")
            .expect("recovery snapshot load succeeds")
            .expect("create_session must write a recovery snapshot");
        let snapshot = serde_json::to_value(snapshot).expect("snapshot serializes");
        assert_eq!(snapshot["session_id"], json!("rd-recovery-create"));
        assert_eq!(snapshot["selected_resource_ura"], json!(ura));
        assert_eq!(snapshot["lifecycle_state"], json!("negotiating"));
        assert_eq!(snapshot["terminal_receipt"], Value::Null);
    }

    #[test]
    fn create_session_uses_explicit_input_control_consent_for_display_interactive_scope() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-input-consent-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let response = handle(
            Arc::clone(&plugin),
            env.clone(),
            with_input_control_consent_ticket(
                &plugin,
                &env,
                json!({
                    "session_id": "rd-input-consent-display",
                    "mode": "interactive",
                    "input_policy": {
                        "keyboard_enabled": true,
                        "pointer_enabled": true
                    }
                }),
            ),
        )
        .unwrap();

        assert_eq!(
            response["consent"]["grant_scope"]["input_control"],
            json!(true)
        );
        assert_eq!(
            response["input_policy"]["input_scope"],
            json!("display_global")
        );
        assert_eq!(response["input_policy"]["keyboard_enabled"], json!(true));
        assert_eq!(response["input_policy"]["pointer_enabled"], json!(true));
        assert_eq!(
            response["scope_audit"]["input_scope_reason"],
            json!("input_control_granted")
        );
        if crate::daemon::plugins::remote_desktop::input::input_injection_available() {
            assert_eq!(
                response["input_readiness"]["effective_mode"],
                json!("interactive")
            );
            assert_eq!(
                response["input_readiness"]["interactive_ready"],
                json!(true)
            );
            assert_eq!(response["input_readiness"]["blocked_reason"], Value::Null);
        } else {
            assert_eq!(
                response["input_readiness"]["effective_mode"],
                json!("view_only")
            );
            assert_eq!(
                response["input_readiness"]["interactive_ready"],
                json!(false)
            );
            assert_eq!(
                response["input_readiness"]["blocked_reason"],
                json!("input_injection_unavailable")
            );
        }
    }

    #[test]
    fn create_session_duplicate_session_id_does_not_consume_consent_ticket() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-duplicate-preflight-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        handle(
            Arc::clone(&plugin),
            env.clone(),
            with_consent_ticket(
                &plugin,
                &env,
                json!({"session_id": "rd-duplicate-preflight", "mode": "view_only"}),
            ),
        )
        .expect("first session inserts");

        let issued = plugin
            .consent_registry()
            .issue(
                env.caller(),
                env.subject(),
                crate::daemon::plugins::remote_desktop::consent_registry::CONSENT_INTENT,
            )
            .expect("second consent ticket issues");
        let err = handle(
            Arc::clone(&plugin),
            env.clone(),
            json!({
                "session_id": "rd-duplicate-preflight",
                "mode": "view_only",
                "consent_ticket": issued.ticket.clone(),
            }),
        )
        .unwrap_err();

        assert!(err.to_string().contains("already exists"));
        plugin
            .consent_registry()
            .consume(
                &issued.ticket,
                env.caller(),
                env.subject(),
                crate::daemon::plugins::remote_desktop::consent_registry::CONSENT_INTENT,
            )
            .expect("duplicate-session preflight must fail before consuming consent");
    }

    #[test]
    fn create_session_rejects_unbound_display_identity_before_session_insert() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = resources::upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                kind: ResourceType::Display,
                binding: ResourceBinding::LocalDevice,
                hardware_id: "remote-desktop-display-without-identity",
                display_name: "Display without identity",
                metadata: json!({}),
            },
        )
        .unwrap();
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let err = handle(
            Arc::clone(&plugin),
            env.clone(),
            with_consent_ticket(
                &plugin,
                &env,
                json!({"session_id": "rd-missing-binding", "mode": "view_only"}),
            ),
        )
        .unwrap_err();

        assert!(err.to_string().contains("display_identity_missing"));
        plugin.session_store().with_sessions(|sessions| {
            assert!(
                !sessions.contains_key("rd-missing-binding"),
                "target resolution failure must not insert a session row"
            );
        });
    }

    #[test]
    fn create_session_rejects_stale_window_inventory_before_session_insert() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = resources::upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                kind: ResourceType::Window,
                binding: ResourceBinding::LocalDevice,
                hardware_id: "remote-desktop-stale-window",
                display_name: "Closed window",
                metadata: json!({
                    "availability": "unavailable",
                    "stale_reason": "target_not_found",
                    "window_id": 7,
                    "pid": 4242,
                }),
            },
        )
        .unwrap();
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let err = handle(
            Arc::clone(&plugin),
            env.clone(),
            with_consent_ticket(
                &plugin,
                &env,
                json!({"session_id": "rd-stale-window", "mode": "view_only"}),
            ),
        )
        .unwrap_err();

        assert!(err.to_string().contains("target_not_found"));
        assert!(err.to_string().contains("frontend_action=refresh_targets"));
        plugin.session_store().with_sessions(|sessions| {
            assert!(
                !sessions.contains_key("rd-stale-window"),
                "stale target failure must not insert a session row"
            );
        });
    }

    #[test]
    fn create_session_rejects_weak_window_identity_before_session_insert() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = resources::upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                kind: ResourceType::Window,
                binding: ResourceBinding::LocalDevice,
                hardware_id: "remote-desktop-weak-window-identity",
                display_name: "Terminal — same-looking shell",
                metadata: json!({
                    "availability": "available",
                    "freshness": {
                        "observed_at_ms": 1,
                        "stale_after_ms": u64::MAX,
                        "source": "live_refresh",
                    },
                    "window_id": 7,
                    "app_name": "Terminal",
                    "title": "same-looking shell",
                }),
            },
        )
        .unwrap();
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let err = handle(
            Arc::clone(&plugin),
            env.clone(),
            with_consent_ticket(
                &plugin,
                &env,
                json!({"session_id": "rd-weak-window", "mode": "view_only"}),
            ),
        )
        .unwrap_err();

        assert!(err.to_string().contains("target_identity_ambiguous"));
        plugin.session_store().with_sessions(|sessions| {
            assert!(
                !sessions.contains_key("rd-weak-window"),
                "weak target identity failure must not insert a session row"
            );
        });
    }
}
