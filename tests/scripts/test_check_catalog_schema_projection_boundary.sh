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

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/catalog"
cp "$SCRIPT" "$SB/tools/scripts/check-catalog-schema-projection-boundary.sh"
touch "$SB/src/daemon/ability/catalog/assembly_tests.rs"

cat >"$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
pub fn input_schema_for(name: &str) -> serde_json::Value {
    CatalogSchemaProjection::for_input_name(name).into_schema()
}

enum CatalogSchemaProjection {
    Declared(serde_json::Value),
    UndeclaredObject,
}

impl CatalogSchemaProjection {
    fn for_input_name(name: &str) -> Self {
        match Self::declared_input_schema(name) {
            Some(schema) => Self::Declared(schema),
            None => Self::UndeclaredObject,
        }
    }

    fn declared_input_schema(name: &str) -> Option<serde_json::Value> {
        None
    }

    fn into_schema(self) -> serde_json::Value {
        match self {
            Self::Declared(schema) => schema,
            Self::UndeclaredObject => serde_json::json!({ "type": "object" }),
        }
    }
}
RS

( cd "$SB" && bash tools/scripts/check-catalog-schema-projection-boundary.sh )

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
