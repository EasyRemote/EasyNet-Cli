#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-chat-ability-input-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-chat-ability-input-boundary.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

make_sandbox() {
  local sandbox
  sandbox="$(mktemp -d)"
  mkdir -p "$sandbox"
  cp -R "$REPO_ROOT/src" "$sandbox/src"
  echo "$sandbox"
}

run_check() {
  local sandbox="$1"
  (cd "$sandbox" && CHECK_CHAT_ABILITY_INPUT_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT")
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || {
  rm -rf "$SB"
  fail "happy: clean tree should pass"
}
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/parse_accepts_canonical_minimal_prompt_args/parse_accepts_legacy_prompt_only_args/' \
  "$SB/src/daemon/ability/builtins/agents/chat.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired legacy_prompt test name should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/ability/builtins/agents/chat.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = "impl ChatArgs {"
path.write_text(
    text.replace(
        needle,
        "// legacy prompt alias accepted during migration\n" + needle,
        1,
    ),
    encoding="utf-8",
)
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired legacy prompt alias language should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/parse_accepts_canonical_prompt_and_context_args/parse_accepts_old_prompt_and_context_args/' \
  "$SB/src/daemon/ability/builtins/agents/chat.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing canonical prompt+context test should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/enum ChatTurnSessionId/enum ChatSessionFallback/' \
  "$SB/src/daemon/ability/builtins/agents/chat.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing chat session selector should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/ability/builtins/agents/chat.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = "enum ChatTurnSessionId"
path.write_text(
    text.replace(
        needle,
        "// session id fallback kept for compatibility\n" + needle,
        1,
    ),
    encoding="utf-8",
)
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "session fallback vocabulary should exit 1 (got $rc)"

echo "test_check_chat_ability_input_boundary.sh: all cases passed"
