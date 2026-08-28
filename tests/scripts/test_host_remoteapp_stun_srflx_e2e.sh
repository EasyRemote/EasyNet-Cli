#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOST_RUNNER="$ROOT/tools/scripts/host-remoteapp-stun-srflx-e2e.sh"
PROJECTOR="$ROOT/tools/scripts/project-remoteapp-network-scenario.py"
VERIFIER="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"

fail() {
  printf 'test_host_remoteapp_stun_srflx_e2e: %s\n' "$1" >&2
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
mkdir -p "$OUT_DIR/frontend-without-sdp-filter/scripts"
cp "$ROOT/../EasyNet/Frontend/scripts/remoteapp-browser-lifecycle.mjs" \
  "$OUT_DIR/frontend-without-sdp-filter/scripts/remoteapp-browser-lifecycle.mjs"
perl -0pi -e "s#return admittedDescription\('outbound', super\.localDescription\)#return super.localDescription#" \
  "$OUT_DIR/frontend-without-sdp-filter/scripts/remoteapp-browser-lifecycle.mjs"
if EASYNET_REMOTEAPP_STUN_E2E_FRONTEND_ROOT="$OUT_DIR/frontend-without-sdp-filter" \
    "$HOST_RUNNER" --self-test --out-dir "$OUT_DIR/no-sdp-filter" \
      >/tmp/remoteapp-stun-no-sdp-filter.out 2>&1; then
  fail "host runner accepted a Browser probe that leaked embedded local SDP candidates"
fi
grep -q 'does not apply outbound admission to local SDP' \
  "$OUT_DIR/no-sdp-filter/report.json" || fail "missing local-SDP admission failure was not explicit"
if EASYNET_REMOTEAPP_BROWSER_EMAIL=test@example.invalid \
    EASYNET_REMOTEAPP_BROWSER_PASSWORD=not-a-real-secret \
    EASYNET_REMOTEAPP_STUN_E2E_HOST=192.0.2.1 \
    EASYNET_REMOTEAPP_STUN_E2E_BROWSER_RUN_DEADLINE_SECONDS=0 \
    "$HOST_RUNNER" --run --device-id 00000000-0000-0000-0000-000000000000 \
      --out-dir "$OUT_DIR/invalid-deadline" >/tmp/remoteapp-stun-invalid-deadline.out 2>&1; then
  fail "host runner accepted an unbounded Browser proof deadline"
fi
grep -q 'Browser run deadline must be a positive integer number of seconds' \
  "$OUT_DIR/invalid-deadline/report.json" || fail "invalid Browser deadline rejection was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/coturn.log" "$OUT_DIR/times.json" <<'PY'
from datetime import datetime
import json
import sys

browser_path, log_path, times_path = sys.argv[1:]
binding = int(datetime.strptime(
    "2026-08-25T22:31:02.247+0000", "%Y-%m-%dT%H:%M:%S.%f%z"
).timestamp() * 1000)
constraints = binding - 100
pair_time = binding + 100
frame_time = pair_time + 100
terminal_time = frame_time + 1000
subject = "easynet:///r/localhost/resource/device.receiver/streams/window.editor"
session = "rdp-stun-projector-test"
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
        "client_ice_url_schemes": ["stun"],
        "candidate_admission": {
            "outbound": {
                "allowed_types": ["prflx", "srflx"],
                "accepted": 1,
                "rejected": 2,
            },
            "inbound": {
                "allowed_types": ["host", "prflx", "srflx"],
                "accepted": 1,
                "rejected": 0,
            },
        },
        "ice_connection_state": "connected",
        "selected_candidate_pair": {
            "candidate_pair_id": "pair-stun",
            "local_candidate_id": "local-stun",
            "remote_candidate_id": "remote-stun",
            "local_candidate_type": "srflx",
            "remote_candidate_type": "host",
            "selected_route_class": "stun_srflx",
            "ice_transport_policy": "all",
            "local_description_candidate_types": ["srflx"],
            "remote_description_candidate_types": ["host", "srflx"],
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
            "candidate_pair_id": "pair-stun",
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
open(log_path, "w", encoding="utf-8").write(
    "2026-08-25T22:31:02.247+0000 INFO session 1: realm <> user <>: incoming packet BINDING processed, success\n"
)
json.dump({"constraints": constraints}, open(times_path, "w", encoding="utf-8"))
PY

constraints="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["constraints"])' "$OUT_DIR/times.json")"
python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
  --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
  --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/evidence.json"
"$VERIFIER" --run --required-routes stun_srflx \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null || \
  fail "projected STUN child proof did not satisfy the focused verifier"
grep -q '"stun_srflx": true' "$OUT_DIR/verified/report.json" || \
  fail "focused STUN report did not expose route coverage"
grep -q 'browser_reflexive_outbound_plus_provider_host_return_and_server_binding' "$OUT_DIR/evidence.json" || \
  fail "STUN evidence did not record the applied constraint method"

python3 - "$OUT_DIR/coturn.log" "$OUT_DIR/native-stun.jsonl" <<'PY'
from datetime import datetime
import json
import sys

coturn_path, native_path = sys.argv[1:]
timestamp = datetime.strptime(
    open(coturn_path, encoding="utf-8").read().split(" INFO", 1)[0],
    "%Y-%m-%dT%H:%M:%S.%f%z",
)
with open(native_path, "w", encoding="utf-8") as output:
    output.write(json.dumps({
        "schema": "easynet.remoteapp.stun-binding-event.v1",
        "event": "stun_binding_succeeded",
        "observed_at_ms": int(timestamp.timestamp() * 1000),
    }, separators=(",", ":")) + "\n")
PY
python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
  --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
  --binding-log "$OUT_DIR/native-stun.jsonl" --output "$OUT_DIR/native-stun-evidence.json"
python3 - "$OUT_DIR/native-stun-evidence.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
binding = evidence["scenarios"][0]["network_fixture"]["stun_binding"]
assert binding["observer_kinds"] == ["native_rfc5389_fixture"]
assert binding["binding_count"] == 1
PY

: >"$OUT_DIR/no-binding.log"
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/no-binding.log" --output "$OUT_DIR/no-binding.json" \
    >/tmp/remoteapp-stun-projector-no-binding.out 2>&1; then
  fail "projector accepted STUN evidence without a server binding"
fi
grep -q 'STUN server did not report a binding transaction' \
  /tmp/remoteapp-stun-projector-no-binding.out || fail "missing STUN binding rejection was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/no-rejected-direct.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["network_transport"]["candidate_admission"]["outbound"]["rejected"] = 0
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/no-rejected-direct.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/no-rejected-direct-evidence.json" \
    >/tmp/remoteapp-stun-no-rejected-direct.out 2>&1; then
  fail "projector accepted STUN evidence without blocking outbound Browser host candidates"
fi
grep -q 'STUN projection requires rejected outbound Browser host candidates' \
  /tmp/remoteapp-stun-no-rejected-direct.out || fail "direct-candidate rejection failure was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/host-in-local-sdp.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["network_transport"]["selected_candidate_pair"]["local_description_candidate_types"] = [
    "host", "srflx"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/host-in-local-sdp.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/host-in-local-sdp-evidence.json" \
    >/tmp/remoteapp-stun-host-in-local-sdp.out 2>&1; then
  fail "projector accepted a Browser offer containing a direct host candidate"
fi
grep -q 'Browser local SDP to contain only reflexive candidates' \
  /tmp/remoteapp-stun-host-in-local-sdp.out || fail "embedded Browser host-candidate rejection was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/wrong-inbound-policy.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["network_transport"]["candidate_admission"]["inbound"]["allowed_types"] = [
    "prflx", "srflx"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/wrong-inbound-policy.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/wrong-inbound-policy-evidence.json" \
    >/tmp/remoteapp-stun-wrong-inbound-policy.out 2>&1; then
  fail "projector accepted a policy that removed the provider host return route"
fi
grep -q 'inbound candidate admission types' \
  /tmp/remoteapp-stun-wrong-inbound-policy.out || fail "provider host return-policy failure was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/wrong-selected-side.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
pair = evidence["network_transport"]["selected_candidate_pair"]
pair["local_candidate_type"] = "host"
pair["remote_candidate_type"] = "srflx"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/wrong-selected-side.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/wrong-selected-side-evidence.json" \
    >/tmp/remoteapp-stun-wrong-selected-side.out 2>&1; then
  fail "projector accepted a reflexive candidate only on the provider side"
fi
grep -q 'selected Browser-local candidate to be reflexive' \
  /tmp/remoteapp-stun-wrong-selected-side.out || fail "Browser-local reflexive selection failure was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/turn-scheme.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["network_transport"]["client_ice_url_schemes"] = ["turn"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/turn-scheme.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/turn-scheme-evidence.json" \
    >/tmp/remoteapp-stun-turn-scheme.out 2>&1; then
  fail "projector accepted TURN configuration as STUN evidence"
fi
grep -q 'STUN projection requires only redacted stun/stuns URL schemes' \
  /tmp/remoteapp-stun-turn-scheme.out || fail "STUN scheme rejection was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/no-stun-url.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["network_transport"]["client_ice_url_count"] = 0
evidence["network_transport"]["client_ice_url_schemes"] = []
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/no-stun-url.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/no-stun-url-evidence.json" \
    >/tmp/remoteapp-stun-no-url.out 2>&1; then
  fail "projector accepted STUN evidence without a daemon-projected STUN URL"
fi
grep -q 'STUN projection requires a positive daemon-projected ICE URL count' \
  /tmp/remoteapp-stun-no-url.out || fail "missing projected STUN URL rejection was not explicit"

python3 - "$OUT_DIR/browser.json" "$OUT_DIR/no-admission-counters.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
del evidence["network_transport"]["candidate_admission"]["outbound"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/no-admission-counters.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/no-admission-counters-evidence.json" \
    >/tmp/remoteapp-stun-no-admission-counters.out 2>&1; then
  fail "projector accepted STUN evidence without outbound admission counters"
fi
grep -q 'candidate_admission.outbound must be an object' \
  /tmp/remoteapp-stun-no-admission-counters.out || fail "missing admission-counter rejection was not explicit"

binding_ms=$(( constraints + 100 ))
late_constraints=$(( binding_ms + 1 ))
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
    --route-kind stun_srflx --constraints-applied-at-ms "$late_constraints" \
    --binding-log "$OUT_DIR/coturn.log" --output "$OUT_DIR/wrong-order-evidence.json" \
    >/tmp/remoteapp-stun-wrong-order.out 2>&1; then
  fail "projector accepted a STUN binding observed before fixture constraints"
fi
grep -q 'STUN binding must be observed after constraints and before pair selection' \
  /tmp/remoteapp-stun-wrong-order.out || fail "STUN ordering rejection was not explicit"

echo "test_host_remoteapp_stun_srflx_e2e: ok"
