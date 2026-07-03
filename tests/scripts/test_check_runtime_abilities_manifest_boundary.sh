#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-runtime-abilities-manifest-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/execution/mission"
cp "$SCRIPT" "$SB/tools/scripts/check-runtime-abilities-manifest-boundary.sh"
cat > "$SB/src/daemon/execution/mission/agent_ability_specs.rs" <<'RS'
fn open_entry_directory() {
    eprintln!("registry row is missing root_path");
    eprintln!("root_path /tmp/a belongs to agent \"other\"");
}

fn abilities_for_returns_empty_when_root_path_missing() {}
fn entry_without_root_path_publishes_no_abilities() {}
fn abilities_for_publication_synthesizes_default_chat_without_root_path() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-runtime-abilities-manifest-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/daemon/execution/mission/agent_ability_specs.rs" <<'RS'
fn chat_ability() {}
fn ensure_chat_manifest() {}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-abilities-manifest-boundary.sh
) >/tmp/check-runtime-abilities.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "synthetic chat helpers should exit 1 (got $rc)"

cat > "$SB/src/daemon/execution/mission/agent_ability_specs.rs" <<'RS'
fn open_entry_directory() {
    let _root = crate::persistence::config::agents_root().join(agent_name);
    eprintln!("registry row is missing root_path");
    eprintln!("root_path /tmp/a belongs to agent \"other\"");
}

fn abilities_for_returns_empty_when_root_path_missing() {}
fn entry_without_root_path_publishes_no_abilities() {}
fn abilities_for_publication_synthesizes_default_chat_without_root_path() {}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-abilities-manifest-boundary.sh
) >/tmp/check-runtime-abilities.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "agents_root fallback should exit 1 (got $rc)"

echo "test_check_runtime_abilities_manifest_boundary.sh: all cases passed"
