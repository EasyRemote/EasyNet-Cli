// EasyNet CLI — remote desktop WebRTC SDP/ICE helpers
// ===================================================
//
// File: plugins/remote-desktop/src/sdp.rs
// Description: Pure SDP and ICE candidate normalization for the direct WebRTC
//              transport path.
//
// Protocol Responsibility:
// - Preserve browser/device SDP semantics while enforcing negotiated
//   device-to-browser video and, when offered, audio senders.
//
// Implementation Approach:
// - Normalize line endings/candidates and inspect media-section direction
//   without owning peer-connection or session state.
//
// Usage Contract:
// - Callers must validate a generated answer before publishing it as an
//   active remote-desktop transport.
//
// Architectural Position:
// - Transport parsing boundary. This module owns string/value normalization
//   only; it does not touch session state, media encoders, or plugin stores.

use serde_json::Value;
use webrtc::peer_connection::RTCIceCandidateInit;

use crate::daemon::plugins::remote_desktop::constants::{
    MAX_ICE_CANDIDATE_BYTES, MAX_SIGNALING_DESCRIPTION_BYTES,
};

/// Ensure a browser/device SDP carries an explicit end-of-candidates marker.
pub(in crate::daemon::plugins::remote_desktop) fn ensure_sdp_end_of_candidates(
    sdp: &str,
) -> String {
    if sdp.contains("a=end-of-candidates") {
        return sdp.to_string();
    }
    let mut out = sdp.to_string();
    if !out.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    out.push_str("a=end-of-candidates\r\n");
    out
}

/// Normalize the device answer before returning it to a browser caller.
///
/// RTCP component candidates are filtered because the media path uses RTP/RTCP
/// mux. Keeping them in the answer confuses some browser ICE stacks while adding
/// no viable transport path.
pub(in crate::daemon::plugins::remote_desktop) fn normalize_browser_answer_sdp(
    sdp: &str,
) -> String {
    let mut out = String::with_capacity(sdp.len());
    for line in ensure_sdp_end_of_candidates(sdp).lines() {
        if is_rtcp_component_candidate(line) {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out
}

/// Normalize a caller offer without dropping mDNS host candidates.
///
/// With `MulticastDnsMode::QueryOnly`, the ICE agent resolves `.local` host
/// candidates itself. Stripping them would starve localhost Chrome sessions,
/// where mDNS host plus srflx may be the only offered candidates.
pub(in crate::daemon::plugins::remote_desktop) fn normalize_remote_offer_sdp(sdp: &str) -> String {
    ensure_sdp_end_of_candidates(sdp)
}

/// Reject plainly malformed browser offers before transport setup can mutate
/// session state.
pub(in crate::daemon::plugins::remote_desktop) fn validate_remote_offer_sdp(
    sdp: &str,
) -> anyhow::Result<()> {
    validate_sdp_size(sdp)?;
    let mut has_version = false;
    let mut has_video = false;
    for line in sdp.lines().map(str::trim_end) {
        if line == "v=0" {
            has_version = true;
        }
        if line.starts_with("m=video ") {
            has_video = true;
        }
    }
    if has_version && has_video {
        return Ok(());
    }
    anyhow::bail!("remote WebRTC offer SDP must include v=0 and a video media section")
}

pub(in crate::daemon::plugins::remote_desktop) fn validate_sdp_size(
    sdp: &str,
) -> anyhow::Result<()> {
    if sdp.len() <= MAX_SIGNALING_DESCRIPTION_BYTES {
        return Ok(());
    }
    anyhow::bail!("remote WebRTC SDP exceeds {MAX_SIGNALING_DESCRIPTION_BYTES} bytes")
}

pub(in crate::daemon::plugins::remote_desktop) fn validate_signaling_description_size(
    description: &Value,
) -> anyhow::Result<()> {
    if let Some(sdp) = description.get("sdp").and_then(Value::as_str) {
        validate_sdp_size(sdp)?;
    }
    let bytes = serde_json::to_vec(description)?;
    if bytes.len() <= MAX_SIGNALING_DESCRIPTION_BYTES {
        return Ok(());
    }
    anyhow::bail!(
        "remote desktop signaling description exceeds {MAX_SIGNALING_DESCRIPTION_BYTES} bytes"
    )
}

pub(in crate::daemon::plugins::remote_desktop) fn validate_ice_candidate_size(
    candidate: &Value,
) -> anyhow::Result<()> {
    if let Some(candidate_text) = candidate.get("candidate").and_then(Value::as_str) {
        if candidate_text.len() > MAX_ICE_CANDIDATE_BYTES {
            anyhow::bail!("ICE candidate row exceeds {MAX_ICE_CANDIDATE_BYTES} bytes");
        }
    }
    let bytes = serde_json::to_vec(candidate)?;
    if bytes.len() <= MAX_ICE_CANDIDATE_BYTES {
        return Ok(());
    }
    anyhow::bail!("ICE candidate row exceeds {MAX_ICE_CANDIDATE_BYTES} bytes")
}

pub(in crate::daemon::plugins::remote_desktop) fn validate_ice_candidate_row(
    candidate: &Value,
) -> anyhow::Result<()> {
    if candidate.is_null() {
        validate_ice_candidate_size(candidate)?;
        return Ok(());
    }
    validate_ice_candidate_size(candidate)?;
    ice_candidate_text(candidate)?;
    Ok(())
}

/// Require the answer's video media section to send device media.
///
/// A connected ICE/DTLS transport is not proof that remote desktop media was
/// negotiated: data channels can connect while an unmatched local track leaves
/// the video m-section `inactive` or `recvonly`. Publishing that answer would
/// create a false-active session whose RTP writer has no browser receiver.
pub(in crate::daemon::plugins::remote_desktop) fn ensure_answer_sends_video(
    sdp: &str,
) -> anyhow::Result<()> {
    let video_direction = media_section_direction(sdp, "video");

    if matches!(video_direction, Some("sendonly" | "sendrecv")) {
        return Ok(());
    }
    anyhow::bail!(
        "direct WebRTC answer has no device-to-browser video sender; \
         direction={}; reason=webrtc_video_sender_not_negotiated",
        video_direction.unwrap_or("missing")
    )
}

pub(in crate::daemon::plugins::remote_desktop) fn remote_offer_accepts_audio(sdp: &str) -> bool {
    media_section_direction(sdp, "audio")
        .is_some_and(|direction| matches!(direction, "recvonly" | "sendrecv"))
}

pub(in crate::daemon::plugins::remote_desktop) fn ensure_answer_sends_audio(
    sdp: &str,
) -> anyhow::Result<()> {
    let direction = media_section_direction(sdp, "audio");
    if matches!(direction, Some("sendonly" | "sendrecv")) {
        return Ok(());
    }
    anyhow::bail!(
        "direct WebRTC answer has no device-to-browser audio sender; \
         direction={}; reason=webrtc_audio_sender_not_negotiated",
        direction.unwrap_or("missing")
    )
}

fn media_section_direction<'a>(sdp: &'a str, media_kind: &str) -> Option<&'a str> {
    let mut in_section = false;
    let mut direction = None;
    for line in sdp.lines().map(str::trim_end) {
        if line.starts_with("m=") {
            if in_section {
                break;
            }
            in_section = line.starts_with(&format!("m={media_kind} "));
            continue;
        }
        if !in_section {
            continue;
        }
        direction = match line {
            "a=sendonly" => Some("sendonly"),
            "a=sendrecv" => Some("sendrecv"),
            "a=recvonly" => Some("recvonly"),
            "a=inactive" => Some("inactive"),
            _ => direction,
        };
    }
    direction
}

fn is_rtcp_component_candidate(line: &str) -> bool {
    if !line.starts_with("a=candidate:") {
        return false;
    }
    line.split_whitespace().nth(1) == Some("2")
}

/// Decode one JSON ICE candidate value into WebRTC candidate init records.
pub(in crate::daemon::plugins::remote_desktop) fn remote_ice_candidate_inits(
    candidate: &Value,
) -> anyhow::Result<Vec<RTCIceCandidateInit>> {
    if candidate.is_null() {
        return Ok(Vec::new());
    }
    validate_ice_candidate_row(candidate)?;
    let candidate_init: RTCIceCandidateInit = serde_json::from_value(candidate.clone())?;
    Ok(vec![candidate_init])
}

/// Return the explicit ICE candidate string from a schema-bound candidate row.
///
/// Empty candidate strings are valid end-of-candidates markers. Missing,
/// non-object, or non-string `candidate` fields are malformed signaling state.
pub(in crate::daemon::plugins::remote_desktop) fn ice_candidate_text(
    candidate: &Value,
) -> anyhow::Result<&str> {
    let object = require_ice_candidate_object(candidate)?;
    object
        .get("candidate")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("ICE candidate row must include string `candidate`"))
}

fn require_ice_candidate_object(
    candidate: &Value,
) -> anyhow::Result<&serde_json::Map<String, Value>> {
    candidate
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("ICE candidate row must be an object or null end marker"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remote_offer_sdp_keeps_browser_mdns_host_candidates() {
        let offer = "v=0\r\na=candidate:1 1 UDP 2122252543 abc.local 54400 typ host\r\n";
        let normalized = normalize_remote_offer_sdp(offer);

        assert!(
            normalized.contains("abc.local"),
            "mDNS host candidates must be preserved for browser localhost ICE"
        );
        assert!(
            normalized.contains("a=end-of-candidates"),
            "normalized offers must include end-of-candidates"
        );
    }

    #[test]
    fn trickle_mdns_candidate_is_passed_through() {
        let mdns = json!({
            "candidate": "candidate:1 1 UDP 2122252543 abc.local 54400 typ host",
            "sdpMid": "0",
            "sdpMLineIndex": 0
        });
        let srflx = json!({
            "candidate": "candidate:2 1 UDP 1686052607 203.0.113.1 50000 typ srflx",
            "sdpMid": "0",
            "sdpMLineIndex": 0
        });

        let mdns_kept = remote_ice_candidate_inits(&mdns).unwrap();
        assert_eq!(
            mdns_kept[0].candidate,
            mdns["candidate"].as_str().expect("candidate string")
        );
        let kept = remote_ice_candidate_inits(&srflx).unwrap();
        assert_eq!(
            kept[0].candidate,
            srflx["candidate"].as_str().expect("candidate string")
        );
    }

    #[test]
    fn remote_offer_sdp_validation_rejects_non_sdp_before_transport_setup() {
        let err = validate_remote_offer_sdp("not an sdp")
            .expect_err("plain text must not reach WebRTC endpoint setup")
            .to_string();
        assert!(err.contains("video media section"));
    }

    #[test]
    fn ice_candidate_rows_reject_schema_incomplete_values() {
        for (candidate, expected) in [
            (json!("candidate:1"), "must be an object or null"),
            (json!({}), "must include string `candidate`"),
            (json!({"candidate": 7}), "must include string `candidate`"),
        ] {
            let err = remote_ice_candidate_inits(&candidate)
                .expect_err("schema-incomplete candidate must fail closed")
                .to_string();
            assert!(err.contains(expected), "expected {expected:?}; got {err}");
        }
    }

    #[test]
    fn signaling_rejects_oversized_sdp_and_ice_rows() {
        let oversized_sdp = format!(
            "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n{}",
            "a=x\r\n".repeat((MAX_SIGNALING_DESCRIPTION_BYTES / 5) + 2)
        );
        let sdp_err = validate_remote_offer_sdp(&oversized_sdp)
            .expect_err("oversized SDP must fail before transport setup")
            .to_string();
        assert!(sdp_err.contains("exceeds"), "got {sdp_err}");

        let candidate = json!({
            "candidate": format!(
                "candidate:1 1 UDP 2122252543 {} 54400 typ host",
                "x".repeat(MAX_ICE_CANDIDATE_BYTES)
            ),
            "sdpMid": "0",
            "sdpMLineIndex": 0
        });
        let ice_err = remote_ice_candidate_inits(&candidate)
            .expect_err("oversized ICE candidate must fail before storage")
            .to_string();
        assert!(ice_err.contains("exceeds"), "got {ice_err}");
    }

    #[test]
    fn null_and_empty_candidate_are_explicit_end_markers() {
        assert!(remote_ice_candidate_inits(&Value::Null).unwrap().is_empty());
        let empty = json!({"candidate": "", "sdpMid": "0", "sdpMLineIndex": 0});
        let decoded = remote_ice_candidate_inits(&empty).unwrap();
        assert_eq!(decoded[0].candidate, "");
        assert_eq!(ice_candidate_text(&empty).unwrap(), "");
    }

    #[test]
    fn answer_requires_device_to_browser_video_direction() {
        for direction in ["sendonly", "sendrecv"] {
            let sdp = format!(
                "v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
                 a=sendrecv\r\nm=video 9 UDP/TLS/RTP/SAVPF 109\r\na={direction}\r\n"
            );
            ensure_answer_sends_video(&sdp).unwrap();
        }

        for direction in ["recvonly", "inactive"] {
            let sdp = format!("v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 109\r\na={direction}\r\n");
            let err = ensure_answer_sends_video(&sdp).unwrap_err().to_string();
            assert!(err.contains("webrtc_video_sender_not_negotiated"));
            assert!(err.contains(direction));
        }
    }

    #[test]
    fn answer_rejects_missing_video_media_section() {
        let err = ensure_answer_sends_video(
            "v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=sendrecv\r\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("direction=missing"));
    }

    #[test]
    fn audio_sender_is_required_only_when_offer_accepts_audio() {
        let offer = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 109\r\na=recvonly\r\n\
                     m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=recvonly\r\n";
        assert!(remote_offer_accepts_audio(offer));
        let answer = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 109\r\na=sendonly\r\n\
                      m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=sendonly\r\n";
        ensure_answer_sends_audio(answer).unwrap();

        let video_only = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 109\r\na=recvonly\r\n";
        assert!(!remote_offer_accepts_audio(video_only));
        let err = ensure_answer_sends_audio(video_only)
            .unwrap_err()
            .to_string();
        assert!(err.contains("webrtc_audio_sender_not_negotiated"));
    }
}
