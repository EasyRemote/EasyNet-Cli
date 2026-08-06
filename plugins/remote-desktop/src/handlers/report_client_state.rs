// EasyNet CLI — remote desktop client-media report handler
// =========================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_REPORT_CLIENT_STATE;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::view::serialize_session;

pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_REPORT_CLIENT_STATE)?;
    let state = require_str(&args, "state", ABILITY_REPORT_CLIENT_STATE)?;
    if !matches!(state, "presenting" | "stalled" | "detached") {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: "state must be presenting, stalled, or detached".to_string(),
        }
        .into());
    }
    let epoch = args
        .get("transport_epoch")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: "positive transport_epoch is required".to_string(),
        })?;
    plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<Value> {
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_REPORT_CLIENT_STATE,
                    session_id: session_id.to_string(),
                }
            })?;
            ensure_session_control_access(
                &plugin,
                ABILITY_REPORT_CLIENT_STATE,
                &env,
                &args,
                session,
            )?;
            if !session.report_client_media_state(TransportEpoch::new(epoch), state) {
                return Err(RemoteDesktopError::TransportEpochMismatch {
                    ability: ABILITY_REPORT_CLIENT_STATE,
                    epoch,
                }
                .into());
            }
            Ok(serialize_session(session))
        })
}
