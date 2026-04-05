#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# bump-version.sh — Fetch the latest version via `tide` and update all
# manifest files + lock files in the project.
#
# Usage:
#   ./scripts/bump-version.sh              # auto-detect via `tide mark --local-only`
#   ./scripts/bump-version.sh 1.2.3        # use an explicit version
#   ./scripts/bump-version.sh --dry-run    # show what would happen without changing files
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="${ROOT_DIR}/VERSION"
UPDATE_SCRIPT="${ROOT_DIR}/scripts/update-project-version.sh"

DRY_RUN=false
EXPLICIT_VERSION=""

for arg in "$@"; do
  case "${arg}" in
    --dry-run) DRY_RUN=true ;;
    *)         EXPLICIT_VERSION="${arg}" ;;
  esac
done

# ── Resolve the current (old) version from VERSION file ──────────────
OLD_VERSION=""
if [[ -f "${VERSION_FILE}" ]]; then
  OLD_VERSION="$(cat "${VERSION_FILE}")"
fi

# ── Determine the new version ────────────────────────────────────────
if [[ -n "${EXPLICIT_VERSION}" ]]; then
  NEW_VERSION="${EXPLICIT_VERSION}"
elif command -v tide >/dev/null 2>&1; then
  echo "Fetching latest version from tide …"
  NEW_VERSION="$(tide mark --local-only)"
  if [[ -z "${NEW_VERSION}" ]]; then
    echo "error: tide mark --local-only returned empty output" >&2
    exit 1
  fi
else
  echo "error: tide is not installed and no version argument was provided" >&2
  echo "  Install tide or pass a version explicitly: $0 <version>" >&2
  exit 1
fi

# ── Validate semver format ───────────────────────────────────────────
if ! [[ "${NEW_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z-]+)*$ ]]; then
  echo "error: invalid semver-like version '${NEW_VERSION}'" >&2
  exit 1
fi

# ── Show summary ─────────────────────────────────────────────────────
echo ""
echo "  Old version : ${OLD_VERSION:-<none>}"
echo "  New version : ${NEW_VERSION}"
echo ""

if [[ "${OLD_VERSION}" == "${NEW_VERSION}" ]]; then
  echo "Version is already ${NEW_VERSION} — nothing to do."
  exit 0
fi

if "${DRY_RUN}"; then
  echo "[dry-run] Would update ${OLD_VERSION:-<none>} → ${NEW_VERSION}"
  echo "[dry-run] No files were changed."
  exit 0
fi

# ── Delegate to the full update script (manifests + locks) ───────────
if [[ ! -x "${UPDATE_SCRIPT}" ]]; then
  echo "error: update script not found or not executable: ${UPDATE_SCRIPT}" >&2
  exit 1
fi

"${UPDATE_SCRIPT}" "${NEW_VERSION}"

echo ""
echo "Done. Version bumped from ${OLD_VERSION:-<none>} to ${NEW_VERSION}."
