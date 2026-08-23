// EasyNet CLI — remote desktop client-media report handler
// =========================================================

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_REPORT_CLIENT_STATE;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::view::serialize_session;

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
            Ok(serialize_session(session))
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
            stats.insert("render_probe".to_string(), Value::Object(probe));
        }
    }
    if stats.is_empty() {
        return Ok(None);
    }
    stats.insert("client_reported_state".to_string(), json!(state));
    Ok(Some(Value::Object(stats)))
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
}
