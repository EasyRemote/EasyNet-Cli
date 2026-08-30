#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOST_RUNNER="$ROOT/tools/scripts/host-remoteapp-direct-e2e.sh"
PROJECTOR="$ROOT/tools/scripts/project-remoteapp-network-scenario.py"
VERIFIER="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"

fail() {
  printf 'test_host_remoteapp_direct_e2e: %s\n' "$1" >&2
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
grep -q 'EASYNET_REMOTEAPP_DIRECT_E2E_PROVIDER_STOP_ACTION' "$HOST_RUNNER" ||
  fail "direct runner cannot target an external provider lifecycle"
grep -q 'EASYNET_REMOTEAPP_DIRECT_E2E_BROWSER_DOCKER' "$HOST_RUNNER" ||
  fail "direct runner cannot place Browser beside a container provider"
grep -q 'remoteapp-browser-chrome/Dockerfile' "$HOST_RUNNER" ||
  fail "direct runner does not build a pinned H.264-capable Browser image"
(
  cd "$OUT_DIR"
  "$HOST_RUNNER" --self-test --out-dir relative-self-test >/dev/null
)
grep -q '"status": "passed"' "$OUT_DIR/relative-self-test/report.json" || \
  fail "relative output directory was not anchored before the Browser working-directory change"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/times.json" <<'PY'
import json
import sys

browser_path, times_path = sys.argv[1:]
constraints = 1787696000000
pair_time = constraints + 100
frame_time = pair_time + 100
terminal_time = frame_time + 1000
subject = "easynet:///r/localhost/resource/device.receiver/streams/window.editor"
session = "rdp-direct-projector-test"
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
        "client_ice_url_count": 0,
        "client_ice_url_schemes": [],
        "ice_connection_state": "connected",
        "selected_candidate_pair": {
            "candidate_pair_id": "pair-direct",
            "local_candidate_id": "local-direct",
            "remote_candidate_id": "remote-direct",
            "local_candidate_type": "host",
            "remote_candidate_type": "host",
            "selected_route_class": "direct",
            "ice_transport_policy": "all",
            "local_description_candidate_types": ["host"],
            "remote_description_candidate_types": ["host"],
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
            "candidate_pair_id": "pair-direct",
            "frames_rendered": 3,
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
json.dump({"constraints": constraints}, open(times_path, "w", encoding="utf-8"))
PY

constraints="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["constraints"])' "$OUT_DIR/times.json")"
python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
  --route-kind direct --constraints-applied-at-ms "$constraints" \
  --output "$OUT_DIR/evidence.json"
"$VERIFIER" --run --required-routes direct \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null || \
  fail "projected direct child proof did not satisfy the focused verifier"
grep -q '"direct": true' "$OUT_DIR/verified/report.json" || \
  fail "focused direct report did not expose route coverage"
grep -q 'daemon_zero_ice_servers_plus_host_only_sdp' "$OUT_DIR/evidence.json" || \
  fail "direct evidence did not record the applied constraint method"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/with-ice.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["network_transport"]["client_ice_url_count"] = 1
evidence["network_transport"]["client_ice_url_schemes"] = ["stun"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/with-ice.json" \
    --route-kind direct --constraints-applied-at-ms "$constraints" \
    --output "$OUT_DIR/with-ice-evidence.json" >/tmp/remoteapp-direct-with-ice.out 2>&1; then
  fail "projector accepted a direct proof with daemon-projected ICE servers"
fi
grep -q 'direct projection requires zero daemon-projected ICE server URLs' \
  /tmp/remoteapp-direct-with-ice.out || fail "ICE-server rejection was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/missing-ice-observation.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
del evidence["network_transport"]["client_ice_url_count"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/missing-ice-observation.json" \
    --route-kind direct --constraints-applied-at-ms "$constraints" \
    --output "$OUT_DIR/missing-ice-observation-evidence.json" >/tmp/remoteapp-direct-missing-ice-observation.out 2>&1; then
  fail "projector treated a missing ICE-server observation as zero servers"
fi
grep -q 'direct projection requires zero daemon-projected ICE server URLs' \
  /tmp/remoteapp-direct-missing-ice-observation.out || fail "missing ICE observation rejection was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/srflx-sdp.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["network_transport"]["selected_candidate_pair"]["local_description_candidate_types"] = ["host", "srflx"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/srflx-sdp.json" \
    --route-kind direct --constraints-applied-at-ms "$constraints" \
    --output "$OUT_DIR/srflx-sdp-evidence.json" >/tmp/remoteapp-direct-srflx-sdp.out 2>&1; then
  fail "projector accepted non-host direct SDP"
fi
grep -q 'direct projection requires host-only local and remote SDP' \
  /tmp/remoteapp-direct-srflx-sdp.out || fail "host-only SDP rejection was not explicit"

echo "test_host_remoteapp_direct_e2e: ok"
