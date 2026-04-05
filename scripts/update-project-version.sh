#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# update-project-version.sh — Synchronise the version string across
# every manifest and lock file in EasyNet-Cli.
#
# Usage:
#   ./scripts/update-project-version.sh 0.2.0
#   ./scripts/update-project-version.sh          # reads from `tide` or VERSION
# ──────────────────────────────────────────────────────────────────────
set -eo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="${ROOT_DIR}/VERSION"

if [[ "${1:-}" != "" ]]; then
  NEW_VERSION="$1"
elif command -v tide >/dev/null 2>&1; then
  NEW_VERSION="$(tide mark --local-only)"
elif [[ -f "${VERSION_FILE}" ]]; then
  NEW_VERSION="$(cat "${VERSION_FILE}")"
else
  echo "warning: tide not found and VERSION file missing; skip version sync" >&2
  exit 0
fi

if ! [[ "${NEW_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z-]+)*$ ]]; then
  echo "error: invalid semver-like version '${NEW_VERSION}'" >&2
  exit 1
fi

export NEW_VERSION

# ── Write VERSION file ───────────────────────────────────────────────
printf '%s\n' "${NEW_VERSION}" > "${VERSION_FILE}"

# ── Helper: update `version = "..."` in TOML files ──────────────────
update_toml_version() {
  local file="$1"
  perl -0pi -e 's/(^[[:space:]]*version[[:space:]]*=[[:space:]]*")[^"]*(")/$1$ENV{"NEW_VERSION"}$2/m' "${file}"
}

# ── Helper: update `"version": "..."` in JSON files ─────────────────
update_json_version() {
  local file="$1"
  perl -0pi -e 's/("version"\s*:\s*")[^"]*(")/$1$ENV{"NEW_VERSION"}$2/m' "${file}"
}

# ── Find and update all Cargo.toml files ─────────────────────────────
CARGO_FILES=()
while IFS= read -r file; do
  CARGO_FILES+=("${file}")
done < <(find "${ROOT_DIR}" \
  -name 'Cargo.toml' \
  -not -path '*/.git/*' \
  -not -path '*/target/*' \
  -not -path '*/node_modules/*')

for file in "${CARGO_FILES[@]}"; do
  echo "  ✎ ${file}"
  update_toml_version "${file}"
done

# ── Find and update all package.json files ───────────────────────────
PACKAGE_JSON_FILES=()
while IFS= read -r file; do
  PACKAGE_JSON_FILES+=("${file}")
done < <(find "${ROOT_DIR}" \
  -name 'package.json' \
  -not -path '*/.git/*' \
  -not -path '*/target/*' \
  -not -path '*/node_modules/*' \
  -not -path '*/dist/*' 2>/dev/null || true)

if [[ ${#PACKAGE_JSON_FILES[@]} -gt 0 ]]; then
  for file in "${PACKAGE_JSON_FILES[@]}"; do
    echo "  ✎ ${file}"
    update_json_version "${file}"
  done
fi

# ── Lock-file regeneration ───────────────────────────────────────────
if command -v cargo >/dev/null 2>&1; then
  for file in "${CARGO_FILES[@]}"; do
    lock_dir="$(dirname "${file}")"
    if [[ -f "${lock_dir}/Cargo.lock" ]]; then
      echo "  ↻ cargo: ${lock_dir}/Cargo.lock"
      (cd "${lock_dir}" && cargo generate-lockfile --quiet 2>/dev/null) || \
        echo "  ⚠ cargo generate-lockfile failed in ${lock_dir}" >&2
    fi
  done
else
  echo "  ℹ cargo not found; skipping Cargo.lock regeneration" >&2
fi

echo "Updated version to ${NEW_VERSION}"
