#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-catalog-schema-projection-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

bash "$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/catalog" "$SB/src/daemon/ability"
cp "$SCRIPT" "$SB/tools/scripts/check-catalog-schema-projection-boundary.sh"

cat >"$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
pub fn try_input_schema_for(name: &str) -> anyhow::Result<serde_json::Value> {
    Ok(CatalogSchemaProjection::try_for_input_name(name)?.into_schema())
}

enum CatalogSchemaProjection {
    Declared(serde_json::Value),
    UndeclaredObject,
}

impl CatalogSchemaProjection {
    fn try_for_input_name(name: &str) -> anyhow::Result<Self> {
        Ok(match Self::try_declared_input_schema(name)? {
            Some(schema) => Self::Declared(schema),
            None => Self::UndeclaredObject,
        })
    }

    fn try_declared_input_schema(name: &str) -> anyhow::Result<Option<serde_json::Value>> {
        if let Some(schema) = crate::daemon::plugins::try_builtin_input_schema_for(name)? {
            return Ok(Some(schema));
        }
        if let Some(schema) = crate::daemon::plugins::try_input_schema_for(name)? {
            return Ok(Some(schema));
        }
        Ok(authored_static_input_schema(name))
    }

    fn into_schema(self) -> serde_json::Value {
        match self {
            Self::Declared(schema) => schema,
            Self::UndeclaredObject => serde_json::json!({ "type": "object" }),
        }
    }
}
RS

cat >"$SB/src/daemon/ability/dispatch.rs" <<'RS'
fn register(ability: String) -> anyhow::Result<()> {
    crate::daemon::ability::catalog::try_input_schema_for(&ability)?;
    Ok(())
}
RS

cat >"$SB/src/daemon/ability/catalog/assembly_tests.rs" <<'RS'
#[test]
fn fallible_input_schema_projection_does_not_treat_absent_plugin_as_failure() {
    let _ = try_input_schema_for("observe.health");
}
RS

( cd "$SB" && bash tools/scripts/check-catalog-schema-projection-boundary.sh )

cat >>"$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
pub fn input_schema_for(name: &str) -> serde_json::Value {
    try_input_schema_for(name).unwrap_or_else(|_| serde_json::json!({ "type": "object" }))
}
RS

if ( cd "$SB" && bash tools/scripts/check-catalog-schema-projection-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected retired input_schema_for fallback facade to fail"
fi

python3 - "$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'PY'
from pathlib import Path
path = Path(__import__("sys").argv[1])
text = path.read_text()
text = text.replace('pub fn input_schema_for(name: &str) -> serde_json::Value {\n    try_input_schema_for(name).unwrap_or_else(|_| serde_json::json!({ "type": "object" }))\n}\n', '')
path.write_text(text)
PY

cat >>"$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
fn legacy_schema(name: &str) -> serde_json::Value {
    match name {
        _ => serde_json::json!({ "type": "object" }),
    }
}
RS

if ( cd "$SB" && bash tools/scripts/check-catalog-schema-projection-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected bare object catch-all schema to fail"
fi

python3 - "$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'PY'
from pathlib import Path
path = Path(__import__("sys").argv[1])
text = path.read_text()
text = text.replace('fn legacy_schema(name: &str) -> serde_json::Value {\n    match name {\n        _ => serde_json::json!({ "type": "object" }),\n    }\n}\n', '')
text += "// Unknown names fall back to object schema\n"
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-catalog-schema-projection-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected fallback vocabulary to fail"
fi
