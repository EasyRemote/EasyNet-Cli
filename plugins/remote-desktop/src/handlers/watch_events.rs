// EasyNet CLI — remote desktop watch-events handler
// =================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::{EnvelopeContext, StreamSource};
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_WATCH_EVENTS, REASON_SESSION_NOT_FOUND,
};
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;

/// Handle `remote_desktop.watch_events`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<StreamSource> {
    let session_id = require_str(&args, "session_id", ABILITY_WATCH_EVENTS)?;
    let from_sequence = args
        .get("from_sequence")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<StreamSource> {
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_WATCH_EVENTS}: session {session_id:?} not found; reason={REASON_SESSION_NOT_FOUND}"
                )
            })?;
            ensure_session_control_access(&plugin, ABILITY_WATCH_EVENTS, &env, &args, session)?;
            let live_rx = session.subscribe_events();
            let events = session
                .events()
                .iter()
                .filter(|event| {
                    event
                        .get("sequence")
                        .and_then(Value::as_u64)
                        .map(|sequence| sequence > from_sequence)
                        .unwrap_or(true)
                })
                .cloned()
                .collect();
            Ok(StreamSource::SnapshotThenLive(events, live_rx))
        })
}
