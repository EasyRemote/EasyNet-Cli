#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh"

fail() {
  printf 'test_remoteapp_cross_device_product_smoke: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR" >/tmp/remoteapp-cross-device-smoke-default.out

python3 - "$OUT_DIR/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["status"] == "skipped"
assert report["coverage"]["cross_device_hub_routing"] is False
assert report["coverage"]["synthetic_stream_bidi_carrier"] is False
assert report["coverage"]["real_os_window_application_capture"] is False
assert any("does not prove real OS window/application capture" in item for item in report["non_claims"])
PY

grep -q "SKIPPED" /tmp/remoteapp-cross-device-smoke-default.out || \
  fail "default mode must be explicit skipped evidence, not a pass"

grep -q "docker-two-node-easyremote-cli-e2e.sh" "$SCRIPT" || \
  fail "cross-device gate must compose the two-node routing smoke"
grep -q "docker-media-bidi-e2e.sh" "$SCRIPT" || \
  fail "cross-device gate must compose the synthetic media/bidi smoke"
grep -q "does not prove direct/STUN/TURN/EasyNet relay deployment" "$SCRIPT" || \
  fail "cross-device gate must not claim NAT/TURN deployment evidence"

echo "test_remoteapp_cross_device_product_smoke: ok"
