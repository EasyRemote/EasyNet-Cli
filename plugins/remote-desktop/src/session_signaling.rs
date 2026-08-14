// EasyNet CLI — remote desktop signaling state
// =============================================
//
// File: plugins/remote-desktop/src/session_signaling.rs
// Description: SDP, ICE, and WebRTC diagnostic state for one remote desktop session.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::constants::{
    MAX_LOCAL_ICE_CANDIDATES, MAX_REMOTE_ICE_CANDIDATES, TRANSPORT_WEBRTC,
};

/// SDP description accepted for one side of a remote desktop session.
///
/// This type keeps raw JSON at the ability/WebRTC boundary while preventing
/// the session aggregate from storing unlabelled `serde_json::Value` as domain
/// state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSessionDescription {
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

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        self.value.clone()
    }
}

/// ICE candidate accepted for a remote desktop session.
///
/// What this is NOT: a WebRTC crate candidate object. Conversion to
/// `RTCIceCandidateInit` remains in the transport layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopIceCandidate {
    value: Value,
}

impl RemoteDesktopIceCandidate {
    fn new(value: Value) -> Self {
        Self { value }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        self.value.clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub(in crate::daemon::plugins::remote_desktop) enum RemoteDesktopSignalingError {
    #[error("{side} ICE candidate cap exceeded ({limit})")]
    IceCandidateLimitExceeded { side: &'static str, limit: usize },
    #[error("remote ICE candidate reservation is missing")]
    RemoteIceCandidateReservationMissing,
}

/// Negotiated media codec metadata for a direct WebRTC endpoint.
///
/// This is session-domain metadata. View serialization projects it to JSON;
/// transport startup owns the actual codec registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopNegotiatedCodec {
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

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "codec": self.codec,
            "profile": self.profile,
            "transport": self.transport,
            "endpoint": self.endpoint,
            "backend_id": self.backend_id,
            "production_ready": self.production_ready,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn production_ready(&self) -> bool {
        self.production_ready
    }
}

/// Diagnostic facts carried by a WebRTC callback event.
///
/// The full event payload is still projected into the event log, but the
/// session state only stores typed fields used for lifecycle/view decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteDesktopWebRtcDiagnostic {
    ice_connection_state: Option<String>,
    peer_connection_state: Option<String>,
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
        let peer_connection_state = payload
            .get("peer_connection_state")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|state| !state.is_empty())
            .map(str::to_string);
        Self {
            ice_connection_state,
            peer_connection_state,
            error,
        }
    }
}

/// WebRTC signaling projection owned by a remote desktop session.
///
/// The session lifecycle decides when mutation is allowed; this type only
/// maintains signaling facts and derived transport diagnostics.
#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSignalingState {
    local_description: Option<RemoteDesktopSessionDescription>,
    remote_description: Option<RemoteDesktopSessionDescription>,
    remote_ice_candidates: Vec<RemoteDesktopIceCandidate>,
    remote_ice_candidate_reservations: usize,
    local_ice_candidates: Vec<RemoteDesktopIceCandidate>,
    webrtc_ice_state: Option<String>,
    webrtc_peer_state: Option<String>,
    webrtc_error: Option<String>,
    negotiated_codec: Option<RemoteDesktopNegotiatedCodec>,
}

impl RemoteDesktopSignalingState {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            local_description: None,
            remote_description: None,
            remote_ice_candidates: Vec::new(),
            remote_ice_candidate_reservations: 0,
            local_ice_candidates: Vec::new(),
            webrtc_ice_state: None,
            webrtc_peer_state: None,
            webrtc_error: None,
            negotiated_codec: None,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn local_description(&self) -> Option<Value> {
        self.local_description
            .as_ref()
            .map(RemoteDesktopSessionDescription::to_value)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn remote_description(&self) -> Option<Value> {
        self.remote_description
            .as_ref()
            .map(RemoteDesktopSessionDescription::to_value)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn remote_ice_candidates(&self) -> Vec<Value> {
        self.remote_ice_candidates
            .iter()
            .map(RemoteDesktopIceCandidate::to_value)
            .collect()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn local_ice_candidates(&self) -> Vec<Value> {
        self.local_ice_candidates
            .iter()
            .map(RemoteDesktopIceCandidate::to_value)
            .collect()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn webrtc_ice_state(&self) -> Option<&str> {
        self.webrtc_ice_state.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn webrtc_peer_state(&self) -> Option<&str> {
        self.webrtc_peer_state.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn webrtc_error(&self) -> Option<&str> {
        self.webrtc_error.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn negotiated_codec(&self) -> Option<Value> {
        self.negotiated_codec
            .as_ref()
            .map(RemoteDesktopNegotiatedCodec::to_value)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn production_codec_negotiated(&self) -> bool {
        self.negotiated_codec
            .as_ref()
            .is_some_and(RemoteDesktopNegotiatedCodec::production_ready)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn has_description(&self) -> bool {
        self.local_description.is_some() || self.remote_description.is_some()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn set_description(
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

    pub(in crate::daemon::plugins::remote_desktop) fn push_remote_ice_candidate(
        &mut self,
        candidate: Value,
    ) -> anyhow::Result<usize> {
        self.ensure_remote_ice_candidate_capacity()?;
        self.remote_ice_candidates
            .push(RemoteDesktopIceCandidate::new(candidate));
        Ok(self.remote_ice_candidates.len())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn reserve_remote_ice_candidate_slot(
        &mut self,
    ) -> anyhow::Result<()> {
        self.ensure_remote_ice_candidate_capacity()?;
        self.remote_ice_candidate_reservations += 1;
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn commit_reserved_remote_ice_candidate(
        &mut self,
        candidate: Value,
    ) -> anyhow::Result<usize> {
        if self.remote_ice_candidate_reservations == 0 {
            return Err(RemoteDesktopSignalingError::RemoteIceCandidateReservationMissing.into());
        }
        self.remote_ice_candidate_reservations -= 1;
        self.remote_ice_candidates
            .push(RemoteDesktopIceCandidate::new(candidate));
        Ok(self.remote_ice_candidates.len())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn release_remote_ice_candidate_slot(&mut self) {
        if self.remote_ice_candidate_reservations > 0 {
            self.remote_ice_candidate_reservations -= 1;
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn push_local_ice_candidate(
        &mut self,
        candidate: Value,
    ) -> anyhow::Result<usize> {
        if self.local_ice_candidates.len() >= MAX_LOCAL_ICE_CANDIDATES {
            return Err(RemoteDesktopSignalingError::IceCandidateLimitExceeded {
                side: "local",
                limit: MAX_LOCAL_ICE_CANDIDATES,
            }
            .into());
        }
        self.local_ice_candidates
            .push(RemoteDesktopIceCandidate::new(candidate));
        Ok(self.local_ice_candidates.len())
    }

    fn ensure_remote_ice_candidate_capacity(&self) -> anyhow::Result<()> {
        if self.remote_ice_candidates.len() + self.remote_ice_candidate_reservations
            >= MAX_REMOTE_ICE_CANDIDATES
        {
            return Err(RemoteDesktopSignalingError::IceCandidateLimitExceeded {
                side: "remote",
                limit: MAX_REMOTE_ICE_CANDIDATES,
            }
            .into());
        }
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn set_webrtc_error(&mut self, reason: &str) {
        self.webrtc_error = Some(reason.to_string());
    }

    pub(in crate::daemon::plugins::remote_desktop) fn set_local_webrtc_answer(
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

    pub(in crate::daemon::plugins::remote_desktop) fn record_webrtc_diagnostic(
        &mut self,
        error: Option<String>,
        payload: &Value,
    ) {
        let diagnostic = RemoteDesktopWebRtcDiagnostic::from_payload(error, payload);
        if let Some(state) = diagnostic.ice_connection_state {
            self.webrtc_ice_state = Some(state);
        }
        if let Some(state) = diagnostic.peer_connection_state {
            self.webrtc_peer_state = Some(state);
        }
        if let Some(error) = diagnostic.error {
            self.webrtc_error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::{
        MAX_LOCAL_ICE_CANDIDATES, MAX_REMOTE_ICE_CANDIDATES,
    };

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

    #[test]
    fn remote_desktop_signaling_records_peer_connection_state() {
        let mut signaling = RemoteDesktopSignalingState::new();

        signaling.record_webrtc_diagnostic(
            None,
            &json!({
                "peer_connection_state": "connected",
            }),
        );

        assert_eq!(signaling.webrtc_peer_state(), Some("connected"));
    }

    #[test]
    fn remote_desktop_signaling_rejects_more_than_ten_thousand_candidates_without_growth() {
        const FLOOD_CANDIDATES: usize = 10_001;
        let mut signaling = RemoteDesktopSignalingState::new();
        let mut remote_rejected = 0;
        let mut local_rejected = 0;

        for index in 0..FLOOD_CANDIDATES {
            let candidate = json!({
                "candidate": format!("candidate:{index} 1 UDP 2122252543 127.0.0.1 {} typ host", 42000 + (index % 1000)),
                "sdpMid": "0",
                "sdpMLineIndex": 0
            });
            if signaling
                .push_remote_ice_candidate(candidate.clone())
                .is_err()
            {
                remote_rejected += 1;
            }
            if signaling.push_local_ice_candidate(candidate).is_err() {
                local_rejected += 1;
            }
        }

        assert_eq!(
            signaling.remote_ice_candidates().len(),
            MAX_REMOTE_ICE_CANDIDATES,
            "remote serialized session view must remain bounded under candidate flood"
        );
        assert_eq!(
            signaling.local_ice_candidates().len(),
            MAX_LOCAL_ICE_CANDIDATES,
            "local serialized session view must remain bounded under candidate flood"
        );
        assert_eq!(
            remote_rejected,
            FLOOD_CANDIDATES - MAX_REMOTE_ICE_CANDIDATES
        );
        assert_eq!(local_rejected, FLOOD_CANDIDATES - MAX_LOCAL_ICE_CANDIDATES);
    }

    #[test]
    fn remote_ice_candidate_reservations_count_against_candidate_cap() {
        let mut signaling = RemoteDesktopSignalingState::new();
        for index in 0..(MAX_REMOTE_ICE_CANDIDATES - 1) {
            signaling
                .push_remote_ice_candidate(json!({
                    "candidate": format!("candidate:{index} 1 UDP 2122252543 127.0.0.1 {} typ host", 43000 + index),
                    "sdpMid": "0",
                    "sdpMLineIndex": 0
                }))
                .expect("candidate within cap records");
        }

        signaling
            .reserve_remote_ice_candidate_slot()
            .expect("last slot can be reserved before transport apply");
        let err = signaling
            .push_remote_ice_candidate(json!({
                "candidate": "candidate:blocked 1 UDP 2122252543 127.0.0.1 49999 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect_err("reserved slot must block extra remote candidate admission")
            .to_string();
        assert!(
            err.contains("remote ICE candidate cap exceeded"),
            "got {err}"
        );
        assert_eq!(
            signaling.remote_ice_candidates().len(),
            MAX_REMOTE_ICE_CANDIDATES - 1,
            "reserved but uncommitted candidate must not enter serialized state"
        );

        signaling.release_remote_ice_candidate_slot();
        signaling
            .push_remote_ice_candidate(json!({
                "candidate": "candidate:after-release 1 UDP 2122252543 127.0.0.1 50000 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("released reservation restores one candidate slot");
        assert_eq!(
            signaling.remote_ice_candidates().len(),
            MAX_REMOTE_ICE_CANDIDATES
        );
    }
}
