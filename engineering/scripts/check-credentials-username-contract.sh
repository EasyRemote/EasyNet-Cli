#!/usr/bin/env bash
#
# Guard joined device credentials against username migration fallbacks.

set -euo pipefail

ROOT="${CHECK_CREDENTIALS_USERNAME_CONTRACT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-credentials-username-contract: $*" >&2
    exit 1
}

CONFIG_RS="src/persistence/config.rs"
JOIN_RS="src/cli/join.rs"
START_RS="src/cli/start.rs"

for file in "$CONFIG_RS" "$JOIN_RS" "$START_RS"; do
    [[ -f "$file" ]] || fail "missing $file"
done

bad_fallback="$(
    grep -nE 'backfill_credentials_username|auth\.json\.username|auth session|migration window|Optional during the migration window|older credentials files may still miss|load_session\(\)' \
        "$JOIN_RS" "$START_RS" "$CONFIG_RS" 2>/dev/null || true
)"
if [[ -n "$bad_fallback" ]]; then
    fail "credentials username contract still contains migration fallback:
$bad_fallback"
fi

grep -q 'credentials file is missing username' "$CONFIG_RS" \
    || fail "load/save credential validation must reject missing username"

grep -q 'pairing response missing username' "$JOIN_RS" \
    || fail "join pairing response validation must reject missing username"

grep -q 'username_slug' "$START_RS" \
    || fail "runtime start must derive bootstrap identity from validated credentials"

echo "check-credentials-username-contract: ok"
