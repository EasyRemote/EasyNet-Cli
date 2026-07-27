#!/usr/bin/env bash
#
# check-pending-dispatch-target-boundary.sh
# =========================================
#
# CI gate for canonical remote dispatch terminality.
#
# Pending dispatch registration must be bound to the selected execution
# host URA. A no-target registration path recreates the retired behavior
# where offline targets are discovered only after a transport timeout or a
# dropped oneshot receiver, producing non-deterministic lifecycle terminality.

set -euo pipefail

ROOT="${CHECK_PENDING_DISPATCH_TARGET_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

echo "== check-pending-dispatch-target-boundary.sh =="

violations=0

record_violation() {
    local title="$1"
    local detail="$2"
    echo "ERROR: $title"
    echo "$detail"
    violations=$((violations + 1))
}

require_file() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        record_violation "required file missing" "$file"
        return 1
    fi
    return 0
}

pending="src/daemon/invocation/bidi/state/pending_dispatch.rs"
unary_dispatcher="src/daemon/invocation/dispatch/unary_dispatcher.rs"
stream_dispatcher="src/daemon/invocation/streams/stream_dispatcher.rs"

if require_file "$pending"; then
    bare_method_defs="$(
        rg -n 'pub[[:space:]]+fn[[:space:]]+register_pending[[:space:]]*\(&self\)' "$pending" || true
    )"
    if [[ -n "$bare_method_defs" ]]; then
        record_violation "pending dispatch exposes no-target registration" \
            "$bare_method_defs
Use register_pending_for(target_ura) so presence-loss cancellation has an explicit lifecycle key."
    fi

    bare_calls="$(
        rg -n '\.register_pending[[:space:]]*\(' "$pending" || true
    )"
    if [[ -n "$bare_calls" ]]; then
        record_violation "pending dispatch tests or implementation still call no-target registration" \
            "$bare_calls
All pending dispatch registrations, including tests, must supply an execution-host target_ura."
    fi

    for required in \
        'pub fn register_pending_for(&self, target_ura: &str) -> PendingHandle' \
        'pub fn register_pending_for(&self, target_ura: &str) -> PendingStreamHandle' \
        'fn register_pending_for_policy(' \
        'fn require_pending_target_ura(target_ura: &str) -> &str' \
        'let target_ura = target_ura.trim();' \
        '"pending dispatch target_ura is required"' \
        'unary_pending_registration_rejects_empty_target_ura' \
        'stream_pending_registration_rejects_empty_target_ura'
    do
        if ! grep -Fq "$required" "$pending"; then
            record_violation "pending dispatch target boundary token missing" "$required"
        fi
    done

    stale_compat_words="$(
        rg -n 'legacy "wait until oneshot drops"|not wired|won.t be auto-cancelled|empty target_ura|register_pending\(call_id\)' "$pending" || true
    )"
    if [[ -n "$stale_compat_words" ]]; then
        record_violation "pending dispatch still documents the retired no-target compatibility model" \
            "$stale_compat_words"
    fi
fi

if require_file "$unary_dispatcher"; then
    if ! grep -Fq 'register_pending_for(&selected_route.execution_host_ura)' "$unary_dispatcher"; then
        record_violation "unary remote dispatch is not bound to the selected execution host URA" \
            "$unary_dispatcher must call register_pending_for(&selected_route.execution_host_ura)."
    fi
fi

if require_file "$stream_dispatcher"; then
    if ! grep -Fq 'register_pending_for(&selected_route.execution_host_ura)' "$stream_dispatcher"; then
        record_violation "stream remote dispatch is not bound to the selected execution host URA" \
            "$stream_dispatcher must call register_pending_for(&selected_route.execution_host_ura)."
    fi
fi

if (( violations > 0 )); then
    echo "check-pending-dispatch-target-boundary: FAILED ($violations violation(s))" >&2
    exit 1
fi

echo "ok (pending dispatch target boundary is clean)"
