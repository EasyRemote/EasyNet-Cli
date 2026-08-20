#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/check-catalog-description-projection-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

[[ -x "$SCRIPT" ]] || fail "missing executable script: $SCRIPT"
bash "$SCRIPT"
bash "$SCRIPT" --self-test

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/catalog" "$SB/src/daemon/ability" "$SB/src/daemon/plugins"
cp "$SCRIPT" "$SB/tools/scripts/check-catalog-description-projection-boundary.sh"

cat >"$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
pub fn try_description_for_owned(name: &str) -> anyhow::Result<String> {
    if let Some(description) = crate::daemon::plugins::try_builtin_description_for_owned(name)? {
        return Ok(description);
    }
    if let Some(description) = crate::daemon::plugins::try_description_for_owned(name)? {
        return Ok(description);
    }
    Ok(description_for(name).to_string())
}
RS
cat >"$SB/src/daemon/plugins/mod.rs" <<'RS'
pub(crate) fn try_description_for_owned(name: &str) -> Result<Option<String>> {
    Ok(try_descriptor_for(name)?.map(|descriptor| descriptor.description().to_string()))
}

pub(crate) fn try_builtin_description_for_owned(name: &str) -> Result<Option<String>> {
    Ok(try_builtin_descriptor_for(name)?.map(|descriptor| descriptor.description().to_string()))
}

pub(crate) fn try_input_schema_for(name: &str) -> Result<Option<Value>> {
    Ok(try_descriptor_for(name)?.map(|descriptor| descriptor.input_schema().clone()))
}

pub(crate) fn try_builtin_input_schema_for(name: &str) -> Result<Option<Value>> {
    Ok(try_builtin_descriptor_for(name)?.map(|descriptor| descriptor.input_schema().clone()))
}
RS
cat >"$SB/src/daemon/ability/dispatch.rs" <<'RS'
fn register(ability: String) -> anyhow::Result<()> {
    crate::daemon::ability::catalog::try_description_for_owned(&ability)?;
    Ok(())
}
RS
cat >"$SB/src/daemon/ability/catalog/assembly_tests.rs" <<'RS'
#[test]
fn fallible_input_schema_projection_does_not_treat_absent_plugin_as_failure() {}
RS

( cd "$SB" && bash tools/scripts/check-catalog-description-projection-boundary.sh )

cat >>"$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'

pub fn description_for_owned(name: &str) -> String {
    try_description_for_owned(name).unwrap_or_else(|_| description_for(name).to_string())
}
RS

if ( cd "$SB" && bash tools/scripts/check-catalog-description-projection-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected catalog infallible description facade to fail"
fi

python3 - "$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
text = text.split("\npub fn description_for_owned", 1)[0]
path.write_text(text)
PY

cat >>"$SB/src/daemon/plugins/mod.rs" <<'RS'

pub fn input_schema_for(name: &str) -> Option<Value> {
    try_input_schema_for(name).ok().flatten()
}
RS

if ( cd "$SB" && bash tools/scripts/check-catalog-description-projection-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected plugin infallible schema facade to fail"
fi

echo "test_check_catalog_description_projection_boundary: ok"
