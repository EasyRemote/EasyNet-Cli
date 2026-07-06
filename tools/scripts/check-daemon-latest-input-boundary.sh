#!/usr/bin/env bash
#
# Guard daemon request DTOs against legacy input alias compatibility paths.

set -euo pipefail

ROOT="${CHECK_DAEMON_LATEST_INPUT_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-daemon-latest-input-boundary: $*" >&2
    exit 1
}

bad="$(
    rg -n '#\s*\[\s*serde\s*\([^]]*\balias\s*=' \
        src/daemon/invocation/dispatch \
        src/protocol \
        -g'*.rs' 2>/dev/null || true
)"

if [[ -n "$bad" ]]; then
    fail "daemon/protocol request DTOs expose legacy serde input aliases:
$bad"
fi

echo "check-daemon-latest-input-boundary: ok"
