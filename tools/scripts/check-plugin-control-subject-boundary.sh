#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/commands/groups/plugin.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum PluginControlSubject' "$TARGET" >/dev/null; then
  fail "plugin control subject resolution must use an explicit state enum"
fi

if ! rg -n 'load_credentials_optional\(\)\?' "$TARGET" >/dev/null; then
  fail "plugin control subject resolution must distinguish missing credentials with load_credentials_optional"
fi

if rg -n 'is_missing_or_incomplete_credentials|plugin_control_subject_ura|load_credentials\(\)|no credentials found|credentials file is incomplete|contains\("credentials' "$TARGET"; then
  fail "plugin control must not sniff credential error strings or collapse incomplete credentials into unpaired state"
fi

if ! rg -n 'plugin_control_subject_rejects_malformed_credentials' "$TARGET" >/dev/null; then
  fail "plugin control must test malformed credentials as fail-closed"
fi

if ! rg -n 'plugin_control_subject_rejects_incomplete_credentials' "$TARGET" >/dev/null; then
  fail "plugin control must test incomplete credentials as fail-closed"
fi

echo "check-plugin-control-subject-boundary: ok"
