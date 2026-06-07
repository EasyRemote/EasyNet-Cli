#!/usr/bin/env bash
#
# Guard active system ability manifests against retired public schema aliases.

set -euo pipefail

ROOT="${CHECK_SYSTEM_ABILITY_RETIRED_ALIASES_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-system-ability-retired-aliases: $*" >&2
    exit 1
}

[[ -d abilities/system ]] || fail "missing abilities/system"

bad="$(
    find abilities/system -name '*.ability.toml' -print 2>/dev/null \
        | sort \
        | xargs grep -niE 'target_node_uri|deprecated alias|migration window|legacy alias|canonical dotted' 2>/dev/null || true
)"

if [[ -n "$bad" ]]; then
    fail "active system ability manifests expose retired compatibility language:
$bad"
fi

echo "check-system-ability-retired-aliases: ok"
