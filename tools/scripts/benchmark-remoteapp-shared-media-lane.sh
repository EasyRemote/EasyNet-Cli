#!/usr/bin/env bash
# Reproducible local carrier benchmark for RemoteApp payload-pipe v1 versus
# generation-fenced shared-media-lane v2. This is not live capture/WebRTC E2E.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${EASYNET_REMOTEAPP_SHARED_LANE_BENCHMARK_JSON:-$ROOT/target/e2e/remoteapp-shared-media-lane/benchmark.json}"
PROFILE="${EASYNET_REMOTEAPP_SHARED_LANE_BENCHMARK_PROFILE:-release}"

case "$PROFILE" in
  debug|release) ;;
  *) echo "invalid benchmark profile: $PROFILE" >&2; exit 64 ;;
esac

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  rg -q 'REMOTEAPP_SHARED_LANE_BENCHMARK_JSON=' \
    "$ROOT/plugins/remote-desktop/native-protocol/tests/shared_media_lane_benchmark.rs"
  echo 'benchmark-remoteapp-shared-media-lane self-test ok'
  exit 0
fi

if [[ "${1:-}" == "--out" ]]; then
  OUT="${2:?--out requires a JSON path}"
  shift 2
fi
[[ "$#" == 0 ]] || { echo "unknown argument: $1" >&2; exit 64; }

RUN_ROOT="$(mktemp -d)"
cleanup() {
  find "$RUN_ROOT" -depth -delete 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$(dirname "$OUT")"
CARGO_ARGS=(test --manifest-path "$ROOT/plugins/remote-desktop/native-protocol/Cargo.toml")
if [[ "$PROFILE" == "release" ]]; then
  CARGO_ARGS+=(--release)
fi
CARGO_ARGS+=(--test shared_media_lane_benchmark --
  --exact shared_lane_benchmark_emits_comparative_evidence --nocapture)

cargo "${CARGO_ARGS[@]}" 2>&1 | tee "$RUN_ROOT/cargo-output.txt"

python3 - "$RUN_ROOT/cargo-output.txt" "$OUT" "$PROFILE" \
  "$(uname -s)" "$(uname -m)" "$(rustc --version)" <<'PY'
import json
import pathlib
import sys

log_path, out_path, profile, operating_system, architecture, rustc_version = sys.argv[1:]
prefix = "REMOTEAPP_SHARED_LANE_BENCHMARK_JSON="
rows = [line[len(prefix):] for line in pathlib.Path(log_path).read_text().splitlines()
        if line.startswith(prefix)]
if len(rows) != 1:
    raise SystemExit(f"expected exactly one benchmark evidence row, got {len(rows)}")
report = json.loads(rows[0])
shared = report["shared_v2"]
pipe = report["payload_pipe_v1"]
frames = report["frame_count"]
if shared["allocation_calls"] > frames * 2:
    raise SystemExit("shared lane exceeded two small ownership allocations per frame")
if shared["allocated_bytes"] > frames * 256:
    raise SystemExit("shared lane allocated payload-sized storage on the measured hot path")
if shared["allocated_bytes"] * 16 >= pipe["allocated_bytes"]:
    raise SystemExit("shared lane did not reduce allocation volume by at least 16x")
if shared["throughput_mib_per_s"] * 2 <= pipe["throughput_mib_per_s"]:
    raise SystemExit("shared lane throughput regressed by more than 50 percent")
if shared["latency_ns"]["p95"] >= pipe["latency_ns"]["p95"] * 2:
    raise SystemExit("shared lane p95 latency regressed by more than 2x")
report["environment"] = {
    "profile": profile,
    "operating_system": operating_system,
    "architecture": architecture,
    "rustc": rustc_version,
}
report["scope"] = {
    "kind": "same-process validated media hot-path microbenchmark",
    "includes_generation_and_h264_state_machine": True,
    "includes_webrtc_bytes_view": True,
    "proves_live_capture": False,
    "proves_webrtc_packetization": False,
    "proves_cross_device_transport": False,
}
path = pathlib.Path(out_path)
path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
print(f"benchmark-remoteapp-shared-media-lane: wrote {path}")
PY
