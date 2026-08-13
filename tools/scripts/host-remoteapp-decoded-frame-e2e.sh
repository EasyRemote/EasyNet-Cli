#!/usr/bin/env bash
# Host-gated decoded-frame E2E for remote app/window sessions.
#
# This script intentionally does NOT run by default. It validates the SPEC's
# strongest host-only evidence requirement: after a live picker selects a
# window/application resource URA and remote_desktop.create_session starts a
# WebRTC session, decoded media frames must contain the selected target and
# must not contain unrelated full-display sentinel content.
#
# The script is a harness boundary, not a static source check. With --run it
# requires a probe command that performs host GUI/WebRTC work and writes the
# evidence JSON described below. If no probe is supplied, --run fails closed.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${EASYNET_REMOTEAPP_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/host-remoteapp-decoded-frame/$TIMESTAMP}"
RUN="${EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E:-0}"
SELF_TEST=0
PROBE_CMD="${EASYNET_REMOTEAPP_FRAME_PROBE_CMD:-}"
TARGET_KIND="${EASYNET_REMOTEAPP_E2E_TARGET_KIND:-window}"

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/host-remoteapp-decoded-frame-e2e.sh [options]

Options:
  --run                 Run the host decoded-frame E2E harness.
  --probe-cmd CMD       Command that drives the real GUI/WebRTC probe and writes
                        EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON.
  --target-kind KIND    Target kind: window or application. Default: window.
  --out-dir DIR         Report directory.
  --self-test           Validate harness structure with a synthetic probe.
  -h, --help            Show this help.

Environment:
  EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E=1
                        Equivalent to --run.
  EASYNET_REMOTEAPP_FRAME_PROBE_CMD
                        Same as --probe-cmd.

Probe contract:
  The probe command receives:
    EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON=<path>
    EASYNET_REMOTEAPP_E2E_TARGET_KIND=<window|application>

  It must create the JSON file and prove:
    - live picker used resource.refresh_remote_targets or resource.watch_remote_targets
    - selected resource_ura was used as Invocation.subject
    - remote_desktop.create_session returned a target_binding for that target
    - WebRTC media was decoded into at least one frame
    - decoded frames included selected target content
    - decoded frames excluded unrelated full-display sentinel content
    - scope_audit.scope_widened=false
    - scope_audit.display_fallback_used=false

Exit semantics:
  0  evidence is valid, or script was skipped because not enabled.
  1  --run was requested but evidence is missing, malformed, or failed.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) RUN=1; shift ;;
    --probe-cmd) PROBE_CMD="${2:?missing value for --probe-cmd}"; shift 2 ;;
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

mkdir -p "$OUT_DIR"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
EVIDENCE_JSON="$OUT_DIR/decoded-frame-evidence.json"

write_skip_report() {
  python3 - "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json
import sys

report = {
    "enabled": False,
    "status": "skipped",
    "reason": "host remoteapp decoded-frame e2e requires --run or EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E=1",
}
open(sys.argv[1], "w", encoding="utf-8").write(json.dumps(report, indent=2, sort_keys=True) + "\n")
with open(sys.argv[2], "w", encoding="utf-8") as f:
    f.write("# Host remoteapp decoded-frame E2E report\n\n")
    f.write("- Status: `skipped`\n")
    f.write("- Reason: `host remoteapp decoded-frame e2e requires --run or EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E=1`\n")
PY
}

validate_evidence() {
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" "$TARGET_KIND" <<'PY'
import json
import sys

evidence_path, report_path, md_path, expected_kind = sys.argv[1:5]
with open(evidence_path, encoding="utf-8") as f:
    evidence = json.load(f)

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def get(path, default=None):
    value = evidence
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return default
        value = value[part]
    return value

selected_resource_ura = get("selected_resource_ura")
invocation_subject_ura = get("invocation.subject_ura")
inventory_ability = get("live_inventory.ability")
target_kind = get("target_binding.target_kind")
capture_scope = get("target_binding.capture_scope")
scope_widened = get("target_binding.scope_audit.scope_widened")
display_fallback_used = get("target_binding.scope_audit.display_fallback_used")
decoded_frame_count = get("decoded_frames.count")
transport_kind = get("transport.kind")

require(evidence.get("status") == "passed", "probe status must be passed")
require(inventory_ability in {"resource.refresh_remote_targets", "resource.watch_remote_targets"},
        "live inventory ability must be resource.refresh_remote_targets or resource.watch_remote_targets")
require(isinstance(selected_resource_ura, str) and selected_resource_ura.startswith("easynet:///"),
        "selected_resource_ura must be an EasyNet URA")
require("uri" not in json.dumps(evidence).lower(), "evidence must use URA vocabulary only")
require(invocation_subject_ura == selected_resource_ura,
        "Invocation.subject must equal the selected resource_ura")
require(get("invocation.ability") == "remote_desktop.create_session",
        "invocation ability must be remote_desktop.create_session")
require(target_kind == expected_kind, f"target_binding.target_kind must be {expected_kind}")
if expected_kind == "window":
    require(capture_scope == "WindowSurface", "window target must use WindowSurface")
else:
    require(capture_scope == "AppSurface", "application target must use AppSurface")
require(scope_widened is False, "scope_audit.scope_widened must be false")
require(display_fallback_used is False, "scope_audit.display_fallback_used must be false")
require(transport_kind == "webrtc", "transport.kind must be webrtc")
require(isinstance(decoded_frame_count, int) and decoded_frame_count > 0,
        "decoded_frames.count must be a positive integer")
require(get("decoded_frames.selected_content_present") is True,
        "decoded frames must include selected target content")
require(get("decoded_frames.unrelated_sentinel_present") is False,
        "decoded frames must exclude unrelated sentinel content")
require(get("decoded_frames.full_display_leak_detected") is False,
        "decoded frames must not show full-display leakage")
require(get("artifacts.decoded_frame_sample"),
        "evidence must include a decoded_frame_sample artifact path")

report = {
    "enabled": True,
    "status": "failed" if errors else "passed",
    "target_kind": expected_kind,
    "evidence_json": evidence_path,
    "errors": errors,
    "assertions": {
        "live_inventory": inventory_ability,
        "selected_subject": invocation_subject_ura,
        "target_kind": target_kind,
        "capture_scope": capture_scope,
        "display_fallback_used": display_fallback_used,
        "decoded_frame_count": decoded_frame_count,
        "selected_content_present": get("decoded_frames.selected_content_present"),
        "unrelated_sentinel_present": get("decoded_frames.unrelated_sentinel_present"),
        "full_display_leak_detected": get("decoded_frames.full_display_leak_detected"),
    },
}
open(report_path, "w", encoding="utf-8").write(json.dumps(report, indent=2, sort_keys=True) + "\n")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# Host remoteapp decoded-frame E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Target kind: `{expected_kind}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    if errors:
        f.write("\n## Errors\n\n")
        for error in errors:
            f.write(f"- {error}\n")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    sys.exit(1)
PY
}

if [[ "$SELF_TEST" == "1" ]]; then
  need_cmd python3
  bash -n "$0"
  grep -q "resource.refresh_remote_targets" "$0"
  grep -q "resource.watch_remote_targets" "$0"
  grep -q "remote_desktop.create_session" "$0"
  grep -q "Invocation.subject" "$0"
  grep -q "decoded_frames.unrelated_sentinel_present" "$0"
  grep -q "display_fallback_used" "$0"
  cat >"$EVIDENCE_JSON" <<'JSON'
{
  "status": "passed",
  "live_inventory": {"ability": "resource.refresh_remote_targets"},
  "selected_resource_ura": "easynet:///r/localhost/resource/device.dev/streams/window.test",
  "invocation": {
    "ability": "remote_desktop.create_session",
    "subject_ura": "easynet:///r/localhost/resource/device.dev/streams/window.test"
  },
  "target_binding": {
    "target_kind": "window",
    "capture_scope": "WindowSurface",
    "scope_audit": {
      "scope_widened": false,
      "display_fallback_used": false
    }
  },
  "transport": {"kind": "webrtc"},
  "decoded_frames": {
    "count": 3,
    "selected_content_present": true,
    "unrelated_sentinel_present": false,
    "full_display_leak_detected": false
  },
  "artifacts": {
    "decoded_frame_sample": "target/e2e/sample-frame.png"
  }
}
JSON
  validate_evidence
  echo "host-remoteapp-decoded-frame-e2e self-test ok"
  exit 0
fi

need_cmd python3

if [[ "$RUN" != "1" ]]; then
  write_skip_report
  echo "SKIP: $REPORT_MD"
  exit 0
fi

[[ -n "$PROBE_CMD" ]] || die "--run requires --probe-cmd or EASYNET_REMOTEAPP_FRAME_PROBE_CMD"

rm -f "$EVIDENCE_JSON"
export EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON="$EVIDENCE_JSON"
export EASYNET_REMOTEAPP_E2E_TARGET_KIND="$TARGET_KIND"

bash -lc "$PROBE_CMD"
[[ -s "$EVIDENCE_JSON" ]] || die "probe did not write evidence JSON: $EVIDENCE_JSON"

validate_evidence
echo "PASS: $REPORT_MD"
