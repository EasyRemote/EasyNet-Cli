// EasyNet CLI — remote desktop set-description handler
// ====================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_SET_DESCRIPTION;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::sdp::{
    validate_remote_offer_sdp, validate_signaling_description_size,
};
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::transport::{
    negotiate_remote_offer, RemoteOfferNegotiation,
};

/// Handle `remote_desktop.set_description`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_SET_DESCRIPTION)?.to_string();
    let side = require_str(&args, "side", ABILITY_SET_DESCRIPTION)?.to_string();
    if side != "local" && side != "remote" {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_SET_DESCRIPTION,
            detail: "side must be local or remote".to_string(),
        }
        .into());
    }
    let description_ref =
        args.get("description")
            .ok_or_else(|| RemoteDesktopError::InvalidArgument {
                ability: ABILITY_SET_DESCRIPTION,
                detail: "`description` is required".to_string(),
            })?;
    validate_signaling_description_size(description_ref).map_err(|err| {
        RemoteDesktopError::InvalidArgument {
            ability: ABILITY_SET_DESCRIPTION,
            detail: err.to_string(),
        }
    })?;
    let offer_sdp = if side == "remote" {
        description_ref
            .get("sdp")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    } else {
        None
    };
    let direct_webrtc = offer_sdp
        .as_ref()
        .map(|_| {
            description_ref
                .get("type")
                .and_then(Value::as_str)
                .map(|t| t == "offer")
                .unwrap_or(false)
        })
        .unwrap_or(false);
    let description = description_ref.clone();

    if !direct_webrtc {
        return plugin
            .session_store()
            .with_sessions(|sessions| -> anyhow::Result<Value> {
                let session = sessions.get_mut(&session_id).ok_or_else(|| {
                    RemoteDesktopError::SessionNotFound {
                        ability: ABILITY_SET_DESCRIPTION,
                        session_id: session_id.clone(),
                    }
                })?;
                ensure_session_control_access(
                    &plugin,
                    ABILITY_SET_DESCRIPTION,
                    &env,
                    &args,
                    session,
                )?;
                session.set_description(&side, description)?;
                Ok(plugin.session_view(session))
            });
    }
    let offer_sdp = offer_sdp.ok_or_else(|| RemoteDesktopError::InvalidArgument {
        ability: ABILITY_SET_DESCRIPTION,
        detail: "remote offer SDP is required for direct WebRTC negotiation".to_string(),
    })?;
    validate_remote_offer_sdp(&offer_sdp).map_err(|err| RemoteDesktopError::InvalidArgument {
        ability: ABILITY_SET_DESCRIPTION,
        detail: err.to_string(),
    })?;

    let session_id = negotiate_remote_offer(RemoteOfferNegotiation {
        plugin: Arc::clone(&plugin),
        access_env: env,
        access_args: args,
        session_id,
        side,
        description,
        offer_sdp,
    })?;
    plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<Value> {
            let session = sessions.get_mut(&session_id).ok_or_else(|| {
                RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_SET_DESCRIPTION,
                    session_id: session_id.clone(),
                }
            })?;
            Ok(plugin.session_view(session))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::constants::{
        MAX_SIGNALING_DESCRIPTION_BYTES, REASON_INVALID_ARGUMENT,
    };
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    #[cfg(not(target_os = "macos"))]
    use crate::daemon::plugins::remote_desktop::test_support::seed_window;
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, seed_xcap_display, test_lock, test_plugin,
        test_session_init,
    };

    #[test]
    fn signaling_requires_bound_resource_subject() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-caller-subject-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-caller-subject-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap();

        let error = handle(
            Arc::clone(&plugin),
            env_for("easynet:///r/acme/user/dev"),
            json!({
                "session_id": "rd-caller-subject-test",
                "session_token": token,
                "side": "local",
                "description": { "type": "answer", "sdp": "v=0" }
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not match session subject"));

        let signaled = handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-caller-subject-test",
                "session_token": token,
                "side": "local",
                "description": { "type": "answer", "sdp": "v=0" }
            }),
        )
        .unwrap();

        assert_eq!(signaled["session_id"], json!("rd-caller-subject-test"));
        assert_eq!(signaled["state"], json!("negotiating"));
        plugin.session_store().with_sessions(|sessions| {
            assert!(sessions
                .get_mut("rd-caller-subject-test")
                .unwrap()
                .begin_webrtc_negotiation(
                    crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch::new(1),
                ));
        });

        let ice = crate::daemon::plugins::remote_desktop::handlers::add_ice_candidate::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-caller-subject-test",
                "session_token": token,
                "transport_epoch": 1,
                "candidate": { "candidate": "candidate:caller-subject" }
            }),
        )
        .unwrap();
        assert_eq!(
            ice["signaling"]["ice_candidate_count"],
            json!(1),
            "ICE signaling stays bound to the session resource subject"
        );
    }

    #[test]
    fn set_description_rejects_oversized_sdp_before_session_mutation() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let subject = "easynet:///r/acme/resource/display.sdp-flood";
        let session = RemoteDesktopSession::new(test_session_init("rd-sdp-flood", subject, vec![]));
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert("rd-sdp-flood".to_string(), session);
        });
        let oversized_sdp = format!(
            "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n{}",
            "a=x\r\n".repeat((MAX_SIGNALING_DESCRIPTION_BYTES / 5) + 2)
        );
        let err = handle(
            Arc::clone(&plugin),
            env_for(subject),
            json!({
                "session_id": "rd-sdp-flood",
                "session_token": "token",
                "side": "remote",
                "description": { "type": "offer", "sdp": oversized_sdp }
            }),
        )
        .expect_err("oversized SDP must fail before WebRTC setup or session mutation")
        .to_string();

        assert!(err.contains("exceeds"), "got {err}");
        assert!(err.contains(REASON_INVALID_ARGUMENT), "got {err}");
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get("rd-sdp-flood").unwrap();
            assert!(
                session.signaling_view(Value::Null)["remote_description"].is_null(),
                "oversized SDP must not mutate existing session signaling"
            );
        });
    }

    // Window/application WebRTC capture is blocked until the native selector
    // path is wired. The block is auditable and distinct from the diagnostic
    // display relay.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn remote_offer_backend_gate_blocks_without_committing_signaling() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_window(&mut file, "remote-desktop-no-webrtc-backend");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-no-webrtc-backend",
                "video": { "max_width": 320, "max_height": 180, "max_fps": 144 },
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap();
        let signaled = handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-no-webrtc-backend",
                "session_token": token,
                "side": "remote",
                "description": { "type": "offer", "sdp": "v=0\r\n" }
            }),
        )
        .unwrap();

        assert_eq!(
            signaled["signaling"]["webrtc_error"],
            json!("webrtc_transport_backend_unavailable")
        );
        assert_eq!(
            signaled["transport"]["unavailable_reason"],
            json!("webrtc_transport_backend_unavailable")
        );
        assert_eq!(signaled["signaling"]["remote_description"], Value::Null);
        assert_eq!(signaled["signaling"]["local_description"], Value::Null);
        assert!(
            signaled["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["event_type"] == json!("TRANSPORT_BLOCKED")),
            "transport block must be auditable: {signaled:?}"
        );
        assert!(
            signaled["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|event| event["event_type"] != json!("DESCRIPTION_SET")),
            "transport backend gate must not partially commit signaling: {signaled:?}"
        );
    }

    #[test]
    fn bad_remote_offer_does_not_commit_partial_session_state() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_xcap_display(&mut file, "remote-desktop-bad-offer-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-bad-offer",
                "video": { "max_width": 320, "max_height": 180, "max_fps": 60 },
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap();

        let err = handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-bad-offer",
                "session_token": token,
                "side": "remote",
                "description": { "type": "offer", "sdp": "not an sdp" }
            }),
        )
        .expect_err("invalid SDP must fail before mutating session state");
        assert!(
            err.to_string().contains("sdp") || err.to_string().contains("SDP"),
            "error should describe SDP failure, got: {err}"
        );

        let shown = crate::daemon::plugins::remote_desktop::handlers::show_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-bad-offer", "session_token": token}),
        )
        .unwrap();
        assert_eq!(shown["signaling"]["remote_description"], Value::Null);
        assert_eq!(shown["signaling"]["local_description"], Value::Null);
        assert!(
            shown["events"]
                .as_array()
                .expect("events")
                .iter()
                .all(|event| event["event_type"] != json!("DESCRIPTION_SET")),
            "failed remote offer must not emit DESCRIPTION_SET: {:?}",
            shown["events"]
        );
    }
}
