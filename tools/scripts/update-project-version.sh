#!/usr/bin/env bash
# Synchronize the EasyNet Runtime release coordinate without touching separately
# versioned SDK distributions.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION_FILE="${ROOT_DIR}/VERSION"
FEATURE_FILE="${ROOT_DIR}/sdk/conformance/fixtures/feature-discovery.v7.json"
AXON_LOCK="${ROOT_DIR}/compatibility/axon.lock.json"
CHECK_ONLY=0
EXPLICIT_VERSION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      CHECK_ONLY=1
      shift
      ;;
    --help|-h)
      echo "usage: $0 [--check] [VERSION]"
      exit 0
      ;;
    -*)
      echo "unknown option: $1" >&2
      exit 2
      ;;
    *)
      if [[ -n "${EXPLICIT_VERSION}" ]]; then
        echo "only one version may be supplied" >&2
        exit 2
      fi
      EXPLICIT_VERSION="$1"
      shift
      ;;
  esac
done

if [[ -n "${EXPLICIT_VERSION}" ]]; then
  NEW_VERSION="${EXPLICIT_VERSION}"
elif command -v tide >/dev/null 2>&1; then
  NEW_VERSION="$(tide mark --local-only)"
elif [[ -f "${VERSION_FILE}" ]]; then
  NEW_VERSION="$(tr -d '[:space:]' < "${VERSION_FILE}")"
else
  echo "tide is unavailable and VERSION is missing" >&2
  exit 1
fi

if ! [[ "${NEW_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z-]+)*$ ]]; then
  echo "invalid version: ${NEW_VERSION}" >&2
  exit 1
fi

CARGO_FILES=()
while IFS= read -r file; do
  CARGO_FILES+=("${file}")
done < <(
  find "${ROOT_DIR}" -type f -name Cargo.toml \
    ! -path '*/.git/*' \
    ! -path '*/target/*' \
    ! -path '*/node_modules/*' \
    | sort
)

LOCK_FILES=()
while IFS= read -r file; do
  LOCK_FILES+=("${file}")
done < <(
  find "${ROOT_DIR}" -type f -name Cargo.lock \
    ! -path '*/.git/*' \
    ! -path '*/target/*' \
    | sort
)

TARGETS=("${VERSION_FILE}" "${FEATURE_FILE}" "${AXON_LOCK}")
TARGETS+=("${CARGO_FILES[@]}")
TARGETS+=("${LOCK_FILES[@]}")

read_package_version() {
  awk -F'"' '
    /^\[package\]$/ { in_package=1; next }
    /^\[/ { in_package=0 }
    in_package && /^version[[:space:]]*=/ { print $2; exit }
  ' "$1"
}

json_value() {
  python3 - "$1" "$2" <<'PY'
import json
from pathlib import Path
import sys

value = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
for component in sys.argv[2].split("."):
    value = value[component]
print(value)
PY
}

check_alignment() {
  local failed=0
  local actual
  actual="$(tr -d '[:space:]' < "${VERSION_FILE}")"
  if [[ "${actual}" != "${NEW_VERSION}" ]]; then
    echo "VERSION mismatch: ${actual} != ${NEW_VERSION}" >&2
    failed=1
  fi
  for file in "${CARGO_FILES[@]}"; do
    actual="$(read_package_version "${file}")"
    if [[ "${actual}" != "${NEW_VERSION}" ]]; then
      echo "Runtime Cargo version mismatch: ${file#"${ROOT_DIR}/"}: ${actual} != ${NEW_VERSION}" >&2
      failed=1
    fi
  done
  actual="$(json_value "${FEATURE_FILE}" sdk_version)"
  if [[ "${actual}" != "${NEW_VERSION}" ]]; then
    echo "feature discovery version mismatch: ${actual} != ${NEW_VERSION}" >&2
    failed=1
  fi
  actual="$(json_value "${AXON_LOCK}" cli.runtime_version)"
  if [[ "${actual}" != "${NEW_VERSION}" ]]; then
    echo "CLI lock Runtime version mismatch: ${actual} != ${NEW_VERSION}" >&2
    failed=1
  fi
  if [[ ${failed} -ne 0 ]]; then
    return 1
  fi
  (cd "${ROOT_DIR}" && cargo metadata --locked --no-deps --format-version 1 >/dev/null)
  echo "Runtime manifests and locks are aligned to ${NEW_VERSION}."
}

if [[ ${CHECK_ONLY} -eq 1 ]]; then
  check_alignment
  exit 0
fi

if git -C "${ROOT_DIR}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  dirty="$(git -C "${ROOT_DIR}" status --porcelain=v1 -- "${TARGETS[@]}")"
  if [[ -n "${dirty}" ]]; then
    echo "Runtime version targets must be clean before synchronization" >&2
    exit 1
  fi
fi

BACKUP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/easynet-cli-version.XXXXXX")"
COMPLETED=0

restore_targets() {
  local file relative backup
  for file in "${TARGETS[@]}"; do
    relative="${file#"${ROOT_DIR}/"}"
    backup="${BACKUP_DIR}/${relative}"
    if [[ -f "${backup}" ]]; then
      cp "${backup}" "${file}"
    fi
  done
}

finish() {
  local status=$?
  if [[ ${COMPLETED} -eq 0 ]]; then
    restore_targets
  fi
  rm -rf "${BACKUP_DIR}"
  exit "${status}"
}
trap finish EXIT INT TERM

for file in "${TARGETS[@]}"; do
  relative="${file#"${ROOT_DIR}/"}"
  mkdir -p "${BACKUP_DIR}/$(dirname "${relative}")"
  cp "${file}" "${BACKUP_DIR}/${relative}"
done

export NEW_VERSION
for file in "${CARGO_FILES[@]}"; do
  perl -0pi -e '
    s{(^\[package\][^\[]*?^\s*version\s*=\s*")[^"]*(")}
     {$1$ENV{NEW_VERSION}$2}msx
  ' "${file}"
done

python3 - "${FEATURE_FILE}" "${AXON_LOCK}" "${NEW_VERSION}" <<'PY'
import json
import os
from pathlib import Path
import sys
import tempfile

def replace(path: Path, value: object) -> None:
    descriptor, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent, text=True)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)

feature_path = Path(sys.argv[1])
lock_path = Path(sys.argv[2])
version = sys.argv[3]
feature = json.loads(feature_path.read_text(encoding="utf-8"))
feature["sdk_version"] = version
replace(feature_path, feature)
lock = json.loads(lock_path.read_text(encoding="utf-8"))
lock["cli"]["runtime_version"] = version
replace(lock_path, lock)
PY

command -v cargo >/dev/null 2>&1 || {
  echo "cargo is required to regenerate Runtime locks" >&2
  exit 1
}
for lock in "${LOCK_FILES[@]}"; do
  echo "  regenerate ${lock#"${ROOT_DIR}/"}"
  (cd "$(dirname "${lock}")" && cargo generate-lockfile --quiet)
done

printf '%s\n' "${NEW_VERSION}" > "${VERSION_FILE}"
check_alignment
COMPLETED=1
trap - EXIT INT TERM
rm -rf "${BACKUP_DIR}"
echo "Updated Runtime version to ${NEW_VERSION}."
