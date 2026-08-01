// EasyNet CLI — remote desktop ability handlers
// ==============================================
//
// File: plugins/remote-desktop/src/handlers/mod.rs
// Description: Ability-level handler boundary for builtin remote desktop.

pub(in crate::daemon::plugins::remote_desktop) mod add_ice_candidate;
pub(in crate::daemon::plugins::remote_desktop) mod attach;
pub(in crate::daemon::plugins::remote_desktop) mod create_session;
pub(in crate::daemon::plugins::remote_desktop) mod end_session;
pub(in crate::daemon::plugins::remote_desktop) mod grant_consent;
pub(in crate::daemon::plugins::remote_desktop) mod permission_status;
pub(in crate::daemon::plugins::remote_desktop) mod refresh_lease;
pub(in crate::daemon::plugins::remote_desktop) mod request_permission;
pub(in crate::daemon::plugins::remote_desktop) mod set_description;
pub(in crate::daemon::plugins::remote_desktop) mod show_session;
pub(in crate::daemon::plugins::remote_desktop) mod watch_events;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::{add_ice_candidate, create_session, end_session, set_description, watch_events};
    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::constants::MAX_ATTACH_FPS;
    use crate::daemon::plugins::remote_desktop::media::{
        REMOTE_DESKTOP_MEDIA_SDK_ID, XCAP_MACOS_RECORDER_MAX_FPS,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, test_lock, test_plugin,
    };

    #[test]
    fn create_show_signal_watch_end_round_trip() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-display");
        resources::save(&file).unwrap();

        let created = create_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        assert_eq!(created["session_id"], json!("rd-test"));
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns session_token")
            .to_string();
        assert_eq!(created["transport"]["kind"], json!("webrtc"));
        assert_eq!(created["media_transport_ready"], json!(false));
        assert_eq!(created["quality"]["target_fps"], json!(144));
        assert_eq!(created["quality"]["max_frame_queue_depth"], json!(1));
        assert_eq!(
            created["media_sdk"]["sdk_id"],
            json!(REMOTE_DESKTOP_MEDIA_SDK_ID)
        );
        let expected_device_max_fps = if cfg!(target_os = "macos") {
            MAX_ATTACH_FPS
        } else {
            XCAP_MACOS_RECORDER_MAX_FPS
        };
        assert_eq!(
            created["device_capabilities"]["max_fps"],
            json!(expected_device_max_fps)
        );
        assert_eq!(
            created["device_capabilities"]["requested_fps_ceiling"],
            json!(MAX_ATTACH_FPS)
        );

        let signaled = set_description::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-test",
                "session_token": token.clone(),
                "side": "local",
                "description": { "type": "offer", "sdp": "v=0" }
            }),
        )
        .unwrap();
        assert_eq!(signaled["state"], json!("negotiating"));
        assert!(
            signaled.get("session_token").is_none(),
            "session_token is create-only and must not appear in session views"
        );

        add_ice_candidate::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-test",
                "session_token": token.clone(),
                "candidate": { "candidate": "candidate:1" }
            }),
        )
        .unwrap();

        let frames = watch_events::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-test",
                "session_token": token.clone()
            }),
        )
        .unwrap()
        .into_snapshot();
        assert!(
            frames
                .iter()
                .any(|event| event["event_type"] == json!("ICE_CANDIDATE_ADDED")),
            "watch_events must include ICE event: {frames:?}"
        );

        let ended = end_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-test",
                "session_token": token.clone()
            }),
        )
        .unwrap();
        assert_eq!(ended["state"], json!("closed"));
        assert!(
            ended.get("session_token").is_none(),
            "terminal session views must not leak session_token"
        );
        let ended_again = end_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-test",
                "session_token": token.clone()
            }),
        )
        .unwrap();
        assert_eq!(ended_again["already_ended"], json!(true));
    }
}
