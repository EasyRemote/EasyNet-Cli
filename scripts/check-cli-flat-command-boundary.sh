#!/usr/bin/env bash
#
# Guard the layered CLI boundary against retired top-level command aliases.

set -euo pipefail

ROOT="${CHECK_CLI_FLAT_COMMAND_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-cli-flat-command-boundary: $*" >&2
    exit 1
}

CLI_MOD="src/facade/cli/mod.rs"
[[ -f "$CLI_MOD" ]] || fail "missing $CLI_MOD"

bad="$(
    grep -nE 'Command::(Join|Start|Stop)|^[[:space:]]*(Join|Start|Stop)\(|alias of '\''(device join|runtime start|runtime stop)'\''|Shortcut for '\''easynet (device join|runtime start|runtime stop)'\''|Top-level shortcuts|Quickstart' \
        "$CLI_MOD" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "retired top-level CLI aliases still exist:
$bad"
fi

grep -q 'Device(groups::device::DeviceArgs)' "$CLI_MOD" \
    || fail "layered device command is missing"
grep -q 'Runtime(groups::runtime::RuntimeArgs)' "$CLI_MOD" \
    || fail "layered runtime command is missing"

bad_text="$(
    grep -RInE 'easynet (join|start|stop)([[:space:]`'\''")]|$)' \
        src/facade/cli scripts tests 2>/dev/null \
        | grep -vE 'device join|runtime start|runtime stop|scripts/check-cli-flat-command-boundary\.sh|tests/scripts/test_check_cli_flat_command_boundary\.sh' \
        || true
)"
if [[ -n "$bad_text" ]]; then
    fail "user-facing text still advertises retired flat commands:
$bad_text"
fi

echo "check-cli-flat-command-boundary: ok"
