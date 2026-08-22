#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="${REPO_ROOT}/tools/scripts/update-python-sdk-version.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "${SANDBOX}"' EXIT

mkdir -p "${SANDBOX}/sdk/python" "${SANDBOX}/bin"

write_fixture() {
  local version="$1"
  printf '[project]\nname = "easynet-sdk"\nversion = "%s"\n\n[tool.uv]\n' "${version}" \
    > "${SANDBOX}/sdk/python/pyproject.toml"
  printf 'version = 1\n\n[[package]]\nname = "easynet-sdk"\nversion = "%s"\nsource = { editable = "." }\n' "${version}" \
    > "${SANDBOX}/sdk/python/uv.lock"
}

cat > "${SANDBOX}/bin/uv" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
sdk_dir=""
while [[ $# -gt 0 ]]; do
  if [[ "$1" == "--project" ]]; then
    sdk_dir="$2"
    break
  fi
  shift
done
version="$(awk -F'"' '/^version = "/ { print $2; exit }' "${sdk_dir}/pyproject.toml")"
perl -0pi -e 's/(name = "easynet-sdk"\nversion = ")[^"]*/$1$ENV{LOCK_VERSION}/' \
  "${sdk_dir}/uv.lock"
SH
chmod +x "${SANDBOX}/bin/uv"

cat > "${SANDBOX}/bin/tide" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$#" -eq 2 && "$1" == "mark" && "$2" == "--local-only" ]]
printf '%s\n' "${TIDE_VERSION:?}"
SH
chmod +x "${SANDBOX}/bin/tide"

write_fixture "1.2.3"
EASYNET_VERSION_ROOT="${SANDBOX}" bash "${SCRIPT}" --check 1.2.3 >/dev/null

manifest_before="$(shasum -a 256 "${SANDBOX}/sdk/python/pyproject.toml")"
lock_before="$(shasum -a 256 "${SANDBOX}/sdk/python/uv.lock")"
if EASYNET_VERSION_ROOT="${SANDBOX}" bash "${SCRIPT}" --check 1.2.4 >/dev/null 2>&1; then
  echo "expected drift check to fail" >&2
  exit 1
fi
[[ "${manifest_before}" == "$(shasum -a 256 "${SANDBOX}/sdk/python/pyproject.toml")" ]]
[[ "${lock_before}" == "$(shasum -a 256 "${SANDBOX}/sdk/python/uv.lock")" ]]

export LOCK_VERSION="1.2.4"
export TIDE_VERSION="1.2.4"
PATH="${SANDBOX}/bin:${PATH}" EASYNET_VERSION_ROOT="${SANDBOX}" \
  bash "${SCRIPT}" >/dev/null
EASYNET_VERSION_ROOT="${SANDBOX}" bash "${SCRIPT}" --check 1.2.4 >/dev/null

printf 'runtime-version\n' > "${SANDBOX}/VERSION"
printf '[package]\nversion = "9.9.9"\n' > "${SANDBOX}/Cargo.toml"
runtime_before="$(shasum -a 256 "${SANDBOX}/VERSION" "${SANDBOX}/Cargo.toml")"
export LOCK_VERSION="1.2.4"
PATH="${SANDBOX}/bin:${PATH}" EASYNET_VERSION_ROOT="${SANDBOX}" \
  bash "${SCRIPT}" 1.2.4 >/dev/null
[[ "${runtime_before}" == "$(shasum -a 256 "${SANDBOX}/VERSION" "${SANDBOX}/Cargo.toml")" ]]

cat > "${SANDBOX}/bin/uv" <<'SH'
#!/usr/bin/env bash
exit 1
SH
chmod +x "${SANDBOX}/bin/uv"
manifest_before="$(shasum -a 256 "${SANDBOX}/sdk/python/pyproject.toml")"
lock_before="$(shasum -a 256 "${SANDBOX}/sdk/python/uv.lock")"
if PATH="${SANDBOX}/bin:${PATH}" EASYNET_VERSION_ROOT="${SANDBOX}" \
  bash "${SCRIPT}" 1.2.5 >/dev/null 2>&1; then
  echo "expected failed lock regeneration" >&2
  exit 1
fi
[[ "${manifest_before}" == "$(shasum -a 256 "${SANDBOX}/sdk/python/pyproject.toml")" ]]
[[ "${lock_before}" == "$(shasum -a 256 "${SANDBOX}/sdk/python/uv.lock")" ]]

echo "test_update_python_sdk_version.sh: all cases passed"
