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

runtime_dispatch_fallback="$(
    rg -n 'default_mode|mode_omitted_defaults_to_rpc|backwards compat|backwards-compat|stale runtime|stale axon-runtime|legacy single-line|no-mode request' \
        src/daemon/control/runtime_dispatch.rs 2>/dev/null || true
)"

if [[ -n "$runtime_dispatch_fallback" ]]; then
    fail "runtime-dispatch must require the latest explicit mode field:
$runtime_dispatch_fallback"
fi

echo "check-daemon-latest-input-boundary: ok"
