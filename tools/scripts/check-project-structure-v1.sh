#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

failures=()

fail() {
  failures+=("$1")
}

require_file() {
  local path="$1"
  [[ -f "$ROOT/$path" ]] || fail "missing file: $path"
}

require_dir() {
  local path="$1"
  [[ -d "$ROOT/$path" ]] || fail "missing directory: $path"
}

forbid_path() {
  local path="$1"
  [[ ! -e "$ROOT/$path" ]] || fail "forbidden path exists: $path"
}

contains_name() {
  local name="$1"
  shift

  local expected_name
  for expected_name in "$@"; do
    [[ "$name" == "$expected_name" ]] && return 0
  done

  return 1
}

check_root_contract() {
  local allowed_files=(
    .dockerignore
    .gitignore
    Cargo.toml
    Cargo.lock
    README.md
    README.pdf
    PROJECT_STRUCTURE.md
    VERSION
    build.rs
  )
  local allowed_dirs=(
    .github
    ability-descriptors
    benches
    docs
    examples
    gallery
    include
    packaging
    platforms
    plugins
    schemas
    sdk
    skills
    src
    tests
    tools
  )

  local entry top actual name
  if git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    while IFS= read -r entry; do
      [[ -n "$entry" ]] || continue
      [[ -e "$ROOT/$entry" ]] || continue
      top="${entry%%/*}"
      if [[ "$entry" == "$top" ]]; then
        contains_name "$top" "${allowed_files[@]}" \
          || fail "unexpected tracked root file: $top"
      else
        contains_name "$top" "${allowed_dirs[@]}" \
          || fail "unexpected tracked root directory: $top"
      fi
    done < <(git -C "$ROOT" ls-files)
    return 0
  fi

  while IFS= read -r actual; do
    name="$(basename "$actual")"
    [[ "$name" == ".git" ]] && continue
    if [[ -f "$actual" ]]; then
      contains_name "$name" "${allowed_files[@]}" \
        || fail "unexpected root file: $name"
    elif [[ -d "$actual" ]]; then
      contains_name "$name" "${allowed_dirs[@]}" \
        || fail "unexpected root directory: $name"
    fi
  done < <(find "$ROOT" -mindepth 1 -maxdepth 1 | sort)
}

require_only_dirs() {
  local parent="$1"
  shift
  local expected=("$@")
  require_dir "$parent"
  [[ -d "$ROOT/$parent" ]] || return 0

  local actual name allowed
  while IFS= read -r actual; do
    name="$(basename "$actual")"
    allowed=false
    for expected_name in "${expected[@]}"; do
      if [[ "$name" == "$expected_name" ]]; then
        allowed=true
        break
      fi
    done
    [[ "$allowed" == true ]] || fail "unexpected directory under $parent: $name"
  done < <(find "$ROOT/$parent" -mindepth 1 -maxdepth 1 -type d | sort)

  for expected_name in "${expected[@]}"; do
    require_dir "$parent/$expected_name"
  done
}

require_only_files() {
  local parent="$1"
  shift
  local expected=("$@")
  require_dir "$parent"
  [[ -d "$ROOT/$parent" ]] || return 0

  local actual name allowed
  while IFS= read -r actual; do
    name="$(basename "$actual")"
    allowed=false
    for expected_name in "${expected[@]}"; do
      if [[ "$name" == "$expected_name" ]]; then
        allowed=true
        break
      fi
    done
    [[ "$allowed" == true ]] || fail "unexpected file under $parent: $name"
  done < <(find "$ROOT/$parent" -mindepth 1 -maxdepth 1 -type f | sort)

  for expected_name in "${expected[@]}"; do
    require_file "$parent/$expected_name"
  done
}

require_file Cargo.toml
require_file Cargo.lock
require_file README.md
require_file README.pdf
require_file PROJECT_STRUCTURE.md
require_file VERSION
require_file build.rs
require_file include/easynet_cli.h
check_root_contract

require_only_files src/bin \
  easynet.rs \
  easynet-daemon.rs \
  easynet-keyring.rs \
  gen-ability-tomls.rs \
  real-user-smoke.rs \
  real-publish-smoke.rs

require_only_dirs src \
  bin core daemon cli ffi eal support

require_only_dirs src/core \
  ability agent identity ura domain

require_only_dirs src/daemon \
  boot control invocation ability execution resources identity trust keyring federation plugins persistence axon_bridge telemetry

require_only_dirs src/daemon/invocation \
  admission routing dispatch receipts streams bidi

require_only_dirs src/daemon/ability \
  names descriptors authority impl_bindings catalog wire builtins

require_only_dirs src/daemon/ability/builtins \
  agents device_control resources automation integrations governance

require_only_dirs src/daemon/execution \
  pty mcp mission schedule loop_instance permission session

require_only_dirs src/daemon/resources \
  skills pages context files media remote_desktop

require_only_dirs src/cli \
  commands presentation daemon_client mcp

require_only_dirs src/ffi \
  daemon client invocation errors strings

require_only_dirs src/eal \
  parser interpreter runtime diagnostics

require_only_dirs src/support \
  async_bridge shellguard platform

require_only_dirs sdk \
  go python node java swift schemas conformance

require_only_dirs sdk/conformance \
  cases fixtures runner

require_only_dirs ability-descriptors/system \
  agents device_control resources automation integrations governance

require_dir schemas/descriptor
require_dir schemas/receipt
require_file schemas/control_plane.proto
require_file schemas/common.proto

require_dir plugins
require_dir skills
require_dir examples
require_dir gallery
require_dir docs
require_dir benches
require_dir tools
require_dir packaging/docker
require_dir packaging/release
require_dir platforms/macos
require_dir platforms/windows
require_dir .github/workflows

require_dir tests/e2e
require_dir tests/conformance
require_dir tests/fixtures
require_dir tests/scripts
require_dir tests/support

for forbidden in \
  engineering \
  scripts \
  demos \
  crates \
  runtime \
  services \
  sdk/rust \
  src/runtime \
  src/services \
  src/facade \
  src/persistence \
  src/plugins \
  src/registry
do
  forbid_path "$forbidden"
done

while IFS= read -r descriptor; do
  fail "flat system ability descriptor must be grouped: ${descriptor#$ROOT/}"
done < <(find "$ROOT/ability-descriptors/system" -mindepth 1 -maxdepth 1 -type f -name '*.ability.toml' 2>/dev/null | sort)

while IFS= read -r file; do
  fail "flat ffi source file is not final structure: ${file#$ROOT/}"
done < <(find "$ROOT/src/ffi" -mindepth 1 -maxdepth 1 -type f ! -name 'mod.rs' 2>/dev/null | sort)

while IFS= read -r file; do
  fail "flat eal source file is not final structure: ${file#$ROOT/}"
done < <(find "$ROOT/src/eal" -mindepth 1 -maxdepth 1 -type f ! -name 'mod.rs' 2>/dev/null | sort)

while IFS= read -r file; do
  fail "flat support source file is not final structure: ${file#$ROOT/}"
done < <(find "$ROOT/src/support" -mindepth 1 -maxdepth 1 -type f ! -name 'mod.rs' 2>/dev/null | sort)

if ((${#failures[@]} > 0)); then
  printf 'project-structure-v1 failed:\n' >&2
  printf '  - %s\n' "${failures[@]}" >&2
  exit 1
fi

printf 'project-structure-v1 ok\n'
