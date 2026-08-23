#!/usr/bin/env bash
# RemoteApp media adaptation E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by a real RemoteApp media runner.
# - It does not encode, capture, or simulate media. A live pass requires either
#   --evidence-json from an external runner or --runner-cmd that writes the
#   evidence JSON path provided through
#   EASYNET_REMOTEAPP_MEDIA_ADAPTATION_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=skip
OUT_DIR="${EASYNET_REMOTEAPP_MEDIA_ADAPTATION_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-media-adaptation/$(date -u +%Y%m%d-%H%M%S)-$$}"
RUNNER_CMD="${EASYNET_REMOTEAPP_MEDIA_ADAPTATION_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_MEDIA_ADAPTATION_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  remoteapp-media-adaptation-e2e.sh --run --evidence-json PATH
  remoteapp-media-adaptation-e2e.sh --run --runner-cmd CMD
  remoteapp-media-adaptation-e2e.sh --self-test

Options:
  --run                 Verify real RemoteApp media adaptation evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --runner-cmd CMD      Command that drives real media scenarios and writes
                        evidence to EASYNET_REMOTEAPP_MEDIA_ADAPTATION_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by a real media runner.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_MEDIA_ADAPTATION_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove a real media adaptation matrix, not source-only
  codec configuration. It must include baseline, degraded_network, and
  backpressure scenarios over the same selected Resource URA and media pipeline
  identity, with negotiated video codec, host audio, FPS/bitrate telemetry,
  adaptation/drop/backpressure evidence, rendered media, public RemoteApp
  session abilities, selected Resource URA subject binding, and a visible
  terminal receipt.

Non-claims:
  A skipped report or self-test does not prove media product readiness. This
  harness verifies one media artifact; OS capture, input injection, network
  fallback, frontend Browser/Tauri lifecycle, and cross-device product behavior
  still require their own evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_MEDIA_ADAPTATION_E2E:-0}" == "1" ]]; then
  MODE=run
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --runner-cmd) RUNNER_CMD="${2:?missing value for --runner-cmd}"; shift 2 ;;
    --evidence-json) EVIDENCE_INPUT="${2:?missing value for --evidence-json}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

mkdir -p "$OUT_DIR"
EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
RUNNER_STDOUT="$OUT_DIR/runner.stdout.txt"
RUNNER_STDERR="$OUT_DIR/runner.stderr.txt"

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, evidence_path = sys.argv[1:6]
report = {
    "script": "tools/scripts/remoteapp-media-adaptation-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "coverage": {
        "baseline": False,
        "degraded_network": False,
        "backpressure": False,
    },
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Media Adaptation E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n"
    f"- Evidence: `{evidence_path}`\n",
    encoding="utf-8",
)
PY
}

validate_evidence() {
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json
import pathlib
import sys

evidence_path, report_path, md_path = sys.argv[1:4]
with open(evidence_path, encoding="utf-8") as f:
    evidence = json.load(f)

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def is_ura(value):
    return isinstance(value, str) and value.startswith("easynet:///")

def number(value, default=0.0):
    try:
        return float(value)
    except (TypeError, ValueError):
        return default

def integer(value, default=0):
    try:
        return int(value)
    except (TypeError, ValueError):
        return default

def nested_get(value, path, default=None):
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return default
        value = value[part]
    return value

required_scenarios = {"baseline", "degraded_network", "backpressure"}
allowed_video_codecs = {"h264", "avc1", "vp8", "vp9", "av1", "hevc"}
allowed_audio_codecs = {"opus", "aac", "pcm", "flac"}
allowed_transports = {"webrtc", "raw_stream_v8", "native_webrtc"}
required_abilities = (
    "remote_desktop.create_session",
    "remote_desktop.attach",
    "remote_desktop.watch_events",
    "remote_desktop.end_session",
)
terminal_reasons = {"caller_ended", "user_cancelled", "media_adaptation_e2e_cleanup"}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_media_adaptation_matrix",
        "proof_mode must be real_media_adaptation_matrix")
require(evidence.get("component_mock") is False, "component_mock must be false")
require(evidence.get("real_backend_runtime") is True, "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")

scenarios = evidence.get("scenarios")
require(isinstance(scenarios, list) and scenarios, "scenarios must be a non-empty list")
scenario_by_name = {}
if isinstance(scenarios, list):
    for scenario in scenarios:
        if not isinstance(scenario, dict):
            errors.append("each scenario entry must be an object")
            continue
        name = scenario.get("scenario")
        if name in scenario_by_name:
            errors.append(f"duplicate scenario entry: {name}")
        scenario_by_name[name] = scenario

missing = sorted(required_scenarios - set(scenario_by_name))
require(not missing, "missing media scenarios: " + ", ".join(missing))

scenario_reports = []
for scenario_name in sorted(required_scenarios):
    scenario = scenario_by_name.get(scenario_name)
    if not isinstance(scenario, dict):
        continue
    prefix = scenario_name
    require(scenario.get("status") == "passed", f"{prefix}: status must be passed")
    require(scenario.get("source_only_proof") is False,
            f"{prefix}: source_only_proof must be false")
    require(scenario.get("policy_only") is False,
            f"{prefix}: policy_only must be false")
    subject_ura = scenario.get("selected_resource_ura")
    session_id = scenario.get("session_id")
    media_pipeline_id = scenario.get("media_pipeline_id")
    require(is_ura(subject_ura), f"{prefix}: selected_resource_ura must be canonical")
    require(isinstance(session_id, str) and session_id,
            f"{prefix}: session_id must be recorded")
    require(isinstance(media_pipeline_id, str) and media_pipeline_id,
            f"{prefix}: media_pipeline_id must be recorded")

    abilities = scenario.get("abilities")
    require(isinstance(abilities, list) and abilities,
            f"{prefix}: abilities must be non-empty")
    ability_by_name = {}
    if isinstance(abilities, list):
        for ability in abilities:
            if isinstance(ability, dict) and isinstance(ability.get("name"), str):
                ability_by_name[ability["name"]] = ability
    for ability_name in required_abilities:
        ability = ability_by_name.get(ability_name)
        require(isinstance(ability, dict), f"{prefix}: missing ability {ability_name}")
        if isinstance(ability, dict):
            require(ability.get("subject_ura") == subject_ura,
                    f"{prefix}: {ability_name} must bind selected Resource URA")
            if ability_name != "remote_desktop.create_session":
                require(ability.get("session_id") == session_id,
                        f"{prefix}: {ability_name} must bind session_id")

    video = scenario.get("video")
    require(isinstance(video, dict), f"{prefix}: video evidence must be present")
    if not isinstance(video, dict):
        video = {}
    codec = str(video.get("codec", "")).lower()
    transport = str(video.get("transport", "")).lower()
    require(video.get("codec_negotiated") is True,
            f"{prefix}: video.codec_negotiated must be true")
    require(codec in allowed_video_codecs,
            f"{prefix}: video.codec must be a supported negotiated codec")
    require(isinstance(video.get("payload_content_type"), str)
            and video.get("payload_content_type"),
            f"{prefix}: video.payload_content_type must be present")
    require(transport in allowed_transports,
            f"{prefix}: video.transport must be WebRTC or raw_stream_v8")
    require(integer(video.get("frames_encoded")) > 0,
            f"{prefix}: video.frames_encoded must be positive")
    require(integer(video.get("frames_rendered")) > 0,
            f"{prefix}: video.frames_rendered must be positive")
    require(integer(video.get("duration_ms")) >= 3000,
            f"{prefix}: video.duration_ms must be at least 3000")
    requested_fps = number(video.get("requested_fps"))
    effective_fps = number(video.get("effective_fps"))
    measured_fps = number(video.get("measured_fps"))
    require(1 <= requested_fps <= 240,
            f"{prefix}: video.requested_fps must be within product bounds")
    require(1 <= effective_fps <= requested_fps,
            f"{prefix}: video.effective_fps must be positive and <= requested_fps")
    require(measured_fps > 0,
            f"{prefix}: video.measured_fps must be positive")
    require(measured_fps <= effective_fps * 1.20,
            f"{prefix}: video.measured_fps must not exceed effective_fps by more than 20%")
    require(integer(video.get("target_bitrate_kbps")) > 0,
            f"{prefix}: video.target_bitrate_kbps must be positive")
    require(integer(video.get("observed_bitrate_kbps")) > 0,
            f"{prefix}: video.observed_bitrate_kbps must be positive")
    require(integer(video.get("keyframe_interval_frames")) > 0,
            f"{prefix}: video.keyframe_interval_frames must be positive")
    require(number(video.get("p95_frame_latency_ms")) > 0
            and number(video.get("p95_frame_latency_ms")) <= 250,
            f"{prefix}: video.p95_frame_latency_ms must be bounded")

    audio = scenario.get("audio")
    require(isinstance(audio, dict), f"{prefix}: audio evidence must be present")
    if not isinstance(audio, dict):
        audio = {}
    audio_codec = str(audio.get("codec", "")).lower()
    require(audio.get("status") == "passed",
            f"{prefix}: audio.status must be passed")
    require(audio.get("codec_negotiated") is True,
            f"{prefix}: audio.codec_negotiated must be true")
    require(audio_codec in allowed_audio_codecs,
            f"{prefix}: audio.codec must be negotiated")
    require(integer(audio.get("sample_rate_hz")) in {16000, 24000, 44100, 48000},
            f"{prefix}: audio.sample_rate_hz must be a real capture/playback rate")
    require(integer(audio.get("channels")) in {1, 2},
            f"{prefix}: audio.channels must be mono or stereo")
    require(integer(audio.get("packets_rendered")) > 0
            or integer(audio.get("samples_rendered")) > 0,
            f"{prefix}: audio must render packets or samples")
    require(audio.get("host_audio_not_implemented") is not True,
            f"{prefix}: host audio unsupported state is not product media evidence")
    require(audio.get("muted") is False,
            f"{prefix}: audio.muted must be false")

    queue = scenario.get("queue")
    require(isinstance(queue, dict), f"{prefix}: queue evidence must be present")
    if not isinstance(queue, dict):
        queue = {}
    max_depth = integer(queue.get("max_depth"))
    observed_depth = integer(queue.get("observed_max_depth"))
    require(1 <= max_depth <= 8, f"{prefix}: queue.max_depth must be bounded")
    require(0 <= observed_depth <= max_depth,
            f"{prefix}: queue.observed_max_depth must not exceed max_depth")
    require(queue.get("bounded") is True,
            f"{prefix}: queue.bounded must be true")

    drop_policy = scenario.get("drop_policy")
    require(isinstance(drop_policy, dict), f"{prefix}: drop_policy must be present")
    if not isinstance(drop_policy, dict):
        drop_policy = {}
    require(drop_policy.get("name") in {"latest_frame_bounded_gop", "stale_frame_drop"},
            f"{prefix}: drop_policy.name must be explicit")
    require(drop_policy.get("unbounded_queue") is False,
            f"{prefix}: drop_policy.unbounded_queue must be false")
    require(drop_policy.get("preserves_terminal_frame") is True,
            f"{prefix}: drop_policy.preserves_terminal_frame must be true")

    adaptation = scenario.get("adaptation")
    require(isinstance(adaptation, dict), f"{prefix}: adaptation evidence must be present")
    if not isinstance(adaptation, dict):
        adaptation = {}
    events = adaptation.get("events")
    if not isinstance(events, list):
        events = []
    event_types = {event.get("type") for event in events if isinstance(event, dict)}
    require(adaptation.get("algorithm") in {"webrtc_cc", "transport_feedback", "native_encoder_feedback"},
            f"{prefix}: adaptation.algorithm must be explicit")
    if scenario_name == "degraded_network":
        require("bitrate_downshift" in event_types,
                f"{prefix}: degraded_network must include bitrate_downshift")
        require("fps_downshift" in event_types or "frame_drop" in event_types,
                f"{prefix}: degraded_network must include fps_downshift or frame_drop")
        require(integer(video.get("frames_rendered_after_adaptation")) > 0,
                f"{prefix}: video.frames_rendered_after_adaptation must be positive")
    if scenario_name == "backpressure":
        require("backpressure_detected" in event_types,
                f"{prefix}: backpressure must include backpressure_detected")
        require("frame_drop" in event_types,
                f"{prefix}: backpressure must include frame_drop")
        require(integer(drop_policy.get("frames_dropped")) > 0,
                f"{prefix}: drop_policy.frames_dropped must be positive")

    terminal = scenario.get("terminal_receipt")
    require(isinstance(terminal, dict), f"{prefix}: terminal_receipt must be visible")
    if not isinstance(terminal, dict):
        terminal = {}
    require(terminal.get("terminal") is True,
            f"{prefix}: terminal_receipt.terminal must be true")
    require(terminal.get("session_id") == session_id,
            f"{prefix}: terminal_receipt must bind session_id")
    require(terminal.get("reason_code") in terminal_reasons,
            f"{prefix}: terminal_receipt.reason_code must be a known cleanup/end reason")

    scenario_reports.append({
        "scenario": scenario_name,
        "video_codec": codec,
        "video_transport": transport,
        "audio_codec": audio_codec,
        "selected_resource_ura": subject_ura,
        "media_pipeline_id": media_pipeline_id,
        "measured_fps": measured_fps,
        "observed_bitrate_kbps": integer(video.get("observed_bitrate_kbps")),
        "frames_rendered": integer(video.get("frames_rendered")),
    })

baseline = scenario_by_name.get("baseline")
degraded = scenario_by_name.get("degraded_network")
backpressure = scenario_by_name.get("backpressure")
if isinstance(baseline, dict) and isinstance(degraded, dict):
    baseline_target_bitrate = integer(nested_get(baseline, "video.target_bitrate_kbps"))
    degraded_target_bitrate = integer(nested_get(degraded, "video.target_bitrate_kbps"))
    baseline_observed_bitrate = integer(nested_get(baseline, "video.observed_bitrate_kbps"))
    degraded_observed_bitrate = integer(nested_get(degraded, "video.observed_bitrate_kbps"))
    baseline_effective_fps = number(nested_get(baseline, "video.effective_fps"))
    degraded_effective_fps = number(nested_get(degraded, "video.effective_fps"))
    degraded_frames_dropped = integer(nested_get(degraded, "drop_policy.frames_dropped"))
    require(degraded_target_bitrate < baseline_target_bitrate,
            "degraded_network target_bitrate_kbps must be lower than baseline")
    require(degraded_observed_bitrate < baseline_observed_bitrate,
            "degraded_network observed_bitrate_kbps must be lower than baseline")
    require(degraded_effective_fps < baseline_effective_fps or degraded_frames_dropped > 0,
            "degraded_network must reduce effective_fps or drop frames versus baseline")
if isinstance(baseline, dict) and isinstance(backpressure, dict):
    baseline_frames_dropped = integer(nested_get(baseline, "drop_policy.frames_dropped"))
    backpressure_frames_dropped = integer(nested_get(backpressure, "drop_policy.frames_dropped"))
    require(backpressure_frames_dropped > baseline_frames_dropped,
            "backpressure frames_dropped must exceed baseline")

if all(isinstance(scenario_by_name.get(name), dict) for name in required_scenarios):
    comparable_fields = (
        ("selected_resource_ura", "selected_resource_ura must match across media scenarios"),
        ("media_pipeline_id", "media_pipeline_id must match across media scenarios"),
        ("video.codec", "video.codec must match across media scenarios"),
        ("video.transport", "video.transport must match across media scenarios"),
        ("audio.codec", "audio.codec must match across media scenarios"),
    )
    for field_path, message in comparable_fields:
        baseline_value = nested_get(baseline, field_path)
        for scenario_name in sorted(required_scenarios - {"baseline"}):
            require(nested_get(scenario_by_name[scenario_name], field_path) == baseline_value,
                    message)

if errors:
    report = {
        "script": "tools/scripts/remoteapp-media-adaptation-e2e.sh",
        "status": "failed",
        "errors": errors,
        "product_complete_claim": False,
    }
    pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    pathlib.Path(md_path).write_text(
        "# RemoteApp Media Adaptation E2E\n\n"
        "- Status: `failed`\n"
        + "\n".join(f"- {error}" for error in errors)
        + "\n",
        encoding="utf-8",
    )
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)

report = {
    "script": "tools/scripts/remoteapp-media-adaptation-e2e.sh",
    "status": "passed",
    "proof_mode": evidence.get("proof_mode"),
    "coverage": {name: name in scenario_by_name for name in sorted(required_scenarios)},
    "scenarios": scenario_reports,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Media Adaptation E2E\n\n"
    "- Status: `passed`\n"
    "- Proof mode: `real_media_adaptation_matrix`\n"
    + "\n".join(
        f"- {item['scenario']}: video `{item['video_codec']}`, audio `{item['audio_codec']}`, "
        f"fps `{item['measured_fps']}`, bitrate `{item['observed_bitrate_kbps']}kbps`, "
        f"frames `{item['frames_rendered']}`"
        for item in scenario_reports
    )
    + "\n",
    encoding="utf-8",
)
PY
}

write_self_test_evidence() {
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

def abilities(subject, session_id):
    return [
        {"name": "remote_desktop.create_session", "subject_ura": subject},
        {"name": "remote_desktop.attach", "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id},
    ]

def scenario(name, *, degraded=False, backpressure=False):
    subject = "easynet:///r/acme/resource/device.dev/display.primary"
    session_id = f"sess-{name}"
    events = [{"type": "steady_state", "at_ms": 1000}]
    frames_after = 0
    frames_dropped = 0
    if degraded:
        events = [
            {"type": "bitrate_downshift", "from_kbps": 6000, "to_kbps": 2500},
            {"type": "fps_downshift", "from_fps": 60, "to_fps": 30},
        ]
        frames_after = 120
        frames_dropped = 12
    if backpressure:
        events = [
            {"type": "backpressure_detected", "buffered_bytes": 1048576},
            {"type": "frame_drop", "count": 18},
        ]
        frames_after = 90
        frames_dropped = 18
    return {
        "scenario": name,
        "status": "passed",
        "source_only_proof": False,
        "policy_only": False,
        "selected_resource_ura": subject,
        "session_id": session_id,
        "media_pipeline_id": "remoteapp-media-h264-opus-webrtc",
        "abilities": abilities(subject, session_id),
        "video": {
            "codec_negotiated": True,
            "codec": "h264",
            "payload_content_type": "video/h264; stream-format=annexb",
            "transport": "webrtc",
            "frames_encoded": 240,
            "frames_rendered": 238,
            "duration_ms": 8000,
            "requested_fps": 60,
            "effective_fps": 60 if not degraded else 30,
            "measured_fps": 59.5 if not degraded else 29.2,
            "target_bitrate_kbps": 2500 if degraded else 6000,
            "observed_bitrate_kbps": 5800 if not degraded else 2400,
            "keyframe_interval_frames": 30,
            "p95_frame_latency_ms": 42,
            "frames_rendered_after_adaptation": frames_after,
        },
        "audio": {
            "status": "passed",
            "codec_negotiated": True,
            "codec": "opus",
            "sample_rate_hz": 48000,
            "channels": 2,
            "packets_rendered": 380,
            "samples_rendered": 384000,
            "host_audio_not_implemented": False,
            "muted": False,
        },
        "queue": {
            "max_depth": 3,
            "observed_max_depth": 2,
            "bounded": True,
        },
        "drop_policy": {
            "name": "latest_frame_bounded_gop",
            "unbounded_queue": False,
            "preserves_terminal_frame": True,
            "frames_dropped": frames_dropped,
        },
        "adaptation": {
            "algorithm": "webrtc_cc",
            "events": events,
        },
        "terminal_receipt": {
            "terminal": True,
            "session_id": session_id,
            "reason_code": "media_adaptation_e2e_cleanup",
        },
    }

evidence = {
    "status": "passed",
    "proof_mode": "real_media_adaptation_matrix",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "scenarios": [
        scenario("baseline"),
        scenario("degraded_network", degraded=True),
        scenario("backpressure", backpressure=True),
    ],
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

case "$MODE" in
  skip)
    write_report "skipped" "set EASYNET_REMOTEAPP_MEDIA_ADAPTATION_E2E=1 or pass --run with real media evidence"
    echo "remoteapp-media-adaptation-e2e skipped; report: $REPORT_JSON"
    ;;
  self-test)
    write_self_test_evidence
    validate_evidence
    echo "remoteapp-media-adaptation-e2e self-test ok"
    ;;
  run)
    if [[ -n "$RUNNER_CMD" ]]; then
      EASYNET_REMOTEAPP_MEDIA_ADAPTATION_EVIDENCE_JSON="$EVIDENCE_JSON" \
        bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"
    elif [[ -n "$EVIDENCE_INPUT" ]]; then
      cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
    else
      write_report "failed" "run mode requires --evidence-json or --runner-cmd"
      echo "remoteapp-media-adaptation-e2e failed; report: $REPORT_JSON" >&2
      exit 64
    fi
    validate_evidence
    echo "remoteapp-media-adaptation-e2e passed; report: $REPORT_JSON"
    ;;
  *)
    echo "unknown mode: $MODE" >&2
    exit 64
    ;;
esac
