#!/usr/bin/env bash
#
# Guard the Pages CLI facade against scattered local ability-name construction.

set -euo pipefail

ROOT="${CHECK_PAGES_CLI_ABILITY_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-pages-cli-ability-boundary: $*" >&2
    exit 1
}

PAGES_RS="src/cli/commands/pages.rs"
[[ -f "$PAGES_RS" ]] || fail "missing $PAGES_RS"

grep -q 'enum PagesAbilityVerb' "$PAGES_RS" \
    || fail "Pages CLI must model pages verbs with PagesAbilityVerb"

grep -q 'struct PagesAbility' "$PAGES_RS" \
    || fail "Pages CLI must route through the typed PagesAbility selector"

grep -q 'fn local_registry_ability(&self)' "$PAGES_RS" \
    || fail "PagesAbility must own local registry key projection"

invoke_body="$(
    python3 - "$PAGES_RS" <<'PY'
import sys
from pathlib import Path

source = Path(sys.argv[1]).read_text()
marker = "fn invoke_pages_ability("
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
[[ -n "$invoke_body" ]] \
    || fail "Pages CLI must route all ability calls through invoke_pages_ability"

grep -q 'let target = ability.local_target(&identity.realm)?;' <<<"$invoke_body" \
    || fail "invoke_pages_ability must derive the typed LocalAbilityTarget from PagesAbility"

grep -q 'SystemInvocationTargetIssuer::local_target_root(&target, args, CallMode::Rpc)?' <<<"$invoke_body" \
    || fail "invoke_pages_ability must issue a canonical local target root invocation"

grep -q 'LocalDaemonSystemAbilityIssuer::invoke_issued_target_root_timeout(' <<<"$invoke_body" \
    || fail "invoke_pages_ability must use the named daemon-system ability issuer"

grep -q '&invocation,' <<<"$invoke_body" \
    || fail "invoke_pages_ability must submit the issued invocation, not a raw ability string"

bad="$(
    grep -nE 'format!\([^)]*pages\.(publish|list|get|unpublish)|let ability = format!\(|invoke_local_ability\(&ability,|"\.pages\.(publish|list|get|unpublish)"' "$PAGES_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "Pages CLI still scatters local ability-name construction:
$bad"
fi

echo "check-pages-cli-ability-boundary: ok"
