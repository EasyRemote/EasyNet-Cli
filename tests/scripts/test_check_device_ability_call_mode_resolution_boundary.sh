#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-device-ability-call-mode-resolution-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

bash "$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/builtins/device_control/ability_management"
cp "$SCRIPT" "$SB/tools/scripts/check-device-ability-call-mode-resolution-boundary.sh"

cat >"$SB/src/daemon/ability/builtins/device_control/ability_management/registrar.rs" <<'RS'
enum AbilityDeploymentCallModeResolution {
    Rpc,
    Stream,
    Bidi,
}

impl AbilityDeploymentCallModeResolution {
    fn from_manifest(manifest: &AbilityManifest) -> anyhow::Result<Self> {
        Ok(Self::Stream)
    }

    fn from_runtime_modes(modes: AbilityCallModes) -> Self {
        Self::Rpc
    }

    fn from_descriptor_mode(mode: DescriptorCallMode) -> Self {
        Self::Bidi
    }

    fn descriptor_mode(self) -> DescriptorCallMode {
        DescriptorCallMode::Rpc
    }

    fn axon_mode(self) -> AxonCallMode {
        AxonCallMode::Rpc
    }
}

fn install(manifest: &AbilityManifest, modes: AbilityCallModes, mode: DescriptorCallMode) {
    let _ = AbilityDeploymentCallModeResolution::from_manifest(manifest);
    let _ = AbilityDeploymentCallModeResolution::from_runtime_modes(modes);
    let _ = AbilityDeploymentCallModeResolution::from_descriptor_mode(mode);
}
RS

( cd "$SB" && bash tools/scripts/check-device-ability-call-mode-resolution-boundary.sh )

cat >>"$SB/src/daemon/ability/builtins/device_control/ability_management/registrar.rs" <<'RS'
fn descriptor_call_mode_for_manifest() {}
RS

if ( cd "$SB" && bash tools/scripts/check-device-ability-call-mode-resolution-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected obsolete descriptor_call_mode_for_manifest helper to fail"
fi

python3 - "$SB/src/daemon/ability/builtins/device_control/ability_management/registrar.rs" <<'PY'
from pathlib import Path
path = Path(__import__("sys").argv[1])
text = path.read_text()
text = text.replace("fn descriptor_call_mode_for_manifest() {}\n", "")
text += "// Call mode is inferred from exec.kind\n"
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-device-ability-call-mode-resolution-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected inferred call-mode vocabulary to fail"
fi
