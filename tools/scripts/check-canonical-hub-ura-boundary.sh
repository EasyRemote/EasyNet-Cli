#!/usr/bin/env bash
#
# Guard canonical Hub identity ownership across Axon and EasyNet-Cli.

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

require_grep 'hub       easynet:///r/<realm>/hub' "$AXON_URA_RS" \
    "Axon URA docs must advertise /hub as the canonical Hub identity"
require_grep 'Self(format!("{URA_SCHEME}{realm}/hub"))' "$AXON_URA_RS" \
    "Axon hub builder must generate easynet:///r/<realm>/hub"
require_grep '"hub" => {' "$AXON_URA_RS" \
    "Axon parser must have an explicit Hub role arm"
require_grep 'return Err(ParseError::HubUnexpectedTail(tail.to_string()));' "$AXON_URA_RS" \
    "Axon parser must reject Hub URAs with extra tail segments"
require_grep 'assert_eq!(hub_ura("localhost"), "easynet:///r/localhost/hub");' "$AXON_URA_RS" \
    "Axon tests must pin hub_ura(realm) to /hub"
require_grep 'assert!(parse_ura("easynet:///r/localhost/hub").is_ok());' "$AXON_URA_RS" \
    "Axon tests must accept the canonical /hub identity"
require_grep 'assert!(parse_ura("easynet:///r/localhost/hub/extra").is_err());' "$AXON_URA_RS" \
    "Axon tests must reject non-singleton Hub tails"
reject_grep '{URA_SCHEME}{realm}/hub/{realm}' "$AXON_URA_RS" \
    "Axon must not generate a Hub identity with tail"

require_grep 'hub       easynet:///r/<realm>/hub' src/core/ura/mod.rs \
    "CLI URA facade docs must advertise /hub as the canonical Hub identity"
require_grep 'easynet:///r/localhost/hub' src/core/ura/mod.rs \
    "CLI URA facade examples must include the canonical /hub identity"
reject_grep 'hub       easynet:///r/<realm>/hub/<id>' src/core/ura/mod.rs \
    "CLI URA facade docs must not advertise Hub identities with tail"

require_any_grep src/daemon/invocation/routing/remote_invoke.rs \
    "parse_node_ura must delegate grammar to Axon" \
    'use axon_sdk::ura::{parse_ura, URAKind};' \
    'use axon_sdk::ura::{URAKind, parse_ura};'
require_grep 'whose clean protocol identity is `easynet:///r/<realm>/hub`' src/daemon/invocation/routing/remote_invoke.rs \
    "parse_node_ura docs must state the canonical /hub identity"
require_grep 'fn parse_node_ura_accepts_protocol_hub_identity()' src/daemon/invocation/routing/remote_invoke.rs \
    "parse_node_ura tests must accept literal canonical /hub"
require_grep 'fn parse_node_ura_rejects_hub_with_tail()' src/daemon/invocation/routing/remote_invoke.rs \
    "parse_node_ura tests must reject Hub URAs with tail"
require_grep 'parse_node_ura("easynet:///r/realm/hub/extra")' src/daemon/invocation/routing/remote_invoke.rs \
    "parse_node_ura negative fixture must pin Hub tail rejection"
require_grep 'assert!(facade.is_federated_caller("easynet:///r/peer-realm/hub"));' \
    src/daemon/invocation/admission/admission_facade.rs \
    "admission facade must accept canonical Hub callers"
require_grep 'assert!(!facade.is_federated_caller("easynet:///r/peer-realm/hub/extra"));' \
    src/daemon/invocation/admission/admission_facade.rs \
    "admission facade must reject Hub callers with tail"
# The service behavior tests consume the same helper through the
# admission facade; the realm-extraction assertions live there now.
require_grep 'parse_realm_from_ura("easynet:///r/peer-realm/hub")' \
    src/daemon/invocation/admission/admission_facade.rs \
    "daemon realm extraction must accept canonical Hub URAs"
require_grep 'parse_realm_from_ura("easynet:///r/peer-realm/hub/extra")' \
    src/daemon/invocation/admission/admission_facade.rs \
    "daemon realm extraction must reject Hub URAs with tail"
require_grep 'parse_realm_from_ura("easynet:///r/abc/hub")' \
    src/daemon/invocation/admission/register_device_pubkey.rs \
    "register_device_pubkey realm extraction must accept canonical Hub URAs"
require_grep 'parse_realm_from_ura("easynet:///r/abc/hub/extra")' \
    src/daemon/invocation/admission/register_device_pubkey.rs \
    "register_device_pubkey realm extraction must reject Hub URAs with tail"

bad_docs="$(
    find src tests docs tools/scripts -type f \( -name '*.rs' -o -name '*.md' -o -name '*.sh' -o -name '*.toml' \) -print 2>/dev/null \
        | sort \
        | grep -v -E '(^|/)(tools/scripts/check-canonical-hub-ura-boundary\.sh|tests/scripts/test_check_canonical_hub_ura_boundary\.sh)$' \
        | xargs grep -nE 'malformed hub identity shape `easynet:///r/<realm>/hub`|canonical hub shape:[[:space:]]*$' 2>/dev/null \
        || true
)"
if [[ -n "$bad_docs" ]]; then
    fail "stale Hub identity wording found:
$bad_docs"
fi

echo "check-canonical-hub-ura-boundary: ok"
