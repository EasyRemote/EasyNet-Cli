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

selector_start = body.find("pub(crate) fn for_target_owned_selector_for_mode(")
selector_end = body.find("\n    pub(crate) fn ", selector_start + 1)
if selector_end < 0:
    selector_end = body.find("\n}\n", selector_start)
selector = body[selector_start:selector_end] if selector_start >= 0 and selector_end >= 0 else ""
if "RemoteRootAbilityAdmission::evaluate(&public_ability)" not in selector:
    raise SystemExit("selector_factory_missing_target_owned_admission")

subject_start = body.find("fn target_owned_remote_system_subject(")
subject_end = body.find("\nfn ", subject_start + 1)
subject = body[subject_start:subject_end] if subject_start >= 0 and subject_end >= 0 else ""
if "RemoteRootAbilityAdmission::evaluate(target.public_ability())" not in subject:
    raise SystemExit("issuer_subject_missing_target_owned_admission")

if "is_receipt_history_ability" in subject:
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
