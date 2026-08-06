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
    plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<Value> {
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
                return Ok(view);
            }
            stop_session_transports(&plugin, &session_id, session);
            session.close(&reason);
            Ok(serialize_session(session))
        })
}
