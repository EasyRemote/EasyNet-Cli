#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

check_root() {
  local root="${1:-$ROOT}"
  local contracts="$root/src/daemon/ability/catalog/daemon_invocation_contracts.rs"
  [[ -f "$contracts" ]] || fail "missing daemon Invocation contracts source: $contracts"

  if rg -n 'description_for\(ability\)\.unwrap_or|input_schema_for\(ability\)\.unwrap_or|unwrap_or_else\(closed_empty_schema\)|"\(daemon Invocation ability\)"' "$contracts"; then
    fail "daemon Invocation descriptor manifest still default-fills missing description or schema"
  fi

  if ! rg -n 'description_for\(ability\)\.ok_or_else' "$contracts" >/dev/null; then
    fail "daemon Invocation descriptor manifest must fail closed on missing description"
  fi

  if ! rg -n 'input_schema_for\(ability\)\.ok_or_else' "$contracts" >/dev/null; then
    fail "daemon Invocation descriptor manifest must fail closed on missing input schema"
  fi

  if ! rg -n 'manifest_for_rejects_missing_contract_metadata' "$contracts" >/dev/null; then
    fail "daemon Invocation contract metadata boundary lacks a missing-metadata negative test"
  fi
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/src/daemon/ability/catalog"
  cat >"$tmp/src/daemon/ability/catalog/daemon_invocation_contracts.rs" <<'RS'
fn manifest_for(ability: &str) -> anyhow::Result<()> {
    let _description = description_for(ability).ok_or_else(|| anyhow::anyhow!("missing descriptor description"))?;
    let _schema = input_schema_for(ability).ok_or_else(|| anyhow::anyhow!("missing input schema"))?;
    Ok(())
}

#[test]
fn manifest_for_rejects_missing_contract_metadata() {}
RS
  check_root "$tmp"

  perl -0pi -e 's/description_for\(ability\)\.ok_or_else\(\|\| anyhow::anyhow!\("missing descriptor description"\)\)\?/description_for(ability).unwrap_or("(daemon Invocation ability)")/' \
    "$tmp/src/daemon/ability/catalog/daemon_invocation_contracts.rs"
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected description fallback to fail"
  fi

  cat >"$tmp/src/daemon/ability/catalog/daemon_invocation_contracts.rs" <<'RS'
fn manifest_for(ability: &str) -> anyhow::Result<()> {
    let _description = description_for(ability).ok_or_else(|| anyhow::anyhow!("missing descriptor description"))?;
    let _schema = input_schema_for(ability).unwrap_or_else(closed_empty_schema);
    Ok(())
}

#[test]
fn manifest_for_rejects_missing_contract_metadata() {}
RS
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected schema fallback to fail"
  fi

  echo "check-daemon-invocation-contract-metadata-boundary self-test: ok"
  exit 0
fi

check_root "$ROOT"
echo "check-daemon-invocation-contract-metadata-boundary: ok"
