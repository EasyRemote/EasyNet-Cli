#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/host-remoteapp-terminal-receipt-crash-e2e.sh"
HARNESS_LIB="$ROOT/tools/scripts/remoteapp-lifecycle-harness-lib.sh"
OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

fail() {
  echo "test_host_remoteapp_terminal_receipt_crash_e2e: $*" >&2
  exit 1
}

bash -n "$SCRIPT"
grep -q -- '--node "$SHOW_CALLEE_URA"' "$SCRIPT" || \
  fail "show_session replay must use the exact paired-user remote callee path"
grep -q -- '--node "$END_CALLEE_URA"' "$SCRIPT" || \
  fail "repeated end_session must use the exact paired-user remote callee path"

cat >"$OUT_DIR/catalog.json" <<'JSON'
[
  {
    "name": "remote_desktop.show_session",
    "call_mode": "rpc",
    "ability_ura": "easynet:///r/test/ability/system-agent.dev.remote-desktop.remote_desktop.show_session",
    "owner_ura": "easynet:///r/test/agent/device.dev.remote-desktop",
    "descriptor_ref": "easynet:///r/test/ability/system-agent.dev.remote-desktop.remote_desktop.show_session@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
  }
]
JSON
source "$HARNESS_LIB"
[[ "$(remoteapp_resolve_rpc_owner_ura "$OUT_DIR/catalog.json" remote_desktop.show_session)" == \
  "easynet:///r/test/agent/device.dev.remote-desktop" ]] || fail "owner resolver drifted"
[[ "$(remoteapp_resolve_rpc_descriptor_ref "$OUT_DIR/catalog.json" remote_desktop.show_session)" == *"@1.0.0#"* ]] || \
  fail "descriptor resolver did not preserve the descriptor-bound invocation ref"

"$SCRIPT" --self-test --out-dir "$OUT_DIR/good" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/good/report.json" || fail "self-test did not pass"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/replaced.json" <<'PY'
import json
import sys

value = json.load(open(sys.argv[1], encoding="utf-8"))
value["show_after_restart"]["terminal_receipt"]["terminal_event_id"] = value["session_id"] + ":999"
json.dump(value, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
cp "$OUT_DIR/replaced.json" "$OUT_DIR/good/evidence.json"
if "$SCRIPT" --verify-self-test-evidence "$OUT_DIR/replaced.json" \
    --out-dir "$OUT_DIR/replaced-report" >"$OUT_DIR/replaced.out" 2>&1; then
  fail "verifier accepted a replaced terminal event identity"
fi
grep -q "must preserve one exact terminal receipt" "$OUT_DIR/replaced.out" || \
  fail "terminal identity replacement rejection was not explicit"

ARM="$OUT_DIR/arm.json"
MARKER="$OUT_DIR/marker.json"
EASYNET_REMOTEAPP_E2E_SESSION_ID=rdp-test-arm \
EASYNET_REMOTEAPP_E2E_RESOURCE_URA=easynet:///r/localhost/resource/device.dev/streams/window.test \
  "$SCRIPT" --fixture-arm "$ARM" "$MARKER"
[[ -f "$ARM" ]] || fail "fixture arm was not written"
[[ "$(stat -f '%Lp' "$ARM" 2>/dev/null || stat -c '%a' "$ARM")" == 600 ]] || \
  fail "fixture arm must be mode 600"
python3 - "$ARM" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["fault"] == "crash_after_terminal_promotion"
assert value["session_id"] == "rdp-test-arm"
assert value["reason"] == "caller_ended"
assert value["marker_path"].endswith("marker.json")
PY

echo "test_host_remoteapp_terminal_receipt_crash_e2e: ok"
