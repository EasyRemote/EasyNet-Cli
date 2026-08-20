#!/usr/bin/env bash

set -euo pipefail

ROOT="${CHECK_FRONTEND_ABILITY_CONTRACT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
DESCRIPTOR_DIRS=(
  "$ROOT/ability-descriptors/system"
  "$ROOT/plugins"
)

fail() {
  printf 'check-frontend-ability-contract-boundary: %s\n' "$*" >&2
  exit 1
}

found_descriptor_dir=0
for descriptor_dir in "${DESCRIPTOR_DIRS[@]}"; do
  if [[ -d "$descriptor_dir" ]]; then
    found_descriptor_dir=1
  fi
done
[[ "$found_descriptor_dir" == "1" ]] || fail "missing descriptor directories"

count=0
while IFS= read -r -d '' descriptor; do
  count=$((count + 1))
  relative="${descriptor#"$ROOT/"}"
  field() {
    awk -F' = ' -v key="$1" '$1 == key { value=$2; gsub(/"/, "", value); print value }' "$descriptor"
  }

  [[ "$(field schema_version)" == "3" ]] || fail "$relative must use schema_version 3"
  case "$(field exposure)" in
    internal|operator|task) ;;
    *) fail "$relative must declare one canonical exposure" ;;
  esac
  surface="$(field dedicated_surface)"
  case "$surface" in
    none|terminal|file_transfer|media|voice|browser|pages|remote_desktop) ;;
    *) fail "$relative must declare one canonical dedicated_surface" ;;
  esac
  subject_kind="$(field subject_contract_kind)"
  case "$subject_kind" in
    authenticated-user|route-target|explicit-ura|dedicated-surface) ;;
    *) fail "$relative must declare one canonical subject_contract_kind" ;;
  esac
  if [[ "$surface" == "none" && "$subject_kind" == "dedicated-surface" ]]; then
    fail "$relative declares dedicated-surface subject without a dedicated surface"
  fi
  if [[ "$surface" != "none" && "$subject_kind" != "dedicated-surface" ]]; then
    fail "$relative dedicated surface must own subject construction"
  fi
  name="$(field name)"
  if [[ "$name" == remote_desktop.* && "$surface" != "remote_desktop" ]]; then
    fail "$relative remote_desktop abilities must use dedicated_surface=remote_desktop"
  fi
  if [[ "$surface" == "remote_desktop" && "$name" != remote_desktop.* ]]; then
    fail "$relative only remote_desktop abilities may use dedicated_surface=remote_desktop"
  fi
  if [[ ( "$name" == pages.* || "$name" == "project_list" ) && "$surface" != "pages" ]]; then
    fail "$relative pages abilities must use dedicated_surface=pages"
  fi
  if [[ "$surface" == "pages" && "$name" != pages.* && "$name" != "project_list" ]]; then
    fail "$relative only pages abilities may use dedicated_surface=pages"
  fi
done < <(find "${DESCRIPTOR_DIRS[@]}" -name '*.ability.toml' -print0 2>/dev/null)

[[ "$count" -gt 0 ]] || fail "no system descriptors found"
printf 'check-frontend-ability-contract-boundary: ok (%s descriptors)\n' "$count"
