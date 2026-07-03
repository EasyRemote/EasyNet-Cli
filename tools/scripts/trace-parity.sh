#!/usr/bin/env bash
#
# trace-parity.sh — structural parity check for EAL Mission IR.
#
# Why this script exists:
#   Any PR that touches the runtime or EAL layer can silently alter the
#   structural shape of a compiled mission (field renamed, step order
#   permuted, new field leaked, phase layout changed). This script takes
#   a fixed `.eal` input and compares the compiled Mission IR JSON
#   (structure only, not values) against a frozen fixture.
#
#   The check is *structural*: we assert the set of JSON keys and their
#   nesting is identical, but field values are allowed to differ. This
#   catches schema drift without being a brittle golden-output test.
#
# Why compile-only, not run-only:
#   Running a mission requires a live Axon hub and real agents/devices.
#   That is not reproducible in CI. `mission compile --emit-ir` is a
#   pure function of the source and the compiler, so it's the right
#   invariant to pin.
#
# Exit codes:
#   0  — structure matches fixture
#   1  — structure diverged (prints a unified diff of the key-set dumps)
#   2  — environment error (missing binary, missing fixture, etc.)
#
# Usage:
#   tools/scripts/trace-parity.sh
#
# To regenerate the fixture after a *deliberate* schema change:
#   tools/scripts/trace-parity.sh --update
#
set -euo pipefail

# Resolve REPO_ROOT as the current working directory when a caller sets
# it explicitly (e.g. integration tests running the script inside a
# sandbox that shadows only the fixture directory). Otherwise walk up
# from the script's location to the repo. The fixture and binary are
# always read relative to REPO_ROOT, so the two paths are consistent.
if [[ -n "${TRACE_PARITY_REPO_ROOT:-}" ]]; then
  REPO_ROOT="$TRACE_PARITY_REPO_ROOT"
else
  REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fi
cd "$REPO_ROOT"

FIXTURE_EAL="examples/diamond.eal"
FIXTURE_KEYS="tests/fixtures/trace-parity/diamond.keys.txt"

# --- Environment checks ---------------------------------------------------

if ! command -v jq >/dev/null 2>&1; then
  echo "trace-parity: 'jq' is required but not installed" >&2
  exit 2
fi

if [[ ! -f "$FIXTURE_EAL" ]]; then
  echo "trace-parity: input file missing: $FIXTURE_EAL" >&2
  exit 2
fi

# Prefer a pre-built binary (fast path for CI) and fall back to `cargo run`
# when invoked from a fresh worktree. Both paths must emit the same IR.
BIN="target/debug/easynet"
if [[ ! -x "$BIN" ]]; then
  cargo build --quiet
fi
if [[ ! -x "$BIN" ]]; then
  echo "trace-parity: debug binary missing after build: $BIN" >&2
  exit 2
fi

# --- Collect the sorted key set of the compiled IR ------------------------
#
# `jq --stream` enumerates every path in the JSON tree. We drop the
# value on each line (field 0 = path array, field 1 = leaf value) and
# keep only the unique, sorted path set. Two IRs with the same schema
# but different literal values will produce identical key dumps.
current_keys() {
  "$BIN" mission compile "$FIXTURE_EAL" --emit-ir \
    | jq --stream -c 'map(if type == "number" then "#" else . end) | .[0]' \
    | sort -u
}

# --- Actions --------------------------------------------------------------

update_fixture() {
  mkdir -p "$(dirname "$FIXTURE_KEYS")"
  current_keys >"$FIXTURE_KEYS"
  echo "trace-parity: fixture refreshed: $FIXTURE_KEYS"
}

compare() {
  if [[ ! -f "$FIXTURE_KEYS" ]]; then
    echo "trace-parity: fixture missing: $FIXTURE_KEYS" >&2
    echo "  (run 'tools/scripts/trace-parity.sh --update' to seed it)" >&2
    exit 2
  fi

  local tmp
  tmp="$(mktemp)"
  current_keys >"$tmp"

  if diff -u "$FIXTURE_KEYS" "$tmp"; then
    echo "trace-parity: OK (Mission IR structure matches fixture)"
    rm -f "$tmp"
  else
    echo "" >&2
    echo "trace-parity: FAIL — Mission IR structure diverged from fixture." >&2
    echo "  Fixture: $FIXTURE_KEYS" >&2
    echo "  Input:   $FIXTURE_EAL" >&2
    echo "" >&2
    echo "If the change is deliberate, refresh the fixture with:" >&2
    echo "  tools/scripts/trace-parity.sh --update" >&2
    echo "and commit it alongside the schema change." >&2
    rm -f "$tmp"
    exit 1
  fi
}

case "${1:-check}" in
  --update|update)
    update_fixture
    ;;
  --help|-h)
    sed -n '/^#/!q;p' "$0"
    ;;
  *)
    compare
    ;;
esac
