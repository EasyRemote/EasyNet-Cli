#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"

fail() {
  printf 'test_remoteapp_network_fallback_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-network-fallback-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/remoteapp-network-fallback-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_NETWORK_FALLBACK_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null
if "$SCRIPT" --run --evidence-json "$OUT_DIR/good/evidence.json" \
    --out-dir "$OUT_DIR/self-test-as-live" >/tmp/remoteapp-network-origin.out 2>&1; then
  fail "verifier accepted contract self-test evidence in run mode"
fi
grep -q "evidence_origin must be live_runner" /tmp/remoteapp-network-origin.out || \
  fail "self-test provenance rejection was not explicit"
python3 - "$OUT_DIR/good/evidence.json" <<'PY'
import json
import sys

path = sys.argv[1]
evidence = json.load(open(path, encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
json.dump(evidence, open(path, "w", encoding="utf-8"), indent=2)
PY

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/stun-local-sdp-host.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
scenario = next(item for item in evidence["scenarios"] if item["route_kind"] == "stun_srflx")
scenario["webrtc"]["selected_candidate_pair"]["local_description_candidate_types"] = [
    "host", "srflx"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/stun-local-sdp-host.json" \
    --out-dir "$OUT_DIR/stun-local-sdp-host" >/tmp/remoteapp-network-stun-local-sdp-host.out 2>&1; then
  fail "verifier accepted a STUN proof whose Browser offer still advertised host candidates"
fi
grep -q "STUN Browser local SDP must contain only reflexive candidates" \
  /tmp/remoteapp-network-stun-local-sdp-host.out || \
  fail "Browser local-SDP host-candidate rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/device-caller.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["caller_ura"] = "easynet:///r/localhost/device/browser"
evidence["scenarios"][0]["webrtc"]["caller_ura"] = "easynet:///r/localhost/device/browser"
for ability in evidence["scenarios"][0]["abilities"]:
    ability["caller_ura"] = "easynet:///r/localhost/device/browser"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/device-caller.json" \
    --out-dir "$OUT_DIR/device-caller" >/tmp/remoteapp-network-device-caller.out 2>&1; then
  fail "verifier accepted a Browser product call modelled as a Device caller"
fi
grep -q "caller_ura must identify an admitted User, Agent, or Authority" \
  /tmp/remoteapp-network-device-caller.out || \
  fail "Device caller rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/diagnostic-attach-only.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
scenario = evidence["scenarios"][0]
scenario["abilities"] = [
    {
        **ability,
        "name": "remote_desktop.attach" if ability["name"] == "remote_desktop.set_description" else ability["name"],
    }
    for ability in scenario["abilities"]
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/diagnostic-attach-only.json" \
    --out-dir "$OUT_DIR/diagnostic-attach-only" >/tmp/remoteapp-network-attach-only.out 2>&1; then
  fail "verifier accepted diagnostic attach in place of production WebRTC signalling"
fi
grep -q "missing ability remote_desktop.set_description" \
  /tmp/remoteapp-network-attach-only.out || \
  fail "diagnostic attach-only rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/missing-turn.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"] = [
    scenario for scenario in evidence["scenarios"]
    if scenario["route_kind"] != "turn_relay"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-turn.json" --out-dir "$OUT_DIR/missing-turn" >/tmp/remoteapp-network-fallback-missing-turn.out 2>&1; then
  fail "verifier accepted evidence without TURN relay scenario"
fi
grep -q "missing route scenarios: turn_relay" /tmp/remoteapp-network-fallback-missing-turn.out || \
  fail "missing TURN relay failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/leaked-secret.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][2]["turn_password"] = "not-redacted"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/leaked-secret.json" --out-dir "$OUT_DIR/leaked-secret" >/tmp/remoteapp-network-fallback-secret.out 2>&1; then
  fail "verifier accepted evidence with raw TURN secret"
fi
grep -q "raw credential/secret fields are forbidden" /tmp/remoteapp-network-fallback-secret.out || \
  fail "credential leakage failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-media.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["media"]["frames_rendered"] = 0
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-media.json" --out-dir "$OUT_DIR/no-media" >/tmp/remoteapp-network-fallback-no-media.out 2>&1; then
  fail "verifier accepted evidence without rendered media frames"
fi
grep -q "media.frames_rendered must be positive" /tmp/remoteapp-network-fallback-no-media.out || \
  fail "rendered media failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/wrong-webrtc-session.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["webrtc"]["session_id"] = "rd-network-unrelated-session"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-webrtc-session.json" --out-dir "$OUT_DIR/wrong-webrtc-session" >/tmp/remoteapp-network-fallback-wrong-webrtc-session.out 2>&1; then
  fail "verifier accepted WebRTC candidate-pair evidence from a different session"
fi
grep -q "webrtc session_id must bind session_id" /tmp/remoteapp-network-fallback-wrong-webrtc-session.out || \
  fail "WebRTC session binding failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/missing-candidate-pair-id.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
del evidence["scenarios"][0]["webrtc"]["selected_candidate_pair"]["candidate_pair_id"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-candidate-pair-id.json" --out-dir "$OUT_DIR/missing-candidate-pair-id" >/tmp/remoteapp-network-fallback-missing-pair-id.out 2>&1; then
  fail "verifier accepted selected candidate-pair evidence without a stable pair id"
fi
grep -q "selected_candidate_pair.candidate_pair_id must be recorded" /tmp/remoteapp-network-fallback-missing-pair-id.out || \
  fail "candidate-pair id failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/wrong-media-pair.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["media"]["candidate_pair_id"] = "pair-from-unrelated-connection"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-media-pair.json" --out-dir "$OUT_DIR/wrong-media-pair" >/tmp/remoteapp-network-fallback-wrong-media-pair.out 2>&1; then
  fail "verifier accepted rendered media that was not bound to the selected candidate pair"
fi
grep -q "media candidate_pair_id must match selected_candidate_pair" /tmp/remoteapp-network-fallback-wrong-media-pair.out || \
  fail "media candidate-pair binding failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/relay-only-sdp.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
scenario = next(item for item in evidence["scenarios"] if item["route_kind"] == "turn_relay")
pair = scenario["webrtc"]["selected_candidate_pair"]
pair.update({
    "local_candidate_type": "prflx",
    "remote_candidate_type": "host",
    "ice_transport_policy": "relay",
    "local_description_candidate_types": ["relay"],
    "remote_description_candidate_types": ["host", "relay"],
    "relay_path_proof": "relay_only_policy_and_local_sdp",
})
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/relay-only-sdp.json" \
  --out-dir "$OUT_DIR/relay-only-sdp" >/dev/null || \
  fail "verifier rejected TURN selected-pair evidence backed by relay-only policy, relay SDP, and server allocation"

python3 - "$OUT_DIR/relay-only-sdp.json" "$OUT_DIR/missing-relay-allocation.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
scenario = next(item for item in evidence["scenarios"] if item["route_kind"] == "turn_relay")
del scenario["network_fixture"]["relay_allocation"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-relay-allocation.json" \
    --out-dir "$OUT_DIR/missing-relay-allocation" >/tmp/remoteapp-network-missing-allocation.out 2>&1; then
  fail "verifier accepted a relay-only SDP claim without server-observed allocation evidence"
fi
grep -q "relay route must include server-observed relay_allocation evidence" \
  /tmp/remoteapp-network-missing-allocation.out || \
  fail "missing relay allocation rejection was not explicit"

echo "test_remoteapp_network_fallback_e2e: ok"
