// EasyNet CLI — native remote desktop media strategy
// ==================================================
//
// File: plugins/remote-desktop/src/media/native.rs
// Description: macOS native WebRTC media helpers for remote desktop.

#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use rtc::statistics::report::{RTCStatsReport, RTCStatsReportEntry};
#[cfg(target_os = "macos")]
use rtc::statistics::stats::ice_candidate::RTCIceCandidateStats;
#[cfg(target_os = "macos")]
use rtc::statistics::StatsSelector;
#[cfg(target_os = "macos")]
use serde_json::{json, Value};
#[cfg(target_os = "macos")]
use webrtc::peer_connection::PeerConnection;

#[cfg(target_os = "macos")]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::VideoResolution;
#[cfg(target_os = "macos")]
use crate::daemon::plugins::remote_desktop::constants::NATIVE_MIN_BITRATE_KBPS;

#[cfg(target_os = "macos")]
const NATIVE_BITRATE_STEP_KBPS: u32 = 500;

/// Adaptive bitrate controller for the native macOS media path.
///
/// Invariant 1: `current_kbps` never falls below `min_kbps`.
/// Invariant 2: drops or queue pressure reduce bitrate before any increase is
/// considered.
/// Invariant 3: small bitrate deltas are suppressed to avoid encoder churn.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeAdaptiveBitrate {
    pub(in crate::daemon::plugins::remote_desktop) target_kbps: u32,
    pub(in crate::daemon::plugins::remote_desktop) current_kbps: u32,
    pub(in crate::daemon::plugins::remote_desktop) min_kbps: u32,
    last_input_dropped: u64,
    last_output_dropped: u64,
}

#[cfg(target_os = "macos")]
impl NativeAdaptiveBitrate {
    pub(in crate::daemon::plugins::remote_desktop) fn new(target_kbps: u32) -> Self {
        let target_kbps = target_kbps.max(NATIVE_MIN_BITRATE_KBPS);
        Self {
            target_kbps,
            current_kbps: target_kbps,
            min_kbps: NATIVE_MIN_BITRATE_KBPS.min(target_kbps),
            last_input_dropped: 0,
            last_output_dropped: 0,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn update(
        &mut self,
        input_dropped: u64,
        output_dropped: u64,
        queued_units: usize,
        in_flight_frames: usize,
        available_outgoing_bitrate_bps: Option<f64>,
    ) -> Option<u32> {
        let input_delta = input_dropped.saturating_sub(self.last_input_dropped);
        let output_delta = output_dropped.saturating_sub(self.last_output_dropped);
        self.last_input_dropped = input_dropped;
        self.last_output_dropped = output_dropped;

        let next = adaptive_bitrate_kbps(
            self.current_kbps,
            self.target_kbps,
            self.min_kbps,
            input_delta.saturating_add(output_delta),
            queued_units,
            in_flight_frames,
            available_outgoing_bitrate_bps,
        );
        if next.abs_diff(self.current_kbps) < NATIVE_BITRATE_STEP_KBPS {
            return None;
        }
        self.current_kbps = next;
        Some(next)
    }
}

#[cfg(target_os = "macos")]
fn adaptive_bitrate_kbps(
    current_kbps: u32,
    target_kbps: u32,
    min_kbps: u32,
    dropped_delta: u64,
    queued_units: usize,
    in_flight_frames: usize,
    available_outgoing_bitrate_bps: Option<f64>,
) -> u32 {
    let min_kbps = min_kbps.min(target_kbps).max(1);
    if dropped_delta > 0 || queued_units > 1 || in_flight_frames > 1 {
        return current_kbps
            .saturating_mul(80)
            .saturating_div(100)
            .max(min_kbps);
    }
    if let Some(available_bps) = available_outgoing_bitrate_bps.filter(|bps| *bps > 0.0) {
        let available_kbps = (available_bps / 1000.0) as u32;
        let ceiling = available_kbps
            .saturating_mul(85)
            .saturating_div(100)
            .max(min_kbps);
        if current_kbps > ceiling {
            return ceiling;
        }
        if current_kbps < target_kbps && available_kbps > current_kbps.saturating_mul(130) / 100 {
            return current_kbps
                .saturating_mul(110)
                .saturating_div(100)
                .min(target_kbps)
                .max(min_kbps);
        }
        return current_kbps;
    }
    if current_kbps < target_kbps {
        return current_kbps
            .saturating_mul(105)
            .saturating_div(100)
            .min(target_kbps)
            .max(min_kbps);
    }
    current_kbps
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct NativeLatencyAccumulator {
    samples: u64,
    total_ms: u64,
    max_ms: u64,
    last_ms: u64,
}

#[cfg(target_os = "macos")]
impl NativeLatencyAccumulator {
    fn record(&mut self, value_ms: u64) {
        self.samples = self.samples.saturating_add(1);
        self.total_ms = self.total_ms.saturating_add(value_ms);
        self.max_ms = self.max_ms.max(value_ms);
        self.last_ms = value_ms;
    }

    fn to_json(&self) -> Value {
        let avg_ms = if self.samples == 0 {
            0.0
        } else {
            self.total_ms as f64 / self.samples as f64
        };
        json!({
            "samples": self.samples,
            "last_ms": self.last_ms,
            "avg_ms": (avg_ms * 10.0).round() / 10.0,
            "max_ms": self.max_ms,
        })
    }
}

/// Latency counters emitted into remote desktop media diagnostics.
///
/// Invariant 1: every field is monotonic and saturating.
/// Invariant 2: conversion to JSON is side-effect free so it can be sampled
/// from the media loop without resetting counters.
#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeLatencyStats {
    encode_submit_to_output: NativeLatencyAccumulator,
    encoder_output_to_rtp_write: NativeLatencyAccumulator,
    encode_submit_to_rtp_write: NativeLatencyAccumulator,
    rtp_write_call: NativeLatencyAccumulator,
}

#[cfg(target_os = "macos")]
impl NativeLatencyStats {
    pub(in crate::daemon::plugins::remote_desktop) fn record_encoded_unit(
        &mut self,
        encode_submitted_at_ms: u64,
        encoded_at_ms: u64,
        encode_latency_ms: u64,
        rtp_write_started_ms: u64,
        rtp_write_finished_ms: u64,
    ) {
        self.encode_submit_to_output.record(encode_latency_ms);
        self.encoder_output_to_rtp_write
            .record(rtp_write_finished_ms.saturating_sub(encoded_at_ms));
        self.encode_submit_to_rtp_write
            .record(rtp_write_finished_ms.saturating_sub(encode_submitted_at_ms));
        self.rtp_write_call
            .record(rtp_write_finished_ms.saturating_sub(rtp_write_started_ms));
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_json(&self) -> Value {
        json!({
            "encode_submit_to_output": self.encode_submit_to_output.to_json(),
            "encoder_output_to_rtp_write": self.encoder_output_to_rtp_write.to_json(),
            "encode_submit_to_rtp_write": self.encode_submit_to_rtp_write.to_json(),
            "rtp_write_call": self.rtp_write_call.to_json(),
        })
    }
}

#[cfg(target_os = "macos")]
pub(in crate::daemon::plugins::remote_desktop) async fn webrtc_stats_snapshot(
    peer_connection: &std::sync::Arc<dyn PeerConnection>,
) -> (Value, Option<f64>) {
    let report = peer_connection
        .get_stats(Instant::now(), StatsSelector::None)
        .await;
    let transport = report.transport();
    let selected_pair_id = transport
        .map(|stats| stats.selected_candidate_pair_id.clone())
        .filter(|id| !id.is_empty());
    let selected_pair = selected_pair_id
        .as_deref()
        .and_then(|id| report.candidate_pairs().find(|pair| pair.stats.id == id))
        .or_else(|| report.candidate_pairs().find(|pair| pair.nominated));
    let local_candidate = selected_pair.and_then(|pair| {
        candidate_stats_for_pair_id(&report, &pair.local_candidate_id, IceCandidateSide::Local)
    });
    let remote_candidate = selected_pair.and_then(|pair| {
        candidate_stats_for_pair_id(&report, &pair.remote_candidate_id, IceCandidateSide::Remote)
    });
    let outbound = report.outbound_rtp_streams().next();
    let remote_inbound = outbound
        .and_then(|out| report.get(&out.remote_id))
        .and_then(|entry| match entry {
            RTCStatsReportEntry::RemoteInboundRtp(stats) => Some(stats),
            _ => None,
        });

    let available_outgoing_bitrate_bps = selected_pair
        .map(|pair| pair.available_outgoing_bitrate)
        .filter(|bps| *bps > 0.0);

    (
        json!({
            "transport": transport.map(|stats| json!({
                "selected_candidate_pair_id": stats.selected_candidate_pair_id,
                "selected_candidate_pair_changes": stats.selected_candidate_pair_changes,
                "dtls_role": format!("{:?}", stats.dtls_role),
                "srtp_cipher": stats.srtp_cipher,
                "ccfb_messages_sent": stats.ccfb_messages_sent,
                "ccfb_messages_received": stats.ccfb_messages_received,
            })),
            "selected_candidate_pair": selected_pair.map(|pair| json!({
                "id": pair.stats.id,
                "local_candidate_id": pair.local_candidate_id,
                "remote_candidate_id": pair.remote_candidate_id,
                "local_candidate_type": candidate_type_value(local_candidate),
                "remote_candidate_type": candidate_type_value(remote_candidate),
                "selected_route_class": selected_candidate_route_class(local_candidate, remote_candidate),
                "protocol": selected_candidate_protocol(local_candidate, remote_candidate),
                "local_candidate_stats_found": local_candidate.is_some(),
                "remote_candidate_stats_found": remote_candidate.is_some(),
                "state": format!("{:?}", pair.state),
                "nominated": pair.nominated,
                "packets_sent": pair.packets_sent,
                "packets_received": pair.packets_received,
                "bytes_sent": pair.bytes_sent,
                "bytes_received": pair.bytes_received,
                "current_round_trip_time_ms": (pair.current_round_trip_time * 1000.0).round(),
                "available_outgoing_bitrate_bps": pair.available_outgoing_bitrate,
                "available_incoming_bitrate_bps": pair.available_incoming_bitrate,
                "packets_discarded_on_send": pair.packets_discarded_on_send,
                "bytes_discarded_on_send": pair.bytes_discarded_on_send,
            })),
            "outbound_rtp": outbound.map(|out| json!({
                "packets_sent": out.sent_rtp_stream_stats.packets_sent,
                "bytes_sent": out.sent_rtp_stream_stats.bytes_sent,
                "target_bitrate_bps": out.target_bitrate,
                "frames_per_second": out.frames_per_second,
                "frames_sent": out.frames_sent,
                "frames_encoded": out.frames_encoded,
                "key_frames_encoded": out.key_frames_encoded,
                "nack_count": out.nack_count,
                "fir_count": out.fir_count,
                "pli_count": out.pli_count,
                "quality_limitation_reason": format!("{:?}", out.quality_limitation_reason),
            })),
            "remote_inbound_rtp": remote_inbound.map(|remote| json!({
                "round_trip_time_ms": (remote.round_trip_time * 1000.0).round(),
                "fraction_lost": remote.fraction_lost,
                "round_trip_time_measurements": remote.round_trip_time_measurements,
                "packets_lost": remote.received_rtp_stream_stats.packets_lost,
                "jitter_ms": (remote.received_rtp_stream_stats.jitter * 1000.0).round(),
            })),
        }),
        available_outgoing_bitrate_bps,
    )
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum IceCandidateSide {
    Local,
    Remote,
}

#[cfg(target_os = "macos")]
impl IceCandidateSide {
    fn report_id(self, pair_candidate_id: &str) -> String {
        match self {
            Self::Local => format!("RTCLocalIceCandidate_{pair_candidate_id}"),
            Self::Remote => format!("RTCRemoteIceCandidate_{pair_candidate_id}"),
        }
    }
}

#[cfg(target_os = "macos")]
fn candidate_stats_for_pair_id<'a>(
    report: &'a RTCStatsReport,
    pair_candidate_id: &str,
    side: IceCandidateSide,
) -> Option<&'a RTCIceCandidateStats> {
    let report_id = side.report_id(pair_candidate_id);
    candidate_stats_entry(report, pair_candidate_id, side)
        .or_else(|| candidate_stats_entry(report, &report_id, side))
}

#[cfg(target_os = "macos")]
fn candidate_stats_entry<'a>(
    report: &'a RTCStatsReport,
    candidate_id: &str,
    side: IceCandidateSide,
) -> Option<&'a RTCIceCandidateStats> {
    match (side, report.get(candidate_id)) {
        (IceCandidateSide::Local, Some(RTCStatsReportEntry::LocalCandidate(stats))) => Some(stats),
        (IceCandidateSide::Remote, Some(RTCStatsReportEntry::RemoteCandidate(stats))) => {
            Some(stats)
        }
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn candidate_type_value(candidate: Option<&RTCIceCandidateStats>) -> Option<String> {
    candidate
        .map(|stats| stats.candidate_type.to_string())
        .filter(|candidate_type| candidate_type != "Unspecified")
}

#[cfg(target_os = "macos")]
fn selected_candidate_route_class(
    local_candidate: Option<&RTCIceCandidateStats>,
    remote_candidate: Option<&RTCIceCandidateStats>,
) -> Option<&'static str> {
    let candidate_types: Vec<_> = [local_candidate, remote_candidate]
        .into_iter()
        .flatten()
        .filter_map(|candidate| candidate_type_value(Some(candidate)))
        .collect();
    if candidate_types
        .iter()
        .any(|candidate_type| candidate_type == "relay")
    {
        Some("relay")
    } else if candidate_types
        .iter()
        .any(|candidate_type| matches!(candidate_type.as_str(), "srflx" | "prflx"))
    {
        Some("stun_srflx")
    } else if candidate_types.len() == 2
        && candidate_types
            .iter()
            .all(|candidate_type| candidate_type == "host")
    {
        Some("direct")
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn selected_candidate_protocol(
    local_candidate: Option<&RTCIceCandidateStats>,
    remote_candidate: Option<&RTCIceCandidateStats>,
) -> Option<String> {
    [local_candidate, remote_candidate]
        .into_iter()
        .flatten()
        .map(|stats| stats.protocol.trim().to_ascii_lowercase())
        .find(|protocol| !protocol.is_empty())
}

#[cfg(target_os = "macos")]
pub(in crate::daemon::plugins::remote_desktop) fn latest_native_rtp_units(
    units: Vec<crate::daemon::plugins::remote_desktop::videotoolbox_encoder::EncodedAccessUnit>,
    decoder_primed: bool,
) -> (
    Vec<crate::daemon::plugins::remote_desktop::videotoolbox_encoder::EncodedAccessUnit>,
    usize,
) {
    if units.len() <= 1 {
        return (units, 0);
    }
    let selected_index = if decoder_primed {
        units.len() - 1
    } else {
        units
            .iter()
            .rposition(|unit| unit.is_keyframe)
            .unwrap_or(units.len() - 1)
    };
    let dropped = units.len().saturating_sub(1);
    let mut selected = None;
    for (index, unit) in units.into_iter().enumerate() {
        if index == selected_index {
            selected = Some(unit);
            break;
        }
    }
    (selected.into_iter().collect(), dropped)
}

#[cfg(target_os = "macos")]
pub(in crate::daemon::plugins::remote_desktop) fn is_webrtc_sender_backpressure(
    err: &impl std::fmt::Display,
) -> bool {
    let message = err.to_string();
    message.contains("SenderRtp") && message.contains("Full(")
}

#[cfg(target_os = "macos")]
pub(in crate::daemon::plugins::remote_desktop) fn native_rtp_sample_duration(
    last_written_pts_ms: Option<u64>,
    pts_ms: u64,
    nominal: Duration,
) -> Duration {
    const MIN_SAMPLE_DURATION_MS: u64 = 1;
    const MAX_SAMPLE_DURATION_MS: u64 = 250;

    let Some(last_pts_ms) = last_written_pts_ms else {
        return nominal;
    };
    let delta_ms = pts_ms.saturating_sub(last_pts_ms);
    if delta_ms == 0 {
        return nominal;
    }
    Duration::from_millis(delta_ms.clamp(MIN_SAMPLE_DURATION_MS, MAX_SAMPLE_DURATION_MS))
}

#[cfg(target_os = "macos")]
pub(in crate::daemon::plugins::remote_desktop) fn native_capture_dimensions(
    options: &crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions,
    native_dimensions: impl FnOnce() -> anyhow::Result<(usize, usize)>,
) -> anyhow::Result<(usize, usize)> {
    let native = native_dimensions()?;
    let (width, height) = match options.resolution {
        Some(requested) => fit_resolution_to_native_aspect(native, requested),
        None => native,
    };
    Ok((width.max(2), height.max(2)))
}

#[cfg(target_os = "macos")]
fn fit_resolution_to_native_aspect(
    (native_width, native_height): (usize, usize),
    requested: VideoResolution,
) -> (usize, usize) {
    let native_width = native_width.max(2);
    let native_height = native_height.max(2);
    let max_width = (requested.width as usize).max(2);
    let max_height = (requested.height as usize).max(2);
    let scale = (max_width as f64 / native_width as f64)
        .min(max_height as f64 / native_height as f64)
        .min(1.0);
    let width = even_dimension((native_width as f64 * scale).round() as usize);
    let height = even_dimension((native_height as f64 * scale).round() as usize);
    (width, height)
}

#[cfg(target_os = "macos")]
fn even_dimension(value: usize) -> usize {
    let value = value.max(2);
    if value.is_multiple_of(2) {
        value
    } else {
        value - 1
    }
    .max(2)
}

#[cfg(target_os = "macos")]
pub(in crate::daemon::plugins::remote_desktop) fn webrtc_cmtime(
    value: u64,
    fps: u32,
) -> objc2_core_media::CMTime {
    objc2_core_media::CMTime {
        value: value as i64,
        timescale: fps.max(1) as i32,
        flags: objc2_core_media::CMTimeFlags::Valid,
        epoch: 0,
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    use rtc::peer_connection::transport::{
        RTCIceCandidateType, RTCIceServerTransportProtocol, RTCIceTcpCandidateType,
    };
    use rtc::statistics::stats::ice_candidate::RTCIceCandidateStats;
    use rtc::statistics::stats::{RTCStats, RTCStatsType};
    use std::time::Instant;

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
    use crate::daemon::plugins::remote_desktop::videotoolbox_encoder::EncodedAccessUnit;

    fn native_unit(pts_ms: u64, is_keyframe: bool) -> EncodedAccessUnit {
        EncodedAccessUnit {
            annexb: vec![pts_ms as u8],
            is_keyframe,
            pts_ms,
            encode_submitted_at_ms: pts_ms,
            encoded_at_ms: pts_ms,
            encode_latency_ms: 0,
        }
    }

    fn ice_candidate_stat(
        id: &str,
        stats_type: RTCStatsType,
        candidate_type: RTCIceCandidateType,
        protocol: &str,
    ) -> RTCIceCandidateStats {
        RTCIceCandidateStats {
            stats: RTCStats {
                timestamp: Instant::now(),
                typ: stats_type,
                id: id.to_string(),
            },
            transport_id: "transport".to_string(),
            address: Some("192.0.2.1".to_string()),
            port: 3478,
            protocol: protocol.to_string(),
            candidate_type,
            priority: 1,
            url: "turn:secret.example.test".to_string(),
            relay_protocol: RTCIceServerTransportProtocol::Unspecified,
            foundation: "foundation".to_string(),
            related_address: String::new(),
            related_port: 0,
            username_fragment: String::new(),
            tcp_type: RTCIceTcpCandidateType::Unspecified,
        }
    }

    #[test]
    fn native_rtp_drain_sends_latest_unit_after_decoder_is_primed() {
        let (selected, dropped) = latest_native_rtp_units(
            vec![
                native_unit(1, true),
                native_unit(2, false),
                native_unit(3, false),
            ],
            true,
        );

        assert_eq!(dropped, 2);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].pts_ms, 3);
    }

    #[test]
    fn selected_candidate_pair_route_evidence_uses_typed_candidate_stats() {
        let local = ice_candidate_stat(
            "local",
            RTCStatsType::LocalCandidate,
            RTCIceCandidateType::Relay,
            "UDP",
        );
        let remote = ice_candidate_stat(
            "remote",
            RTCStatsType::RemoteCandidate,
            RTCIceCandidateType::Srflx,
            "udp",
        );

        assert_eq!(
            candidate_type_value(Some(&local)),
            Some("relay".to_string())
        );
        assert_eq!(
            candidate_type_value(Some(&remote)),
            Some("srflx".to_string())
        );
        assert_eq!(
            selected_candidate_protocol(Some(&local), Some(&remote)),
            Some("udp".to_string())
        );
        assert_eq!(
            selected_candidate_route_class(Some(&local), Some(&remote)),
            Some("relay")
        );
    }

    #[test]
    fn selected_candidate_pair_ids_map_to_rtc_candidate_report_ids() {
        assert_eq!(
            IceCandidateSide::Local.report_id("candidate:local-id"),
            "RTCLocalIceCandidate_candidate:local-id"
        );
        assert_eq!(
            IceCandidateSide::Remote.report_id("candidate:remote-id"),
            "RTCRemoteIceCandidate_candidate:remote-id"
        );
    }

    #[test]
    fn selected_candidate_pair_route_evidence_does_not_guess_missing_stats() {
        let local_host = ice_candidate_stat(
            "local-host",
            RTCStatsType::LocalCandidate,
            RTCIceCandidateType::Host,
            "udp",
        );
        assert_eq!(candidate_type_value(None), None);
        assert_eq!(selected_candidate_route_class(None, None), None);
        assert_eq!(
            selected_candidate_route_class(Some(&local_host), None),
            None
        );
        assert_eq!(selected_candidate_protocol(None, None), None);
    }

    #[test]
    fn selected_candidate_pair_route_class_distinguishes_direct_and_stun() {
        let local_host = ice_candidate_stat(
            "local-host",
            RTCStatsType::LocalCandidate,
            RTCIceCandidateType::Host,
            "udp",
        );
        let remote_host = ice_candidate_stat(
            "remote-host",
            RTCStatsType::RemoteCandidate,
            RTCIceCandidateType::Host,
            "udp",
        );
        let remote_srflx = ice_candidate_stat(
            "remote-srflx",
            RTCStatsType::RemoteCandidate,
            RTCIceCandidateType::Srflx,
            "udp",
        );

        assert_eq!(
            selected_candidate_route_class(Some(&local_host), Some(&remote_host)),
            Some("direct")
        );
        assert_eq!(
            selected_candidate_route_class(Some(&local_host), Some(&remote_srflx)),
            Some("stun_srflx")
        );
    }

    #[test]
    fn native_rtp_drain_primes_decoder_with_latest_keyframe() {
        let (selected, dropped) = latest_native_rtp_units(
            vec![
                native_unit(1, false),
                native_unit(2, true),
                native_unit(3, false),
            ],
            false,
        );

        assert_eq!(dropped, 2);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].pts_ms, 2);
        assert!(selected[0].is_keyframe);
    }

    #[test]
    fn native_capture_dimensions_preserve_desktop_aspect_inside_requested_bounds() {
        let options = ScreenCaptureOptions {
            resolution: Some(VideoResolution {
                width: 1920,
                height: 1080,
            }),
            fps: 144,
            region: None,
        };

        let (width, height) =
            native_capture_dimensions(&options, || Ok((2560, 1664))).expect("dimensions");

        assert_eq!((width, height), (1662, 1080));
    }

    #[test]
    fn native_capture_dimensions_do_not_upscale_small_desktops() {
        let options = ScreenCaptureOptions {
            resolution: Some(VideoResolution {
                width: 1920,
                height: 1080,
            }),
            fps: 144,
            region: None,
        };

        let (width, height) =
            native_capture_dimensions(&options, || Ok((1512, 982))).expect("dimensions");

        assert_eq!((width, height), (1512, 982));
    }

    #[test]
    fn native_rtp_sample_duration_follows_capture_pts_gap() {
        let duration = native_rtp_sample_duration(Some(1_000), 1_042, Duration::from_micros(6_944));

        assert_eq!(duration, Duration::from_millis(42));
    }

    #[test]
    fn native_rtp_sample_duration_uses_nominal_for_first_or_non_monotonic_pts() {
        let nominal = Duration::from_micros(6_944);

        assert_eq!(native_rtp_sample_duration(None, 1_000, nominal), nominal);
        assert_eq!(
            native_rtp_sample_duration(Some(1_000), 900, nominal),
            nominal
        );
    }
}
