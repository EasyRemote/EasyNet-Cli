#!/usr/bin/env bash
# check-kernel-boundary.sh
# =========================
#
# Historical CI entrypoint for the daemon boundary. The final
# project-structure-v1 layout no longer has `src/daemon/kernel` or
# `src/runtime`; this script now protects that terminal shape rather
# than carrying an allowlist for the retired runtime namespace.
#
# Rules:
#   1. Final-forbidden source roots must not exist.
#   2. Retired crate-root namespaces must not be imported from `src/`.
#   3. Daemon control/invocation code must not depend on CLI/FFI edges.
#   4. Execution does not import federation transport implementations.
#
# Exit codes
#   0 - all rules satisfied
#   1 - at least one violation found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

violations=0

echo "== check-kernel-boundary.sh =="

production_non_comment_hits() {
    local pattern="$1"
    shift

    local paths=()
    for path in "$@"; do
        [[ -e "$path" ]] && paths+=("$path")
    done
    [[ "${#paths[@]}" -gt 0 ]] || return 0

    while IFS= read -r -d '' file; do
        awk '
            /^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$/ {
                pending_test_attr = 1
                next
            }
            pending_test_attr && /^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]+tests[[:space:]]*\{/ {
                in_test_module = 1
                next
            }
            pending_test_attr {
                pending_test_attr = 0
            }
            in_test_module {
                next
            }
            {
                print FILENAME ":" NR ":" $0
            }
        ' "$file"
    done < <(find "${paths[@]}" -type f -name '*.rs' \
        ! -name '*_tests.rs' \
        ! -path '*_tests/*' \
        ! -path '*/tests/*' \
        -print0 2>/dev/null) \
        | grep -E "$pattern" \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true
}

# Rule 1: final-forbidden source roots.
for dir in \
    src/runtime \
    src/services \
    src/facade \
    src/persistence \
    src/plugins \
    src/registry
do
    if [[ -e "$dir" ]]; then
        echo "ERROR: final-forbidden source root exists: $dir"
        echo "  Move ownership into src/core, src/daemon, src/cli, src/ffi, src/eal, or src/support."
        violations=$((violations + 1))
    fi
done

# Rule 2: no retired crate-root namespaces in active Rust source.
retired_ns_hits="$(production_non_comment_hits '\b(crate|easynet_cli)::(runtime|services|facade|persistence|plugins|registry)::' src)"
if [[ -n "$retired_ns_hits" ]]; then
    echo "ERROR: retired crate-root namespace imports found:"
    echo "$retired_ns_hits"
    echo "  Use the final project-structure-v1 owners instead of compatibility namespaces."
    violations=$((violations + 1))
fi

# Rule 3: daemon control/invocation must stay below product edges.
edge_hits="$(production_non_comment_hits '\b(crate|easynet_cli)::(cli|ffi)::' src/daemon/control src/daemon/invocation)"
if [[ -n "$edge_hits" ]]; then
    echo "ERROR: daemon control/invocation imports CLI or FFI edge modules:"
    echo "$edge_hits"
    echo "  CLI and FFI may call into daemon clients; daemon internals must not depend on them."
    violations=$((violations + 1))
fi

# Rule 4: Execution layer must not reach into federation transport
# implementations. Network publication and remote calls are owned by the
# daemon Invocation/session layers.
if [[ -d "src/daemon/execution" ]]; then
    gateway_hits="$(production_non_comment_hits '\bcrate::daemon::federation::(gateway|client|directory)\b' src/daemon/execution || true)"
    if [[ -n "$gateway_hits" ]]; then
        echo "ERROR: execution layer imports a federation transport implementation:"
        echo "$gateway_hits"
        echo "  Route network work through daemon::invocation or its session supervisor."
        violations=$((violations + 1))
    fi
fi

# Rule 5: final daemon layout has no daemon/kernel compatibility tree.
if [[ -e "src/daemon/kernel" ]]; then
    echo "ERROR: retired daemon kernel directory exists: src/daemon/kernel"
    echo "  Route invocation semantics through src/daemon/invocation/* and execution services."
    violations=$((violations + 1))
fi

if [ "$violations" -eq 0 ]; then
    echo "ok (no kernel-boundary violations)"
    exit 0
fi

echo "FAILED: $violations rule(s) violated."
exit 1
