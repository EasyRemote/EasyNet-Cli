// EasyNet CLI — remote desktop create-session handler
// ===================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_CREATE_SESSION;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{now_ms, RemoteDesktopSession};
use crate::daemon::plugins::remote_desktop::session_creation::RemoteDesktopSessionCreationWorkflow;
use crate::daemon::plugins::remote_desktop::session_lifecycle::prune_inactive_sessions;
use crate::daemon::plugins::remote_desktop::view::serialize_session_with_token;

/// Handle `remote_desktop.create_session`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args)?
        .consume_consent(&plugin.consent_registry(), &env)?
        .resolve_target()?;
    let session_id = workflow.session_id().to_string();
    let session = RemoteDesktopSession::new(workflow.into_session_init());
    let now = now_ms();
    let (watchdog_session_id, tracker_session_id, lease_expires_at_ms, view) = plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<_> {
            prune_inactive_sessions(&plugin, sessions, now);
            let active_sessions = sessions
                .values()
                .filter(|session| !session.is_terminal())
                .count();
            if active_sessions >= plugin.config().max_sessions() {
                return Err(RemoteDesktopError::SessionStoreFull {
                    ability: ABILITY_CREATE_SESSION,
                }
                .into());
            }
            if sessions.contains_key(&session_id) {
                return Err(RemoteDesktopError::InvalidArgument {
                    ability: ABILITY_CREATE_SESSION,
                    detail: format!("session_id {session_id:?} already exists"),
                }
                .into());
            }
            let watchdog_session_id = session_id.clone();
            let tracker_session_id = session_id.clone();
            let lease_expires_at_ms = session.lease_expires_at_ms();
            let view = serialize_session_with_token(&session);
            sessions.insert(session_id, session);
            Ok((
                watchdog_session_id,
                tracker_session_id,
                lease_expires_at_ms,
                view,
            ))
        })?;
    RemoteDesktopPlugin::schedule_session_lease(&plugin, watchdog_session_id, lease_expires_at_ms);
    RemoteDesktopPlugin::track_session_target(&plugin, tracker_session_id);
    Ok(view)
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
}
