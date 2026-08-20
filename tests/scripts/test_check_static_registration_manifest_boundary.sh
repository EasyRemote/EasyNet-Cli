#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/check-static-registration-manifest-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

[[ -x "$SCRIPT" ]] || fail "missing executable script: $SCRIPT"
bash "$SCRIPT"
bash "$SCRIPT" --self-test

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/builtins/governance" "$SB/src/daemon/ability"
cp "$SCRIPT" "$SB/tools/scripts/check-static-registration-manifest-boundary.sh"

cat >"$SB/src/daemon/ability/dispatch.rs" <<'RS'
fn commit(owner: OwnerKind, ability: &str) -> anyhow::Result<()> {
    if matches!(owner, OwnerKind::Agent(_)) {
        anyhow::bail!("agent-owned ability {ability:?} requires an explicit manifest; descriptor publication must not synthesize fallback metadata");
    }
    Ok(())
}
RS
cat >"$SB/src/daemon/ability/builtins/governance/meta.rs" <<'RS'
fn agent_owned_static_registration_rejects_fallback_manifest_publication() {
    let _ = "fallback metadata";
}
RS

( cd "$SB" && bash tools/scripts/check-static-registration-manifest-boundary.sh )

python3 - "$SB/src/daemon/ability/dispatch.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
text = text.replace(
    '    if matches!(owner, OwnerKind::Agent(_)) {\n        anyhow::bail!("agent-owned ability {ability:?} requires an explicit manifest; descriptor publication must not synthesize fallback metadata");\n    }\n',
    '',
)
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-static-registration-manifest-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected missing Agent manifest rejection to fail"
fi

echo "test_check_static_registration_manifest_boundary: ok"
