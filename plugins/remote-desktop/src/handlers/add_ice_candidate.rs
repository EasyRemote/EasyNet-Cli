// EasyNet CLI — remote desktop add-ice-candidate handler
// ======================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_ADD_ICE_CANDIDATE;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::sdp::{
    remote_ice_candidate_inits, validate_ice_candidate_size,
};
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::session_signaling::RemoteDesktopSignalingError;
use crate::daemon::plugins::remote_desktop::transport::apply_remote_ice_candidate_values;
use crate::daemon::plugins::remote_desktop::view::serialize_session;

enum RemoteIceAdmission {
    Reserved,
    Committed(Value),
}

/// Handle `remote_desktop.add_ice_candidate`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_ADD_ICE_CANDIDATE)?.to_string();
    let candidate_ref =
        args.get("candidate")
            .ok_or_else(|| RemoteDesktopError::InvalidArgument {
                ability: ABILITY_ADD_ICE_CANDIDATE,
                detail: "`candidate` is required".to_string(),
            })?;
    validate_ice_candidate_size(candidate_ref).map_err(|err| {
        RemoteDesktopError::InvalidArgument {
            ability: ABILITY_ADD_ICE_CANDIDATE,
            detail: err.to_string(),
        }
    })?;
    remote_ice_candidate_inits(candidate_ref).map_err(|err| {
        RemoteDesktopError::InvalidArgument {
            ability: ABILITY_ADD_ICE_CANDIDATE,
            detail: format!("invalid ICE candidate: {err}"),
        }
    })?;
    let candidate = candidate_ref.clone();
    for attempt in 0..2 {
        let endpoint = plugin.endpoint(&session_id);
        let admission = plugin.session_store().with_sessions(
            |sessions| -> anyhow::Result<Option<RemoteIceAdmission>> {
                let session = sessions.get_mut(&session_id).ok_or_else(|| {
                    RemoteDesktopError::SessionNotFound {
                        ability: ABILITY_ADD_ICE_CANDIDATE,
                        session_id: session_id.clone(),
                    }
                })?;
                ensure_session_control_access(
                    &plugin,
                    ABILITY_ADD_ICE_CANDIDATE,
                    &env,
                    &args,
                    session,
                )?;
                let current = plugin.endpoint(&session_id);
                let stable = match (&endpoint, &current) {
                    (None, None) => true,
                    (Some(applied), Some(active)) => applied.epoch == active.epoch,
                    _ => false,
                };
                if !stable
                    || endpoint.as_ref().is_some_and(|applied| {
                        session.transport_epoch() != Some(applied.epoch.value())
                    })
                {
                    return Ok(None);
                }
                if endpoint.is_some() {
                    let reserved = session
                        .reserve_remote_ice_candidate_slot()
                        .map_err(map_signaling_admission_error)?;
                    if !reserved {
                        return Ok(Some(RemoteIceAdmission::Committed(serialize_session(
                            session,
                        ))));
                    }
                    return Ok(Some(RemoteIceAdmission::Reserved));
                }
                session
                    .add_remote_ice_candidate(candidate.clone(), "pending", None)
                    .map_err(map_signaling_admission_error)?;
                Ok(Some(RemoteIceAdmission::Committed(serialize_session(
                    session,
                ))))
            },
        )?;

        let Some(admission) = admission else {
            if attempt == 1 {
                anyhow::bail!(
                    "{ABILITY_ADD_ICE_CANDIDATE}: endpoint changed during candidate admission"
                );
            }
            continue;
        };
        if let RemoteIceAdmission::Committed(view) = admission {
            return Ok(view);
        }

        let endpoint = endpoint.expect("reserved admission requires an active endpoint");
        if let Err(err) = apply_remote_ice_candidate_values(
            &plugin.transport_manager(),
            &endpoint.peer_connection,
            std::slice::from_ref(&candidate),
        ) {
            release_reserved_remote_ice_candidate(&plugin, &session_id);
            return Err(err);
        }
        let committed =
            plugin
                .session_store()
                .with_sessions(|sessions| -> anyhow::Result<Option<Value>> {
                    let session = sessions.get_mut(&session_id).ok_or_else(|| {
                        RemoteDesktopError::SessionNotFound {
                            ability: ABILITY_ADD_ICE_CANDIDATE,
                            session_id: session_id.clone(),
                        }
                    })?;
                    if let Err(err) = ensure_session_control_access(
                        &plugin,
                        ABILITY_ADD_ICE_CANDIDATE,
                        &env,
                        &args,
                        session,
                    ) {
                        return Err(err);
                    }
                    let current = plugin.endpoint(&session_id);
                    let stable = current
                        .as_ref()
                        .is_some_and(|active| endpoint.epoch == active.epoch);
                    if !stable || session.transport_epoch() != Some(endpoint.epoch.value()) {
                        session.release_remote_ice_candidate_slot();
                        return Ok(None);
                    }
                    session
                        .commit_reserved_remote_ice_candidate(
                            candidate.clone(),
                            "applied",
                            Some(endpoint.epoch),
                        )
                        .map_err(map_signaling_admission_error)?;
                    Ok(Some(serialize_session(session)))
                });
        let committed = match committed {
            Ok(committed) => committed,
            Err(err) => {
                release_reserved_remote_ice_candidate(&plugin, &session_id);
                return Err(err);
            }
        };
        if let Some(view) = committed {
            return Ok(view);
        }
        if attempt == 1 {
            anyhow::bail!(
                "{ABILITY_ADD_ICE_CANDIDATE}: endpoint changed during candidate application"
            );
        }
    }
    unreachable!("bounded candidate application loop returns or errors")
}

fn map_signaling_admission_error(error: anyhow::Error) -> RemoteDesktopError {
    if let Some(RemoteDesktopSignalingError::IceCandidateLimitExceeded { .. }) =
        error.downcast_ref::<RemoteDesktopSignalingError>()
    {
        return RemoteDesktopError::ResourceExhausted {
            ability: ABILITY_ADD_ICE_CANDIDATE,
            detail: error.to_string(),
        };
    }
    RemoteDesktopError::InvalidArgument {
        ability: ABILITY_ADD_ICE_CANDIDATE,
        detail: error.to_string(),
    }
}

fn release_reserved_remote_ice_candidate(plugin: &RemoteDesktopPlugin, session_id: &str) {
    let _ = plugin.session_store().with_sessions(|sessions| {
        if let Some(session) = sessions.get_mut(session_id) {
            session.release_remote_ice_candidate_slot();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::{
        MAX_REMOTE_ICE_CANDIDATES, REASON_INVALID_ARGUMENT, REASON_RESOURCE_EXHAUSTED,
        TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, test_lock, test_plugin, test_session_init,
    };

    fn insert_test_session(plugin: &RemoteDesktopPlugin, session_id: &str, subject: &str) {
        plugin.session_store().with_sessions(|sessions| {
            let mut init =
                test_session_init(session_id, subject, vec![TRANSPORT_WEBRTC.to_string()]);
            init.lease_ttl_ms = 60_000;
            sessions.insert(session_id.to_string(), RemoteDesktopSession::new(init));
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

    #[test]
    fn add_ice_candidate_rejects_flood_after_bounded_remote_candidate_cap() {
        const FLOOD_CANDIDATES: usize = 10_001;
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let subject = "easynet:///r/acme/resource/display.ice-flood";
        insert_test_session(&plugin, "rd-ice-flood", subject);

        let mut accepted = 0;
        let mut rejected = 0;
        let mut overflow_error = None;

        for index in 0..FLOOD_CANDIDATES {
            let result = handle(
                Arc::clone(&plugin),
                env_for(subject),
                json!({
                    "session_id": "rd-ice-flood",
                    "session_token": "token",
                    "candidate": {
                        "candidate": format!("candidate:{index} 1 UDP 2122252543 127.0.0.1 {} typ host", 40000 + index),
                        "sdpMid": "0",
                        "sdpMLineIndex": 0
                    }
                }),
            );
            match result {
                Ok(view) => {
                    accepted += 1;
                    assert_eq!(
                        view["signaling"]["ice_candidate_count"],
                        json!(accepted),
                        "serialized session view should expose bounded candidate count"
                    );
                }
                Err(err) => {
                    rejected += 1;
                    overflow_error = Some(err.to_string());
                }
            }
        }

        assert_eq!(accepted, MAX_REMOTE_ICE_CANDIDATES);
        assert_eq!(rejected, FLOOD_CANDIDATES - MAX_REMOTE_ICE_CANDIDATES);
        let err = overflow_error.expect("candidate over cap must fail closed");
        assert!(
            err.contains("remote ICE candidate cap exceeded"),
            "got {err}"
        );
        assert!(err.contains(REASON_RESOURCE_EXHAUSTED), "got {err}");

        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get("rd-ice-flood").unwrap();
            assert_eq!(
                session.remote_ice_candidates().len(),
                MAX_REMOTE_ICE_CANDIDATES,
                "serialized session view must remain bounded at the remote candidate cap"
            );
        });
    }
}
