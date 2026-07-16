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

[[ -f "$SKILL_RS" ]] || fail "missing $SKILL_RS"

grep -q 'fn managed_skill_dir_for_layout' "$SKILL_RS" \
    || fail "skill list must centralize managed directory selection"

managed_dir_selector="$(
    sed -n '/^fn managed_skill_dir_for_layout/,/^struct SkillListScope/p' "$SKILL_RS"
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

grep -q 'managed_skill_dir_for_claude_code_uses_native_project_dir_only' "$SKILL_RS" \
    || fail "unit tests must pin claude-code managed skill directory"

bad="$(
    grep -nE 'legacy|pre-fix|skill_dirs|root\.join\("skills"\).*legacy|ClaudeCode[[:space:][:punct:][:alnum:]_]*root\.join\("skills"\)|<root>/skills' \
        "$SKILL_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "skill list still carries retired legacy scan language or plumbing:
$bad"
fi

echo "check-skill-list-managed-dir-boundary: ok"
