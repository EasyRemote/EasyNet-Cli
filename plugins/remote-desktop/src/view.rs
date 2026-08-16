// EasyNet CLI — remote desktop session view projection
// ====================================================
//
// File: plugins/remote-desktop/src/view.rs
// Description: JSON response projection for remote desktop sessions.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::input::{
    input_injection_available, input_policy_for_binding, INPUT_DATA_CHANNEL_LABEL,
};
use crate::daemon::plugins::remote_desktop::media::{
    backend_catalog_view, production_gate_view, sdk_contract_view,
};
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::view_device::{
    device_capabilities_view, empty_pipeline_metrics, quality_targets,
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
    let input_policy =
        input_policy_for_binding(session.input_policy().to_value(), session.target_binding());
    let media_stats = session.media_stats();
    let production_media_ready = session.production_media_ready();
    let transport_route_state = transport_view.route_state();
    let signaling = session.signaling_view(transport_route_state.clone());
    let production_readiness = production_readiness_view(session, &transport_view);
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
        "video": video.clone(),
        "input_policy": input_policy.clone(),
        "consent": session.consent_state().to_value(),
        "media_transport_ready": session.media_transport_ready(),
        "client_media_ready": session.client_media_ready(),
        "transport_epoch": session.transport_epoch(),
        "transport_state": session.transport_state(),
        "input_plane": {
            "kind": "webrtc_data_channel",
            "label": INPUT_DATA_CHANNEL_LABEL,
            "policy": input_policy,
            "input_injection_available": input_injection_available(),
        },
        "quality": quality_targets(&video),
        "media_sdk": sdk_contract_view(),
        "media_backends": backend_catalog_view(),
        "production_gate": production_gate_view(),
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

fn production_readiness_view(
    session: &RemoteDesktopSession,
    transport_view: &RemoteDesktopTransportView,
) -> Value {
    json!({
        "ready": session.production_media_ready(),
        "blocked_reason": production_readiness_blocked_reason(session),
        "target_scope_ready": session.target_scope_ready(),
        "requires_production_codec": true,
        "production_codec_negotiated": session.production_codec_negotiated(),
        "media_transport_ready": session.media_transport_ready(),
        "client_media_ready": session.client_media_ready(),
        "production_route_ready": transport_view.production_route_ready(),
        "route_state": transport_view.route_state(),
        "route_readiness_blocker": transport_view.readiness_blocker(),
    })
}

fn production_readiness_blocked_reason(session: &RemoteDesktopSession) -> Value {
    if session.production_media_ready() {
        Value::Null
    } else if !session.target_scope_ready() {
        json!("target_scope_not_ready")
    } else if !session.production_codec_negotiated() {
        json!("production_codec_not_negotiated")
    } else if !session.media_transport_ready() {
        json!("media_transport_not_ready")
    } else if !session.client_media_ready() {
        json!("client_media_not_presenting")
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
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::target::{
        RemoteAppTargetResolver, ResourceEntryTargetResolver,
    };
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
    }
}
