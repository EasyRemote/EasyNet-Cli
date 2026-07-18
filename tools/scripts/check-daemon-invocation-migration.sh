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

if require_file "src/daemon/invocation/routing/target.rs" \
    && require_file "src/daemon/invocation/routing/remote_invoke.rs" \
    && require_file "src/support/platform/local_daemon_grpc.rs" \
    && require_file "src/daemon/invocation/dispatch/local_runtime_invoker.rs" \
    && require_file "src/daemon/invocation/dispatch/daemon_route_runtime.rs" \
    && require_file "src/daemon/axon_bridge/dispatch_shim.rs" \
    && require_file "src/daemon/axon_bridge/local_runtime_request.rs" \
    && require_file "src/daemon/invocation/dispatch/daemon_invocation_service.rs"; then
    bad_route_tuple_ownership="$(
        python3 - <<'PY'
from pathlib import Path
import re

root = Path(".")
violations: list[str] = []

def read(path: str) -> str:
    return (root / path).read_text(encoding="utf-8", errors="replace")

def require(path: str, token: str, detail: str) -> None:
    if token not in read(path):
        violations.append(f"{path}: missing {detail}: {token}")

def production_prefix(text: str) -> str:
    match = re.search(r"(?m)^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*$", text)
    return text if match is None else text[:match.start()]

def enclosing_function_name(text: str, offset: int):
    found = None
    for match in re.finditer(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(", text[:offset]):
        found = match.group(1)
    return found

target = "src/daemon/invocation/routing/target.rs"
target_text = read(target)
for token, detail in (
    ("pub enum InvocationPlanIngress", "explicit ingress authority state"),
    ("DaemonSystem", "named daemon-system tuple derivation state"),
    ("PublicIngress", "public-ingress explicit tuple state"),
    ("plan.ingress.into_target_bindings()?", "resolver tuple handoff"),
    ("InvocationSubject::daemon_system_derived()", "named system subject policy"),
    ("pub fn public_root_derived()", "named public root causal policy"),
    ("InvocationCausalContext::daemon_system_root()", "named system causal policy"),
    ("InvocationSubject::explicit(subject)", "public subject preservation"),
    ("InvocationCausalContext::explicit(causal_context)", "public causal context preservation"),
):
    if token not in target_text:
        violations.append(f"{target}: missing {detail}: {token}")

for path in sorted((root / "src").rglob("*.rs")):
    rel = path.relative_to(root).as_posix()
    if path.name in {"real_invoke_tests.rs", "tests.rs"} or path.name.endswith("_tests.rs"):
        continue
    if "/tests/" in f"/{rel}/":
        continue
    production = production_prefix(path.read_text(encoding="utf-8", errors="replace"))
    for match in re.finditer(r"\.with_(?:subject|causal_context)\s*\(", production):
        violations.append(
            f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: production InvocationTarget tuple patching is forbidden"
        )
    if rel != "src/daemon/invocation/routing/remote_invoke.rs":
        for match in re.finditer(r"RemoteInvocationRequest::new\s*\(", production):
            violations.append(
                f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: production remote invocation ingress must use RemoteInvocationTuplePlan"
            )

remote = "src/daemon/invocation/routing/remote_invoke.rs"
remote_text = read(remote)
for token, detail in (
    ("pub(crate) enum RemoteInvocationSubject", "named remote subject derivation state"),
    ("PairedOwnerDerived", "public omitted-subject derivation policy"),
    ("TargetOwnedSystem", "daemon system subject derivation policy"),
    ("pub(crate) enum RemoteInvocationNonce", "named remote nonce derivation state"),
    ("pub(crate) struct RemoteInvocationTuplePlan", "inspectable remote tuple plan"),
    ("pub(crate) fn public_root", "public remote root tuple constructor"),
    ("pub(crate) fn public_with_causal_context", "public remote child tuple constructor"),
    ("pub(crate) fn daemon_system_root", "daemon-system remote root tuple constructor"),
    ("InvocationCausalContext::public_root_derived()", "shared public root causal policy"),
    ("InvocationCausalContext::daemon_system_root()", "shared daemon-system causal policy"),
):
    if token not in remote_text:
        violations.append(f"{remote}: missing {detail}: {token}")

for path in (
    "src/cli/commands/invoke.rs",
    "src/cli/daemon_client/remote_system_ability.rs",
):
    production = production_prefix(read(path))
    for token in (
        "RemoteInvocationRequest::new",
        "axon_sdk::invocation::fresh_nonce()",
        "axon_sdk::invocation::CausalContext::None",
    ):
        if token in production:
            violations.append(
                f"{path}: public/daemon remote ingress may not use anonymous tuple default: {token}"
            )

local_loopback = "src/support/platform/local_daemon_grpc.rs"
local_text = read(local_loopback)
for token, detail in (
    ("struct LocalDaemonLoopbackTuplePlan", "inspectable local loopback tuple plan"),
    ("enum LocalDaemonLoopbackDerivationPolicy", "named local loopback derivation state"),
    ("targeted_explicit_causal", "local explicit-causal tuple constructor"),
    ("local_daemon_loopback_invocation_from_tuple_plan", "single local loopback lowering helper"),
):
    if token not in local_text:
        violations.append(f"{local_loopback}: missing {detail}: {token}")

local_production = production_prefix(local_text)
for token in (
    "LocalDaemonSubjectPolicy",
    "local_daemon_loopback_invocation_from_subject_policy",
):
    if token in local_production:
        violations.append(f"{local_loopback}: obsolete local loopback subject-only policy remains: {token}")

for match in re.finditer(r"LocalDaemonLoopbackInvocation::from_target\s*\(", local_production):
    if enclosing_function_name(local_production, match.start()) != "into_invocation":
        violations.append(
            f"{local_loopback}:{local_production.count(chr(10), 0, match.start()) + 1}: local loopback ingress must lower through LocalDaemonLoopbackTuplePlan"
        )

for match in re.finditer(r"InvocationDerivationPolicy::FreshRoot", local_production):
    if enclosing_function_name(local_production, match.start()) != "as_axon":
        violations.append(
            f"{local_loopback}:{local_production.count(chr(10), 0, match.start()) + 1}: anonymous FreshRoot derivation must be owned by LocalDaemonLoopbackDerivationPolicy"
        )

local_invoker = "src/daemon/invocation/dispatch/local_runtime_invoker.rs"
for token, detail in (
    ("SystemInvocationIssuer::request_for_descriptor_ref", "local daemon-system descriptor-bound issuer"),
    ("invoke_descriptor_bound_request_async(request)", "local RPC descriptor-bound LocalRuntime call"),
    ("invoke_descriptor_bound_stream_request_async(request)", "local stream descriptor-bound LocalRuntime call"),
    ("invoke_descriptor_bound_bidi_request_async(request)", "local bidi descriptor-bound LocalRuntime call"),
    ("resolved_subject_ura(callee_ura)", "single subject resolution point"),
    ("resolved_causal_context()", "single causal context resolution point"),
):
    require(local_invoker, token, detail)

runtime = "src/daemon/invocation/dispatch/daemon_route_runtime.rs"
for token, detail in (
    ("pub(crate) struct DaemonRouteRuntimeAdapter", "exact route adapter owner"),
    ("pub(crate) async fn register_for_owners", "unary exact route registration API"),
    ("for route in DaemonUnaryRoute::ALL.iter().copied()", "complete unary route inventory registration"),
    ("pub(crate) async fn register_streams", "stream exact route registration API"),
    ("for route in DaemonStreamRoute::ALL.iter().copied()", "complete stream route inventory registration"),
    ("pub(crate) async fn register_bidis", "bidi exact route registration API"),
    ("for route in DaemonBidiRoute::ALL.iter().copied()", "complete bidi route inventory registration"),
    ("self.runtime.register_many(registrations).await", "atomic LocalRuntime route install"),
    ("dispatch_rpc_admitted", "exact unary adapter enters admitted LocalRuntime dispatch"),
    ("open_stream_admitted", "exact stream adapter enters admitted LocalRuntime dispatch"),
    ("open_bidi_external_signed", "exact bidi adapter enters admitted LocalRuntime dispatch"),
):
    require(runtime, token, detail)

shim = "src/daemon/axon_bridge/dispatch_shim.rs"
for token, detail in (
    ("descriptor_bound_from_wire_parts", "wire tuple reassembly through Axon descriptor-bound parser"),
    ("LocalRuntimeRequestFactory::request_for", "external signed request factory"),
    ("SystemInvocationIssuer::request_for_complete_envelope", "trusted-local system issuer"),
    ("invoke_descriptor_bound_request_async(prepared.request)", "RPC LocalRuntime descriptor-bound dispatch"),
    ("invoke_descriptor_bound_stream_request_async(prepared.request)", "stream LocalRuntime descriptor-bound dispatch"),
    ("invoke_descriptor_bound_bidi_request_async(prepared.request)", "bidi LocalRuntime descriptor-bound dispatch"),
):
    require(shim, token, detail)

factory = "src/daemon/axon_bridge/local_runtime_request.rs"
for token, detail in (
    ("enum LocalRuntimeIngress", "typed LocalRuntime ingress classification"),
    ("DescriptorBoundInvocationRequest::externally_signed", "Axon public descriptor-bound constructor"),
    ("pub(crate) struct SystemInvocationIssuer", "named daemon system issuer"),
    ("request_for_descriptor_ref", "complete descriptor-ref system request"),
    ("request_for_complete_envelope", "already-complete system envelope request"),
    ("LOCAL_SYSTEM_AGENT_URA", "single local system caller authority"),
):
    require(factory, token, detail)

service = "src/daemon/invocation/dispatch/daemon_invocation_service.rs"
for token, detail in (
    ("pub(crate) enum DaemonUnaryRoute", "typed unary route inventory"),
    ("pub(crate) const DAEMON_INVOCATION_UNARY_ROUTES", "unary inventory export"),
    ("pub(crate) enum DaemonStreamRoute", "typed stream route inventory"),
    ("pub(crate) const DAEMON_INVOCATION_STREAM_ROUTES", "stream inventory export"),
    ("pub(crate) enum DaemonBidiRoute", "typed bidi route inventory"),
    ("pub(crate) const DAEMON_INVOCATION_BIDI_ROUTES", "bidi inventory export"),
    ("register_daemon_unary_routes_for_owners", "unary route registration lifecycle"),
    ("register_daemon_stream_routes", "stream route registration lifecycle"),
    ("register_daemon_bidi_routes", "bidi route registration lifecycle"),
    (".dispatch_daemon_route_runtime(route, request, ingress)", "unary route adapter dispatch"),
    ("streams.dispatch_daemon_route_runtime(route, &inner).await", "stream route adapter dispatch"),
    (".dispatch_daemon_route_runtime(route, envelope_open, up)", "bidi route adapter dispatch"),
):
    require(service, token, detail)

if violations:
    print("\n".join(violations))
PY
    )"
    if [[ -n "$bad_route_tuple_ownership" ]]; then
        record_violation "RF-7/RF-8 route tuple ownership gate failed" \
            "$bad_route_tuple_ownership
Ability routes must enter descriptor-bound LocalRuntime through the typed route inventories, and production public ingress must not patch missing subject or causal_context after target construction."
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
