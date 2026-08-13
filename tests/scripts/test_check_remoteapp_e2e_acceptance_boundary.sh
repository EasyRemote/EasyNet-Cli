#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh"
HARNESS="$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/tools/scripts" "$SANDBOX/docs/design"
cp "$HARNESS" "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
cat >"$SANDBOX/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
| E2E-03 exact window session | decoded stream excludes sentinel |
| E2E-04 exact application session | decoded stream excludes other apps |
| E2E-07 display fallback forbidden | no decoded full display |
MD

CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

PROBE="$SANDBOX/fake_probe.py"
cat >"$PROBE" <<'PY'
import json
import os

subject = "easynet:///r/localhost/resource/device.dev/streams/window.test"
evidence = {
    "status": "passed",
    "live_inventory": {"ability": "resource.refresh_remote_targets"},
    "selected_resource_ura": subject,
    "invocation": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
    },
    "target_binding": {
        "target_kind": "window",
        "capture_scope": "WindowSurface",
        "scope_audit": {
            "scope_widened": False,
            "display_fallback_used": False,
        },
    },
    "transport": {"kind": "webrtc"},
    "decoded_frames": {
        "count": 2,
        "selected_content_present": True,
        "unrelated_sentinel_present": False,
        "full_display_leak_detected": False,
    },
    "artifacts": {
        "decoded_frame_sample": "target/e2e/sample-frame.png",
    },
}
with open(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"], "w", encoding="utf-8") as f:
    json.dump(evidence, f)
PY
"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --run \
  --probe-cmd "python3 '$PROBE'" \
  --out-dir "$SANDBOX/e2e-out" >/dev/null

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/unrelated_sentinel_present/unrelated_sentinel_omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing sentinel exclusion assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

perl -0pi -e 's/remote_desktop\.create_session/remote_desktop.open_session/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing create_session assertion" >&2
  exit 1
fi

echo "test_check_remoteapp_e2e_acceptance_boundary.sh: all cases passed"
