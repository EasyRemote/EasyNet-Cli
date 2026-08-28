#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOST_RUNNER="$ROOT/tools/scripts/host-remoteapp-easynet-relay-e2e.sh"
PROJECTOR="$ROOT/tools/scripts/project-remoteapp-network-scenario.py"
REFRESH_VERIFIER="$ROOT/tools/scripts/verify-remoteapp-relay-refresh.py"
NETWORK_VERIFIER="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"

fail() {
  printf 'test_host_remoteapp_easynet_relay_e2e: %s\n' "$1" >&2
  exit 1
}

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$HOST_RUNNER" --out-dir "$OUT_DIR/skip" >/dev/null
jq -e '.status == "skipped" and .coverage.easynet_relay == false and
  .coverage.relay_lease_refresh_resume == false' "$OUT_DIR/skip/report.json" >/dev/null ||
  fail "default skip must not claim live relay coverage"

"$HOST_RUNNER" --self-test --refresh-resume --out-dir "$OUT_DIR/self-test" >/dev/null
jq -e '.status == "passed" and .coverage.easynet_relay == false and
  .coverage.relay_lease_refresh_resume == false' "$OUT_DIR/self-test/report.json" >/dev/null ||
  fail "contract self-test must not claim live relay coverage"
grep -q 'coturn_log_is_ready' "$HOST_RUNNER" ||
  fail "host relay runner must use a pipefail-safe coturn readiness parser"
if grep -Eq 'docker logs .*\| *grep +-q' "$HOST_RUNNER"; then
  fail "host relay runner must not combine docker logs with grep -q under pipefail"
fi

python3 - "$ROOT" "$OUT_DIR/browser.json" "$OUT_DIR/coturn.log" "$OUT_DIR/release.json" <<'PY'
from datetime import datetime
import importlib.util
import json
from pathlib import Path
import sys

root, browser_path, allocation_path, release_path = sys.argv[1:]
module_path = Path(root) / "tools/scripts/verify-remoteapp-relay-refresh.py"
spec = importlib.util.spec_from_file_location("relay_refresh_contract", module_path)
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)
browser = module.self_test_fixture()
browser["evidence_origin"] = "live_runner"
session_id = browser["session_id"]
subject = browser["selected_resource_ura"]
caller = "easynet:///r/localhost/user/alice"
callee = "easynet:///r/localhost/agent/device.receiver.remote-desktop"
provider = "easynet:///r/localhost/device/receiver"
def timestamp_ms(value):
    return int(datetime.strptime(value, "%Y-%m-%dT%H:%M:%S.%f%z").timestamp() * 1000)

initial_observed = timestamp_ms("2026-08-27T00:00:01.000+0000")
refreshed_observed = timestamp_ms("2026-08-27T00:00:01.300+0000")
pair_observed = timestamp_ms("2026-08-27T00:00:01.500+0000")
frame_observed = timestamp_ms("2026-08-27T00:00:01.800+0000")
terminal_observed = timestamp_ms("2026-08-27T00:00:02.000+0000")
release_observed = timestamp_ms("2026-08-27T00:00:02.100+0000")

initial, refreshed, resumed = browser["relay_lease_snapshots"]
initial.update({
    "observed_at_ms": initial_observed,
    "issued_at_ms": initial_observed,
    "expires_at_ms": initial_observed + 30_000,
})
refreshed.update({
    "observed_at_ms": refreshed_observed,
    "issued_at_ms": refreshed_observed,
    "expires_at_ms": refreshed_observed + 30_000,
})
resumed.update({
    "observed_at_ms": refreshed_observed + 50,
    "issued_at_ms": refreshed_observed,
    "expires_at_ms": refreshed_observed + 30_000,
})
refresh = browser["transport_resume"]["relay_lease_refresh"]
refresh.update({
    "initial_observed_at_ms": initial_observed,
    "refreshed_observed_at_ms": refreshed_observed,
    "initial_issued_at_ms": initial_observed,
    "refreshed_issued_at_ms": refreshed_observed,
    "initial_expires_at_ms": initial_observed + 30_000,
    "refreshed_expires_at_ms": refreshed_observed + 30_000,
    "resumed_observed_at_ms": refreshed_observed + 50,
    "resumed_issued_at_ms": refreshed_observed,
    "resumed_expires_at_ms": refreshed_observed + 30_000,
})
step_by_name = {step["name"]: step for step in browser["steps"]}
step_by_name["relay_lease_refreshed_before_daemon_disconnect"]["observed_at_ms"] = refreshed_observed + 100
step_by_name["transport_disconnected"]["observed_at_ms"] = refreshed_observed + 150
step_by_name["transport_reconnected"]["observed_at_ms"] = refreshed_observed + 200
step_by_name["relay_lease_bound_to_replacement_transport"]["observed_at_ms"] = refreshed_observed + 201
step_by_name["terminal_receipt_visible"]["observed_at_ms"] = terminal_observed
step_by_name["terminal_receipt_visible"]["reason_code"] = "caller_ended"

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
        ability["session_id"] = session_id
    abilities.append(ability)

browser["network_transport"] = {
    "caller_ura": caller,
    "callee_ura": callee,
    "provider_device_ura": provider,
    "client_endpoint_id": "browser-relay-refresh-self-test",
    "selected_resource_ura": subject,
    "session_id": session_id,
    "client_ice_url_count": 1,
    "client_ice_url_schemes": ["turn"],
    "easynet_relay": {
        "provider": "easynet_relay",
        "state": "active",
        "lease_id": initial["lease_id"],
        "session_id": session_id,
        "resource_ura": subject,
        "url_count": 1,
        "ephemeral_auth_configured": True,
    },
    "ice_connection_state": "connected",
    "bytes_sent": 4096,
    "bytes_received": 8192,
    "selected_candidate_pair": {
        "candidate_pair_id": "pair-easynet-relay-refresh",
        "local_candidate_id": "local-relay",
        "remote_candidate_id": "remote-relay",
        "local_candidate_type": "relay",
        "remote_candidate_type": "relay",
        "selected_route_class": "relay",
        "ice_transport_policy": "relay",
        "local_description_candidate_types": ["relay"],
        "remote_description_candidate_types": ["relay"],
        "relay_path_proof": "relay_only_policy_and_local_sdp",
        "protocol": "udp",
        "state": "succeeded",
        "selected": True,
        "nominated": True,
        "current_round_trip_time_ms": 2,
        "selected_pair_observed_at_ms": pair_observed,
    },
    "media": {
        "candidate_pair_id": "pair-easynet-relay-refresh",
        "frames_rendered": 4,
        "first_rendered_frame_at_ms": frame_observed,
        "rendered_after_selected_pair": True,
    },
    "abilities": abilities,
}

Path(browser_path).write_text(json.dumps(browser, indent=2) + "\n", encoding="utf-8")
Path(allocation_path).write_text(
    "2026-08-27T00:00:01.200+0000 DEBUG Global turn allocation count incremented, now 1\n",
    encoding="utf-8",
)
Path(release_path).write_text(json.dumps({
    "status_code": 409,
    "terminal_reacquire_rejected": True,
    "observed_at_ms": release_observed,
}, indent=2) + "\n", encoding="utf-8")
PY

python3 "$REFRESH_VERIFIER" --browser-evidence "$OUT_DIR/browser.json" \
  --output "$OUT_DIR/relay-refresh.json"
constraints="$(python3 - <<'PY'
from datetime import datetime
print(int(datetime.strptime("2026-08-27T00:00:01.100+0000", "%Y-%m-%dT%H:%M:%S.%f%z").timestamp() * 1000))
PY
)"
python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
  --route-kind easynet_relay --constraints-applied-at-ms "$constraints" \
  --allocation-log "$OUT_DIR/coturn.log" --release-probe "$OUT_DIR/release.json" \
  --relay-refresh "$OUT_DIR/relay-refresh.json" --output "$OUT_DIR/evidence.json"
"$NETWORK_VERIFIER" --run --required-routes easynet_relay \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null ||
  fail "projected refresh/resume proof did not satisfy the network verifier"
jq -e '.status == "passed" and .coverage.easynet_relay == true and
  .relay_lease_refresh_resume == true' "$OUT_DIR/verified/report.json" >/dev/null ||
  fail "focused report did not expose real relay refresh/resume coverage"

python3 - "$OUT_DIR/relay-refresh.json" "$OUT_DIR/wrong-refresh.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["session_id"] = "rd-wrong-session"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if python3 "$PROJECTOR" --browser-evidence "$OUT_DIR/browser.json" \
    --route-kind easynet_relay --constraints-applied-at-ms "$constraints" \
    --allocation-log "$OUT_DIR/coturn.log" --release-probe "$OUT_DIR/release.json" \
    --relay-refresh "$OUT_DIR/wrong-refresh.json" --output "$OUT_DIR/wrong.json" \
    >/tmp/remoteapp-easynet-relay-wrong-refresh.out 2>&1; then
  fail "projector accepted relay refresh evidence from another session"
fi
grep -q "must bind the projected session and Resource" \
  /tmp/remoteapp-easynet-relay-wrong-refresh.out ||
  fail "cross-session relay refresh rejection was not explicit"

echo "test_host_remoteapp_easynet_relay_e2e: ok"
