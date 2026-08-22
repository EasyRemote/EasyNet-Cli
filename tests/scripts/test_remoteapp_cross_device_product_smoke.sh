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
assert report["source"]["revision"]
assert isinstance(report["source"]["dirty"], bool)
assert report["runtime"]["image"]
assert "image_id" in report["runtime"]
assert "image_created" in report["runtime"]
assert report["runtime"]["build_requested"] is False
assert report["coverage"]["cross_device_hub_routing"] is False
assert report["coverage"]["synthetic_stream_bidi_carrier"] is False
assert report["coverage"]["real_os_window_application_capture"] is False
assert any("does not prove real OS window/application capture" in item for item in report["non_claims"])
PY

grep -q "SKIPPED" /tmp/remoteapp-cross-device-smoke-default.out || \
  fail "default mode must be explicit skipped evidence, not a pass"

DISK_FAIL_DIR="$OUT_DIR/disk-fail"
if EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_MIN_FREE_KIB=999999999999 \
  "$SCRIPT" --run --out-dir "$DISK_FAIL_DIR" \
  >/tmp/remoteapp-cross-device-smoke-disk.out \
  2>/tmp/remoteapp-cross-device-smoke-disk.err; then
  fail "run mode must fail before child E2Es when report filesystem lacks free space"
fi

python3 - "$DISK_FAIL_DIR/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["status"] == "failed"
assert report["source"]["revision"]
assert report["runtime"]["image"]
assert "insufficient free space" in report["reason"]
assert report["coverage"]["cross_device_hub_routing"] is False
assert report["coverage"]["synthetic_stream_bidi_carrier"] is False
PY

FAKE_BIN="$OUT_DIR/fake-bin"
mkdir -p "$FAKE_BIN"
cat >"$FAKE_BIN/docker" <<'SH'
#!/usr/bin/env bash
if [[ "${1:-}" == "info" ]]; then
  sleep 5
  exit 0
fi
exit 1
SH
chmod +x "$FAKE_BIN/docker"

DOCKER_FAIL_DIR="$OUT_DIR/docker-fail"
if PATH="$FAKE_BIN:$PATH" \
  EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_MIN_FREE_KIB=1 \
  EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_DOCKER_INFO_TIMEOUT_SECONDS=1 \
  "$SCRIPT" --run --out-dir "$DOCKER_FAIL_DIR" \
  >/tmp/remoteapp-cross-device-smoke-docker.out \
  2>/tmp/remoteapp-cross-device-smoke-docker.err; then
  fail "run mode must fail with a report when docker info hangs"
fi

python3 - "$DOCKER_FAIL_DIR/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["status"] == "failed"
assert report["source"]["revision"]
assert report["runtime"]["image"]
assert "docker info timed out" in report["reason"]
assert report["coverage"]["cross_device_hub_routing"] is False
assert report["coverage"]["synthetic_stream_bidi_carrier"] is False
PY

grep -q "docker-two-node-easyremote-cli-e2e.sh" "$SCRIPT" || \
  fail "cross-device gate must compose the two-node routing smoke"
grep -q "docker-media-bidi-e2e.sh" "$SCRIPT" || \
  fail "cross-device gate must compose the synthetic media/bidi smoke"
grep -q "does not prove direct/STUN/TURN/EasyNet relay deployment" "$SCRIPT" || \
  fail "cross-device gate must not claim NAT/TURN deployment evidence"

echo "test_remoteapp_cross_device_product_smoke: ok"
