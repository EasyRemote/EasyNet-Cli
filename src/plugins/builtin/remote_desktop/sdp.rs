// EasyNet CLI — remote desktop WebRTC SDP/ICE helpers
// ===================================================
//
// File: src/plugins/builtin/remote_desktop/sdp.rs
// Description: Pure SDP and ICE candidate normalization for the direct WebRTC
//              transport path.
//
// Architectural Position:
// - Transport parsing boundary. This module owns string/value normalization
//   only; it does not touch session state, media encoders, or plugin stores.

use serde_json::Value;
use webrtc::peer_connection::RTCIceCandidateInit;

/// Ensure a browser/device SDP carries an explicit end-of-candidates marker.
pub(in crate::plugins::builtin::remote_desktop) fn ensure_sdp_end_of_candidates(
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
pub(in crate::plugins::builtin::remote_desktop) fn normalize_browser_answer_sdp(
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
pub(in crate::plugins::builtin::remote_desktop) fn normalize_remote_offer_sdp(sdp: &str) -> String {
    ensure_sdp_end_of_candidates(sdp)
}

fn is_rtcp_component_candidate(line: &str) -> bool {
    if !line.starts_with("a=candidate:") {
        return false;
    }
    line.split_whitespace().nth(1) == Some("2")
}

/// Decode one JSON ICE candidate value into WebRTC candidate init records.
pub(in crate::plugins::builtin::remote_desktop) fn remote_ice_candidate_inits(
    candidate: &Value,
) -> anyhow::Result<Vec<RTCIceCandidateInit>> {
    if candidate.is_null() {
        return Ok(Vec::new());
    }
    let candidate_init: RTCIceCandidateInit = serde_json::from_value(candidate.clone())?;
    Ok(vec![candidate_init])
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
}
