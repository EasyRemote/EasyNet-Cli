#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

check_root() {
  local root="${1:-$ROOT}"
  local catalog="$root/src/daemon/ability/catalog/catalog_metadata.rs"
  local dispatch="$root/src/daemon/ability/dispatch.rs"
  local assembly_tests="$root/src/daemon/ability/catalog/assembly_tests.rs"
  [[ -f "$catalog" ]] || fail "missing ${catalog#$root/}"
  [[ -f "$dispatch" ]] || fail "missing ${dispatch#$root/}"
  [[ -f "$assembly_tests" ]] || fail "missing ${assembly_tests#$root/}"

  if ! rg -n 'enum CatalogSchemaProjection' "$catalog" >/dev/null; then
    fail "catalog metadata must own schema publication through CatalogSchemaProjection"
  fi

  for state in 'Declared\(serde_json::Value\)' 'UndeclaredObject'; do
    if ! rg -n "$state" "$catalog" >/dev/null; then
      fail "CatalogSchemaProjection is missing state: $state"
    fi
  done

  for method in \
    'pub fn try_input_schema_for\(name: &str\) -> anyhow::Result<serde_json::Value>' \
    'fn try_for_input_name\(name: &str\) -> anyhow::Result<Self>' \
    'fn try_declared_input_schema\(name: &str\) -> anyhow::Result<Option<serde_json::Value>>' \
    'fn into_schema\(self\) -> serde_json::Value'
  do
    if ! rg -n "$method" "$catalog" >/dev/null; then
      fail "CatalogSchemaProjection is missing required fallible method pattern: $method"
    fi
  done

  for required in \
    'Ok\(CatalogSchemaProjection::try_for_input_name\(name\)\?\.into_schema\(\)\)' \
    'Self::try_declared_input_schema\(name\)\?' \
    'crate::daemon::plugins::try_builtin_input_schema_for\(name\)\?' \
    'crate::daemon::plugins::try_input_schema_for\(name\)\?' \
    'Self::UndeclaredObject => serde_json::json!\(\{ "type": "object" \}\)'
  do
    if ! rg -n "$required" "$catalog" >/dev/null; then
      fail "catalog schema projection is missing fail-closed token: $required"
    fi
  done

  if ! rg -n 'crate::daemon::ability::catalog::try_input_schema_for\(&ability\)\?' "$dispatch" >/dev/null; then
    fail "dispatch automatic manifest construction must use fallible catalog schema projection"
  fi

  if ! rg -n 'fallible_input_schema_projection_does_not_treat_absent_plugin_as_failure' "$assembly_tests" >/dev/null; then
    fail "catalog schema projection must have a fail-closed assembly test"
  fi

  if ! rg -n 'try_input_schema_for\("observe.health"\)' "$assembly_tests" >/dev/null; then
    fail "catalog schema projection test must prove absent plugin lookup does not break system schema projection"
  fi

  if rg -n 'fn for_input_name\(name: &str\) -> Self|CatalogSchemaProjection::for_input_name|fn declared_input_schema\(name: &str\) -> Option<serde_json::Value>|crate::daemon::plugins::builtin_input_schema_for\(name\)|crate::daemon::plugins::input_schema_for\(name\)' "$catalog"; then
    fail "catalog schema projection preserves retired infallible metadata lookup"
  fi

  if rg -n 'pub fn input_schema_for\(name: &str\) -> serde_json::Value|try_input_schema_for\(name\)\.unwrap_or_else' "$catalog"; then
    fail "catalog schema projection preserves retired infallible input_schema_for fallback facade"
  fi

  if rg -n 'Unknown names fall back|empty-object default|schema fallback|default fallback|unwrap_or_else\(\|\| serde_json::json!\(\{"type": "object"\}\)' "$catalog" "$assembly_tests"; then
    fail "catalog schema publication still uses fallback/default vocabulary or local object fallback"
  fi

  if rg -n '_ => serde_json::json!\(\{ "type": "object" \}\)' "$catalog"; then
    fail "catalog metadata still has a bare match catch-all object schema"
  fi
}

self_test() {
  local tmp
  tmp="$(mktemp -d)"
  trap "rm -rf '$tmp'" RETURN
  mkdir -p "$tmp/src/daemon/ability/catalog" "$tmp/src/daemon/ability"

  cat >"$tmp/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
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
  cat >"$tmp/src/daemon/ability/dispatch.rs" <<'RS'
fn register(ability: String) -> anyhow::Result<()> {
    crate::daemon::ability::catalog::try_input_schema_for(&ability)?;
    Ok(())
}
RS
  cat >"$tmp/src/daemon/ability/catalog/assembly_tests.rs" <<'RS'
#[test]
fn fallible_input_schema_projection_does_not_treat_absent_plugin_as_failure() {
    let _ = try_input_schema_for("observe.health");
}
RS
  check_root "$tmp"

  cp "$tmp/src/daemon/ability/catalog/catalog_metadata.rs" \
    "$tmp/src/daemon/ability/catalog/catalog_metadata.retired.rs"
  perl -0pi -e 's/try_for_input_name\(name\)\?\.into_schema\(\)/for_input_name(name).into_schema()/g; s/fn try_for_input_name\(name: &str\) -> anyhow::Result<Self>/fn for_input_name(name: \&str) -> Self/g; s/Self::try_declared_input_schema\(name\)\?/Self::declared_input_schema(name)/g; s/Ok\(match Self::declared_input_schema\(name\) \{\n            Some\(schema\) => Self::Declared\(schema\),\n            None => Self::UndeclaredObject,\n        \}\)/match Self::declared_input_schema(name) {\n            Some(schema) => Self::Declared(schema),\n            None => Self::UndeclaredObject,\n        }/g; s/fn try_declared_input_schema\(name: &str\) -> anyhow::Result<Option<serde_json::Value>>/fn declared_input_schema(name: \&str) -> Option<serde_json::Value>/g; s/crate::daemon::plugins::try_builtin_input_schema_for\(name\)\?/crate::daemon::plugins::builtin_input_schema_for(name)/g; s/crate::daemon::plugins::try_input_schema_for\(name\)\?/crate::daemon::plugins::input_schema_for(name)/g; s/return Ok\(Some\(schema\)\);/return Some(schema);/g; s/Ok\(authored_static_input_schema\(name\)\)/authored_static_input_schema(name)/g' \
    "$tmp/src/daemon/ability/catalog/catalog_metadata.retired.rs"
  mv "$tmp/src/daemon/ability/catalog/catalog_metadata.retired.rs" \
    "$tmp/src/daemon/ability/catalog/catalog_metadata.rs"
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected retired infallible schema projection to fail"
  fi
}

case "${1:-}" in
  --self-test)
    self_test
    printf 'check-catalog-schema-projection-boundary self-test ok\n'
    ;;
  "")
    check_root "$ROOT"
    printf 'check-catalog-schema-projection-boundary: ok\n'
    ;;
  *)
    fail "usage: $0 [--self-test]"
    ;;
esac
