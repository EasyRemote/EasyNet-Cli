// EasyNet CLI — remote desktop session lifecycle
// ===============================================
//
// File: plugins/remote-desktop/src/session_lifecycle.rs
// Description: Lease, liveness, terminal cleanup, and transport teardown.

use std::collections::HashMap;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{now_ms, RemoteDesktopSession};
use crate::daemon::plugins::remote_desktop::session_access::{
    ensure_session_control_identity, ensure_session_resource_identity,
};
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;

pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_resource_identity(ability, env, args, session)?;
    ensure_session_liveness(plugin, ability, session)
}

pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_control_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_control_identity(ability, env, args, session)?;
    ensure_session_liveness(plugin, ability, session)
}

pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_control_audit_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_control_identity(ability, env, args, session)?;
    let _ = expire_session_if_needed(plugin, session, now_ms());
    Ok(())
}

fn ensure_session_liveness(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    if expire_session_if_needed(plugin, session, now_ms()) {
        return Err(RemoteDesktopError::SessionExpired {
            ability,
            session_id: session.session_id().to_string(),
        }
        .into());
    }
    ensure_not_terminal(ability, session)
}

fn ensure_not_terminal(
    ability: &'static str,
    session: &RemoteDesktopSession,
) -> anyhow::Result<()> {
    if session.is_terminal() {
        return Err(RemoteDesktopError::SessionTerminal {
            ability,
            session_id: session.session_id().to_string(),
        }
        .into());
    }
    Ok(())
}

pub(in crate::daemon::plugins::remote_desktop) fn stop_direct_webrtc_endpoint(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
) {
    plugin.transport_manager().stop_endpoint(session_id);
}

pub(in crate::daemon::plugins::remote_desktop) fn stop_session_transports(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    session: &mut RemoteDesktopSession,
) {
    plugin.cancel_session_lease(session_id);
    plugin.cancel_session_target_tracking(session_id);
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

pub(in crate::daemon::plugins::remote_desktop) fn expire_session_from_watchdog(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    expected_lease_expires_at_ms: u64,
) {
    let recovery_snapshot = plugin.session_store().with_sessions(|sessions| {
        let Some(session) = sessions.get_mut(session_id) else {
            return None;
        };
        if session.lease_expires_at_ms() != expected_lease_expires_at_ms {
            return None;
        }
        if expire_session_if_needed(plugin, session, now_ms()) {
            return RemoteDesktopRecoverySnapshot::from_session(session).ok();
        }
        None
    });
    if let Some(recovery_snapshot) = recovery_snapshot {
        if let Err(err) = plugin.persist_recovery_snapshot(&recovery_snapshot) {
            eprintln!(
                "[remote-desktop] failed to persist lease-watchdog recovery snapshot for {session_id}: {err}"
            );
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn prune_inactive_sessions(
    plugin: &RemoteDesktopPlugin,
    sessions: &mut HashMap<String, RemoteDesktopSession>,
    now: u64,
) -> Vec<RemoteDesktopRecoverySnapshot> {
    let mut recovery_snapshots = Vec::new();
    let expired: Vec<String> = sessions
        .iter()
        .filter(|(_, session)| !session.is_terminal() && session.is_expired_at(now))
        .map(|(session_id, _)| session_id.clone())
        .collect();
    for session_id in expired {
        if let Some(session) = sessions.get_mut(&session_id) {
            stop_session_transports(plugin, &session_id, session);
            session.expire(now);
            if let Ok(recovery_snapshot) = RemoteDesktopRecoverySnapshot::from_session(session) {
                recovery_snapshots.push(recovery_snapshot);
            }
        }
    }

    let _ = RemoteDesktopSessionStore::prune_terminal_rows_to_active_bound_locked(sessions);
    recovery_snapshots
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tokio::sync::watch;

    use super::*;
    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::constants::{
        REASON_SESSION_EXPIRED, TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::session::{
        RemoteDesktopSession, RemoteDesktopState,
    };
    use crate::daemon::plugins::remote_desktop::session_store::MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION;
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, test_lock, test_plugin, test_session_init,
    };

    #[test]
    fn expired_session_allows_audit_read_and_then_idempotent_end() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-expired-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
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

        let shown = crate::daemon::plugins::remote_desktop::handlers::show_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-test", "session_token": token.clone()}),
        )
        .unwrap();
        assert_eq!(shown["state"], json!("closed"));
        assert_eq!(shown["end_reason"], json!(REASON_SESSION_EXPIRED));

        let ended = crate::daemon::plugins::remote_desktop::handlers::end_session::handle(
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-expired-preview-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
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

        let shown = crate::daemon::plugins::remote_desktop::handlers::show_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-preview-test", "session_token": token}),
        )
        .unwrap();

        assert_eq!(shown["end_reason"], json!(REASON_SESSION_EXPIRED));
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-watchdog-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
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
    fn create_session_ignores_terminal_tombstones_for_capacity_check() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-capacity-display");
        resources::save(&file).unwrap();
        {
            let mut stale_sessions = Vec::new();
            for index in 0..plugin.config().max_sessions() {
                let now = now_ms();
                let session_id = format!("stale-{index}");
                let mut session = RemoteDesktopSession::new(test_session_init(
                    &session_id,
                    &ura,
                    vec![TRANSPORT_WEBRTC.to_string()],
                ));
                session.set_lease_expires_at_for_test(now.saturating_sub(1));
                session.close("test_stale");
                stale_sessions.push((session_id, session));
            }
            plugin.session_store().with_sessions(|sessions| {
                for (session_id, session) in stale_sessions {
                    sessions.insert(session_id, session);
                }
            });
        }

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
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
            let active_sessions = sessions
                .values()
                .filter(|session| !session.is_terminal())
                .count();
            let terminal_rows = sessions
                .values()
                .filter(|session| session.is_terminal())
                .count();
            assert_eq!(active_sessions, 1);
            assert!(
                terminal_rows
                    <= active_sessions.saturating_mul(MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION)
            );
            assert!(sessions.contains_key("rd-after-prune"));
        });
    }
}
