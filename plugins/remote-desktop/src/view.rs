// EasyNet CLI — remote desktop session view projection
// ====================================================
//
// File: plugins/remote-desktop/src/view.rs
// Description: JSON response projection for remote desktop sessions.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::input::{
    input_injection_available, EffectiveRemoteDesktopInputPolicy, INPUT_DATA_CHANNEL_LABEL,
};
use crate::daemon::plugins::remote_desktop::media::{
    backend_catalog_view, production_gate_view, sdk_contract_view,
};
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::view_device::{
    audio_support_view, device_capabilities_view, empty_pipeline_metrics, quality_targets,
};
use crate::daemon::plugins::remote_desktop::view_transport::RemoteDesktopTransportView;

/// Build the public session response without echoing the session token.
///
/// This is a projection of the current device-side session row using the
/// product-owned wire vocabulary. Axon carries the surrounding canonical
/// invocation and receipt without owning remote desktop lifecycle semantics.
pub(in crate::daemon::plugins::remote_desktop) fn serialize_session(
    session: &RemoteDesktopSession,
) -> Value {
    let transport_view = RemoteDesktopTransportView::from_session(session);
    let video = session.video().to_value();
    let effective_input_policy = EffectiveRemoteDesktopInputPolicy::for_target_state(
        session.input_policy(),
        session.target_snapshot(),
        session.target_binding().input_scope(),
    );
    let input_policy = effective_input_policy.to_value();
    let input_readiness = input_readiness_view(session, &effective_input_policy);
    let input_injection_ready = input_injection_available();
    let media_stats = session.media_stats();
    let production_media_ready = session.production_media_ready();
    let transport_route_state = transport_view.route_state();
    let signaling = session.signaling_view(transport_route_state.clone());
    let production_readiness = production_readiness_view(session, &transport_view);
    let audio = audio_support_view();
    let mut view = json!({
        "session_id": session.session_id(),
        "state": session.state().json_name(),
        "state_proto": session.state().wire_name(),
        "lifecycle_phase": session.lifecycle_phase().as_str(),
        "consent_phase": session.consent_phase().as_str(),
        "subject_ura": session.subject_ura(),
        "subject_type": session.subject_type().as_str(),
        "subject_display_name": session.subject_display_name(),
        "target_binding": session.target_binding().to_value(),
        "scope_audit": session.target_binding().scope_audit_value(),
        "latest_target_diagnostic": session.latest_target_diagnostic(),
        "mode": session.mode(),
        "created_at_ms": session.created_at_ms(),
        "updated_at_ms": session.updated_at_ms(),
        "lease_expires_at_ms": session.lease_expires_at_ms(),
        "end_reason": session.end_reason(),
        "terminal_receipt": session.terminal_receipt(),
        "video": video.clone(),
        "input_policy": input_policy.clone(),
        "input_readiness": input_readiness.clone(),
        "consent": session.consent_state().to_value(),
        "media_transport_ready": session.media_transport_ready(),
        "client_media_ready": session.client_media_ready(),
        "transport_epoch": session.transport_epoch(),
        "transport_state": session.transport_state(),
        "input_plane": {
            "kind": "webrtc_data_channel",
            "label": INPUT_DATA_CHANNEL_LABEL,
            "policy": input_policy,
            "readiness": input_readiness,
            "input_injection_available": input_injection_ready,
        },
        "quality": quality_targets(&video),
        "media_sdk": sdk_contract_view(),
        "media_backends": backend_catalog_view(),
        "production_gate": production_gate_view(),
        "audio": audio.clone(),
        "device_capabilities": device_capabilities_view(),
        "latest_metrics": media_stats.clone().unwrap_or_else(empty_pipeline_metrics),
        "media_stats": media_stats,
        "negotiated_codec": session.negotiated_codec(),
        "transport": transport_view.summary(session),
        "transports": transport_view.transport_list(session),
        "signaling": signaling,
        "events": session.events(),
    });
    if let Some(map) = view.as_object_mut() {
        map.insert(
            "target_tracking".to_string(),
            session.target_tracking_state(),
        );
        map.insert(
            "production_media_ready".to_string(),
            Value::Bool(production_media_ready),
        );
        map.insert("production_readiness".to_string(), production_readiness);
    }
    view
}

fn input_readiness_view(
    session: &RemoteDesktopSession,
    input_policy: &EffectiveRemoteDesktopInputPolicy,
) -> Value {
    let requested_interactive = session.mode() == "interactive";
    let pointer_enabled = input_policy.pointer_enabled();
    let keyboard_enabled = input_policy.keyboard_enabled();
    let any_input_enabled = pointer_enabled || keyboard_enabled;
    let blocked_reason = if !requested_interactive {
        Value::Null
    } else if !session.target_snapshot().input_enabled() {
        json!("target_input_not_ready")
    } else if input_policy.input_scope().as_str() == "view_only" {
        json!(session.target_binding().input_scope_reason())
    } else if let Some(reason) = session.input_runtime_block_reason() {
        json!(reason)
    } else if !input_injection_available() {
        json!("input_injection_unavailable")
    } else if !any_input_enabled {
        json!("input_policy_denied")
    } else {
        Value::Null
    };
    let interactive_ready = requested_interactive && any_input_enabled && blocked_reason.is_null();
    json!({
        "requested_mode": session.mode(),
        "effective_mode": if interactive_ready { "interactive" } else { "view_only" },
        "interactive_ready": interactive_ready,
        "blocked_reason": blocked_reason,
        "input_scope": input_policy.input_scope().as_str(),
        "pointer_enabled": pointer_enabled,
        "keyboard_enabled": keyboard_enabled,
    })
}

fn production_readiness_view(
    session: &RemoteDesktopSession,
    transport_view: &RemoteDesktopTransportView,
) -> Value {
    let video_ready = transport_view.production_ready(session);
    let media_stats = session.media_stats();
    let audio_support = audio_support_view();
    let audio_ready = media_stats
        .as_ref()
        .and_then(|stats| stats.get("audio_ready"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let audio_blocked_reason = if audio_ready {
        Value::Null
    } else {
        media_stats
            .as_ref()
            .and_then(|stats| stats.get("audio_blocker"))
            .filter(|reason| !reason.is_null())
            .cloned()
            .or_else(|| audio_support.get("blocked_reason").cloned())
            .filter(|reason| !reason.is_null())
            .unwrap_or_else(|| json!("host_audio_not_yet_ready"))
    };
    let ready = video_ready && audio_ready;
    json!({
        "ready": ready,
        "blocked_reason": production_readiness_blocked_reason(
            session,
            transport_view,
            audio_ready,
            &audio_blocked_reason,
        ),
        "target_scope_ready": session.target_scope_ready(),
        "media_scope": if audio_support["supported"] == json!(true) { "audio_video" } else { "video_only" },
        "audio_ready": audio_ready,
        "audio_blocked_reason": audio_blocked_reason,
        "requires_production_codec": true,
        "production_codec_negotiated": session.production_codec_negotiated(),
        "media_transport_ready": session.media_transport_ready(),
        "client_media_ready": session.client_media_ready(),
        "production_route_ready": transport_view.production_route_ready(),
        "route_state": transport_view.route_state(),
        "route_readiness_blocker": transport_view.readiness_blocker(),
    })
}

fn production_readiness_blocked_reason(
    session: &RemoteDesktopSession,
    transport_view: &RemoteDesktopTransportView,
    audio_ready: bool,
    audio_blocked_reason: &Value,
) -> Value {
    if !session.target_scope_ready() {
        json!("target_scope_not_ready")
    } else if !session.production_codec_negotiated() {
        json!("production_codec_not_negotiated")
    } else if !session.media_transport_ready() {
        json!("media_transport_not_ready")
    } else if !session.client_media_ready() {
        json!("client_media_not_presenting")
    } else if !transport_view.production_route_ready() {
        json!("production_route_not_ready")
    } else if !audio_ready {
        audio_blocked_reason.clone()
    } else if transport_view.production_ready(session) {
        Value::Null
    } else {
        json!("production_readiness_incomplete")
    }
}

/// Build the create-session response, where the opaque session token is
/// intentionally returned exactly once.
pub(in crate::daemon::plugins::remote_desktop) fn serialize_session_with_token(
    session: &RemoteDesktopSession,
) -> Value {
    let mut view = serialize_session(session);
    if let Some(map) = view.as_object_mut() {
        map.insert(
            "session_token".to_string(),
            json!(session.session_token_for_create_response()),
        );
    }
    view
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
    use crate::daemon::plugins::remote_desktop::input::RemoteDesktopInputPolicy;
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::{
        ResourceEntryTargetResolver, TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation;
    use crate::daemon::plugins::remote_desktop::test_support::{
        live_remote_target_metadata, test_session_init,
    };
    use crate::daemon::plugins::remote_desktop::view::serialize_session;

    #[test]
    fn session_view_projects_effective_view_only_input_scope() {
        let subject = "easynet:///r/acme/resource/window.view-only-input";
        let entry = ResourceEntry {
            resource_ura: subject.into(),
            owner_agent: "easynet:///r/acme/agent/device.dev-1.media".into(),
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "window:macos:cgwindow:10:42".into(),
            display_name: "Cursor".into(),
            metadata: live_remote_target_metadata(json!({
                "window_id": 42,
                "pid": 10,
                "app_name": "Cursor",
                "x": 100,
                "y": 200,
                "width": 800,
                "height": 600,
                "geometry_revision": 1,
            })),
            first_seen_at: "2026-06-01T00:00:00Z".into(),
        };
        let mut init = test_session_init(
            "rd-view-effective-input-policy",
            subject,
            vec!["webrtc".into()],
        );
        init.mode = "interactive".to_string();
        init.target_binding = ResourceEntryTargetResolver
            .resolve_for_session("remote_desktop.create_session", &entry, "interactive", 1)
            .expect("window binding resolves");
        let session = RemoteDesktopSession::new(init);

        let view = serialize_session(&session);

        assert_eq!(view["scope_audit"]["input_mode"], json!("view_only"));
        assert_eq!(
            view["scope_audit"]["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(view["input_policy"]["input_scope"], json!("view_only"));
        assert_eq!(view["input_policy"]["keyboard_enabled"], json!(false));
        assert_eq!(view["input_policy"]["pointer_enabled"], json!(false));
        assert_eq!(
            view["input_plane"]["policy"]["input_scope"],
            json!("view_only")
        );
        assert_eq!(view["input_plane"]["policy"], view["input_policy"]);
        assert_eq!(
            view["input_readiness"]["requested_mode"],
            json!("interactive")
        );
        assert_eq!(
            view["input_readiness"]["effective_mode"],
            json!("view_only")
        );
        assert_eq!(view["input_readiness"]["interactive_ready"], json!(false));
        assert_eq!(
            view["input_readiness"]["blocked_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(view["input_readiness"]["input_scope"], json!("view_only"));
        assert_eq!(view["input_readiness"], view["input_plane"]["readiness"]);
    }

    #[test]
    fn session_view_projects_latest_target_geometry_into_input_policy() {
        let subject = "easynet:///r/acme/resource/window.view-geometry";
        let entry = ResourceEntry {
            resource_ura: subject.into(),
            owner_agent: "easynet:///r/acme/agent/device.dev-1.media".into(),
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "window:macos:cgwindow:10:42".into(),
            display_name: "Cursor".into(),
            metadata: live_remote_target_metadata(json!({
                "window_id": 42,
                "pid": 10,
                "app_name": "Cursor",
                "x": 100,
                "y": 200,
                "width": 800,
                "height": 600,
                "geometry_revision": 1,
            })),
            first_seen_at: "2026-06-01T00:00:00Z".into(),
        };
        let mut init = test_session_init(
            "rd-view-latest-input-policy",
            subject,
            vec!["webrtc".into()],
        );
        init.mode = "interactive".to_string();
        init.target_binding = ResourceEntryTargetResolver
            .resolve_for_session("remote_desktop.create_session", &entry, "interactive", 1)
            .expect("window binding resolves");
        let mut session = RemoteDesktopSession::new(init);

        session.record_target_observation(TargetObservation::GeometryChanged {
            geometry: TargetGeometry {
                x: Some(240.0),
                y: Some(320.0),
                width: Some(1024.0),
                height: Some(768.0),
            },
            target_geometry_revision: 2,
            observed_at_ms: 100,
        });

        let view = serialize_session(&session);

        assert_eq!(
            view["target_tracking"]["target_geometry_revision"],
            json!(2)
        );
        assert_eq!(
            view["input_policy"]["pointer_target"]["target_geometry_revision"],
            view["target_tracking"]["target_geometry_revision"],
            "session view must expose the same live geometry revision to frontend input mapping as target tracking"
        );
        assert_eq!(
            view["input_policy"]["pointer_target"]["origin_x"],
            json!(240.0)
        );
        assert_eq!(
            view["input_policy"]["pointer_target"]["origin_y"],
            json!(320.0)
        );
        assert_eq!(
            view["input_policy"]["pointer_target"]["width"],
            json!(1024.0)
        );
        assert_eq!(
            view["input_policy"]["pointer_target"]["height"],
            json!(768.0)
        );
        assert_eq!(view["input_plane"]["policy"], view["input_policy"]);
    }

    #[test]
    fn session_view_blocks_input_readiness_when_target_tracking_disables_input() {
        let subject = "easynet:///r/acme/resource/display.lost-input";
        let entry = ResourceEntry {
            resource_ura: subject.into(),
            owner_agent: "easynet:///r/acme/agent/device.dev-1.media".into(),
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "display:macos:1".into(),
            display_name: "Built-in Display".into(),
            metadata: live_remote_target_metadata(json!({
                "display_id": 1,
                "primary_display": true,
                "platform": "macos",
                "backend": "macos_core_graphics",
                "geometry_revision": 1,
            })),
            first_seen_at: "2026-06-01T00:00:00Z".into(),
        };
        let mut init = test_session_init(
            "rd-view-target-input-not-ready",
            subject,
            vec!["webrtc".into()],
        );
        init.mode = "interactive".to_string();
        init.input_policy = RemoteDesktopInputPolicy::new(true, true);
        init.target_binding = ResourceEntryTargetResolver
            .resolve_for_session_with_input_consent(
                "remote_desktop.create_session",
                &entry,
                "interactive",
                1,
                true,
            )
            .expect("display binding resolves with input-control consent");
        let mut session = RemoteDesktopSession::new(init);
        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "display disappeared".into(),
            observed_at_ms: 100,
        });
        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "display still missing".into(),
            observed_at_ms: 200,
        });

        let view = serialize_session(&session);

        assert_eq!(view["target_tracking"]["input_enabled"], json!(false));
        assert_eq!(
            view["input_readiness"]["blocked_reason"],
            json!("target_input_not_ready")
        );
        assert_eq!(
            view["input_readiness"]["effective_mode"],
            json!("view_only")
        );
        assert_eq!(view["input_readiness"]["interactive_ready"], json!(false));
        assert_eq!(view["input_readiness"], view["input_plane"]["readiness"]);
    }

    #[test]
    fn session_view_projects_session_local_runtime_input_blocker() {
        let subject = "easynet:///r/acme/resource/display.runtime-input-block";
        let entry = ResourceEntry {
            resource_ura: subject.into(),
            owner_agent: "easynet:///r/acme/agent/device.dev-1.media".into(),
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "display:macos:2".into(),
            display_name: "Studio Display".into(),
            metadata: live_remote_target_metadata(json!({
                "display_id": 2,
                "primary_display": false,
                "platform": "macos",
                "backend": "macos_core_graphics",
                "geometry_revision": 1,
            })),
            first_seen_at: "2026-06-01T00:00:00Z".into(),
        };
        let mut init = test_session_init(
            "rd-view-runtime-input-block",
            subject,
            vec!["webrtc".into()],
        );
        init.mode = "interactive".to_string();
        init.input_policy = RemoteDesktopInputPolicy::new(true, true);
        init.target_binding = ResourceEntryTargetResolver
            .resolve_for_session_with_input_consent(
                "remote_desktop.create_session",
                &entry,
                "interactive",
                1,
                true,
            )
            .expect("display binding resolves with input-control consent");
        let mut session = RemoteDesktopSession::new(init);
        let epoch = TransportEpoch::new(31);

        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-view-runtime-input-block"),
        );
        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert!(session.activate_input_for_transport_epoch(epoch));
        assert!(
            session.block_input_for_runtime_permission(epoch, "accessibility_permission_denied")
        );

        let view = serialize_session(&session);

        assert_eq!(view["target_tracking"]["input_enabled"], json!(true));
        assert_eq!(view["media_transport_ready"], json!(true));
        assert_eq!(view["client_media_ready"], json!(true));
        assert_eq!(view["lifecycle_phase"], json!("media_active"));
        assert_eq!(
            view["input_readiness"]["blocked_reason"],
            json!("accessibility_permission_denied")
        );
        assert_eq!(
            view["input_readiness"]["effective_mode"],
            json!("view_only")
        );
        assert_eq!(view["input_readiness"]["interactive_ready"], json!(false));
        assert_eq!(view["input_readiness"], view["input_plane"]["readiness"]);
    }

    #[test]
    fn session_view_reports_platform_audio_product_state() {
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-view-audio-product-state",
            "easynet:///r/acme/resource/display.audio",
            vec!["webrtc".into()],
        ));

        let view = serialize_session(&session);

        assert_eq!(view["device_capabilities"]["audio"], view["audio"]);
        assert_eq!(view["production_readiness"]["audio_ready"], json!(false));
        #[cfg(target_os = "macos")]
        {
            assert_eq!(view["audio"]["supported"], json!(true));
            assert_eq!(
                view["production_readiness"]["media_scope"],
                json!("audio_video")
            );
            assert_eq!(
                view["production_readiness"]["audio_blocked_reason"],
                json!("host_audio_not_yet_ready")
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(view["audio"]["supported"], json!(false));
            assert_eq!(
                view["audio"]["blocked_reason"],
                json!("host_audio_not_implemented")
            );
            assert_eq!(
                view["production_readiness"]["media_scope"],
                json!("video_only")
            );
            assert_eq!(
                view["production_readiness"]["audio_blocked_reason"],
                json!("host_audio_not_implemented")
            );
        }
    }

    #[test]
    fn session_view_projects_terminal_receipt_only_after_close() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-view-terminal-receipt",
            "easynet:///r/acme/resource/display.terminal-receipt",
            vec!["webrtc".into()],
        ));

        let active_view = serialize_session(&session);
        assert_eq!(active_view["terminal_receipt"], json!(null));

        session.close("caller_ended");
        let terminal_view = serialize_session(&session);

        assert_eq!(
            terminal_view["terminal_receipt"]["receipt_type"],
            json!("remoteapp.session.terminal.v1")
        );
        assert_eq!(
            terminal_view["terminal_receipt"]["session_id"],
            json!("rd-view-terminal-receipt")
        );
        assert_eq!(
            terminal_view["terminal_receipt"]["reason_code"],
            json!("caller_ended")
        );
        assert_eq!(terminal_view["terminal_receipt"]["terminal"], json!(true));
    }
}
