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
#   * Kernel producers and consumers use Axon's descriptor-bound request and
#     finalized receipt objects directly; no CLI runtime model remains.
#
# Historical docs are intentionally out of scope. They may cite
# retired frame names or old canonical formulas while explaining why
# the migration happened.

set -euo pipefail

ROOT="${CHECK_DAEMON_INVOCATION_MIGRATION_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
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

if require_file "src/daemon/control/frames.rs"; then
    check_forbidden_enum_variants \
        "src/daemon/control/frames.rs" \
        "IncomingFrame" \
        "Invoke|OpenBidi|SendBidi|CloseBidi"
    check_forbidden_enum_variants \
        "src/daemon/control/frames.rs" \
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
        | grep -v '^src/daemon/invocation/dispatch/request.rs:' || true
)"
if [[ -n "$bad_daemon_invocation_constructors" ]]; then
    record_violation "DaemonInvocation is directly constructed outside src/daemon/invocation/dispatch/request.rs" \
        "$bad_daemon_invocation_constructors
Use DaemonInvocation::builder(caller, callee, ability, subject, derivation_policy) so caller/callee/ability/subject/nonce/causal_context stay complete and inspectable."
fi

if require_file "src/daemon/invocation/dispatch/request.rs"; then
    bad_builder_state="$(
        python3 - "src/daemon/invocation/dispatch/request.rs" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")


def block(signature: str):
    start = text.find(signature)
    if start < 0:
        return None
    opening = text.find("{", start + len(signature))
    if opening < 0:
        return None
    depth = 0
    for index in range(opening, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[opening + 1:index]
    return None


violations: list[str] = []
required_tokens = (
    "pub struct DaemonInvocationBuilder<ArgsState = InvocationArgsUnset>",
    "args_state: PhantomData<ArgsState>",
    "pub struct InvocationArgsUnset",
    "pub struct InvocationArgsSet",
    "derivation_policy: axon_sdk::invocation::InvocationDerivationPolicy",
)
for token in required_tokens:
    if token not in text:
        violations.append(f"missing builder type-state token: {token}")

unset = block("impl DaemonInvocationBuilder<InvocationArgsUnset>")
generic = block("impl<ArgsState> DaemonInvocationBuilder<ArgsState>")
complete = block("impl DaemonInvocationBuilder<InvocationArgsSet>")
for name, value in (("unset", unset), ("generic", generic), ("complete", complete)):
    if value is None:
        violations.append(f"missing {name} builder state implementation")

if "args_set: bool" in text:
    violations.append("runtime args_set boolean reintroduces an incomplete mutable builder state")
for token in (
    "pub fn nonce(mut self",
    "pub fn causal_context(mut self",
    "axon_sdk::invocation::fresh_nonce()",
):
    if token in text:
        violations.append(
            f"public ingress may not override or silently derive freshness/causality: {token}"
        )
if unset is not None and (
    "derivation_policy: axon_sdk::invocation::InvocationDerivationPolicy" not in unset
):
    violations.append("InvocationArgsUnset constructor must require an explicit derivation policy")
if generic is not None:
    if "Result<DaemonInvocationBuilder<InvocationArgsSet>>" not in generic:
        violations.append("argument setters must transition to InvocationArgsSet")
    for method in ("pub fn inspect", "pub fn build_draft", "pub fn build"):
        if method in generic:
            violations.append(f"generic builder state exposes completion method: {method}")
if unset is not None:
    for method in ("pub fn inspect", "pub fn build_draft", "pub fn build"):
        if method in unset:
            violations.append(f"InvocationArgsUnset exposes completion method: {method}")
if complete is not None:
    for method in ("pub fn inspect", "pub fn build_draft", "pub fn build"):
        if method not in complete:
            violations.append(f"InvocationArgsSet is missing completion method: {method}")

print("\n".join(violations))
PY
    )"
    if [[ -n "$bad_builder_state" ]]; then
        record_violation "DaemonInvocationBuilder does not enforce a complete public tuple" \
            "$bad_builder_state
Use the InvocationArgsUnset -> InvocationArgsSet type-state transition; only the complete state may inspect or build an Invocation."
    fi
fi

if [[ -e "src/daemon/invocation/receipts/runtime_record.rs" ]]; then
    record_violation "obsolete daemon runtime-record authority remains" \
        "src/daemon/invocation/receipts/runtime_record.rs
Delete the CLI-owned runtime model; KernelApi must consume DescriptorBoundInvocationRequest and return FinalizedInvocation."
fi

bad_kernel_runtime_model="$(
    python3 - <<'PY'
from pathlib import Path
import re

root = Path("src")
for path in sorted(root.rglob("*.rs")):
    text = path.read_text(encoding="utf-8")
    production = text.split("#[cfg(test)]", 1)[0]
    for token in ("RuntimeInvocation", "RuntimeCausalContext", "runtime_invocation_id"):
        match = re.search(rf"\b{token}\b", production)
        if match:
            line = production.count("\n", 0, match.start()) + 1
            print(f"{path}:{line}:{token}")
PY
)"
if [[ -n "$bad_kernel_runtime_model" ]]; then
    record_violation "CLI-owned production runtime model reintroduced" \
        "$bad_kernel_runtime_model
Use Axon DescriptorBoundInvocationRequest, InvocationState, FinalizedInvocation, and SignedInvocationReceipt directly."
fi

if require_file "src/daemon/boot/kernel/api.rs"; then
    kernel_api="$(cat src/daemon/boot/kernel/api.rs)"
    for required in "DescriptorBoundInvocationRequest" "FinalizedInvocation"; do
        if [[ "$kernel_api" != *"$required"* ]]; then
            record_violation "KernelApi canonical runtime boundary is incomplete" \
                "src/daemon/boot/kernel/api.rs is missing $required"
        fi
    done
fi

bad_remote_bidi_alias_language="$(
    grep -RniE 'remote bidi.*legacy alias|legacy alias.*remote bidi|preserve legacy aliases|repair_bare_device_agent_alias' \
        src/daemon/invocation 2>/dev/null || true
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
