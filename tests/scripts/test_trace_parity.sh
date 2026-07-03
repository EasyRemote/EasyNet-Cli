#!/usr/bin/env bash
#
# Integration tests for tools/scripts/trace-parity.sh.
#
# Covers:
#   happy:
#     - current fixture matches the checked-in key dump
#     - --update regenerates the fixture deterministically (byte-equal
#       across two consecutive runs on the same sources)
#   failure:
#     - missing fixture file → exit code 2 + guidance to --update
#     - tampered fixture (extra key)   → exit code 1 + unified diff
#     - tampered fixture (removed key) → exit code 1 + unified diff
#   edge:
#     - script is idempotent when the fixture is already up-to-date
#     - the fixture contains only path arrays (no leaf values leak in),
#       protecting against a schema change that trips the "structure
#       only" invariant.
#
# Isolation model:
#   Every mutation of the fixture runs against a private copy in a
#   per-test temp directory — never the committed fixture. This keeps
#   the test safe to run in parallel with other shell tests that share
#   the same repo, and safe to `ctrl-c` without a restore trap.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

SCRIPT="$REPO_ROOT/tools/scripts/trace-parity.sh"
FIXTURE_NAME="diamond.keys.txt"
PRISTINE="$REPO_ROOT/tests/fixtures/trace-parity/$FIXTURE_NAME"

fail() { echo "FAIL: $*" >&2; exit 1; }

# --- Isolation harness ----------------------------------------------
# The trace-parity script resolves the fixture relative to REPO_ROOT
# ($REPO_ROOT/tests/fixtures/trace-parity/diamond.keys.txt). To run the
# script against a mutated fixture without touching the committed one,
# we spin up a sandbox that shadows just the fixture directory via a
# symlink swap: tests/fixtures/trace-parity in the sandbox points to a
# writable copy; the rest of the repo is a set of symlinks to the real
# working tree so cargo artifacts, examples, Cargo.toml all resolve.
run_with_fixture() {
  local fixture_content_source="$1"  # path to the file to seed the fixture with, or "" to omit it entirely
  local sandbox
  sandbox="$(mktemp -d)"
  # Mirror top-level repo entries as symlinks, skipping tests/ which
  # we materialize below. Dotfiles included because Cargo.lock-like
  # hidden files could affect resolution.
  shopt -s dotglob nullglob
  for item in "$REPO_ROOT"/*; do
    local name
    name="$(basename "$item")"
    case "$name" in
      tests) ;;  # will materialize below
      *) ln -s "$item" "$sandbox/$name" ;;
    esac
  done
  shopt -u dotglob nullglob
  # Build tests/ as a directory so we can mutate the fixture inside it
  # without touching the real repo.
  mkdir -p "$sandbox/tests/fixtures/trace-parity"
  # scripts themselves are symlinked via the top-level loop above, so
  # the fixture directory is the only writable surface.
  for real in "$REPO_ROOT"/tests/*; do
    local name
    name="$(basename "$real")"
    case "$name" in
      fixtures) ;;  # we own this subtree in the sandbox
      *) ln -s "$real" "$sandbox/tests/$name" ;;
    esac
  done
  mkdir -p "$sandbox/tests/fixtures"
  for real in "$REPO_ROOT"/tests/fixtures/*; do
    local name
    name="$(basename "$real")"
    case "$name" in
      trace-parity) ;;  # materialized
      *) ln -s "$real" "$sandbox/tests/fixtures/$name" ;;
    esac
  done
  if [[ -n "$fixture_content_source" ]]; then
    cp "$fixture_content_source" "$sandbox/tests/fixtures/trace-parity/$FIXTURE_NAME"
  fi
  echo "$sandbox"
}

# Run the script inside a sandbox rooted at $1. The script reads
# TRACE_PARITY_REPO_ROOT to locate the fixture and binary; everything
# else resolves by walking the shadow tree.
script_in() {
  local sandbox="$1"; shift
  ( cd "$sandbox" && TRACE_PARITY_REPO_ROOT="$sandbox" "$SCRIPT" "$@" )
}

# --- happy: current fixture matches ------------------------------------
SB="$(run_with_fixture "$PRISTINE")"
script_in "$SB" >/dev/null 2>&1 || fail "happy: trace-parity should match on fresh checkout"
rm -rf "$SB"

# --- happy: --update is byte-deterministic ------------------------------
SB="$(run_with_fixture "$PRISTINE")"
script_in "$SB" --update >/dev/null
A="$(shasum -a 256 "$SB/tests/fixtures/trace-parity/$FIXTURE_NAME" | cut -d' ' -f1)"
script_in "$SB" --update >/dev/null
B="$(shasum -a 256 "$SB/tests/fixtures/trace-parity/$FIXTURE_NAME" | cut -d' ' -f1)"
[[ "$A" == "$B" ]] || { rm -rf "$SB"; fail "happy: --update is not deterministic ($A vs $B)"; }
rm -rf "$SB"

# --- edge: fixture is idempotent when unchanged ------------------------
# Updating an up-to-date fixture must leave it byte-identical to the
# committed version (proves the fixture is stable under the current
# compiler).
SB="$(run_with_fixture "$PRISTINE")"
before="$(shasum -a 256 "$SB/tests/fixtures/trace-parity/$FIXTURE_NAME" | cut -d' ' -f1)"
script_in "$SB" --update >/dev/null
script_in "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "edge: idempotent update should still match"; }
after="$(shasum -a 256 "$SB/tests/fixtures/trace-parity/$FIXTURE_NAME" | cut -d' ' -f1)"
[[ "$before" == "$after" ]] || { rm -rf "$SB"; fail "edge: fixture drifted under idempotent --update ($before → $after)"; }
rm -rf "$SB"

# --- edge: fixture contains only JSON paths, no leaf values ------------
# Every line must start with '[' (a jq --stream path array) and end with
# ']'. If a value ever leaks in (e.g. because we misconfigured jq), the
# fixture would contain bare numbers or strings — catch that here.
while IFS= read -r line; do
  [[ "$line" =~ ^\[.*\]$ ]] || fail "edge: fixture leaked non-path line: $line"
done <"$PRISTINE"

# --- failure: missing fixture -----------------------------------------
SB="$(run_with_fixture "")"
rc=0
script_in "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "2" ]] || fail "failure: missing fixture should exit 2 (got $rc)"

# --- failure: fixture tampered with an extra spurious key --------------
SB="$(run_with_fixture "$PRISTINE")"
echo '["spurious","injected_key"]' >>"$SB/tests/fixtures/trace-parity/$FIXTURE_NAME"
rc=0
script_in "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "failure: extra key should exit 1 (got $rc)"

# --- failure: fixture tampered by removing a real key ------------------
SB="$(run_with_fixture "$PRISTINE")"
# sed -i differs across BSD/GNU; use a portable delete-last-line idiom.
tmpf="$(mktemp)"
head -n -1 "$SB/tests/fixtures/trace-parity/$FIXTURE_NAME" >"$tmpf" 2>/dev/null \
  || awk 'NR>1 { print prev } { prev=$0 }' "$SB/tests/fixtures/trace-parity/$FIXTURE_NAME" >"$tmpf"
mv "$tmpf" "$SB/tests/fixtures/trace-parity/$FIXTURE_NAME"
rc=0
script_in "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "failure: removed-key should exit 1 (got $rc)"

echo "test_trace_parity.sh: all cases passed"
