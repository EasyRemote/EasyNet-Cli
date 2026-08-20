#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/check-control-plane-manifest-materialization-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

[[ -x "$SCRIPT" ]] || fail "missing executable script: $SCRIPT"
bash "$SCRIPT"
bash "$SCRIPT" --self-test

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/descriptors" "$SB/src/daemon/ability"
cp "$SCRIPT" "$SB/tools/scripts/check-control-plane-manifest-materialization-boundary.sh"

cat >"$SB/src/daemon/ability/descriptors/surface.rs" <<'RS'
pub fn from_registry_manifest(
    registry_ability: impl Into<String>,
    owner_ura: impl Into<String>,
    manifest: &crate::daemon::ability::manifest::AbilityManifest,
) -> Result<(), ()> {
    let _ = (registry_ability, owner_ura, manifest);
    Ok(())
}
RS

cat >"$SB/src/daemon/ability/control_plane.rs" <<'RS'
fn materialize(manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>) -> Result<(), AbilityControlPlaneError> {
    let _manifest = manifest.ok_or_else(|| AbilityControlPlaneError::MissingManifest {
        ability: "demo.echo".to_string(),
    })?;
    Ok(())
}

#[test]
fn register_rejects_missing_manifest_before_descriptor_materialization() {}
RS

cat >"$SB/src/daemon/ability/control_plane_error.rs" <<'RS'
enum AbilityControlPlaneError {
    MissingManifest { ability: String },
}
RS

( cd "$SB" && bash tools/scripts/check-control-plane-manifest-materialization-boundary.sh )

python3 - "$SB/src/daemon/ability/control_plane.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
text = text.replace(
    'let _manifest = manifest.ok_or_else(|| AbilityControlPlaneError::MissingManifest {\n        ability: "demo.echo".to_string(),\n    })?;\n',
    'let _manifest = manifest.unwrap();\n',
)
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-control-plane-manifest-materialization-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected missing fail-closed materialization to fail"
fi

echo "test_check_control_plane_manifest_materialization_boundary: ok"
