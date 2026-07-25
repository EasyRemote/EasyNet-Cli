#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/check-daemon-invocation-contract-metadata-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

[[ -x "$SCRIPT" ]] || fail "missing executable script: $SCRIPT"
bash "$SCRIPT"
bash "$SCRIPT" --self-test

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/catalog"
cp "$SCRIPT" "$SB/tools/scripts/check-daemon-invocation-contract-metadata-boundary.sh"

cat >"$SB/src/daemon/ability/catalog/daemon_invocation_contracts.rs" <<'RS'
fn manifest_for(ability: &str) -> anyhow::Result<()> {
    let _description = description_for(ability).ok_or_else(|| anyhow::anyhow!("missing descriptor description"))?;
    let _schema = input_schema_for(ability).ok_or_else(|| anyhow::anyhow!("missing input schema"))?;
    Ok(())
}

#[test]
fn manifest_for_rejects_missing_contract_metadata() {}
RS

( cd "$SB" && bash tools/scripts/check-daemon-invocation-contract-metadata-boundary.sh )

python3 - "$SB/src/daemon/ability/catalog/daemon_invocation_contracts.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
text = text.replace(
    'let _schema = input_schema_for(ability).ok_or_else(|| anyhow::anyhow!("missing input schema"))?;',
    'let _schema = input_schema_for(ability).unwrap_or_else(closed_empty_schema);',
)
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-daemon-invocation-contract-metadata-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected schema fallback to fail"
fi

echo "test_check_daemon_invocation_contract_metadata_boundary: ok"
