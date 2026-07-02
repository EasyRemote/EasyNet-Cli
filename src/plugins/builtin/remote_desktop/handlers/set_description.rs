// EasyNet CLI — remote desktop set-description handler
// ====================================================

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::plugins::remote_desktop::constants::{
    ABILITY_SET_DESCRIPTION, REASON_INVALID_ARGUMENT, REASON_SESSION_NOT_FOUND,
};
use crate::plugins::remote_desktop::request::require_str;
use crate::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::plugins::remote_desktop::transport::{negotiate_remote_offer, RemoteOfferNegotiation};
use crate::plugins::remote_desktop::view::serialize_session;

/// Handle `remote_desktop.set_description`.
pub(in crate::plugins::builtin::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_SET_DESCRIPTION)?.to_string();
    let side = require_str(&args, "side", ABILITY_SET_DESCRIPTION)?.to_string();
    if side != "local" && side != "remote" {
        anyhow::bail!(
            "{ABILITY_SET_DESCRIPTION}: side must be local or remote; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    let description = args.get("description").cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SET_DESCRIPTION}: `description` is required; reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    let offer_sdp = if side == "remote" {
        description
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
            description
                .get("type")
                .and_then(Value::as_str)
                .map(|t| t == "offer")
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !direct_webrtc {
        return plugin
            .session_store()
            .with_sessions(|sessions| -> anyhow::Result<Value> {
                let session = sessions.get_mut(&session_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{ABILITY_SET_DESCRIPTION}: session {session_id:?} not found; reason={REASON_SESSION_NOT_FOUND}"
                    )
                })?;
                ensure_session_control_access(
                    &plugin,
                    ABILITY_SET_DESCRIPTION,
                    &env,
                    &args,
                    session,
                )?;
                session.set_description(&side, description)?;
                Ok(serialize_session(session))
            });
    }
    let offer_sdp = offer_sdp.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SET_DESCRIPTION}: remote offer SDP is required for direct WebRTC negotiation; reason={REASON_INVALID_ARGUMENT}"
        )
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
                anyhow::anyhow!(
                    "{ABILITY_SET_DESCRIPTION}: session {session_id:?} not found after direct WebRTC negotiation; reason={REASON_SESSION_NOT_FOUND}"
                )
            })?;
            Ok(serialize_session(session))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::persistence::resources::{self, ResourcesFile};
    #[cfg(not(target_os = "macos"))]
    use crate::plugins::remote_desktop::test_support::seed_window;
    use crate::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, seed_xcap_display, test_lock, test_plugin,
    };

    #[test]
    fn signaling_accepts_caller_subject_with_session_token() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-caller-subject-display");
        resources::save(&file).unwrap();

        let created = crate::plugins::remote_desktop::handlers::create_session::handle(
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

        let signaled = handle(
            Arc::clone(&plugin),
            env_for("easynet:///r/acme/user/dev"),
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

        let ice = crate::plugins::remote_desktop::handlers::add_ice_candidate::handle(
            Arc::clone(&plugin),
            env_for("easynet:///r/acme/user/dev"),
            json!({
                "session_id": "rd-caller-subject-test",
                "session_token": token,
                "candidate": { "candidate": "candidate:caller-subject" }
            }),
        )
        .unwrap();
        assert_eq!(
            ice["signaling"]["ice_candidate_count"],
            json!(1),
            "ICE signaling is session/token scoped, not resource-subject scoped"
        );
    }

    // Window/application WebRTC capture is blocked until the native selector
    // path is wired. The block is auditable and distinct from the diagnostic
    // display relay.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn remote_offer_is_blocked_when_no_webrtc_transport_backend_exists() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_window(&mut file, "remote-desktop-no-webrtc-backend");
        resources::save(&file).unwrap();

        let created = crate::plugins::remote_desktop::handlers::create_session::handle(
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
        assert_eq!(signaled["signaling"]["local_description"], Value::Null);
        assert!(
            signaled["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["event_type"] == json!("TRANSPORT_BLOCKED")),
            "transport block must be auditable: {signaled:?}"
        );
    }

    #[test]
    fn bad_remote_offer_does_not_commit_partial_session_state() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_xcap_display(&mut file, "remote-desktop-bad-offer-display");
        resources::save(&file).unwrap();

        let created = crate::plugins::remote_desktop::handlers::create_session::handle(
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

        let shown = crate::plugins::remote_desktop::handlers::show_session::handle(
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
