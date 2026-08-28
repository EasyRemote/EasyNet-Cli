#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOST_RUNNER="$ROOT/tools/scripts/host-remoteapp-turn-relay-e2e.sh"
PROJECTOR="$ROOT/tools/scripts/project-remoteapp-network-scenario.py"
VERIFIER="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"

fail() {
  printf 'test_host_remoteapp_turn_relay_e2e: %s\n' "$1" >&2
  exit 1
}

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$HOST_RUNNER" --out-dir "$OUT_DIR/skip" >/dev/null
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default host runner mode must be an explicit skip"
"$HOST_RUNNER" --self-test --out-dir "$OUT_DIR/self-test" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/self-test/report.json" || \
  fail "host runner self-test did not pass"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/coturn.log" "$OUT_DIR/times.json" <<'PY'
from datetime import datetime
import json
import sys

browser_path, log_path, times_path = sys.argv[1:]
allocation = int(datetime.strptime(
    "2026-08-25T18:35:04.149+0000", "%Y-%m-%dT%H:%M:%S.%f%z"
).timestamp() * 1000)
constraints = allocation - 100
pair_time = allocation + 100
frame_time = pair_time + 100
terminal_time = frame_time + 1000
subject = "easynet:///r/localhost/resource/device.receiver/streams/window.editor"
session = "rdp-turn-projector-test"
caller = "easynet:///r/localhost/user/alice"
callee = "easynet:///r/localhost/agent/device.receiver.remote-desktop"
provider = "easynet:///r/localhost/device/receiver"
abilities = []
for name in (
    "remote_desktop.create_session",
    "remote_desktop.set_description",
    "remote_desktop.watch_events",
    "remote_desktop.report_client_state",
    "remote_desktop.end_session",
):
    ability = {
        "ability": name,
        "caller_ura": caller,
        "callee_ura": callee,
        "provider_device_ura": provider,
        "subject_ura": subject,
    }
    if name != "remote_desktop.create_session":
        ability["session_id"] = session
    abilities.append(ability)
browser = {
    "status": "passed",
    "evidence_origin": "live_runner",
    "network_transport": {
        "caller_ura": caller,
        "callee_ura": callee,
        "provider_device_ura": provider,
        "client_endpoint_id": "browser-peer-test",
        "selected_resource_ura": subject,
        "session_id": session,
        "client_ice_url_count": 1,
        "client_ice_url_schemes": ["turn"],
        "ice_connection_state": "connected",
        "selected_candidate_pair": {
            "candidate_pair_id": "pair-turn",
            "local_candidate_id": "local-turn",
            "remote_candidate_id": "remote-turn",
            "local_candidate_type": "prflx",
            "remote_candidate_type": "host",
            "selected_route_class": "relay",
            "ice_transport_policy": "relay",
            "local_description_candidate_types": ["relay"],
            "remote_description_candidate_types": ["host", "relay"],
            "relay_path_proof": "relay_only_policy_and_local_sdp",
            "protocol": "udp",
            "state": "succeeded",
            "selected": True,
            "nominated": True,
            "current_round_trip_time_ms": 1,
            "selected_pair_observed_at_ms": pair_time,
        },
        "bytes_sent": 1024,
        "bytes_received": 2048,
        "media": {
            "candidate_pair_id": "pair-turn",
            "frames_rendered": 4,
            "first_rendered_frame_at_ms": frame_time,
            "rendered_after_selected_pair": True,
        },
        "abilities": abilities,
    },
    "steps": [{
        "name": "terminal_receipt_visible",
        "terminal": True,
        "session_id": session,
        "reason_code": "caller_ended",
        "observed_at_ms": terminal_time,
    }],
}
json.dump(browser, open(browser_path, "w", encoding="utf-8"), indent=2)
open(log_path, "w", encoding="utf-8").write(
    "2026-08-25T18:35:04.149+0000 DEBUG Global turn allocation count incremented, now 1\n"
)
json.dump({"constraints": constraints}, open(times_path, "w", encoding="utf-8"))
PY

constraints="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["constraints"])' "$OUT_DIR/times.json")"
python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
  --route-kind turn_relay --constraints-applied-at-ms "$constraints" \
  --allocation-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/evidence.json"
"$VERIFIER" --run --required-routes turn_relay \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null || \
  fail "projected TURN child proof did not satisfy the focused verifier"
grep -q '"turn_relay": true' "$OUT_DIR/verified/report.json" || \
  fail "focused TURN report did not expose route coverage"
grep -q '"direct": false' "$OUT_DIR/verified/report.json" || \
  fail "focused TURN proof incorrectly claimed direct-route coverage"

: >"$OUT_DIR/no-allocation.log"
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
    --route-kind turn_relay --constraints-applied-at-ms "$constraints" \
    --allocation-log "$OUT_DIR/no-allocation.log" --output "$OUT_DIR/no-allocation.json" \
    >/tmp/remoteapp-turn-projector-no-allocation.out 2>&1; then
  fail "projector accepted relay evidence without a server allocation"
fi
grep -q "TURN server did not report a relay allocation" \
  /tmp/remoteapp-turn-projector-no-allocation.out || \
  fail "missing server-allocation failure was not explicit"

if "$VERIFIER" --self-test --required-routes unknown \
    --out-dir "$OUT_DIR/unknown-route" >/tmp/remoteapp-network-unknown-route.out 2>&1; then
  fail "network verifier accepted an unknown focused route"
fi
grep -q "unknown required route" /tmp/remoteapp-network-unknown-route.out || \
  fail "unknown focused route rejection was not explicit"

echo "test_host_remoteapp_turn_relay_e2e: ok"
