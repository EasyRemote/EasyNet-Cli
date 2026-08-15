#!/usr/bin/env bash
#
# Guard EasyNet Hub facade ownership over Axon's canonical Authority identity.

set -euo pipefail

ROOT="${CHECK_CANONICAL_HUB_URA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
AXON_URA_RS="${CHECK_CANONICAL_HUB_URA_AXON_URA_RS:-$ROOT/../EasyNet-Axon/core/ura-rs/src/lib.rs}"

cd "$ROOT"

fail() {
    echo "check-canonical-hub-ura-boundary: $*" >&2
    exit 1
}

require_file() {
    [[ -f "$1" ]] || fail "missing $1"
}

require_grep() {
    local pattern="$1"
    local file="$2"
    local message="$3"
    grep -Fq "$pattern" "$file" || fail "$message"
}

require_any_grep() {
    local file="$1"
    local message="$2"
    shift 2
    local pattern
    for pattern in "$@"; do
        if grep -Fq "$pattern" "$file"; then
            return 0
        fi
    done
    fail "$message"
}

reject_grep() {
    local pattern="$1"
    local file="$2"
    local message="$3"
    if grep -Fq "$pattern" "$file"; then
        fail "$message"
    fi
}

require_file "$AXON_URA_RS"
require_file src/core/ura/mod.rs
require_file src/daemon/invocation/routing/remote_invoke.rs
require_file src/daemon/invocation/admission/admission_facade.rs
require_file src/daemon/invocation/dispatch/daemon_invocation_service.rs
require_file src/daemon/invocation/admission/register_device_pubkey.rs
require_file src/daemon/invocation/admission/runtime_trust.rs

require_grep 'Self(format!("{URA_SCHEME}{realm}/authority"))' "$AXON_URA_RS" \
    "Axon authority builder must generate easynet:///r/<realm>/authority"
require_grep '"authority" => {' "$AXON_URA_RS" \
    "Axon parser must have an explicit Authority role arm"
require_grep 'return Err(ParseError::AuthorityUnexpectedTail(tail.to_string()));' "$AXON_URA_RS" \
    "Axon parser must reject Authority URAs with extra tail segments"
require_grep 'assert_eq!(' "$AXON_URA_RS" \
    "Axon tests must pin authority_ura(realm)"
require_grep 'authority_ura("localhost")' "$AXON_URA_RS" \
    "Axon tests must pin authority_ura(realm)"
require_grep '"easynet:///r/localhost/authority"' "$AXON_URA_RS" \
    "Axon tests must pin authority_ura(realm) to /authority"
require_grep 'assert!(parse_ura("easynet:///r/localhost/authority").is_ok());' "$AXON_URA_RS" \
    "Axon tests must accept the canonical /authority identity"
require_grep 'assert!(parse_ura("easynet:///r/localhost/authority/extra").is_err());' "$AXON_URA_RS" \
    "Axon tests must reject non-singleton Authority tails"
reject_grep '{URA_SCHEME}{realm}/hub' "$AXON_URA_RS" \
    "Axon canonical URA implementation must not generate Hub identities"
reject_grep '"hub" =>' "$AXON_URA_RS" \
    "Axon canonical URA parser must not accept Hub identities"
reject_grep 'AbilityOwner::Hub' "$AXON_URA_RS" \
    "Axon canonical ability owner must not expose Hub"
reject_grep 'URAKind::Hub' "$AXON_URA_RS" \
    "Axon canonical URA kind must not expose Hub"

require_grep 'hub       easynet:///r/<realm>/authority' src/core/ura/mod.rs \
    "CLI URA facade docs must project Hub onto canonical /authority identity"
require_grep 'pub fn hub_ura(realm: &str) -> String {' src/core/ura/mod.rs \
    "CLI must own the product-facing hub_ura facade"
require_grep 'authority_ura(realm)' src/core/ura/mod.rs \
    "CLI hub_ura facade must delegate to Axon authority_ura"
require_grep 'pub fn hub_ability_ura(realm: &str, ability_name: &str) -> String {' src/core/ura/mod.rs \
    "CLI must own the product-facing hub_ability_ura facade"
require_grep 'authority_ability_ura(realm, ability_name)' src/core/ura/mod.rs \
    "CLI hub_ability_ura facade must delegate to Axon authority_ability_ura"
reject_grep 'easynet:///r/localhost/hub' src/core/ura/mod.rs \
    "CLI URA facade examples must not advertise /hub as canonical wire identity"
reject_grep 'hub       easynet:///r/<realm>/hub' src/core/ura/mod.rs \
    "CLI URA facade docs must not advertise /hub as canonical wire identity"

require_grep 'RuntimeIdentityUra::parse(trimmed)' src/daemon/invocation/routing/route_target.rs \
    "remote target parsers must enter through the canonical RuntimeIdentityUra value object"
require_grep 'URAKind::Agent | URAKind::Service | URAKind::Authority =>' src/daemon/invocation/routing/route_target.rs \
    "ability route targets must classify canonical Authority URAs as exact callees"
if python3 - "$ROOT/src/daemon/invocation/routing/route_target.rs" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
match = re.search(
    r"impl RemoteAbilityRouteTarget \{.*?^}",
    text,
    flags=re.M | re.S,
)
if not match:
    raise SystemExit(2)
body = match.group(0)
if "crate::core::ura::parse_ura" in body or "parse_ura(trimmed)" in body:
    raise SystemExit(1)
raise SystemExit(0)
PY
then
    :
else
    fail "RemoteAbilityRouteTarget must not bypass RuntimeIdentityUra with direct parse_ura"
fi
require_grep 'Authority inputs are exact callees' src/daemon/invocation/routing/route_target.rs \
    "route target docs must state Authority is an exact callable identity"
require_grep 'fn ability_route_target_accepts_device_placement_and_exact_actor_callees()' src/daemon/invocation/routing/route_target.rs \
    "route target tests must cover exact Authority callees"
require_grep 'parse_device_placement_ura("easynet:///r/realm/authority/extra")' src/daemon/invocation/routing/route_target.rs \
    "Device placement negative fixture must pin Authority tail rejection"
reject_grep 'RemoteAbilityRouteTarget::parse("easynet:///r/realm/hub' src/daemon/invocation/routing/route_target.rs \
    "route target tests must not accept or pin /hub wire identity"
require_grep 'assert!(facade.is_federated_caller("easynet:///r/peer-realm/authority"));' \
    src/daemon/invocation/admission/admission_facade.rs \
    "admission facade must accept canonical Authority callers as product Hub callers"
require_grep 'assert!(!facade.is_federated_caller("easynet:///r/peer-realm/authority/extra"));' \
    src/daemon/invocation/admission/admission_facade.rs \
    "admission facade must reject Authority callers with tail"
# The service behavior tests consume the same helper through the
# admission facade; the realm-extraction assertions live there now.
require_grep 'crate::core::ura::realm_from_ura("easynet:///r/peer-realm/authority")' \
    src/daemon/invocation/admission/admission_facade.rs \
    "daemon realm extraction must accept canonical Authority URAs"
require_grep 'crate::core::ura::realm_from_ura("easynet:///r/peer-realm/authority/extra")' \
    src/daemon/invocation/admission/admission_facade.rs \
    "daemon realm extraction must reject Authority URAs with tail"
require_grep 'fn register_hub_role_uses_canonical_authority_identity()' \
    src/daemon/invocation/admission/runtime_trust.rs \
    "runtime trust must test Hub role registration through canonical Authority URAs"
require_grep 'let hub_ura = crate::core::ura::hub_ura("realm");' \
    src/daemon/invocation/admission/runtime_trust.rs \
    "runtime trust Hub test must build Hub identity through the core URA facade"
require_grep '"easynet:///r/realm/authority/extra".to_string()' \
    src/daemon/invocation/admission/runtime_trust.rs \
    "runtime trust Hub test must reject Authority URAs with tail"
reject_grep 'parse_realm_from_ura' src/daemon/invocation/admission/register_device_pubkey.rs \
    "register_device_pubkey must not own a duplicate realm parser"

bad_docs="$(
    scan_roots=(src tests tools/scripts docs/spec)
    [[ ! -d docs/stale ]] || scan_roots+=(docs/stale)
    find "${scan_roots[@]}" -type f \( -name '*.rs' -o -name '*.md' -o -name '*.sh' -o -name '*.toml' -o -name '*.tex' \) -print 2>/dev/null \
        | sort \
        | grep -v -E '(^|/)(tools/scripts/check-canonical-hub-ura-boundary\.sh|tests/scripts/test_check_canonical_hub_ura_boundary\.sh)$' \
        | xargs grep -nE 'malformed hub identity shape `easynet:///r/<realm>/hub`|canonical hub shape:[[:space:]]*$|easynet:///r/<realm>/hub|easynet:///r/<r>/hub|/ability/hub\.' 2>/dev/null \
        || true
)"
if [[ -n "$bad_docs" ]]; then
    fail "stale Hub identity wording found:
$bad_docs"
fi

echo "check-canonical-hub-ura-boundary: ok"
