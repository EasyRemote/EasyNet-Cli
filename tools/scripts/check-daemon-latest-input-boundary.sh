#!/usr/bin/env bash
#
# Guard daemon request DTOs and lifecycle against retired compatibility paths.

set -euo pipefail

ROOT="${CHECK_DAEMON_LATEST_INPUT_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-daemon-latest-input-boundary: $*" >&2
    exit 1
}

bad="$(
    rg -n '#\s*\[\s*serde\s*\([^]]*\balias\s*=' \
        src/daemon/invocation/dispatch \
        -g'*.rs' 2>/dev/null || true
)"

if [[ -n "$bad" ]]; then
    fail "daemon request DTOs expose legacy serde input aliases:
$bad"
fi

for retired_path in \
    src/daemon/control/runtime_dispatch.rs \
    src/daemon/control/runtime_dispatch_adapter.rs
do
    if [[ -e "$retired_path" ]]; then
        fail "retired daemon callback surface exists: $retired_path"
    fi
done

retired_lifecycle="$({
    rg -n 'runtime[_-]dispatch|runtime\.register_local_tool' \
        src/bin/easynet-daemon.rs \
        src/daemon/control/mod.rs 2>/dev/null || true
    rg -n 'heartbeat_pid|heartbeat\.pid|legacy-heartbeat|LegacyCleanupFailed|legacy_cleanup_failed' \
        src/cli/commands/stop.rs \
        src/daemon/boot/lifecycle \
        src/daemon/persistence/config.rs \
        src/daemon/persistence/mod.rs 2>/dev/null || true
})"

if [[ -n "$retired_lifecycle" ]]; then
    fail "retired daemon lifecycle branch is still reachable:
$retired_lifecycle"
fi

resolver_fallback="$(
    rg -n 'json_string\(query,\s*"queryName",\s*"query_name"|json_string\(query,\s*"abilityName",\s*"ability_name"|json_string\(query,\s*"realmHint",\s*"realm_hint"|value\.get\("qtype"\)\.or_else\(\|\| value\.get\("qType"\)\)' \
        src/daemon/invocation/routing/route_resolver.rs 2>/dev/null || true
)"

if [[ -n "$resolver_fallback" ]]; then
    fail "daemon namespace resolver still accepts retired input aliases:
$resolver_fallback"
fi

namespace_proxy_fallback="$(
    {
        rg -n 'rename\s*=\s*"(queryName|abilityName|realmHint|callerUra|subjectUra)"' \
            src/daemon/invocation/dispatch/federation_wrappers.rs 2>/dev/null || true
        rg -n '"(queryName|abilityName|realmHint|callerUra|subjectUra)"\s*:' \
            src/daemon/invocation/dispatch/unary_dispatcher.rs 2>/dev/null || true
        rg -n '"(answerKind|canonicalName|releaseProfile|cachePolicy|ttlMs|sharedCacheable|retryAfterUnixMs)"\s*:' \
            src/daemon/invocation/dispatch/unary_dispatcher.rs 2>/dev/null || true
    }
)"

if [[ -n "$namespace_proxy_fallback" ]]; then
    fail "daemon namespace proxy resolve still emits or accepts retired carrier fields:
$namespace_proxy_fallback"
fi

echo "check-daemon-latest-input-boundary: ok"
