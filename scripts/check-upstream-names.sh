#!/usr/bin/env bash
#
# check-upstream-names.sh — forbid upstream-reference keywords in code.
#
# Why this script exists:
#   When a project is inspired by an external implementation, the
#   inspiration should live in design notes and the plan file — not
#   leak into source files or manifests. Names and year tags in code
#   age poorly, tie us to a version we no longer track, and muddy the
#   blame for future contributors who wonder why a specific upstream
#   is cited in a module that has long since diverged.
#
#   This script keeps source and config pristine. Design docs under
#   `docs/`, the top-level README/CHANGELOG, and plan scratch files
#   are exempt — those are the right places to attribute inspiration.
#
# Keyword list (edit here to update):
#   The single source of truth is the FORBIDDEN regex below. Every
#   banned token is a word-boundary match so we don't flag unrelated
#   substrings.
#
# Exit codes:
#   0 — no forbidden tokens found in scanned files
#   1 — at least one forbidden token present (locations printed)
#   2 — environment error
#
# Usage:
#   scripts/check-upstream-names.sh
#
set -euo pipefail

# Resolve REPO_ROOT as the directory whose src/ and Cargo.toml to scan.
# Tests can override this with CHECK_UPSTREAM_REPO_ROOT to scan a
# sandbox instead of the real working tree. Without an override we walk
# up from the script's location.
if [[ -n "${CHECK_UPSTREAM_REPO_ROOT:-}" ]]; then
  REPO_ROOT="$CHECK_UPSTREAM_REPO_ROOT"
else
  REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi
cd "$REPO_ROOT"

# --- Forbidden tokens ----------------------------------------------------
#
# Add new entries as `|\bnew_token\b`. Keep the list short; every entry
# is something we explicitly refuse to cite in code or Cargo metadata.
FORBIDDEN='\bpaseo\b|\bgetpaseo\b|\bPaseo\b|@getpaseo'

# --- Scan scope ----------------------------------------------------------
#
# Scan *.rs under src/ (excluding target/ which is build artefact) and
# the top-level Cargo.toml. Everything else — docs, plans, gallery
# manifests, scripts — is deliberately out of scope.
SCAN_PATHS=("src")
SCAN_TOML=("Cargo.toml")

# --- Environment checks --------------------------------------------------

if ! command -v grep >/dev/null 2>&1; then
  echo "check-upstream-names: 'grep' missing" >&2
  exit 2
fi

# --- Run ----------------------------------------------------------------

hits=0

for path in "${SCAN_PATHS[@]}"; do
  if [[ -d "$path" ]]; then
    if grep -REn --include='*.rs' "$FORBIDDEN" "$path"; then
      hits=1
    fi
  fi
done

for toml in "${SCAN_TOML[@]}"; do
  if [[ -f "$toml" ]]; then
    if grep -En "$FORBIDDEN" "$toml"; then
      hits=1
    fi
  fi
done

if (( hits != 0 )); then
  echo "" >&2
  echo "check-upstream-names: FAIL — forbidden tokens found above." >&2
  echo "  If this is a legitimate inspiration reference, move it to" >&2
  echo "  docs/, README.md, CHANGELOG.md, or a plan file instead." >&2
  exit 1
fi

echo "check-upstream-names: OK"
