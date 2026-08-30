// EasyNet CLI — remote desktop client-media report handler
// =========================================================

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_REPORT_CLIENT_STATE;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;

const MAX_CLIENT_EVIDENCE_STRING_LEN: usize = 256;

pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_REPORT_CLIENT_STATE)?;
    let state = require_str(&args, "state", ABILITY_REPORT_CLIENT_STATE)?;
    if !matches!(state, "presenting" | "stalled" | "detached") {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: "state must be presenting, stalled, or detached".to_string(),
        }
        .into());
    }
    let epoch = args
        .get("transport_epoch")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: "positive transport_epoch is required".to_string(),
        })?;
    let client_media_stats = client_media_stats_from_args(&args, state)?;
    plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<Value> {
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_REPORT_CLIENT_STATE,
                    session_id: session_id.to_string(),
                }
            })?;
            ensure_session_control_access(
                &plugin,
                ABILITY_REPORT_CLIENT_STATE,
                &env,
                &args,
                session,
            )?;
            if let Some(stats) = client_media_stats.as_ref() {
                validate_client_media_stats_binding(session, epoch, stats)?;
            }
            if !session.report_client_media_state(
                TransportEpoch::new(epoch),
                state,
                client_media_stats,
            ) {
                return Err(RemoteDesktopError::TransportEpochMismatch {
                    ability: ABILITY_REPORT_CLIENT_STATE,
                    epoch,
                }
                .into());
            }
            Ok(plugin.session_view(session))
        })
}

fn client_media_stats_from_args(args: &Value, state: &str) -> anyhow::Result<Option<Value>> {
    let mut stats = Map::new();
    let mut webrtc = Map::new();
    if let Some(client_transport) = optional_object(args, "client_transport")? {
        copy_string(client_transport, &mut webrtc, "ice_connection_state")?;
        copy_string(client_transport, &mut webrtc, "peer_connection_state")?;
        copy_string(client_transport, &mut webrtc, "route_kind")?;
        copy_number(client_transport, &mut webrtc, "sampled_at_ms")?;
        if let Some(pair) = optional_nested_object(client_transport, "selected_candidate_pair")? {
            let mut selected_pair = Map::new();
            copy_selected_pair_id(pair, &mut selected_pair)?;
            copy_string(pair, &mut selected_pair, "local_candidate_id")?;
            copy_string(pair, &mut selected_pair, "remote_candidate_id")?;
            copy_string(pair, &mut selected_pair, "local_candidate_type")?;
            copy_string(pair, &mut selected_pair, "remote_candidate_type")?;
            copy_string(pair, &mut selected_pair, "selected_route_class")?;
            copy_string(pair, &mut selected_pair, "protocol")?;
            copy_string(pair, &mut selected_pair, "state")?;
            copy_bool(pair, &mut selected_pair, "selected");
            copy_bool(pair, &mut selected_pair, "nominated");
            copy_number(pair, &mut selected_pair, "current_round_trip_time_ms")?;
            copy_number(pair, &mut selected_pair, "available_outgoing_bitrate_bps")?;
            copy_number(pair, &mut selected_pair, "available_incoming_bitrate_bps")?;
            copy_number(pair, &mut selected_pair, "packets_discarded_on_send")?;
            copy_number(pair, &mut selected_pair, "bytes_discarded_on_send")?;
            if !selected_pair.is_empty() {
                webrtc.insert(
                    "selected_candidate_pair".to_string(),
                    Value::Object(selected_pair),
                );
            }
        }
    }
    if !webrtc.is_empty() {
        stats.insert("webrtc_stats".to_string(), Value::Object(webrtc));
    }
    if let Some(browser_stats) = optional_object(args, "browser_stats")? {
        let mut browser = Map::new();
        copy_number(browser_stats, &mut browser, "sampled_at_ms")?;
        copy_number(browser_stats, &mut browser, "frames_decoded")?;
        copy_number(browser_stats, &mut browser, "frames_dropped")?;
        copy_number(browser_stats, &mut browser, "frames_received")?;
        copy_number(browser_stats, &mut browser, "frame_width")?;
        copy_number(browser_stats, &mut browser, "frame_height")?;
        copy_number(browser_stats, &mut browser, "jitter_buffer_avg_ms")?;
        copy_number(browser_stats, &mut browser, "jitter_buffer_target_avg_ms")?;
        copy_number(browser_stats, &mut browser, "decode_avg_ms")?;
        copy_number(browser_stats, &mut browser, "processing_avg_ms")?;
        copy_number(browser_stats, &mut browser, "freeze_count")?;
        copy_number(browser_stats, &mut browser, "pause_count")?;
        if !browser.is_empty() {
            stats.insert("browser_stats".to_string(), Value::Object(browser));
        }
    }
    if let Some(render_probe) = optional_object(args, "render_probe")? {
        let mut probe = Map::new();
        copy_string(render_probe, &mut probe, "probe_source")?;
        copy_string(render_probe, &mut probe, "selected_resource_ura")?;
        copy_string(render_probe, &mut probe, "session_id")?;
        copy_number(render_probe, &mut probe, "transport_epoch")?;
        copy_string(render_probe, &mut probe, "binding_id")?;
        copy_number(render_probe, &mut probe, "binding_epoch")?;
        copy_number(render_probe, &mut probe, "media_source_epoch")?;
        copy_string(render_probe, &mut probe, "media_pipeline_id")?;
        copy_string(render_probe, &mut probe, "video_codec")?;
        copy_string(render_probe, &mut probe, "video_transport")?;
        copy_string(render_probe, &mut probe, "audio_codec")?;
        copy_number(render_probe, &mut probe, "observed_at_ms")?;
        copy_number(render_probe, &mut probe, "decoded_video_frames")?;
        copy_number(render_probe, &mut probe, "decoded_audio_packets")?;
        copy_number(render_probe, &mut probe, "decoded_audio_samples")?;
        copy_string(render_probe, &mut probe, "video_payload_hash")?;
        copy_string(render_probe, &mut probe, "audio_payload_hash")?;
        copy_number(render_probe, &mut probe, "frame_width")?;
        copy_number(render_probe, &mut probe, "frame_height")?;
        if !probe.is_empty() {
            probe.insert("evidence_authority".to_string(), json!("client_reported"));
            stats.insert("render_probe".to_string(), Value::Object(probe));
        }
    }
    if stats.is_empty() {
        return Ok(None);
    }
    stats.insert("client_reported_state".to_string(), json!(state));
    Ok(Some(Value::Object(stats)))
}

fn validate_client_media_stats_binding(
    session: &RemoteDesktopSession,
    transport_epoch: u64,
    stats: &Value,
) -> anyhow::Result<()> {
    let Some(probe) = stats.get("render_probe") else {
        return Ok(());
    };
    let probe = probe
        .as_object()
        .ok_or_else(|| invalid_client_evidence("render_probe must be an object"))?;
    require_probe_string(probe, "probe_source", "browser_webrtc_receiver")?;
    require_probe_string(probe, "session_id", session.session_id())?;
    require_probe_string(
        probe,
        "selected_resource_ura",
        session.target_binding().subject_ura(),
    )?;
    require_probe_u64(probe, "transport_epoch", transport_epoch)?;
    require_probe_string(probe, "binding_id", session.target_binding().binding_id())?;
    require_probe_u64(
        probe,
        "binding_epoch",
        session.target_binding().binding_epoch(),
    )?;
    require_probe_u64(
        probe,
        "media_source_epoch",
        session.target_binding().media_source_epoch(),
    )?;

    let current_stats = session.media_stats().ok_or_else(|| {
        invalid_client_evidence(
            "render_probe requires current device media stats for pipeline correlation",
        )
    })?;
    let expected_pipeline = current_stats
        .get("media_pipeline_id")
        .or_else(|| current_stats.get("backend_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            invalid_client_evidence("render_probe requires a current device media_pipeline_id")
        })?;
    require_probe_string(probe, "media_pipeline_id", expected_pipeline)?;
    let expected_video_codec = current_stats
        .get("video_codec")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_client_evidence("current device video_codec is required"))?;
    require_probe_string(probe, "video_codec", expected_video_codec)?;
    let expected_video_transport = current_stats
        .get("video_transport")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_client_evidence("current device video_transport is required"))?;
    require_probe_string(probe, "video_transport", expected_video_transport)?;
    require_positive_probe_u64(probe, "observed_at_ms")?;
    require_positive_probe_u64(probe, "decoded_video_frames")?;
    require_positive_probe_u64(probe, "frame_width")?;
    require_positive_probe_u64(probe, "frame_height")?;

    if session
        .negotiated_media_scope()
        .is_some_and(|scope| scope.requires_audio())
    {
        let expected_audio_codec = current_stats
            .get("audio_codec")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_client_evidence("current device audio_codec is required"))?;
        require_probe_string(probe, "audio_codec", expected_audio_codec)?;
    }

    if let Some(previous) = current_stats.get("render_probe").and_then(Value::as_object) {
        for field in [
            "decoded_video_frames",
            "decoded_audio_packets",
            "decoded_audio_samples",
        ] {
            let Some(previous_value) = previous.get(field).and_then(Value::as_u64) else {
                continue;
            };
            let current_value = probe.get(field).and_then(Value::as_u64).ok_or_else(|| {
                invalid_client_evidence(&format!(
                    "render_probe.{field} is required after it was previously reported"
                ))
            })?;
            let regressed = if field == "decoded_video_frames" {
                current_value <= previous_value
            } else {
                current_value < previous_value
            };
            if regressed {
                return Err(invalid_client_evidence(&format!(
                    "render_probe.{field} must be {}",
                    if field == "decoded_video_frames" {
                        "strictly increasing"
                    } else {
                        "monotonic"
                    }
                )));
            }
        }
    }
    Ok(())
}

fn require_probe_string(
    probe: &Map<String, Value>,
    field: &str,
    expected: &str,
) -> anyhow::Result<()> {
    let actual = probe
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_client_evidence(&format!("render_probe.{field} is required")))?;
    if actual != expected {
        return Err(invalid_client_evidence(&format!(
            "render_probe.{field} does not match the active session"
        )));
    }
    Ok(())
}

fn require_probe_u64(probe: &Map<String, Value>, field: &str, expected: u64) -> anyhow::Result<()> {
    let actual = probe
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_client_evidence(&format!("render_probe.{field} is required")))?;
    if actual != expected {
        return Err(invalid_client_evidence(&format!(
            "render_probe.{field} does not match the active session"
        )));
    }
    Ok(())
}

fn require_positive_probe_u64(probe: &Map<String, Value>, field: &str) -> anyhow::Result<u64> {
    probe
        .get(field)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            invalid_client_evidence(&format!("render_probe.{field} must be a positive integer"))
        })
}

fn invalid_client_evidence(detail: &str) -> anyhow::Error {
    RemoteDesktopError::InvalidArgument {
        ability: ABILITY_REPORT_CLIENT_STATE,
        detail: detail.to_string(),
    }
    .into()
}

fn optional_object<'a>(
    value: &'a Value,
    field: &str,
) -> anyhow::Result<Option<&'a Map<String, Value>>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    optional_nested_object(object, field)
}

fn optional_nested_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> anyhow::Result<Option<&'a Map<String, Value>>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(value)) => Ok(Some(value)),
        Some(_) => Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: format!("{field} must be an object"),
        }
        .into()),
    }
}

fn copy_selected_pair_id(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
) -> anyhow::Result<()> {
    let pair_id = source
        .get("candidate_pair_id")
        .or_else(|| source.get("id"))
        .and_then(Value::as_str);
    let Some(pair_id) = pair_id else {
        return Ok(());
    };
    validate_string("selected_candidate_pair.candidate_pair_id", pair_id)?;
    target.insert("id".to_string(), json!(pair_id));
    target.insert("candidate_pair_id".to_string(), json!(pair_id));
    Ok(())
}

fn copy_string(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    field: &str,
) -> anyhow::Result<()> {
    let Some(value) = source.get(field) else {
        return Ok(());
    };
    let Some(value) = value.as_str() else {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: format!("{field} must be a string"),
        }
        .into());
    };
    validate_string(field, value)?;
    target.insert(field.to_string(), json!(value));
    Ok(())
}

fn validate_string(field: &str, value: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > MAX_CLIENT_EVIDENCE_STRING_LEN {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: format!("{field} must be a non-empty bounded string"),
        }
        .into());
    }
    Ok(())
}

fn copy_bool(source: &Map<String, Value>, target: &mut Map<String, Value>, field: &str) {
    if let Some(value) = source.get(field).and_then(Value::as_bool) {
        target.insert(field.to_string(), json!(value));
    }
}

fn copy_number(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    field: &str,
) -> anyhow::Result<()> {
    let Some(value) = source.get(field) else {
        return Ok(());
    };
    if !value.is_number() {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_REPORT_CLIENT_STATE,
            detail: format!("{field} must be a number"),
        }
        .into());
    }
    target.insert(field.to_string(), value.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

    const TEST_MEDIA_PIPELINE_ID: &str = "plugin.macos.screencapturekit.videotoolbox.webrtc.v1";

    fn client_evidence_session() -> (RemoteDesktopSession, TransportEpoch) {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-client-evidence",
            "easynet:///r/acme/resource/display.client-evidence",
            vec!["webrtc".to_string()],
        ));
        let epoch = TransportEpoch::new(7);
        assert!(session.begin_webrtc_negotiation(epoch));
        session.record_media_stats(
            epoch,
            json!({
                "media_pipeline_id": TEST_MEDIA_PIPELINE_ID,
                "backend_id": TEST_MEDIA_PIPELINE_ID,
                "video_codec": "h264",
                "video_transport": "webrtc",
            }),
        );
        (session, epoch)
    }

    fn valid_render_probe(session: &RemoteDesktopSession, epoch: TransportEpoch) -> Value {
        json!({
            "probe_source": "browser_webrtc_receiver",
            "selected_resource_ura": session.target_binding().subject_ura(),
            "session_id": session.session_id(),
            "transport_epoch": epoch.value(),
            "binding_id": session.target_binding().binding_id(),
            "binding_epoch": session.target_binding().binding_epoch(),
            "media_source_epoch": session.target_binding().media_source_epoch(),
            "media_pipeline_id": TEST_MEDIA_PIPELINE_ID,
            "video_codec": "h264",
            "video_transport": "webrtc",
            "observed_at_ms": 200u64,
            "decoded_video_frames": 20u64,
            "decoded_audio_packets": 8u64,
            "decoded_audio_samples": 7_680u64,
            "frame_width": 1280u64,
            "frame_height": 720u64,
        })
    }

    #[test]
    fn client_transport_evidence_is_bounded_and_canonicalized() {
        let stats = client_media_stats_from_args(
            &json!({
                "client_transport": {
                    "ice_connection_state": "connected",
                    "peer_connection_state": "connected",
                    "selected_candidate_pair": {
                        "candidate_pair_id": "pair-1",
                        "local_candidate_id": "local-1",
                        "remote_candidate_id": "remote-1",
                        "local_candidate_type": "host",
                        "remote_candidate_type": "srflx",
                        "selected_route_class": "stun_srflx",
                        "protocol": "udp",
                        "state": "succeeded",
                        "selected": true,
                        "nominated": true,
                        "current_round_trip_time_ms": 12
                    }
                },
                "browser_stats": {
                    "frames_decoded": 144,
                    "frame_width": 1280,
                    "frame_height": 720,
                    "decode_avg_ms": 4.5
                },
                "render_probe": {
                    "probe_source": "browser_webrtc_receiver",
                    "selected_resource_ura": "easynet:///r/acme/resource/window.client",
                    "session_id": "rd-client",
                    "transport_epoch": 9,
                    "binding_id": "binding-client",
                    "binding_epoch": 3,
                    "media_source_epoch": 4,
                    "media_pipeline_id": "plugin.macos.screencapturekit.videotoolbox.webrtc.v1",
                    "video_codec": "h264",
                    "video_transport": "webrtc",
                    "observed_at_ms": 1787470677805u64,
                    "decoded_video_frames": 144,
                    "frame_width": 1280,
                    "frame_height": 720
                }
            }),
            "presenting",
        )
        .expect("client stats normalize")
        .expect("stats emitted");

        assert_eq!(stats["client_reported_state"], json!("presenting"));
        assert_eq!(
            stats["webrtc_stats"]["selected_candidate_pair"]["id"],
            json!("pair-1")
        );
        assert_eq!(
            stats["webrtc_stats"]["selected_candidate_pair"]["candidate_pair_id"],
            json!("pair-1")
        );
        assert_eq!(
            stats["webrtc_stats"]["selected_candidate_pair"]["selected"],
            json!(true)
        );
        assert_eq!(stats["browser_stats"]["frames_decoded"], json!(144));
        assert_eq!(stats["browser_stats"]["decode_avg_ms"], json!(4.5));
        assert_eq!(
            stats["render_probe"]["probe_source"],
            json!("browser_webrtc_receiver")
        );
        assert_eq!(
            stats["render_probe"]["media_pipeline_id"],
            json!("plugin.macos.screencapturekit.videotoolbox.webrtc.v1")
        );
        assert_eq!(stats["render_probe"]["decoded_video_frames"], json!(144));
        assert_eq!(
            stats["render_probe"]["evidence_authority"],
            json!("client_reported")
        );
    }

    #[test]
    fn client_transport_evidence_rejects_unbounded_string_fields() {
        let err = client_media_stats_from_args(
            &json!({
                "client_transport": {
                    "selected_candidate_pair": {
                        "candidate_pair_id": "x".repeat(MAX_CLIENT_EVIDENCE_STRING_LEN + 1)
                    }
                }
            }),
            "presenting",
        )
        .expect_err("oversized client evidence must fail closed")
        .to_string();

        assert!(err.contains("candidate_pair_id"), "got {err}");
    }

    #[test]
    fn render_probe_requires_exact_active_session_binding_tuple() {
        let (session, epoch) = client_evidence_session();
        let valid = valid_render_probe(&session, epoch);
        validate_client_media_stats_binding(
            &session,
            epoch.value(),
            &json!({"render_probe": valid.clone()}),
        )
        .expect("exact active binding tuple is accepted");

        let mismatches = [
            ("session_id", json!("rd-other")),
            (
                "selected_resource_ura",
                json!("easynet:///r/acme/resource/display.other"),
            ),
            ("transport_epoch", json!(epoch.value() + 1)),
            ("binding_id", json!("binding-other")),
            (
                "binding_epoch",
                json!(session.target_binding().binding_epoch() + 1),
            ),
            (
                "media_source_epoch",
                json!(session.target_binding().media_source_epoch() + 1),
            ),
            ("media_pipeline_id", json!("pipeline-other")),
        ];
        for (field, mismatch) in mismatches {
            let mut probe = valid.clone();
            probe[field] = mismatch;
            let error = validate_client_media_stats_binding(
                &session,
                epoch.value(),
                &json!({"render_probe": probe}),
            )
            .expect_err("mismatched evidence tuple must fail closed")
            .to_string();
            assert!(error.contains(field), "field={field}, error={error}");
        }
    }

    #[test]
    fn render_probe_rejects_replay_and_counter_regression() {
        let (mut session, epoch) = client_evidence_session();
        let mut previous = valid_render_probe(&session, epoch);
        previous["evidence_authority"] = json!("client_reported");
        assert!(session.merge_client_media_stats(
            epoch,
            json!({
                "render_probe": previous.clone(),
            }),
        ));

        let mut replay = previous.clone();
        replay["observed_at_ms"] = json!(201u64);
        let replay_error = validate_client_media_stats_binding(
            &session,
            epoch.value(),
            &json!({"render_probe": replay}),
        )
        .expect_err("unchanged decoded video counters are replayable evidence")
        .to_string();
        assert!(
            replay_error.contains("decoded_video_frames"),
            "got {replay_error}"
        );

        for (field, lower) in [
            ("decoded_video_frames", 19u64),
            ("decoded_audio_packets", 7u64),
            ("decoded_audio_samples", 7_679u64),
        ] {
            let mut regressed = previous.clone();
            regressed["observed_at_ms"] = json!(201u64);
            if field != "decoded_video_frames" {
                regressed["decoded_video_frames"] = json!(21u64);
            }
            regressed[field] = json!(lower);
            let error = validate_client_media_stats_binding(
                &session,
                epoch.value(),
                &json!({"render_probe": regressed}),
            )
            .expect_err("decoded media counters must not regress")
            .to_string();
            assert!(error.contains(field), "field={field}, error={error}");
        }
    }
}
