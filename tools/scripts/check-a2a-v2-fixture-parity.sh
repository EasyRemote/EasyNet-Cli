#!/usr/bin/env bash
# Cross-repo drift guard for the v2 node roster label fixture.
#
# Spec: docs/spec/node-roster-label-v2.md §"Contract test (golden fixture)"
# says the byte-for-byte contract between this repo and the EasyNet
# backend is the fixture file — this repo's copy lives at
# `tests/fixtures/a2a-v2/golden.json`; the backend carries an identical
# copy at `backend/internal/axon/testdata/a2a-v2-golden.json`. A diff
# between the two is a three-way merge mistake and must fail CI.
#
# Run from the EasyNet-Cli repo root; points at the sibling EasyNet
# repo via EASYNET_REPO_ROOT (default: ../EasyNet).
#
# Exit 0 if the two files hash-equal, 1 on drift, 2 if either is missing.

set -euo pipefail

CLI_FIXTURE="tests/fixtures/a2a-v2/golden.json"
BACKEND_REPO="${EASYNET_REPO_ROOT:-../EasyNet}"
BACKEND_FIXTURE="${BACKEND_REPO}/backend/internal/axon/testdata/a2a-v2-golden.json"

if [[ ! -f "$CLI_FIXTURE" ]]; then
  echo "check-a2a-v2-fixture-parity: MISSING $CLI_FIXTURE" >&2
  exit 2
fi
if [[ ! -f "$BACKEND_FIXTURE" ]]; then
  echo "check-a2a-v2-fixture-parity: MISSING $BACKEND_FIXTURE (set EASYNET_REPO_ROOT if the sibling repo is elsewhere)" >&2
  exit 2
fi

# Use shasum (macOS) or sha256sum (Linux) — whichever is available.
if command -v sha256sum >/dev/null; then
  HASH_CMD="sha256sum"
elif command -v shasum >/dev/null; then
  HASH_CMD="shasum -a 256"
else
  echo "check-a2a-v2-fixture-parity: no sha256sum or shasum found" >&2
  exit 2
fi

CLI_HASH=$($HASH_CMD "$CLI_FIXTURE" | awk '{print $1}')
BACKEND_HASH=$($HASH_CMD "$BACKEND_FIXTURE" | awk '{print $1}')

if [[ "$CLI_HASH" != "$BACKEND_HASH" ]]; then
  echo "check-a2a-v2-fixture-parity: DRIFT" >&2
  echo "  CLI fixture      $CLI_FIXTURE: $CLI_HASH" >&2
  echo "  Backend fixture  $BACKEND_FIXTURE: $BACKEND_HASH" >&2
  echo >&2
  echo "  Both files must be byte-equal. Update the one that is stale:" >&2
  echo "    cp $CLI_FIXTURE $BACKEND_FIXTURE" >&2
  echo "  or the reverse, depending on which side's rewrite is authoritative." >&2
  exit 1
fi

echo "check-a2a-v2-fixture-parity: OK"
