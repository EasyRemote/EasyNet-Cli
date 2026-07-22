#!/usr/bin/env bash
#
# Guard media screen-target resource bootstrap against provider fallback forks.
#
# Resource bootstrap writes durable resource URAs. macOS must use one
# authoritative screen-target provider (CoreGraphics) instead of silently
# repopulating resources through xcap when the native provider is empty or
# unavailable.

set -euo pipefail

ROOT="${CHECK_MEDIA_SCREEN_TARGET_PROVIDER_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

TARGET="src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"

[[ -f "$TARGET" ]] || fail "required file missing: $TARGET"

python3 - "$TARGET" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
violations: list[str] = []

required = (
    "enum ScreenTargetDiscoveryState",
    "NotAttempted",
    "Scanned",
    "Unavailable",
    "permits_stale_prune",
)
for token in required:
    if token not in text:
        violations.append(f"missing explicit screen-target discovery state token: {token}")

if "screen_targets_scanned: bool" in text:
    violations.append("screen-target discovery state must not collapse to a bool")

match = re.search(
    r"#\[cfg\(target_os = \"macos\"\)\]\s*fn discover_screen_targets\(\) -> anyhow::Result<Vec<DiscoveredResource>> \{(?P<body>.*?)\n\}",
    text,
    flags=re.S,
)
if not match:
    violations.append("missing macOS discover_screen_targets provider boundary")
else:
    body = match.group("body")
    for token in (
        "discover_screen_targets_with_xcap",
        'fallback = "xcap"',
        "native_screen_target_discovery_empty",
        "native_screen_target_discovery_failed",
    ):
        if token in body:
            violations.append(f"macOS screen-target discovery reintroduces xcap fallback: {token}")
    if "macos_screen_targets::discover()" not in body:
        violations.append("macOS screen-target discovery must delegate to the native provider")

if not re.search(
    r"#\[cfg\(not\(target_os = \"macos\"\)\)\]\s*fn discover_screen_targets\(\) -> anyhow::Result<Vec<DiscoveredResource>> \{(?P<body>.*?)discover_screen_targets_with_xcap\(\)",
    text,
    flags=re.S,
):
    violations.append("non-macOS screen-target discovery must keep xcap as its current provider")

if not re.search(
    r"#\[cfg\(not\(target_os = \"macos\"\)\)\]\s*fn discover_screen_targets_with_xcap",
    text,
):
    violations.append("xcap screen-target provider must be cfg(not(target_os = \"macos\"))")

if violations:
    print("\n".join(violations), file=sys.stderr)
    raise SystemExit(1)
PY

echo "check-media-screen-target-provider-boundary: OK"
