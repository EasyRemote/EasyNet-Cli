#!/usr/bin/env bash
# check-daemon-invocation-migration.sh
# =====================================
#
# CI gate for commit-plan Steps 4, 6, and 7.
#
# This script protects the clean target architecture after JSON
# control demotion:
#
#   * `control.sock` remains boot/status only.
#   * product callers build `DaemonInvocation` through the complete
#     tuple builder instead of direct struct construction.
#   * `runtime::invocation` remains a daemon-local adapter over Axon
#     canonical bytes, not a second canonical Invocation model.
#
# Historical docs are intentionally out of scope. They may cite
# retired frame names or old canonical formulas while explaining why
# the migration happened.

set -euo pipefail

ROOT="${CHECK_DAEMON_INVOCATION_MIGRATION_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

echo "== check-daemon-invocation-migration.sh =="

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

check_forbidden_enum_variants() {
    local file="$1"
    local enum_name="$2"
    local forbidden="$3"
    local bad

    bad="$(
        awk -v enum_name="$enum_name" -v forbidden="$forbidden" '
            $0 ~ "pub enum " enum_name { in_enum = 1; next }
            in_enum && /^[[:space:]]*}/ { in_enum = 0 }
            in_enum && $0 !~ /^[[:space:]]*\/\// && $0 ~ "^[[:space:]]*(" forbidden ")([[:space:]]|\\{|,|$)" {
                print FILENAME ":" NR ":" $0
            }
        ' "$file"
    )"
    if [[ -n "$bad" ]]; then
        record_violation "$enum_name exposes retired product frame variants" "$bad"
    fi
}

if require_file "src/services/control/frames.rs"; then
    check_forbidden_enum_variants \
        "src/services/control/frames.rs" \
        "IncomingFrame" \
        "Invoke|OpenBidi|SendBidi|CloseBidi"
    check_forbidden_enum_variants \
        "src/services/control/frames.rs" \
        "OutgoingFrame" \
        "Result|RecvBidi|TerminalBidi|ErrorBidi"
fi

if require_file "README.md"; then
    stale_readme_runtime="$(
        grep -nE 'auto-spawns (one|an? Axon runtime)|Axon runtime started|Requires a local or remote Axon runtime' README.md || true
    )"
    if [[ -n "$stale_readme_runtime" ]]; then
        record_violation "README documents the retired product Axon-runtime start path" \
            "$stale_readme_runtime
Document easynet-daemon as the product runtime; standalone axon-runtime is a protocol reference runtime, not the EasyNet product start path."
    fi
fi

bad_control_constructors="$(
    find src tests -name '*.rs' -print 2>/dev/null \
        | sort \
        | xargs awk '
            /^[[:space:]]*\/\// { next }
            /IncomingFrame::(Invoke|OpenBidi|SendBidi|CloseBidi)([^[:alnum:]_]|$)/ ||
            /OutgoingFrame::(Result|RecvBidi|TerminalBidi|ErrorBidi)([^[:alnum:]_]|$)/ {
                print FILENAME ":" NR ":" $0
            }
        ' 2>/dev/null || true
)"
if [[ -n "$bad_control_constructors" ]]; then
    record_violation "active code constructs retired JSON-control product frames" \
        "$bad_control_constructors"
fi

bad_daemon_invocation_constructors="$(
    find src tests -name '*.rs' -print 2>/dev/null \
        | sort \
        | xargs awk '
            /^[[:space:]]*\/\// { next }
            /DaemonInvocation[[:space:]]*\{/ {
                print FILENAME ":" NR ":" $0
            }
        ' 2>/dev/null \
        | grep -v '^src/daemon/invocation.rs:' || true
)"
if [[ -n "$bad_daemon_invocation_constructors" ]]; then
    record_violation "DaemonInvocation is directly constructed outside src/daemon/invocation.rs" \
        "$bad_daemon_invocation_constructors
Use DaemonInvocation::builder(caller, callee, ability, subject) so caller/callee/ability/subject/nonce/causal_context stay complete and inspectable."
fi

if require_file "src/runtime/invocation.rs"; then
    bad_runtime_semantics="$(
        awk '
            /^[[:space:]]*\/\// { next }
            /^[[:space:]]*(pub[[:space:]]+)?fn[[:space:]]+canonical_bytes[[:space:]]*\(/ ||
            /(^|[^[:alnum:]_])invocation_id_of([^[:alnum:]_]|$)/ ||
            /^[[:space:]]*pub[[:space:]]+struct[[:space:]]+Invocation([^[:alnum:]_]|$)/ ||
            /^[[:space:]]*pub[[:space:]]+enum[[:space:]]+CausalContext([^[:alnum:]_]|$)/ {
                print FILENAME ":" NR ":" $0
            }
        ' src/runtime/invocation.rs
    )"
    if [[ -n "$bad_runtime_semantics" ]]; then
        record_violation "runtime::invocation reintroduces CLI-owned Invocation semantics" \
            "$bad_runtime_semantics
Keep RuntimeInvocation as an adapter over easynet_axon::invocation::canonical_invocation_bytes."
    fi
fi

bad_remote_bidi_alias_language="$(
    grep -RniE 'remote bidi.*legacy alias|legacy alias.*remote bidi|preserve legacy aliases|repair_bare_device_agent_alias' \
        src/services/invocation_transport 2>/dev/null || true
)"
if [[ -n "$bad_remote_bidi_alias_language" ]]; then
    record_violation "active remote-bidi code still references retired alias semantics" \
        "$bad_remote_bidi_alias_language
Remote-bidi target extraction must preserve explicit callee URAs and let current self-target/presence rules accept or reject them; do not describe non-device callees as compatibility aliases."
fi

if [[ "$violations" -eq 0 ]]; then
    echo "ok (daemon Invocation migration guard is clean)"
    exit 0
fi

echo "FAILED: $violations violation(s)."
exit 1
