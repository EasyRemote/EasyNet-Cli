// EasyNet CLI — remote desktop session lifecycle
// ===============================================
//
// File: src/plugins/builtin/remote_desktop/session_lifecycle.rs
// Description: Lease, liveness, terminal cleanup, and transport teardown.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::plugins::remote_desktop::constants::{REASON_SESSION_EXPIRED, REASON_SESSION_TERMINAL};
use crate::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::plugins::remote_desktop::session::{now_ms, RemoteDesktopSession};
use crate::plugins::remote_desktop::session_access::{
    ensure_session_control_identity, ensure_session_resource_identity,
};

pub(in crate::plugins::builtin::remote_desktop) fn ensure_session_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_resource_identity(ability, env, args, session)?;
    ensure_session_liveness(plugin, ability, session)
}

pub(in crate::plugins::builtin::remote_desktop) fn ensure_session_control_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_control_identity(ability, env, args, session)?;
    ensure_session_liveness(plugin, ability, session)
}

fn ensure_session_liveness(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    if expire_session_if_needed(plugin, session, now_ms()) {
        anyhow::bail!(
            "{ability}: session {:?} lease expired at {}; reason={REASON_SESSION_EXPIRED}",
            session.session_id(),
            session.lease_expires_at_ms()
        );
    }
    ensure_not_terminal(ability, session)
}

fn ensure_not_terminal(ability: &str, session: &RemoteDesktopSession) -> anyhow::Result<()> {
    if session.is_terminal() {
        anyhow::bail!(
            "{ability}: session {:?} is terminal; reason={REASON_SESSION_TERMINAL}",
            session.session_id()
        );
    }
    Ok(())
}

pub(in crate::plugins::builtin::remote_desktop) fn stop_direct_webrtc_endpoint(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
) {
    plugin.transport_manager().stop_endpoint(session_id);
}

pub(in crate::plugins::builtin::remote_desktop) fn stop_session_transports(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    session: &mut RemoteDesktopSession,
) {
    if let Some(stop_tx) = session.detach_preview_transport() {
        let _ = stop_tx.send(true);
    }
    stop_direct_webrtc_endpoint(plugin, session_id);
}

fn expire_session_if_needed(
    plugin: &RemoteDesktopPlugin,
    session: &mut RemoteDesktopSession,
    now: u64,
) -> bool {
    if session.is_terminal() || !session.is_expired_at(now) {
        return false;
    }
    let session_id = session.session_id().to_string();
    stop_session_transports(plugin, &session_id, session);
    session.expire(now);
    true
}

pub(in crate::plugins::builtin::remote_desktop) fn spawn_session_lease_watchdog(
    plugin: Arc<RemoteDesktopPlugin>,
    session_id: String,
    lease_expires_at_ms: u64,
) {
    let delay_ms = lease_expires_at_ms.saturating_sub(now_ms());
    let watchdog_plugin = Arc::downgrade(&plugin);
    plugin.transport_manager().spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        let Some(plugin) = watchdog_plugin.upgrade() else {
            return;
        };
        expire_session_from_watchdog(&plugin, &session_id, lease_expires_at_ms);
    });
}

pub(in crate::plugins::builtin::remote_desktop) fn expire_session_from_watchdog(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    expected_lease_expires_at_ms: u64,
) {
    plugin.session_store().with_sessions(|sessions| {
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        if session.lease_expires_at_ms() != expected_lease_expires_at_ms {
            return;
        }
        let _ = expire_session_if_needed(plugin, session, now_ms());
    });
}

pub(in crate::plugins::builtin::remote_desktop) fn prune_inactive_sessions(
    plugin: &RemoteDesktopPlugin,
    sessions: &mut HashMap<String, RemoteDesktopSession>,
    now: u64,
) {
    let mut removed = Vec::new();
    sessions.retain(|session_id, session| {
        let keep = !session.is_terminal() && !session.is_expired_at(now);
        if !keep {
            removed.push((session_id.clone(), session.detach_preview_transport()));
        }
        keep
    });
    for (session_id, preview_stop_tx) in removed {
        if let Some(stop_tx) = preview_stop_tx {
            let _ = stop_tx.send(true);
        }
        stop_direct_webrtc_endpoint(plugin, &session_id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tokio::sync::watch;

    use super::*;
    use crate::persistence::resources::{self, ResourceType, ResourcesFile};
    use crate::plugins::remote_desktop::constants::TRANSPORT_WEBRTC;
    use crate::plugins::remote_desktop::request::{
        RemoteDesktopInputPolicy, RemoteDesktopVideoConstraints,
    };
    use crate::plugins::remote_desktop::session::{RemoteDesktopSessionInit, RemoteDesktopState};
    use crate::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
    use crate::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, test_lock, test_plugin,
    };

    #[test]
    fn expired_session_rejects_non_end_operations() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-expired-display");
        resources::save(&file).unwrap();

        let created = crate::plugins::remote_desktop::handlers::create_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-expired-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();
        {
            plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get_mut("rd-expired-test")
                    .unwrap()
                    .set_lease_expires_at_for_test(now_ms().saturating_sub(1));
            });
        }

        let err = crate::plugins::remote_desktop::handlers::show_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-test", "session_token": token.clone()}),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_SESSION_EXPIRED));

        let ended = crate::plugins::remote_desktop::handlers::end_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-test", "session_token": token.clone()}),
        )
        .unwrap();
        assert_eq!(ended["already_ended"], json!(true));
        assert_eq!(ended["end_reason"], json!(REASON_SESSION_EXPIRED));
    }

    #[test]
    fn expired_session_stops_preview_worker() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-expired-preview-display");
        resources::save(&file).unwrap();

        let created = crate::plugins::remote_desktop::handlers::create_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-expired-preview-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();
        let (stop_tx, mut stop_rx) = watch::channel(false);
        {
            plugin.session_store().with_sessions(|sessions| {
                let session = sessions.get_mut("rd-expired-preview-test").unwrap();
                session.install_preview_transport_for_test(stop_tx);
                session.set_lease_expires_at_for_test(now_ms().saturating_sub(1));
            });
        }

        let err = crate::plugins::remote_desktop::handlers::show_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-preview-test", "session_token": token}),
        )
        .unwrap_err();

        assert!(err.to_string().contains(REASON_SESSION_EXPIRED));
        assert!(
            *stop_rx.borrow_and_update(),
            "lease expiry must signal the preview worker stop channel"
        );
    }

    #[test]
    fn lease_watchdog_terminal_path_closes_transports_without_rpc() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-watchdog-display");
        resources::save(&file).unwrap();

        let created = crate::plugins::remote_desktop::handlers::create_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-watchdog-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let original_lease = created["lease_expires_at_ms"].as_u64().unwrap();
        assert!(original_lease > now_ms());
        let expected_lease = now_ms().saturating_sub(1);
        let (stop_tx, mut stop_rx) = watch::channel(false);
        {
            plugin.session_store().with_sessions(|sessions| {
                let session = sessions.get_mut("rd-watchdog-test").unwrap();
                session.install_preview_transport_for_test(stop_tx);
                session.set_lease_expires_at_for_test(expected_lease);
            });
        }

        expire_session_from_watchdog(&plugin, "rd-watchdog-test", expected_lease);

        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get("rd-watchdog-test").unwrap();
            assert_eq!(session.state(), RemoteDesktopState::Closed);
            assert_eq!(session.end_reason(), Some(REASON_SESSION_EXPIRED));
        });
        assert!(
            *stop_rx.borrow_and_update(),
            "watchdog expiry must signal active transports"
        );
    }

    #[test]
    fn create_session_prunes_inactive_sessions_before_capacity_check() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-capacity-display");
        resources::save(&file).unwrap();
        {
            plugin.session_store().with_sessions(|sessions| {
                for index in 0..plugin.config().max_sessions() {
                    let now = now_ms();
                    let session_id = format!("stale-{index}");
                    let mut session = RemoteDesktopSession::new(RemoteDesktopSessionInit {
                        session_id: session_id.clone(),
                        session_token: format!("token-{index}"),
                        creator_caller_ura: Some("easynet:///r/acme/user/test".to_string()),
                        consent: RemoteDesktopConsentGrant::from_envelope_for_test(&env_for(&ura)),
                        subject_ura: ura.clone(),
                        subject_type: ResourceType::Display,
                        subject_display_name: "Test Display".to_string(),
                        mode: "view_only".to_string(),
                        lease_ttl_ms: 1,
                        transport_preferences: vec![TRANSPORT_WEBRTC.to_string()],
                        video: RemoteDesktopVideoConstraints::default(),
                        input_policy: RemoteDesktopInputPolicy::default(),
                    });
                    session.set_lease_expires_at_for_test(now.saturating_sub(1));
                    session.close("test_stale");
                    sessions.insert(session_id, session);
                }
            });
        }

        let created = crate::plugins::remote_desktop::handlers::create_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-after-prune",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();

        assert_eq!(created["session_id"], json!("rd-after-prune"));
        plugin.session_store().with_sessions(|sessions| {
            assert_eq!(sessions.len(), 1);
            assert!(sessions.contains_key("rd-after-prune"));
        });
    }
}
