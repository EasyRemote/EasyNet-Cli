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
  local plugins="$root/src/daemon/plugins/mod.rs"
  local dispatch="$root/src/daemon/ability/dispatch.rs"
  local assembly_tests="$root/src/daemon/ability/catalog/assembly_tests.rs"

  [[ -f "$catalog" ]] || fail "missing catalog metadata source: $catalog"
  [[ -f "$plugins" ]] || fail "missing plugin metadata source: $plugins"
  [[ -f "$dispatch" ]] || fail "missing dispatch source: $dispatch"
  [[ -f "$assembly_tests" ]] || fail "missing assembly tests source: $assembly_tests"

  if rg -n 'pub fn description_for_owned|fn description_for_owned|pub fn builtin_description_for_owned|pub fn input_schema_for\(name: &str\) -> Option<Value>|pub fn builtin_input_schema_for|try_description_for_owned\(name\)\.unwrap_or_else|try_description_for_owned\(name\)\.ok\(\)\.flatten\(\)|try_input_schema_for\(name\)\.ok\(\)\.flatten\(\)|try_builtin_input_schema_for\(name\)\.ok\(\)\.flatten\(\)' "$catalog" "$plugins"; then
    fail "catalog/plugin metadata projection still exposes an infallible plugin metadata fallback facade"
  fi

  for required in \
    'pub fn try_description_for_owned\(name: &str\) -> anyhow::Result<String>' \
    'crate::daemon::plugins::try_builtin_description_for_owned\(name\)\?' \
    'crate::daemon::plugins::try_description_for_owned\(name\)\?'
  do
    if ! rg -n "$required" "$catalog" >/dev/null; then
      fail "catalog description projection missing fallible token: $required"
    fi
  done

  for required in \
    'pub\(crate\) fn try_description_for_owned\(name: &str\) -> Result<Option<String>>' \
    'pub\(crate\) fn try_builtin_description_for_owned\(name: &str\) -> Result<Option<String>>' \
    'pub\(crate\) fn try_input_schema_for\(name: &str\) -> Result<Option<Value>>' \
    'pub\(crate\) fn try_builtin_input_schema_for\(name: &str\) -> Result<Option<Value>>' \
    'try_descriptor_for\(name\)\?' \
    'try_builtin_descriptor_for\(name\)\?'
  do
    if ! rg -n "$required" "$plugins" >/dev/null; then
      fail "plugin description projection missing fallible token: $required"
    fi
  done

  if ! rg -n 'crate::daemon::ability::catalog::try_description_for_owned\(&ability\)\?' "$dispatch" >/dev/null; then
    fail "dispatch automatic manifest construction must use fallible catalog description projection"
  fi

  if ! rg -n 'fallible_input_schema_projection_does_not_treat_absent_plugin_as_failure' "$assembly_tests" >/dev/null; then
    fail "catalog metadata projection must retain absent-plugin fallible-source coverage"
  fi
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/src/daemon/ability/catalog" "$tmp/src/daemon/ability" "$tmp/src/daemon/plugins"

  cat >"$tmp/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
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
  cat >"$tmp/src/daemon/plugins/mod.rs" <<'RS'
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
  cat >"$tmp/src/daemon/ability/dispatch.rs" <<'RS'
fn register(ability: String) -> anyhow::Result<()> {
    crate::daemon::ability::catalog::try_description_for_owned(&ability)?;
    Ok(())
}
RS
  cat >"$tmp/src/daemon/ability/catalog/assembly_tests.rs" <<'RS'
#[test]
fn fallible_input_schema_projection_does_not_treat_absent_plugin_as_failure() {}
RS
  check_root "$tmp"

  cat >>"$tmp/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'

pub fn description_for_owned(name: &str) -> String {
    try_description_for_owned(name).unwrap_or_else(|_| description_for(name).to_string())
}
RS
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected catalog infallible description facade to fail"
  fi

  python3 - "$tmp/src/daemon/ability/catalog/catalog_metadata.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
text = text.split("\npub fn description_for_owned", 1)[0]
path.write_text(text)
PY
  cat >>"$tmp/src/daemon/plugins/mod.rs" <<'RS'

pub fn description_for_owned(name: &str) -> Option<String> {
    try_description_for_owned(name).ok().flatten()
}
RS
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected plugin infallible description facade to fail"
  fi

  python3 - "$tmp/src/daemon/plugins/mod.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
text = text.split("\npub fn description_for_owned", 1)[0]
path.write_text(text)
PY
  cat >>"$tmp/src/daemon/plugins/mod.rs" <<'RS'

pub fn input_schema_for(name: &str) -> Option<Value> {
    try_input_schema_for(name).ok().flatten()
}
RS
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected plugin infallible schema facade to fail"
  fi

  echo "check-catalog-description-projection-boundary self-test: ok"
  exit 0
fi

check_root "$ROOT"
echo "check-catalog-description-projection-boundary: ok"
