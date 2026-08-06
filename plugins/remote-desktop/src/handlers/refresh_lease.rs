// EasyNet CLI — remote desktop refresh-lease handler
// ==================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_REFRESH_LEASE;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::{parse_lease_ttl_ms, require_str};
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::view::serialize_session;

/// Handle `remote_desktop.refresh_lease`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_REFRESH_LEASE)?.to_string();
    let lease_ttl_ms = parse_lease_ttl_ms(&args)?;
    let (lease_expires_at_ms, view) =
        plugin
            .session_store()
            .with_sessions(|sessions| -> anyhow::Result<_> {
                let session = sessions.get_mut(&session_id).ok_or_else(|| {
                    RemoteDesktopError::SessionNotFound {
                        ability: ABILITY_REFRESH_LEASE,
                        session_id: session_id.clone(),
                    }
                })?;
                ensure_session_control_access(
                    &plugin,
                    ABILITY_REFRESH_LEASE,
                    &env,
                    &args,
                    session,
                )?;
                let now = now_ms();
                let lease_expires_at_ms = session.refresh_lease(now, lease_ttl_ms);
                let view = serialize_session(session);
                Ok((lease_expires_at_ms, view))
            })?;
    RemoteDesktopPlugin::schedule_session_lease(&plugin, session_id, lease_expires_at_ms);
    Ok(view)
}
