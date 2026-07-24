#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_COMPANION_STATUS_PACKAGE_VERSION_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-companion-status-package-version-boundary: %s\n' "$1" >&2
  exit 1
}

PROJECTION="src/daemon/plugins/companion/projection.rs"
[[ -f "$PROJECTION" ]] || fail "missing $PROJECTION"

if ! rg -n 'let package_version = required_string\(obj, "package_version"\)\?;' "$PROJECTION" >/dev/null; then
  fail "Desktop companion status projection must require canonical package_version directly"
fi

if rg -n 'required_string\(obj, "package_version"\).*or_else|or_else\(\|_\| required_string\(obj, "version"\)\)|accepts_version_alias_for_package_version' "$PROJECTION"; then
  fail "Desktop companion status projection must not accept version as a package_version alias"
fi

if ! rg -n 'project_status_rejects_version_alias_for_package_version' "$PROJECTION" >/dev/null; then
  fail "Desktop companion status projection must test rejection of version alias"
fi

echo "check-companion-status-package-version-boundary: ok"
