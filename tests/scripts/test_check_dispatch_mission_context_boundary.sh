#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-dispatch-mission-context-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/execution/mission"
cp "$SCRIPT" "$SB/tools/scripts/check-dispatch-mission-context-boundary.sh"
cat > "$SB/src/daemon/execution/mission/dispatch.rs" <<'RS'
fn check_mission_context_invariant() -> anyhow::Result<()> {
    anyhow::bail!("dispatch::send_to_agent called without a mission context");
}

fn check_run_dir() -> anyhow::Result<()> {
    anyhow::bail!("mission_id=fake does not correspond to an existing mission run dir");
}
RS

(
  cd "$SB"
  bash tools/scripts/check-dispatch-mission-context-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/daemon/execution/mission/dispatch.rs" <<'RS'
fn old_release_branch() -> anyhow::Result<()> {
    crate::op_event!(kind = send_to_agent_missing_mission_context);
    return Ok(());
}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-dispatch-mission-context-boundary.sh
) >/tmp/check-dispatch-mission-context.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "release compatibility branch should exit 1 (got $rc)"

cat > "$SB/src/daemon/execution/mission/dispatch.rs" <<'RS'
#[cfg(debug_assertions)]
fn debug_only() {}

fn check_mission_context_invariant() -> anyhow::Result<()> {
    anyhow::bail!("dispatch::send_to_agent called without a mission context");
}

fn check_run_dir() -> anyhow::Result<()> {
    anyhow::bail!("mission_id=fake does not correspond to an existing mission run dir");
}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-dispatch-mission-context-boundary.sh
) >/tmp/check-dispatch-mission-context.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "debug/release split should exit 1 (got $rc)"

echo "test_check_dispatch_mission_context_boundary.sh: all cases passed"
