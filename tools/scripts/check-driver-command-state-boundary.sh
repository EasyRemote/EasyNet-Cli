#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

ADAPTER="src/daemon/execution/mission/adapter.rs"
CLAUDE="src/daemon/execution/mission/drivers/claude_code.rs"
CODEX="src/daemon/execution/mission/drivers/codex.rs"
EXTERNAL="src/daemon/execution/mission/drivers/external.rs"
DISPATCH="src/daemon/execution/mission/dispatch.rs"
TARGETS=("$ADAPTER" "$CLAUDE" "$CODEX" "$EXTERNAL" "$DISPATCH")

for target in "${TARGETS[@]}"; do
  [[ -f "$target" ]] || fail "missing driver command target: $target"
done

if ! rg -n 'enum DriverCommand' "$ADAPTER" >/dev/null; then
  fail "mission adapter must own DriverCommand"
fi

if ! rg -n 'Default' "$ADAPTER" >/dev/null || ! rg -n 'Explicit\(String\)' "$ADAPTER" >/dev/null; then
  fail "DriverCommand must model Default and Explicit states"
fi

if ! rg -n 'from_registry_value' "$ADAPTER" >/dev/null; then
  fail "DriverCommand must provide the single persisted-string bridge"
fi

if ! rg -n 'pub command: DriverCommand' "$ADAPTER" "$CLAUDE" "$CODEX" >/dev/null; then
  fail "InvokeOpts, ClaudeOptions, and CodexOptions must carry DriverCommand"
fi

if ! rg -n 'command: DriverCommand::from_registry_value\(&entry\.command\)' "$DISPATCH" >/dev/null; then
    fail "dispatch must convert AgentEntry::command into DriverCommand exactly once"
fi

duplicate_registry_bridge="$(
  rg -n 'DriverCommand::from_registry_value\(&entry\.command\)' "$CLAUDE" "$CODEX" "$EXTERNAL" || true
)"
if [[ -n "$duplicate_registry_bridge" ]]; then
  fail "drivers must consume InvokeOpts.command instead of reparsing AgentEntry::command:
$duplicate_registry_bridge"
fi

if ! rg -n 'self\.command\.resolve\(DEFAULT_CLAUDE_BINARY\)' "$CLAUDE" >/dev/null; then
    fail "Claude driver must resolve DriverCommand through its canonical binary"
fi

if ! rg -n 'self\.command\.resolve\(DEFAULT_CODEX_BINARY\)' "$CODEX" >/dev/null; then
  fail "Codex driver must resolve DriverCommand through its canonical binary"
fi

if ! rg -n 'opts\.command\.explicit\(\)' "$EXTERNAL" >/dev/null; then
  fail "External driver must require an explicit command state"
fi

if rg -n \
  'pub command: String|command:\\s*String::new\\(\\)|Empty string means|empty string signalling|empty-string fallback|falls back to `DEFAULT_|falls through to the driver default|opts\\.command\\.clone\\(\\)|entry\\.command\\.clone\\(\\)' \
  "${TARGETS[@]}"; then
  fail "mission driver command seam still uses string sentinel fallback"
fi
