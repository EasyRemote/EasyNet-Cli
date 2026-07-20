// EasyNet CLI — remote desktop add-ice-candidate handler
// ======================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ADD_ICE_CANDIDATE, REASON_INVALID_ARGUMENT, REASON_SESSION_NOT_FOUND,
};
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::sdp::remote_ice_candidate_inits;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::transport::apply_remote_ice_candidate_values;
use crate::daemon::plugins::remote_desktop::view::serialize_session;

/// Handle `remote_desktop.add_ice_candidate`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
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
    remote_ice_candidate_inits(&candidate).map_err(|err| {
        anyhow::anyhow!(
            "{ABILITY_ADD_ICE_CANDIDATE}: invalid ICE candidate; reason={REASON_INVALID_ARGUMENT}; detail={err}"
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::TRANSPORT_WEBRTC;
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, test_lock, test_plugin, test_session_init,
    };

    fn insert_test_session(plugin: &RemoteDesktopPlugin, session_id: &str, subject: &str) {
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(
                session_id.to_string(),
                RemoteDesktopSession::new(test_session_init(
                    session_id,
                    subject,
                    vec![TRANSPORT_WEBRTC.to_string()],
                )),
            );
        });
    }

    #[test]
    fn add_ice_candidate_rejects_malformed_candidate_before_storing() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let subject = "easynet:///r/acme/resource/display.ice";
        insert_test_session(&plugin, "rd-ice-schema", subject);

        let err = handle(
            Arc::clone(&plugin),
            env_for(subject),
            json!({
                "session_id": "rd-ice-schema",
                "session_token": "token",
                "candidate": {}
            }),
        )
        .expect_err("malformed candidate must fail before session projection")
        .to_string();
        assert!(err.contains("invalid ICE candidate"), "got {err}");
        assert!(err.contains(REASON_INVALID_ARGUMENT), "got {err}");

        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get("rd-ice-schema").unwrap();
            assert!(
                session.remote_ice_candidates().is_empty(),
                "malformed remote candidate must not enter session signaling"
            );
        });
    }
}
