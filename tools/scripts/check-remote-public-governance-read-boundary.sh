#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

check_root() {
  local root="${1:-$ROOT}"
  local remote="$root/src/daemon/invocation/routing/remote_invoke.rs"
  [[ -f "$remote" ]] || fail "missing remote invocation source: $remote"

  if ! rg -n 'enum RemotePublicAbilityAdmission' "$remote" >/dev/null; then
    fail "public remote invocation must expose an explicit governance-read admission state"
  fi
  if ! rg -n 'RemotePublicAbilityAdmission::evaluate\(target\.public_ability\(\)\)' "$remote" >/dev/null; then
    fail "public remote tuple construction must evaluate governance-read admission"
  fi
  if ! rg -n '\.require\(target\.public_ability\(\)\)\?' "$remote" >/dev/null; then
    fail "public remote tuple construction must fail closed on governance-read admission"
  fi
  if ! rg -n 'not a public remote action' "$remote" >/dev/null; then
    fail "public remote governance-read rejection must use explicit public-action vocabulary"
  fi
  if ! rg -n 'public_tuple_plan_rejects_receipt_history_before_request_construction' "$remote" >/dev/null; then
    fail "public remote governance-read boundary regression test is missing"
  fi

  python3 - "$remote" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
match = re.search(
    r"pub\(crate\) fn public_explicit\([^)]*\)\s*->\s*anyhow::Result<Self>\s*\{",
    text,
    re.S,
)
if not match:
    raise SystemExit("missing RemoteInvocationTuplePlan::public_explicit")

start = match.end()
body_prefix = text[start : start + 500]
required = (
    "RemotePublicAbilityAdmission::evaluate(target.public_ability())",
    ".require(target.public_ability())?",
    "Self::new(",
)
cursor = 0
for needle in required:
    index = body_prefix.find(needle, cursor)
    if index == -1:
        raise SystemExit(f"public_explicit_missing_or_misordered_{needle}")
    cursor = index + len(needle)
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/src/daemon/invocation/routing"

  cat >"$tmp/src/daemon/invocation/routing/remote_invoke.rs" <<'RS'
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
  check_root "$tmp"

  python3 - "$tmp/src/daemon/invocation/routing/remote_invoke.rs" <<'PY'
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
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected public remote governance-read bypass to fail"
  fi

  echo "check-remote-public-governance-read-boundary self-test: ok"
  exit 0
fi

check_root "$ROOT"
echo "check-remote-public-governance-read-boundary: ok"
