#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-runtime-abilities-manifest-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/scripts" "$SB/src/runtime"
cp "$SCRIPT" "$SB/scripts/check-runtime-abilities-manifest-boundary.sh"
cat > "$SB/src/runtime/abilities.rs" <<'RS'
fn open_entry_directory() {
    eprintln!("registry row is missing root_path");
    eprintln!("root_path /tmp/a belongs to agent \"other\"");
}
RS

(
  cd "$SB"
  bash scripts/check-runtime-abilities-manifest-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/runtime/abilities.rs" <<'RS'
fn chat_ability() {}
fn ensure_chat_manifest() {}
RS
set +e
(
  cd "$SB"
  bash scripts/check-runtime-abilities-manifest-boundary.sh
) >/tmp/check-runtime-abilities.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "synthetic chat helpers should exit 1 (got $rc)"

cat > "$SB/src/runtime/abilities.rs" <<'RS'
fn open_entry_directory() {
    let _root = crate::persistence::config::agents_root().join(agent_name);
    eprintln!("registry row is missing root_path");
    eprintln!("root_path /tmp/a belongs to agent \"other\"");
}
RS
set +e
(
  cd "$SB"
  bash scripts/check-runtime-abilities-manifest-boundary.sh
) >/tmp/check-runtime-abilities.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "agents_root fallback should exit 1 (got $rc)"

echo "test_check_runtime_abilities_manifest_boundary.sh: all cases passed"
