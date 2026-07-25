#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/check-remote-public-governance-read-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

[[ -x "$SCRIPT" ]] || fail "missing executable script: $SCRIPT"
bash "$SCRIPT"
bash "$SCRIPT" --self-test

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p "$SB/tools/scripts" "$SB/src/daemon/invocation/routing"
cp "$SCRIPT" "$SB/tools/scripts/check-remote-public-governance-read-boundary.sh"

cat >"$SB/src/daemon/invocation/routing/remote_invoke.rs" <<'RS'
enum RemotePublicAbilityAdmission {
    Accepted,
    ReceiptHistoryRead,
}

impl RemotePublicAbilityAdmission {
    fn evaluate(public_ability: &str) -> Self { Self::Accepted }
    fn require(self, public_ability: &str) -> anyhow::Result<()> {
        anyhow::bail!("not a public remote action")
    }
}

impl<'a> RemoteInvocationTuplePlan<'a> {
    pub(crate) fn public_explicit(target: &'a RemoteAbilityInvocationTarget) -> anyhow::Result<Self> {
        RemotePublicAbilityAdmission::evaluate(target.public_ability())
            .require(target.public_ability())?;
        Self::new(target)
    }
}

#[test]
fn public_tuple_plan_rejects_receipt_history_before_request_construction() {}
RS

( cd "$SB" && bash tools/scripts/check-remote-public-governance-read-boundary.sh )

python3 - "$SB/src/daemon/invocation/routing/remote_invoke.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
text = text.replace(
    "        RemotePublicAbilityAdmission::evaluate(target.public_ability())\n            .require(target.public_ability())?;\n",
    "",
)
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-remote-public-governance-read-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected public remote governance-read bypass to fail"
fi

echo "test_check_remote_public_governance_read_boundary: ok"
