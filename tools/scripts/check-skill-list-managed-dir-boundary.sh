#!/usr/bin/env bash
#
# Guard skill.list against retired Claude root-level skills scans.

set -euo pipefail

ROOT="${CHECK_SKILL_LIST_MANAGED_DIR_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-skill-list-managed-dir-boundary: $*" >&2
    exit 1
}

SKILL_RS="src/daemon/ability/builtins/resources/skills/list.rs"
STORE_RS="src/daemon/resources/skills/store.rs"

[[ -f "$SKILL_RS" ]] || fail "missing $SKILL_RS"
[[ -f "$STORE_RS" ]] || fail "missing $STORE_RS"

grep -q 'use crate::daemon::resources::skills::store::managed_skill_dir_for;' "$SKILL_RS" \
    || fail "skill list must import the canonical managed directory selector"

grep -q 'managed_skill_dir_for(workspace.root_path(), workspace.skill_layout())' "$SKILL_RS" \
    || fail "skill list must delegate managed directory selection to the store layer"

grep -q 'fn managed_skill_dir_for' "$STORE_RS" \
    || fail "skill list must centralize managed directory selection"

managed_dir_selector="$(
    python3 - "$STORE_RS" <<'PY'
import sys
from pathlib import Path

source = Path(sys.argv[1]).read_text()
marker = "fn managed_skill_dir_for("
start = source.find(marker)
if start < 0:
    raise SystemExit(1)

brace = source.find("{", start)
if brace < 0:
    raise SystemExit(1)

depth = 0
for index in range(brace, len(source)):
    char = source[index]
    if char == "{":
        depth += 1
    elif char == "}":
        depth -= 1
        if depth == 0:
            print(source[start:index + 1])
            raise SystemExit(0)

raise SystemExit(1)
PY
)"

printf '%s\n' "$managed_dir_selector" | awk '
    /AgentSkillLayout::ClaudeCode/ { in_arm = 1; seen = 0 }
    in_arm {
        if ($0 ~ /root\.join\("\.claude"\)\.join\("skills"\)/) {
            found = 1
            exit 0
        }
        seen++
        if (seen > 6) {
            in_arm = 0
        }
    }
    END { exit found ? 0 : 1 }
' || fail "claude-code managed skills must resolve to .claude/skills only"

printf '%s\n' "$managed_dir_selector" | grep -q 'AgentSkillLayout::Codex => root.join(".agents").join("skills")' \
    || fail "codex managed skills must resolve to .agents/skills"

printf '%s\n' "$managed_dir_selector" | grep -q 'AgentSkillLayout::External => root.join("skills")' \
    || fail "external managed skills must keep the generic external directory"

grep -q 'managed_skill_dir_for_claude_code_uses_native_project_dir_only' "$SKILL_RS" \
    || fail "unit tests must pin claude-code managed skill directory"

bad="$(
    grep -nE 'skill_dirs|root\.join\("skills"\).*legacy|ClaudeCode[[:space:][:punct:][:alnum:]_]*root\.join\("skills"\)|legacy <root>/skills compatibility scan' \
        "$SKILL_RS" "$STORE_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "skill list still carries retired legacy scan language or plumbing:
$bad"
fi

echo "check-skill-list-managed-dir-boundary: ok"
