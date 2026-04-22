#!/usr/bin/env bash
#
# Integration tests for scripts/check-upstream-names.sh.
#
# Covers:
#   happy:
#     - a clean tree passes (the committed code must remain clean)
#   failure:
#     - a forbidden token in a *.rs file under src/ trips the script
#     - a forbidden token in Cargo.toml trips the script
#     - exit code is 1 (not 0, not 2) on violations
#   edge:
#     - docs/**/*.md is exempt — citation in design docs is allowed
#     - a forbidden token inside a *string literal in code* still trips
#       it (citations belong in comments in docs, not string constants)
#     - an unbanned variant (e.g. all-caps) does NOT trigger — the
#       banned list is explicit, not a broad wildcard
#
# Isolation model:
#   Every mutation runs in a per-case temp sandbox. We never write to
#   the real src/ or Cargo.toml, so the test is safe to run in parallel
#   with other tests and safe to ctrl-c without a restore trap.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-upstream-names.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

# Build a sandbox that mirrors the repo structure the check script scans
# — `src/**/*.rs` and the top-level Cargo.toml. docs/ is created empty
# and populated per-case where the "exempt" edge matters. We copy real
# src/ content so the clean baseline is genuine (not a minimal stub).
make_sandbox() {
  local sandbox
  sandbox="$(mktemp -d)"
  cp -R "$REPO_ROOT/src" "$sandbox/src"
  cp "$REPO_ROOT/Cargo.toml" "$sandbox/Cargo.toml"
  mkdir -p "$sandbox/docs"
  echo "$sandbox"
}

run_check() {
  local sandbox="$1"
  ( cd "$sandbox" && CHECK_UPSTREAM_REPO_ROOT="$sandbox" "$SCRIPT" )
}

# --- happy: clean sandbox passes ---------------------------------------
SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: clean tree should pass"; }
rm -rf "$SB"

# --- failure: forbidden token in src/*.rs ------------------------------
SB="$(make_sandbox)"
printf '// paseo inspiration note\n' >"$SB/src/__probe_rs.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "failure: forbidden token in *.rs should exit 1 (got $rc)"

# --- failure: forbidden token in Cargo.toml ----------------------------
SB="$(make_sandbox)"
printf '\n# paseo note\n' >>"$SB/Cargo.toml"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "failure: forbidden token in Cargo.toml should exit 1 (got $rc)"

# --- edge: docs/*.md is exempt ----------------------------------------
SB="$(make_sandbox)"
printf 'Paseo is a great reference.\n' >"$SB/docs/note.md"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "edge: docs/ should be exempt"; }
rm -rf "$SB"

# --- edge: string literal in code trips the script --------------------
SB="$(make_sandbox)"
cat >"$SB/src/__probe_str.rs" <<'EOF'
pub fn probe() -> &'static str { "paseo ref" }
EOF
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "edge: forbidden token in a string literal should still trip (got $rc)"

# --- edge: an unbanned variant does not trip --------------------------
# The FORBIDDEN regex is explicit about which forms are banned. All-caps
# "PASEO" is not on the list and must NOT trip. This guards against a
# future accidental broadening of the regex.
SB="$(make_sandbox)"
printf '// PASEO (all caps) is not on the banned list\n' >"$SB/src/__probe_unbanned.rs"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "edge: all-caps variant should not trip"; }
rm -rf "$SB"

echo "test_check_upstream_names.sh: all cases passed"
