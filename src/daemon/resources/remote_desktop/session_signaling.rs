// EasyNet CLI — remote desktop signaling state
// =============================================
//
// File: src/daemon/resources/remote_desktop/session_signaling.rs
// Description: SDP, ICE, and WebRTC diagnostic state for one remote desktop session.

use serde_json::{json, Value};

use crate::daemon::resources::remote_desktop::constants::TRANSPORT_WEBRTC;

/// SDP description accepted for one side of a remote desktop session.
///
/// This type keeps raw JSON at the ability/WebRTC boundary while preventing
/// the session aggregate from storing unlabelled `serde_json::Value` as domain
/// state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::resources::remote_desktop) struct RemoteDesktopSessionDescription {
    side: String,
    value: Value,
}

impl RemoteDesktopSessionDescription {
    fn new(side: &str, value: Value) -> anyhow::Result<Self> {
        match side {
            "local" | "remote" => Ok(Self {
                side: side.to_string(),
                value,
            }),
            _ => anyhow::bail!("side must be local or remote"),
        }
    }

    pub(in crate::daemon::resources::remote_desktop) fn to_value(&self) -> Value {
        self.value.clone()
    }
}

/// ICE candidate accepted for a remote desktop session.
///
/// What this is NOT: a WebRTC crate candidate object. Conversion to
/// `RTCIceCandidateInit` remains in the transport layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::resources::remote_desktop) struct RemoteDesktopIceCandidate {
    value: Value,
}

impl RemoteDesktopIceCandidate {
    fn new(value: Value) -> Self {
        Self { value }
    }

    pub(in crate::daemon::resources::remote_desktop) fn to_value(&self) -> Value {
        self.value.clone()
    }
}

/// Negotiated media codec metadata for a direct WebRTC endpoint.
///
/// This is session-domain metadata. View serialization projects it to JSON;
/// transport startup owns the actual codec registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::resources::remote_desktop) struct RemoteDesktopNegotiatedCodec {
    codec: &'static str,
    profile: &'static str,
    transport: &'static str,
    endpoint: String,
    backend_id: String,
    production_ready: bool,
}

impl RemoteDesktopNegotiatedCodec {
    fn h264_baseline(endpoint: String, backend_id: &str, production_ready: bool) -> Self {
        Self {
            codec: "h264",
            profile: "baseline",
            transport: TRANSPORT_WEBRTC,
            endpoint,
            backend_id: backend_id.to_string(),
            production_ready,
        }
    }

    pub(in crate::daemon::resources::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "codec": self.codec,
            "profile": self.profile,
            "transport": self.transport,
            "endpoint": self.endpoint,
            "backend_id": self.backend_id,
            "production_ready": self.production_ready,
        })
    }
}

/// Diagnostic facts carried by a WebRTC callback event.
///
/// The full event payload is still projected into the event log, but the
/// session state only stores typed fields used for lifecycle/view decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteDesktopWebRtcDiagnostic {
    ice_connection_state: Option<String>,
    error: Option<String>,
}

impl RemoteDesktopWebRtcDiagnostic {
    fn from_payload(error: Option<String>, payload: &Value) -> Self {
        let ice_connection_state = payload
            .get("ice_connection_state")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|state| !state.is_empty())
            .map(str::to_string);
        Self {
            ice_connection_state,
            error,
        }
    }
}

/// WebRTC signaling projection owned by a remote desktop session.
///
/// The session lifecycle decides when mutation is allowed; this type only
/// maintains signaling facts and derived transport diagnostics.
#[derive(Debug, Clone)]
pub(in crate::daemon::resources::remote_desktop) struct RemoteDesktopSignalingState {
    local_description: Option<RemoteDesktopSessionDescription>,
    remote_description: Option<RemoteDesktopSessionDescription>,
    remote_ice_candidates: Vec<RemoteDesktopIceCandidate>,
    local_ice_candidates: Vec<RemoteDesktopIceCandidate>,
    webrtc_ice_state: Option<String>,
    webrtc_error: Option<String>,
    negotiated_codec: Option<RemoteDesktopNegotiatedCodec>,
}

impl RemoteDesktopSignalingState {
    pub(in crate::daemon::resources::remote_desktop) fn new() -> Self {
        Self {
            local_description: None,
            remote_description: None,
            remote_ice_candidates: Vec::new(),
            local_ice_candidates: Vec::new(),
            webrtc_ice_state: None,
            webrtc_error: None,
            negotiated_codec: None,
        }
    }

    pub(in crate::daemon::resources::remote_desktop) fn local_description(&self) -> Option<Value> {
        self.local_description
            .as_ref()
            .map(RemoteDesktopSessionDescription::to_value)
    }

    pub(in crate::daemon::resources::remote_desktop) fn remote_description(&self) -> Option<Value> {
        self.remote_description
            .as_ref()
            .map(RemoteDesktopSessionDescription::to_value)
    }

    pub(in crate::daemon::resources::remote_desktop) fn remote_ice_candidates(&self) -> Vec<Value> {
        self.remote_ice_candidates
            .iter()
            .map(RemoteDesktopIceCandidate::to_value)
            .collect()
    }

    pub(in crate::daemon::resources::remote_desktop) fn local_ice_candidates(&self) -> Vec<Value> {
        self.local_ice_candidates
            .iter()
            .map(RemoteDesktopIceCandidate::to_value)
            .collect()
    }

    pub(in crate::daemon::resources::remote_desktop) fn webrtc_ice_state(&self) -> Option<&str> {
        self.webrtc_ice_state.as_deref()
    }

    pub(in crate::daemon::resources::remote_desktop) fn webrtc_error(&self) -> Option<&str> {
        self.webrtc_error.as_deref()
    }

    pub(in crate::daemon::resources::remote_desktop) fn negotiated_codec(&self) -> Option<Value> {
        self.negotiated_codec
            .as_ref()
            .map(RemoteDesktopNegotiatedCodec::to_value)
    }

    pub(in crate::daemon::resources::remote_desktop) fn has_description(&self) -> bool {
        self.local_description.is_some() || self.remote_description.is_some()
    }

    pub(in crate::daemon::resources::remote_desktop) fn set_description(
        &mut self,
        side: &str,
        description: Value,
    ) -> anyhow::Result<()> {
        match side {
            "local" => {
                self.local_description =
                    Some(RemoteDesktopSessionDescription::new(side, description)?)
            }
            "remote" => {
                self.remote_description =
                    Some(RemoteDesktopSessionDescription::new(side, description)?)
            }
            _ => anyhow::bail!("side must be local or remote"),
        }
        Ok(())
    }

    pub(in crate::daemon::resources::remote_desktop) fn push_remote_ice_candidate(
        &mut self,
        candidate: Value,
    ) -> usize {
        self.remote_ice_candidates
            .push(RemoteDesktopIceCandidate::new(candidate));
        self.remote_ice_candidates.len()
    }

    pub(in crate::daemon::resources::remote_desktop) fn push_local_ice_candidate(
        &mut self,
        candidate: Value,
    ) -> usize {
        self.local_ice_candidates
            .push(RemoteDesktopIceCandidate::new(candidate));
        self.local_ice_candidates.len()
    }

    pub(in crate::daemon::resources::remote_desktop) fn set_webrtc_error(&mut self, reason: &str) {
        self.webrtc_error = Some(reason.to_string());
    }

    pub(in crate::daemon::resources::remote_desktop) fn set_local_webrtc_answer(
        &mut self,
        answer: Value,
        backend_id: &str,
        production_ready: bool,
        endpoint_ura: String,
    ) {
        self.local_description =
            Some(RemoteDesktopSessionDescription::new("local", answer).expect("literal side"));
        self.negotiated_codec = Some(RemoteDesktopNegotiatedCodec::h264_baseline(
            endpoint_ura,
            backend_id,
            production_ready,
        ));
    }

    pub(in crate::daemon::resources::remote_desktop) fn record_webrtc_diagnostic(
        &mut self,
        error: Option<String>,
        payload: &Value,
    ) {
        let diagnostic = RemoteDesktopWebRtcDiagnostic::from_payload(error, payload);
        if let Some(state) = diagnostic.ice_connection_state {
            self.webrtc_ice_state = Some(state);
        }
        if let Some(error) = diagnostic.error {
            self.webrtc_error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RemoteDesktopSignalingState;

    #[test]
    fn remote_desktop_signaling_answer_projects_negotiated_codec() {
        let mut signaling = RemoteDesktopSignalingState::new();

        signaling.set_local_webrtc_answer(
            json!({ "type": "answer", "sdp": "v=0" }),
            "native",
            true,
            "ura://endpoint".to_string(),
        );

        assert_eq!(
            signaling.local_description(),
            Some(json!({ "type": "answer", "sdp": "v=0" }))
        );
        assert_eq!(
            signaling.negotiated_codec(),
            Some(json!({
                "codec": "h264",
                "profile": "baseline",
                "transport": "webrtc",
                "endpoint": "ura://endpoint",
                "backend_id": "native",
                "production_ready": true,
            }))
        );
    }

    #[test]
    fn remote_desktop_signaling_diagnostic_ignores_blank_ice_state() {
        let mut signaling = RemoteDesktopSignalingState::new();

        signaling.record_webrtc_diagnostic(
            Some("first".to_string()),
            &json!({
                "ice_connection_state": "connected",
            }),
        );
        signaling.record_webrtc_diagnostic(
            Some("second".to_string()),
            &json!({
                "ice_connection_state": "   ",
            }),
        );

        assert_eq!(signaling.webrtc_ice_state(), Some("connected"));
        assert_eq!(signaling.webrtc_error(), Some("second"));
    }
}
