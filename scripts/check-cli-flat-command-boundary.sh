#!/usr/bin/env bash
#
# Guard the layered CLI boundary.
#
# `join` / `start` / `stop` are FIRST-CLASS top-level Quickstart commands
# (the highest-frequency device-lifecycle verbs), NOT retired aliases. They
# coexist with the canonical layered forms (`device join`, `runtime start`,
# `runtime stop`) and forward to the same impls. F-039 (which removed them as
# "retired aliases") was reversed: the product wants these three as primary
# top-level UX. This check therefore guards that BOTH the Quickstart shortcuts
# AND the layered groups exist — not that the shortcuts are absent.

set -euo pipefail

ROOT="${CHECK_CLI_FLAT_COMMAND_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-cli-flat-command-boundary: $*" >&2
    exit 1
}

CLI_MOD="src/facade/cli/mod.rs"
[[ -f "$CLI_MOD" ]] || fail "missing $CLI_MOD"

# Quickstart shortcuts must be present and wired.
grep -q 'Join(join::JoinArgs)' "$CLI_MOD" \
    || fail "Quickstart 'join' command is missing"
grep -q 'Start(start::StartArgs)' "$CLI_MOD" \
    || fail "Quickstart 'start' command is missing"
grep -q 'Stop(stop::StopArgs)' "$CLI_MOD" \
    || fail "Quickstart 'stop' command is missing"

# The canonical layered homes must still exist (no behavioural drift: the two
# spellings share the same JoinArgs/StartArgs/StopArgs and run functions).
grep -q 'Device(groups::device::DeviceArgs)' "$CLI_MOD" \
    || fail "layered device command is missing"
grep -q 'Runtime(groups::runtime::RuntimeArgs)' "$CLI_MOD" \
    || fail "layered runtime command is missing"

echo "check-cli-flat-command-boundary: ok"
