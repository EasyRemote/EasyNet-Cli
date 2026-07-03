#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-workspace-agent-directory-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/execution/mission" "$SB/tests"
cp "$SCRIPT" "$SB/tools/scripts/check-workspace-agent-directory-boundary.sh"
cat > "$SB/src/daemon/execution/mission/workspace.rs" <<'RS'
pub fn ensure_from_directory() {}
RS
cat > "$SB/src/daemon/execution/mission/dispatch.rs" <<'RS'
fn dispatch(root: &std::path::Path) {
    let _directory = AgentDirectory::open(root).unwrap();
    anyhow::anyhow!("agent {agent_name:?} registry row is missing root_path");
}
RS

(
  cd "$SB"
  bash tools/scripts/check-workspace-agent-directory-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/daemon/execution/mission/workspace.rs" <<'RS'
fn ensure_workspace() {}
fn spec_from_entry() {}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-workspace-agent-directory-boundary.sh
) >/tmp/check-workspace-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired workspace shim should exit 1 (got $rc)"
cat > "$SB/src/daemon/execution/mission/workspace.rs" <<'RS'
pub fn ensure_from_directory() {}
RS

cat >> "$SB/src/daemon/execution/mission/dispatch.rs" <<'RS'
fn fallback() {
    let cwd_for_adapter = cwd.clone().unwrap_or_default();
}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-workspace-agent-directory-boundary.sh
) >/tmp/check-workspace-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "caller-cwd fallback should exit 1 (got $rc)"

echo "test_check_workspace_agent_directory_boundary.sh: all cases passed"
