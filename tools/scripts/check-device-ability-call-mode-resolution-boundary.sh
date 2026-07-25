#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

REGISTRAR="src/daemon/ability/builtins/device_control/ability_management/registrar.rs"
[[ -f "$REGISTRAR" ]] || fail "missing $REGISTRAR"

if ! rg -n 'enum DeviceAbilityCallModeResolution' "$REGISTRAR" >/dev/null; then
  fail "device ability registrar must own an explicit DeviceAbilityCallModeResolution state object"
fi

for method in \
  'fn from_manifest\(manifest: &AbilityManifest\)' \
  'fn from_runtime_modes\(modes: AbilityCallModes\)' \
  'fn from_descriptor_mode\(mode: DescriptorCallMode\)' \
  'fn descriptor_mode\(self\) -> DescriptorCallMode' \
  'fn axon_mode\(self\) -> AxonCallMode'
do
  if ! rg -n "$method" "$REGISTRAR" >/dev/null; then
    fail "DeviceAbilityCallModeResolution is missing required method pattern: $method"
  fi
done

for state in 'Rpc' 'Stream' 'Bidi'; do
  if ! rg -n "^[[:space:]]*$state," "$REGISTRAR" >/dev/null; then
    fail "DeviceAbilityCallModeResolution is missing state: $state"
  fi
done

if ! rg -n 'DeviceAbilityCallModeResolution::from_manifest' "$REGISTRAR" >/dev/null; then
  fail "manifest-backed registrar paths must resolve call mode through DeviceAbilityCallModeResolution"
fi

if ! rg -n 'DeviceAbilityCallModeResolution::from_runtime_modes' "$REGISTRAR" >/dev/null; then
  fail "runtime-mode registrar paths must resolve call mode through DeviceAbilityCallModeResolution"
fi

if ! rg -n 'DeviceAbilityCallModeResolution::from_descriptor_mode' "$REGISTRAR" >/dev/null; then
  fail "descriptor-to-Axon projection must resolve through DeviceAbilityCallModeResolution"
fi

if rg -n 'descriptor_call_mode_for_manifest|descriptor_call_mode_for_modes|axon_call_mode_for_descriptor_mode' "$REGISTRAR"; then
  fail "device ability registrar still has obsolete procedural call-mode helper(s)"
fi

if rg -n 'infer control-plane mode|infer descriptor call mode|Call mode is inferred' "$REGISTRAR"; then
  fail "device ability registrar still describes call-mode lifecycle as inference"
fi
