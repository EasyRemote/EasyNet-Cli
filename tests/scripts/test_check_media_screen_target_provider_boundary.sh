#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-media-screen-target-provider-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-media-screen-target-provider-boundary.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

make_sandbox() {
  local sandbox
  sandbox="$(mktemp -d)"
  mkdir -p "$sandbox"
  cp -R "$REPO_ROOT/src" "$sandbox/src"
  echo "$sandbox"
}

run_check() {
  local sandbox="$1"
  (cd "$sandbox" && CHECK_MEDIA_SCREEN_TARGET_PROVIDER_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT")
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || {
  rm -rf "$SB"
  fail "happy: clean tree should pass"
}
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/enum DisplayMonitorSelector/enum DisplayMonitorSelection/' \
  "$SB/src/daemon/ability/builtins/resources/media/screen_snapshot.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing explicit monitor selector state should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/ability/builtins/resources/media/screen_snapshot.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace(
        "    match display_monitor_selector(entry)? {",
        "    let mut fallback_primary = None;\n    match display_monitor_selector(entry)? {",
        1,
    ),
    encoding="utf-8",
)
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "exact-selector primary fallback should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = "    macos_screen_targets::discover()\n"
replacement = '''    match macos_screen_targets::discover() {
        Ok(targets) if !targets.is_empty() => Ok(targets),
        Ok(_) => {
            crate::op_event!(
                component = media_resource_bootstrap,
                kind = native_screen_target_discovery_empty,
                fallback = "xcap",
            );
            discover_screen_targets_with_xcap()
        }
        Err(err) => {
            crate::op_event!(
                component = media_resource_bootstrap,
                kind = native_screen_target_discovery_failed,
                reason = err.to_string(),
                fallback = "xcap",
            );
            discover_screen_targets_with_xcap()
        }
    }
'''
path.write_text(text.replace(needle, replacement, 1), encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "macOS xcap fallback should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/screen_target_discovery: ScreenTargetDiscoveryState/screen_targets_scanned: bool/' \
  "$SB/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "boolean discovery state should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace(
        '#[cfg(not(target_os = "macos"))]\nfn discover_screen_targets_with_xcap',
        'fn discover_screen_targets_with_xcap',
        1,
    ),
    encoding="utf-8",
)
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "uncfg-gated xcap provider should exit 1 (got $rc)"

echo "test_check_media_screen_target_provider_boundary.sh: all cases passed"
