// EasyNet CLI — remote desktop end-session handler
// ================================================

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_END_SESSION;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_access::ensure_session_control_identity;
use crate::daemon::plugins::remote_desktop::session_lifecycle::stop_session_transports;
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
use crate::daemon::plugins::remote_desktop::view::serialize_session;

/// Handle `remote_desktop.end_session`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_END_SESSION)?.to_string();
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("caller_ended")
        .to_string();
    let (recovery_snapshot, view) = plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<_> {
            let session = sessions.get_mut(&session_id).ok_or_else(|| {
                RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_END_SESSION,
                    session_id: session_id.clone(),
                }
            })?;
            ensure_session_control_identity(ABILITY_END_SESSION, &env, &args, session)?;
            if session.is_terminal() {
                let mut view = serialize_session(session);
                if let Some(map) = view.as_object_mut() {
                    map.insert("already_ended".into(), json!(true));
                }
                let recovery_snapshot = RemoteDesktopRecoverySnapshot::from_session(session)?;
                return Ok((recovery_snapshot, view));
            }
            stop_session_transports(&plugin, &session_id, session);
            session.close(&reason);
            let recovery_snapshot = RemoteDesktopRecoverySnapshot::from_session(session)?;
            Ok((recovery_snapshot, serialize_session(session)))
        })?;
    plugin.persist_recovery_snapshot(&recovery_snapshot)?;
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::test_support::{
        create_test_session, env_for, reset_store, seed_display, test_lock, test_plugin,
    };

    #[test]
    fn end_session_persists_terminal_recovery_snapshot() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-recovery-end-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&plugin),
            env.clone(),
            json!({"session_id": "rd-recovery-end", "mode": "view_only"}),
        )
        .expect("test session creates");
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns session token")
            .to_string();

        let ended = handle(
            Arc::clone(&plugin),
            env,
            json!({
                "session_id": "rd-recovery-end",
                "session_token": token,
                "reason": "test_end",
            }),
        )
        .expect("end_session succeeds");
        assert_eq!(ended["state"], json!("closed"));
        assert_eq!(ended["end_reason"], json!("test_end"));

        let snapshot = plugin
            .recovery_store()
            .load("rd-recovery-end")
            .expect("recovery snapshot load succeeds")
            .expect("end_session must write a terminal recovery snapshot");
        let snapshot = serde_json::to_value(snapshot).expect("snapshot serializes");
        assert_eq!(snapshot["session_id"], json!("rd-recovery-end"));
        assert_eq!(snapshot["lifecycle_state"], json!("closed"));
        assert!(snapshot["terminal_receipt"].is_object());
        assert!(snapshot["events"].as_array().unwrap().iter().any(|event| {
            event["event_type"] == json!("SESSION_CLOSED")
                && event["payload"]["reason"] == json!("test_end")
        }));
    }
}
