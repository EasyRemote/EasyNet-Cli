// EasyNet CLI — remote desktop signaling state
// =============================================
//
// File: plugins/remote-desktop/src/session_signaling.rs
// Description: SDP, ICE, and WebRTC diagnostic state for one remote desktop session.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::constants::{
    MAX_ICE_CANDIDATE_BYTES, MAX_LOCAL_ICE_CANDIDATES, MAX_REMOTE_ICE_CANDIDATES,
    MAX_SIGNALING_DESCRIPTION_BYTES, TRANSPORT_WEBRTC,
};
use crate::daemon::plugins::remote_desktop::sdp::{
    validate_ice_candidate_row, validate_signaling_description_size,
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
            "local" | "remote" => {
                validate_signaling_description_size(&value)?;
                Ok(Self {
                    side: side.to_string(),
                    value,
                })
            }
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
    fn new(value: Value) -> anyhow::Result<Self> {
        validate_ice_candidate_row(&value)?;
        Ok(Self { value })
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

    /// Start one independent signaling generation.
    ///
    /// SDP, ICE, codec and diagnostic facts are PeerConnection-scoped. Keeping
    /// them across a resumed transport would apply stale ICE candidates to the
    /// new endpoint and could make an old browser callback mutate the current
    /// session generation.
    pub(in crate::daemon::plugins::remote_desktop) fn begin_transport_generation(&mut self) {
        *self = Self::new();
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

    pub(in crate::daemon::plugins::remote_desktop) fn to_bounded_view(
        &self,
        route_state: Value,
    ) -> Value {
        json!({
            "local_description": self.local_description(),
            "remote_description": self.remote_description(),
            "ice_candidate_count": self.remote_ice_candidates.len(),
            "local_ice_candidate_count": self.local_ice_candidates.len(),
            "local_ice_candidates": self.local_ice_candidates(),
            "webrtc_ice_state": self.webrtc_ice_state(),
            "webrtc_peer_state": self.webrtc_peer_state(),
            "webrtc_error": self.webrtc_error(),
            "route_state": route_state,
            "signaling_limits": {
                "remote_ice_candidate_count": MAX_REMOTE_ICE_CANDIDATES,
                "local_ice_candidate_count": MAX_LOCAL_ICE_CANDIDATES,
                "ice_candidate_bytes": MAX_ICE_CANDIDATE_BYTES,
                "description_bytes": MAX_SIGNALING_DESCRIPTION_BYTES,
            },
            "local_ice_candidates_truncated": false,
            "remote_ice_candidates_elided": true,
        })
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
            .push(RemoteDesktopIceCandidate::new(candidate)?);
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
            .push(RemoteDesktopIceCandidate::new(candidate)?);
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
            .push(RemoteDesktopIceCandidate::new(candidate)?);
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
    ) -> anyhow::Result<()> {
        self.local_description = Some(RemoteDesktopSessionDescription::new("local", answer)?);
        self.negotiated_codec = Some(RemoteDesktopNegotiatedCodec::h264_baseline(
            endpoint_ura,
            backend_id,
            production_ready,
        ));
        Ok(())
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
        direct_webrtc_endpoint_ura, MAX_ICE_CANDIDATE_BYTES, MAX_LOCAL_ICE_CANDIDATES,
        MAX_REMOTE_ICE_CANDIDATES, MAX_SIGNALING_DESCRIPTION_BYTES,
    };

    use super::RemoteDesktopSignalingState;

    #[test]
    fn remote_desktop_signaling_answer_projects_negotiated_codec() {
        let mut signaling = RemoteDesktopSignalingState::new();

        signaling
            .set_local_webrtc_answer(
                json!({ "type": "answer", "sdp": "v=0" }),
                "native",
                true,
                direct_webrtc_endpoint_ura("signaling-answer"),
            )
            .expect("local answer records");

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
                "endpoint": direct_webrtc_endpoint_ura("signaling-answer"),
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
    fn remote_desktop_signaling_bounded_view_projects_counts_and_limits() {
        let mut signaling = RemoteDesktopSignalingState::new();
        signaling
            .set_description("remote", json!({"type": "offer", "sdp": "v=0"}))
            .expect("remote description records");
        signaling
            .push_remote_ice_candidate(json!({
                "candidate": "candidate:remote 1 UDP 2122252543 127.0.0.1 41000 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("remote candidate records");
        signaling
            .push_local_ice_candidate(json!({
                "candidate": "candidate:local 1 UDP 2122252543 127.0.0.1 42000 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("local candidate records");
        signaling.record_webrtc_diagnostic(
            Some("relay_unavailable".to_string()),
            &json!({
                "ice_connection_state": "checking",
                "peer_connection_state": "connecting",
            }),
        );

        let view = signaling.to_bounded_view(json!("host_only_no_nat_or_relay"));

        assert_eq!(view["ice_candidate_count"], json!(1));
        assert_eq!(view["local_ice_candidate_count"], json!(1));
        assert_eq!(view["local_ice_candidates"].as_array().unwrap().len(), 1);
        assert_eq!(view["remote_ice_candidates_elided"], json!(true));
        assert_eq!(view["local_ice_candidates_truncated"], json!(false));
        assert_eq!(
            view["signaling_limits"]["remote_ice_candidate_count"],
            json!(MAX_REMOTE_ICE_CANDIDATES)
        );
        assert_eq!(
            view["signaling_limits"]["local_ice_candidate_count"],
            json!(MAX_LOCAL_ICE_CANDIDATES)
        );
        assert_eq!(
            view["signaling_limits"]["ice_candidate_bytes"],
            json!(MAX_ICE_CANDIDATE_BYTES)
        );
        assert_eq!(
            view["signaling_limits"]["description_bytes"],
            json!(MAX_SIGNALING_DESCRIPTION_BYTES)
        );
        assert_eq!(view["route_state"], json!("host_only_no_nat_or_relay"));
        assert_eq!(view["webrtc_ice_state"], json!("checking"));
        assert_eq!(view["webrtc_peer_state"], json!("connecting"));
        assert_eq!(view["webrtc_error"], json!("relay_unavailable"));
    }

    #[test]
    fn signaling_state_rejects_oversized_descriptions_before_storage() {
        let mut signaling = RemoteDesktopSignalingState::new();
        let oversized_sdp = format!(
            "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n{}",
            "a=x\r\n".repeat((MAX_SIGNALING_DESCRIPTION_BYTES / 5) + 2)
        );

        let remote_err = signaling
            .set_description("remote", json!({"type": "offer", "sdp": oversized_sdp}))
            .expect_err("remote description must be bounded by signaling state")
            .to_string();

        assert!(remote_err.contains("exceeds"), "got {remote_err}");
        assert_eq!(signaling.remote_description(), None);

        let oversized_answer = json!({
            "type": "answer",
            "sdp": format!(
                "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n{}",
                "a=y\r\n".repeat((MAX_SIGNALING_DESCRIPTION_BYTES / 5) + 2)
            )
        });
        let local_err = signaling
            .set_local_webrtc_answer(
                oversized_answer,
                "native",
                true,
                direct_webrtc_endpoint_ura("signaling-oversized-answer"),
            )
            .expect_err("generated local answer must be bounded by signaling state")
            .to_string();

        assert!(local_err.contains("exceeds"), "got {local_err}");
        assert_eq!(signaling.local_description(), None);
        assert_eq!(signaling.negotiated_codec(), None);
    }

    #[test]
    fn new_transport_generation_discards_prior_peer_connection_state() {
        let mut signaling = RemoteDesktopSignalingState::new();
        signaling
            .set_description("remote", json!({"type": "offer", "sdp": "v=0"}))
            .expect("remote description records");
        signaling
            .push_remote_ice_candidate(json!({
                "candidate": "candidate:old 1 UDP 2122252543 127.0.0.1 41000 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("remote candidate records");
        signaling
            .push_local_ice_candidate(json!({
                "candidate": "candidate:old-local 1 UDP 2122252543 127.0.0.1 42000 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("local candidate records");
        signaling.record_webrtc_diagnostic(
            Some("old_transport_failed".to_string()),
            &json!({"peer_connection_state": "failed"}),
        );

        signaling.begin_transport_generation();

        assert_eq!(signaling.local_description(), None);
        assert_eq!(signaling.remote_description(), None);
        assert!(signaling.local_ice_candidates().is_empty());
        assert!(signaling.remote_ice_candidates().is_empty());
        assert_eq!(signaling.webrtc_peer_state(), None);
        assert_eq!(signaling.webrtc_error(), None);
        assert_eq!(signaling.negotiated_codec(), None);
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
    fn signaling_state_validates_local_and_remote_ice_rows_before_storage() {
        let mut signaling = RemoteDesktopSignalingState::new();
        let malformed = json!({"sdpMid": "0", "sdpMLineIndex": 0});
        let remote_err = signaling
            .push_remote_ice_candidate(malformed.clone())
            .expect_err("remote signaling state must reject schema-incomplete ICE rows")
            .to_string();
        assert!(
            remote_err.contains("must include string `candidate`"),
            "got {remote_err}"
        );
        let local_err = signaling
            .push_local_ice_candidate(malformed)
            .expect_err("local WebRTC callbacks must not bypass ICE row schema validation")
            .to_string();
        assert!(
            local_err.contains("must include string `candidate`"),
            "got {local_err}"
        );

        let oversized = json!({
            "candidate": format!(
                "candidate:oversized 1 UDP 2122252543 {} 54400 typ host",
                "x".repeat(MAX_ICE_CANDIDATE_BYTES)
            ),
            "sdpMid": "0",
            "sdpMLineIndex": 0
        });
        let local_oversized_err = signaling
            .push_local_ice_candidate(oversized)
            .expect_err("local signaling state must reject oversized ICE rows before storage")
            .to_string();
        assert!(
            local_oversized_err.contains("exceeds"),
            "got {local_oversized_err}"
        );
        assert_eq!(signaling.remote_ice_candidates().len(), 0);
        assert_eq!(signaling.local_ice_candidates().len(), 0);
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
