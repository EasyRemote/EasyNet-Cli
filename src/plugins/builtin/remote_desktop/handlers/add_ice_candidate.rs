// EasyNet CLI — remote desktop add-ice-candidate handler
// ======================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::plugins::remote_desktop::constants::{
    ABILITY_ADD_ICE_CANDIDATE, REASON_INVALID_ARGUMENT, REASON_SESSION_NOT_FOUND,
};
use crate::plugins::remote_desktop::request::require_str;
use crate::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::plugins::remote_desktop::transport::apply_remote_ice_candidate_values;
use crate::plugins::remote_desktop::view::serialize_session;

/// Handle `remote_desktop.add_ice_candidate`.
pub(in crate::plugins::builtin::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_ADD_ICE_CANDIDATE)?.to_string();
    let candidate = args.get("candidate").cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_ADD_ICE_CANDIDATE}: `candidate` is required; reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    let endpoint = plugin.endpoint(&session_id);
    let endpoint = {
        plugin
            .session_store()
            .with_sessions(|sessions| -> anyhow::Result<_> {
                let session = sessions.get_mut(&session_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{ABILITY_ADD_ICE_CANDIDATE}: session {session_id:?} not found; reason={REASON_SESSION_NOT_FOUND}"
                    )
                })?;
                ensure_session_control_access(
                    &plugin,
                    ABILITY_ADD_ICE_CANDIDATE,
                    &env,
                    &args,
                    session,
                )?;
                session.add_remote_ice_candidate(candidate.clone(), endpoint.is_some());
                Ok(endpoint)
            })?
    };
    if let Some(endpoint) = endpoint {
        apply_remote_ice_candidate_values(
            &plugin.transport_manager(),
            &endpoint.peer_connection,
            &[candidate],
        )?;
    }
    plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<Value> {
            let session = sessions.get_mut(&session_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_ADD_ICE_CANDIDATE}: session {session_id:?} not found after candidate append; reason={REASON_SESSION_NOT_FOUND}"
                )
            })?;
            ensure_session_control_access(
                &plugin,
                ABILITY_ADD_ICE_CANDIDATE,
                &env,
                &args,
                session,
            )?;
            Ok(serialize_session(session))
        })
}
