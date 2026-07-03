#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/daemon/execution/mission/dispatch.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if rg -n 'fn ensure_workspace|ensure_workspace\(|fn spec_from_entry|spec_from_entry\(' src tests -g '*.rs'; then
  fail "workspace projection must not expose retired AgentEntry reconstruction shims"
fi

if rg -n 'workspace_provisioning_failed|no_project_level_mcp_or_context|fallback = "no_project_level_mcp_or_context"|runs in the caller.?s cwd|cwd_for_adapter|cwd\.clone\(\)\.unwrap_or_default\(\)' "$TARGET"; then
  fail "dispatch must fail on invalid AgentDirectory state instead of falling back to caller cwd"
fi

if ! rg -n 'registry row is missing root_path' "$TARGET" >/dev/null; then
  fail "dispatch must explicitly reject registry rows without root_path"
fi

if ! rg -n 'AgentDirectory::open\(root\)' "$TARGET" >/dev/null; then
  fail "dispatch must open the registered AgentDirectory before projection"
fi

echo "check-workspace-agent-directory-boundary: ok"
