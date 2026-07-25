#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf 'check-target-owned-history-boundary: %s\n' "$1" >&2
  exit 1
}

TARGET="src/daemon/invocation/routing/remote_invoke.rs"
[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum RemoteRootAbilityAdmission' "$TARGET" >/dev/null; then
  fail "target-owned system ability policy must be an explicit admission state"
fi

for state in Accepted ReceiptHistoryRead; do
  if ! rg -n "$state" "$TARGET" >/dev/null; then
    fail "RemoteRootAbilityAdmission missing state: $state"
  fi
done

for method in \
  'fn evaluate\(public_ability: &str\) -> Self' \
  'fn require\(self, public_ability: &str\) -> anyhow::Result<\(\)>'
do
  if ! rg -n "$method" "$TARGET" >/dev/null; then
    fail "RemoteRootAbilityAdmission missing required method: $method"
  fi
done

if ! python3 - "$TARGET" <<'PY'
import re
import sys
from pathlib import Path

body = Path(sys.argv[1]).read_text()

selector = re.search(
    r"pub\(crate\) fn for_target_owned_selector\([\s\S]*?\n    \}",
    body,
)
if not selector or "RemoteRootAbilityAdmission::evaluate(&public_ability)" not in selector.group(0):
    raise SystemExit("selector_factory_missing_target_owned_admission")

subject = re.search(
    r"fn target_owned_remote_system_subject\([\s\S]*?\n\}",
    body,
)
if not subject or "RemoteRootAbilityAdmission::evaluate(target.public_ability())" not in subject.group(0):
    raise SystemExit("issuer_subject_missing_target_owned_admission")

if "is_receipt_history_ability" in subject.group(0):
    raise SystemExit("issuer_reintroduced_receipt_history_local_guard")
PY
then
  fail "target-owned receipt history policy is not centralized"
fi

for test in \
  target_owned_selector_rejects_receipt_history_before_tuple_build \
  remote_system_issuer_rejects_receipt_history_as_target_owned
do
  if ! rg -n "fn $test" "$TARGET" >/dev/null; then
    fail "missing Rust boundary test: $test"
  fi
done

echo "check-target-owned-history-boundary: ok"
