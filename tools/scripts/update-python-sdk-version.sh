#!/usr/bin/env bash
# Synchronize the independent easynet-sdk Python distribution version.
#
# Usage:
#   ./tools/scripts/update-python-sdk-version.sh
#   ./tools/scripts/update-python-sdk-version.sh --check
#   ./tools/scripts/update-python-sdk-version.sh 0.142.22

set -euo pipefail

ROOT_DIR="${EASYNET_VERSION_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SDK_DIR="${ROOT_DIR}/sdk/python"
MANIFEST="${SDK_DIR}/pyproject.toml"
LOCK_FILE="${SDK_DIR}/uv.lock"
CHECK_ONLY=false
REQUESTED_VERSION=""

usage() {
  echo "usage: $0 [--check] [VERSION]" >&2
}

for arg in "$@"; do
  case "${arg}" in
    --check)
      CHECK_ONLY=true
      ;;
    -*)
      usage
      echo "error: unknown option ${arg}" >&2
      exit 2
      ;;
    *)
      if [[ -n "${REQUESTED_VERSION}" ]]; then
        usage
        exit 2
      fi
      REQUESTED_VERSION="${arg}"
      ;;
  esac
done

if [[ -z "${REQUESTED_VERSION}" ]]; then
  command -v tide >/dev/null 2>&1 || {
    echo "error: tide is required when VERSION is omitted" >&2
    exit 1
  }
  echo "Fetching Python SDK release mark from tide …"
  REQUESTED_VERSION="$(tide mark --local-only)"
  if [[ -z "${REQUESTED_VERSION}" ]]; then
    echo "error: tide mark --local-only returned empty output" >&2
    exit 1
  fi
fi

if ! [[ "${REQUESTED_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z-]+)*$ ]]; then
  echo "error: invalid semver-like Python SDK version '${REQUESTED_VERSION}'" >&2
  exit 2
fi

[[ -f "${MANIFEST}" ]] || { echo "error: missing ${MANIFEST}" >&2; exit 1; }
[[ -f "${LOCK_FILE}" ]] || { echo "error: missing ${LOCK_FILE}" >&2; exit 1; }

manifest_version() {
  awk '
    /^\[project\]$/ { in_project = 1; next }
    /^\[/ { if (in_project) exit; next }
    in_project && /^version[[:space:]]*=/ {
      value = $0
      sub(/^[^\"]*\"/, "", value)
      sub(/\".*$/, "", value)
      print value
      exit
    }
  ' "${MANIFEST}"
}

lock_version() {
  awk '
    /^\[\[package\]\]$/ { package_match = 0; next }
    $0 == "name = \"easynet-sdk\"" { package_match = 1; next }
    package_match && /^version[[:space:]]*=/ {
      value = $0
      sub(/^[^\"]*\"/, "", value)
      sub(/\".*$/, "", value)
      print value
      exit
    }
  ' "${LOCK_FILE}"
}

assert_aligned() {
  local manifest_actual lock_actual
  manifest_actual="$(manifest_version)"
  lock_actual="$(lock_version)"

  if [[ "${manifest_actual}" != "${REQUESTED_VERSION}" ]]; then
    echo "error: sdk/python/pyproject.toml is ${manifest_actual:-<missing>}, expected ${REQUESTED_VERSION}" >&2
    return 1
  fi
  if [[ "${lock_actual}" != "${REQUESTED_VERSION}" ]]; then
    echo "error: sdk/python/uv.lock is ${lock_actual:-<missing>}, expected ${REQUESTED_VERSION}" >&2
    return 1
  fi
}

if "${CHECK_ONLY}"; then
  assert_aligned
  echo "Python SDK manifest and lock are aligned to ${REQUESTED_VERSION}."
  exit 0
fi

command -v uv >/dev/null 2>&1 || {
  echo "error: uv is required to regenerate sdk/python/uv.lock" >&2
  exit 1
}

BACKUP_DIR="$(mktemp -d)"
cp "${MANIFEST}" "${BACKUP_DIR}/pyproject.toml"
cp "${LOCK_FILE}" "${BACKUP_DIR}/uv.lock"
COMMITTED=false

cleanup() {
  local status=$?
  trap - EXIT
  if [[ "${COMMITTED}" != "true" ]]; then
    cp "${BACKUP_DIR}/pyproject.toml" "${MANIFEST}"
    cp "${BACKUP_DIR}/uv.lock" "${LOCK_FILE}"
  fi
  rm -rf "${BACKUP_DIR}"
  exit "${status}"
}
trap cleanup EXIT

export EASYNET_PYTHON_SDK_VERSION="${REQUESTED_VERSION}"
perl -0pi -e '
  s{
    ( ^ \[ project \] [^\[]*?
      ^ \s* version \s* = \s* " )
    [^"]*
    ( " )
  }{$1$ENV{EASYNET_PYTHON_SDK_VERSION}$2}msx
' "${MANIFEST}"

if [[ "$(manifest_version)" != "${REQUESTED_VERSION}" ]]; then
  echo "error: failed to update sdk/python/pyproject.toml" >&2
  exit 1
fi

uv lock --project "${SDK_DIR}"
assert_aligned
COMMITTED=true
echo "Python SDK version synchronized to ${REQUESTED_VERSION}."
