// EasyNet CLI — remote desktop transport view projection
// ======================================================
//
// File: plugins/remote-desktop/src/view_transport.rs
// Description: JSON transport projections for remote desktop session views.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::constants::{
    direct_webrtc_endpoint_ura, TRANSPORT_INVOKE_BIDI, TRANSPORT_PREVIEW_STREAM, TRANSPORT_WEBRTC,
};
use crate::daemon::plugins::remote_desktop::input::INPUT_DATA_CHANNEL_LABEL;
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;

/// Transport view facts derived from one session row.
///
/// This type is a DTO helper only. It does not decide session lifecycle,
/// mutate signaling state, or select media backends.
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTransportView {
    endpoint_ura: Value,
    unavailable_reason: Value,
    route_state: Value,
    message: &'static str,
}

impl RemoteDesktopTransportView {
    /// Derive stable transport view facts from a session.
    pub(in crate::daemon::plugins::remote_desktop) fn from_session(
        session: &RemoteDesktopSession,
    ) -> Self {
        let endpoint_ura = direct_endpoint_ura(session);
        let route_state_projection = CandidateRouteState::from_session(session);
        let unavailable_reason = transport_unavailable_reason(session, &route_state_projection);
        let route_state = transport_route_state(&route_state_projection);
        let message = transport_message(session);
        Self {
            endpoint_ura,
            unavailable_reason,
            route_state,
            message,
        }
    }

    /// Return the typed route-readiness projection.
    pub(in crate::daemon::plugins::remote_desktop) fn route_state(&self) -> Value {
        self.route_state.clone()
    }

    /// Build the canonical transport object.
    pub(in crate::daemon::plugins::remote_desktop) fn summary(
        &self,
        session: &RemoteDesktopSession,
    ) -> Value {
        json!({
            "kind": TRANSPORT_WEBRTC,
            "primary_transport": TRANSPORT_WEBRTC,
            "primary_ready": session.media_transport_ready(),
            "production_ready": session.production_media_ready(),
            "preferred": session.transport_preferences(),
            "endpoint_ura": self.endpoint_ura.clone(),
            "preview_ability": "screen.subscribe",
            "message": self.message,
            "unavailable_reason": self.unavailable_reason.clone(),
            "route_state": self.route_state.clone(),
            "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
            "required_runtime": ["os_capture_stream", "video_encoder", "webrtc_peer_connection", "data_channel_input"]
        })
    }

    /// Build the ordered transport capability list.
    pub(in crate::daemon::plugins::remote_desktop) fn transport_list(
        &self,
        session: &RemoteDesktopSession,
    ) -> Value {
        json!([
            {
                "transport": TRANSPORT_WEBRTC,
                "transport_proto": "REMOTE_DESKTOP_TRANSPORT_WEBRTC",
                "ready": session.media_transport_ready(),
                "production_ready": session.production_media_ready(),
                "endpoint_ura": self.endpoint_ura.clone(),
                "metadata": {
                    "role": "primary",
                    "production_ready": session.production_media_ready(),
                    "signaling_plane": "axon_signed_invocation",
                    "media_plane": "rtp_srtp",
                    "input_plane": "webrtc_data_channel",
                    "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
                    "unavailable_reason": self.unavailable_reason.clone(),
                    "route_state": self.route_state.clone()
                },
            },
            {
                "transport": TRANSPORT_INVOKE_BIDI,
                "transport_proto": "REMOTE_DESKTOP_TRANSPORT_INVOKE_BIDI",
                "ready": session.preview_attached(),
                "endpoint_ura": null,
                "metadata": {
                    "role": "diagnostic_transport",
                    "diagnostic_only": "true",
                    "media_plane": "metadata_json_plus_binary_preview",
                    "drop_stale_frames": "true",
                },
            },
            {
                "transport": TRANSPORT_PREVIEW_STREAM,
                "transport_proto": "REMOTE_DESKTOP_TRANSPORT_PREVIEW_STREAM",
                "ready": false,
                "endpoint_ura": null,
                "metadata": {
                    "role": "debug_preview",
                    "diagnostic_only": "true",
                    "media_plane": "jpeg_preview",
                },
            }
        ])
    }
}

fn transport_route_state(route_state: &CandidateRouteState) -> Value {
    route_state.to_value()
}

#[derive(Debug, Default)]
struct CandidateRouteState {
    host_candidate: bool,
    stun_srflx: bool,
    turn_relay: bool,
    easynet_relay: bool,
    failed: bool,
}

impl CandidateRouteState {
    fn from_session(session: &RemoteDesktopSession) -> Self {
        let mut route_state = CandidateRouteState::default();
        for candidate in session
            .local_ice_candidates()
            .into_iter()
            .chain(session.remote_ice_candidates())
        {
            route_state.observe_candidate(&candidate);
        }
        route_state.failed = session.webrtc_error().is_some()
            || session.webrtc_ice_state() == Some("failed")
            || session.webrtc_peer_state() == Some("failed");
        route_state
    }

    fn observe_candidate(&mut self, candidate: &Value) {
        let Some(candidate_text) = candidate.get("candidate").and_then(Value::as_str) else {
            return;
        };
        if candidate_text.trim().is_empty() {
            return;
        }
        if candidate_type_is(candidate_text, "host") {
            self.host_candidate = true;
        }
        if candidate_type_is(candidate_text, "srflx") {
            self.stun_srflx = true;
        }
        if candidate_type_is(candidate_text, "relay") {
            if candidate_mentions_easynet(candidate) {
                self.easynet_relay = true;
            } else {
                self.turn_relay = true;
            }
        }
    }

    fn to_value(&self) -> Value {
        let nat_traversal_ready = self.nat_traversal_ready();
        let relay_ready = self.relay_ready();
        json!({
            "host_candidate": self.host_candidate,
            "stun_srflx": self.stun_srflx,
            "turn_relay": self.turn_relay,
            "easynet_relay": self.easynet_relay,
            "failed": self.failed,
            "host_only": self.host_candidate && !nat_traversal_ready,
            "nat_traversal_ready": nat_traversal_ready,
            "relay_ready": relay_ready,
            "route_class": self.route_class(),
        })
    }

    fn route_class(&self) -> &'static str {
        if self.failed {
            "failed"
        } else if self.easynet_relay {
            "easynet_relay"
        } else if self.turn_relay {
            "turn_relay"
        } else if self.stun_srflx {
            "stun_srflx"
        } else if self.host_candidate {
            "host_only"
        } else {
            "none"
        }
    }

    fn has_candidate(&self) -> bool {
        self.host_candidate || self.stun_srflx || self.turn_relay || self.easynet_relay
    }

    fn host_only(&self) -> bool {
        self.host_candidate && !self.nat_traversal_ready()
    }

    fn nat_traversal_ready(&self) -> bool {
        self.stun_srflx || self.turn_relay || self.easynet_relay
    }

    fn relay_ready(&self) -> bool {
        self.turn_relay || self.easynet_relay
    }
}

fn candidate_type_is(candidate_text: &str, candidate_type: &str) -> bool {
    let mut previous = None;
    for token in candidate_text.split_whitespace() {
        if previous == Some("typ") && token == candidate_type {
            return true;
        }
        previous = Some(token);
    }
    false
}

fn candidate_mentions_easynet(candidate: &Value) -> bool {
    candidate
        .get("relay_type")
        .and_then(Value::as_str)
        .is_some_and(|relay_type| relay_type == "easynet")
        || candidate
            .get("relay")
            .and_then(Value::as_str)
            .is_some_and(|relay| relay == "easynet")
        || candidate
            .get("easynet_relay")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || candidate
            .get("candidate")
            .and_then(Value::as_str)
            .is_some_and(|candidate_text| candidate_text.contains("easynet"))
}

fn direct_endpoint_ura(session: &RemoteDesktopSession) -> Value {
    if session.local_description().is_some() {
        json!(direct_webrtc_endpoint_ura(session.session_id()))
    } else {
        Value::Null
    }
}

fn transport_unavailable_reason(
    session: &RemoteDesktopSession,
    route_state: &CandidateRouteState,
) -> Value {
    if session.media_transport_ready() {
        Value::Null
    } else if let Some(error) = session.webrtc_error() {
        json!(error)
    } else if session.webrtc_ice_state() == Some("failed") {
        json!("webrtc_ice_failed")
    } else if session.webrtc_peer_state() == Some("connected") {
        json!("webrtc_media_first_frame_pending")
    } else if route_state.host_only() {
        json!("host_only_no_nat_or_relay")
    } else if route_state.has_candidate() && !route_state.relay_ready() {
        json!("relay_unavailable")
    } else if session.local_description().is_some() {
        json!("webrtc_ice_connecting")
    } else {
        json!("webrtc_offer_required")
    }
}

fn transport_message(session: &RemoteDesktopSession) -> &'static str {
    if session.media_transport_ready() {
        "Direct device-side WebRTC endpoint is ready; InvokeBidi and preview_stream remain diagnostic transports."
    } else if session.webrtc_error() == Some("native_media_plugin_required") {
        "Direct WebRTC RTP/SRTP is blocked until a native capture/encode plugin is installed; InvokeBidi remains an explicit diagnostic transport."
    } else if session.webrtc_error() == Some("native_media_pipeline_failed") {
        "Native ScreenCaptureKit/VideoToolbox media pipeline failed before producing frames; check the session failure event for the platform error."
    } else if session.webrtc_error() == Some("webrtc_transport_backend_unavailable") {
        "Direct WebRTC RTP/SRTP is blocked because this capture subject has no available device-side WebRTC backend; InvokeBidi remains an explicit diagnostic transport."
    } else if session.webrtc_peer_state() == Some("connected") {
        "Direct device-side WebRTC is connected and waiting for the first encoded media frame."
    } else if session.local_description().is_some() {
        "Direct device-side WebRTC endpoint is negotiating ICE/DTLS; InvokeBidi and preview_stream remain diagnostic-only transports."
    } else {
        "WebRTC endpoint requires a browser SDP offer; InvokeBidi and preview_stream are diagnostic-only transports until negotiation completes."
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RemoteDesktopTransportView;
    use crate::daemon::plugins::remote_desktop::constants::{
        direct_webrtc_endpoint_ura, TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

    #[test]
    fn peer_connected_without_media_reports_first_frame_pending() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-first-frame-pending",
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        session.begin_webrtc_negotiation(TransportEpoch::new(1));
        session.set_local_webrtc_answer(
            TransportEpoch::new(1),
            json!({ "type": "answer", "sdp": "v=0" }),
            "native",
            true,
            direct_webrtc_endpoint_ura("rd-first-frame-pending"),
        );
        session.record_webrtc_diagnostic(
            "PEER_CONNECTION_STATE_CHANGED",
            None,
            json!({ "peer_connection_state": "connected" }),
        );

        let view = RemoteDesktopTransportView::from_session(&session);
        let summary = view.summary(&session);

        assert_eq!(
            summary["unavailable_reason"],
            json!("webrtc_media_first_frame_pending")
        );
        assert_eq!(summary["primary_ready"], json!(false));
    }

    #[test]
    fn host_only_candidates_are_not_reported_as_nat_or_relay_ready() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-host-only-route",
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        session.begin_webrtc_negotiation(TransportEpoch::new(1));
        session.set_local_webrtc_answer(
            TransportEpoch::new(1),
            json!({ "type": "answer", "sdp": "v=0" }),
            "native",
            true,
            direct_webrtc_endpoint_ura("rd-host-only-route"),
        );
        session
            .record_local_ice_candidate(json!({
                "candidate": "candidate:1 1 UDP 2122252543 127.0.0.1 50000 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("local host candidate records");

        let view = RemoteDesktopTransportView::from_session(&session);
        let summary = view.summary(&session);

        assert_eq!(summary["route_state"]["host_candidate"], json!(true));
        assert_eq!(summary["route_state"]["host_only"], json!(true));
        assert_eq!(summary["route_state"]["stun_srflx"], json!(false));
        assert_eq!(summary["route_state"]["turn_relay"], json!(false));
        assert_eq!(summary["route_state"]["easynet_relay"], json!(false));
        assert_eq!(summary["route_state"]["nat_traversal_ready"], json!(false));
        assert_eq!(summary["route_state"]["relay_ready"], json!(false));
        assert_eq!(summary["route_state"]["route_class"], json!("host_only"));
        assert_eq!(
            summary["unavailable_reason"],
            json!("host_only_no_nat_or_relay")
        );
    }

    #[test]
    fn srflx_without_relay_reports_typed_relay_unavailable_reason() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-srflx-only-route",
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        session.begin_webrtc_negotiation(TransportEpoch::new(1));
        session.set_local_webrtc_answer(
            TransportEpoch::new(1),
            json!({ "type": "answer", "sdp": "v=0" }),
            "native",
            true,
            direct_webrtc_endpoint_ura("rd-srflx-only-route"),
        );
        session
            .record_local_ice_candidate(json!({
                "candidate": "candidate:1 1 UDP 1686052607 203.0.113.1 50000 typ srflx",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("srflx candidate records");

        let view = RemoteDesktopTransportView::from_session(&session);
        let summary = view.summary(&session);

        assert_eq!(summary["route_state"]["stun_srflx"], json!(true));
        assert_eq!(summary["route_state"]["relay_ready"], json!(false));
        assert_eq!(summary["route_state"]["route_class"], json!("stun_srflx"));
        assert_eq!(summary["unavailable_reason"], json!("relay_unavailable"));
    }

    #[test]
    fn turn_and_easynet_relay_route_states_are_distinct() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-relay-route",
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        session
            .record_local_ice_candidate(json!({
                "candidate": "candidate:1 1 UDP 1686052607 203.0.113.1 50000 typ srflx",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("srflx candidate records");
        session
            .add_remote_ice_candidate(
                json!({
                    "candidate": "candidate:2 1 UDP 41819902 turn.example.test 3478 typ relay",
                    "relay_type": "turn",
                    "sdpMid": "0",
                    "sdpMLineIndex": 0
                }),
                "pending",
                None,
            )
            .expect("TURN relay candidate records");
        session
            .add_remote_ice_candidate(
                json!({
                    "candidate": "candidate:3 1 UDP 41819902 easynet-relay.local 443 typ relay",
                    "relay_type": "easynet",
                    "sdpMid": "0",
                    "sdpMLineIndex": 0
                }),
                "pending",
                None,
            )
            .expect("relay candidate records");
        session.record_webrtc_diagnostic(
            "ICE_CONNECTION_STATE_CHANGED",
            None,
            json!({ "ice_connection_state": "failed" }),
        );

        let view = RemoteDesktopTransportView::from_session(&session);
        let summary = view.summary(&session);

        assert_eq!(summary["route_state"]["stun_srflx"], json!(true));
        assert_eq!(summary["route_state"]["turn_relay"], json!(true));
        assert_eq!(summary["route_state"]["easynet_relay"], json!(true));
        assert_eq!(summary["route_state"]["nat_traversal_ready"], json!(true));
        assert_eq!(summary["route_state"]["relay_ready"], json!(true));
        assert_eq!(summary["route_state"]["failed"], json!(true));
        assert_eq!(summary["route_state"]["route_class"], json!("failed"));
    }

    #[test]
    fn easynet_relay_does_not_imply_turn_relay() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-easynet-relay-route",
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        session
            .add_remote_ice_candidate(
                json!({
                    "candidate": "candidate:1 1 UDP 41819902 easynet-relay.local 443 typ relay",
                    "relay_type": "easynet",
                    "sdpMid": "0",
                    "sdpMLineIndex": 0
                }),
                "pending",
                None,
            )
            .expect("EasyNet relay candidate records");

        let view = RemoteDesktopTransportView::from_session(&session);
        let summary = view.summary(&session);

        assert_eq!(summary["route_state"]["turn_relay"], json!(false));
        assert_eq!(summary["route_state"]["easynet_relay"], json!(true));
        assert_eq!(summary["route_state"]["relay_ready"], json!(true));
        assert_eq!(
            summary["route_state"]["route_class"],
            json!("easynet_relay")
        );
    }
}
