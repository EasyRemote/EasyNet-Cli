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
    && require_file "src/daemon/axon_bridge/descriptor_bound_dispatch.rs" \
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

def item_end(lines: list[str], start: int) -> int:
    depth = 0
    seen_open = False
    for index in range(start, len(lines)):
        line = lines[index]
        depth += line.count("{")
        if "{" in line:
            seen_open = True
        depth -= line.count("}")
        if seen_open and depth <= 0:
            return index + 1
        if not seen_open and line.rstrip().endswith(";"):
            return index + 1
    return len(lines)

def production_source(text: str) -> str:
    lines = text.splitlines(keepends=True)
    kept: list[str] = []
    index = 0
    pending_test_cfg = False
    while index < len(lines):
        line = lines[index]
        if re.search(r"#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]", line):
            pending_test_cfg = True
            kept.append("\n")
            index += 1
            continue
        if pending_test_cfg:
            if line.strip().startswith("#["):
                kept.append("\n")
                index += 1
                continue
            if line.strip() == "":
                kept.append(line)
                index += 1
                continue
            end = item_end(lines, index)
            kept.extend("\n" for _ in range(index, end))
            index = end
            pending_test_cfg = False
            continue
        kept.append(line)
        index += 1
    return "".join(kept)

def enclosing_function_name(text: str, offset: int):
    found = None
    for match in re.finditer(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*<[^>{}]*>)?\s*\(", text[:offset]):
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
    ("InvocationCausalContext::daemon_system_root()", "named system causal policy"),
    ("pub struct SystemInvocationTargetIssuer", "canonical daemon-system target issuer"),
    ("pub fn local_root", "daemon-system local target issuer"),
    ("pub fn local_root_for_subject", "daemon-system local subject target issuer"),
    ("pub fn remote_root", "daemon-system remote target issuer"),
    ("pub struct PublicInvocationTargetIssuer", "canonical public-ingress target issuer"),
    ("pub fn local_explicit_tuple", "public-ingress local target issuer"),
    ("InvocationSubject::explicit(subject)", "public subject preservation"),
    ("InvocationCausalContext::explicit(causal_context)", "public causal context preservation"),
):
    if token not in target_text:
        violations.append(f"{target}: missing {detail}: {token}")

for token in (
    "pub fn with_subject(",
    "pub fn with_causal_context(",
    "local_daemon_system_with_subject",
):
    if token in production_source(target_text):
        violations.append(
            f"{target}: retired post-resolution subject vocabulary remains exposed: {token}"
        )

for token in (
    "PublicRootDerived",
    "public_root_derived",
):
    if token in production_source(target_text):
        violations.append(f"{target}: public ingress root derivation must not exist: {token}")

for path in sorted((root / "src").rglob("*.rs")):
    rel = path.relative_to(root).as_posix()
    if path.name in {"real_invoke_tests.rs", "tests.rs"} or path.name.endswith("_tests.rs"):
        continue
    if any(part.endswith("_tests") for part in path.relative_to(root).parts):
        continue
    if "/tests/" in f"/{rel}/":
        continue
    production = production_source(path.read_text(encoding="utf-8", errors="replace"))
    for match in re.finditer(r"\.with_(?:subject|causal_context)\s*\(", production):
        violations.append(
            f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: production InvocationTarget tuple patching is forbidden"
        )
    if rel != "src/daemon/invocation/routing/target.rs":
        for match in re.finditer(
            r"InvocationTarget::(?:local_daemon_system(?:_for_subject)?|remote_daemon_system)\s*\(",
            production,
        ):
            violations.append(
                f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: daemon-system InvocationTarget construction must use SystemInvocationTargetIssuer"
            )
        for match in re.finditer(r"InvocationTarget::local_explicit_tuple\s*\(", production):
            violations.append(
                f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: public explicit InvocationTarget construction must use PublicInvocationTargetIssuer"
            )
    if rel != "src/daemon/invocation/routing/remote_invoke.rs":
        for match in re.finditer(r"RemoteInvocationRequest::new\s*\(", production):
            violations.append(
                f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: production remote invocation ingress must use RemoteInvocationTuplePlan"
            )

    for match in re.finditer(r"\bfresh_nonce\s*\(", production):
        function = enclosing_function_name(production, match.start())
        if (rel, function) not in {
            ("src/support/platform/local_invoke.rs", "root_context"),
            ("src/daemon/invocation/routing/remote_invoke.rs", "target_owned_root_plan"),
            ("src/daemon/invocation/routing/remote_invoke.rs", "catalogue_read_plan"),
            ("src/daemon/invocation/routing/remote_invoke.rs", "child_plan"),
        }:
            violations.append(
                f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: fresh nonce minting must be owned by a named invocation issuer"
            )

    for match in re.finditer(r"InvocationDerivationPolicy::FreshRoot", production):
        function = enclosing_function_name(production, match.start())
        if (rel, function) not in {
            ("src/daemon/invocation/dispatch/invocation_wire.rs", "fresh_root"),
            ("src/support/platform/local_daemon_grpc.rs", "as_axon"),
        }:
            violations.append(
                f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: FreshRoot policy must be issued by RootInvocationDerivationIssuer or loopback lowering"
            )

    for match in re.finditer(r"LocalSystemInvocationContext::new\s*\(", production):
        function = enclosing_function_name(production, match.start())
        if (rel, function) != ("src/support/platform/local_invoke.rs", "root_context"):
            violations.append(
                f"{rel}:{production.count(chr(10), 0, match.start()) + 1}: local system invocation context must be issued by LocalSystemInvocationIssuer"
            )

    for token in (
        "invoke_local_ability_target_explicit_root_timeout",
        "invoke_local_ability_target_stream_explicit_root",
        "invoke_local_ability_target_bidi_json_frames_explicit_root",
        "invoke_local_ability_target_with_invocation_meta",
        "invoke_local_ability_target_with_hosted_agent_delegation",
        "invoke_local_ability_target_with_subject_timeout",
        "invoke_local_ability_target_stream_with_subject",
        "invoke_local_ability_target_bidi_json_frames_with_subject",
        "invoke_local_daemon_ability_targeted_timeout",
        "invoke_local_daemon_ability_targeted_stream_with_subject",
        "invoke_local_daemon_ability_targeted_bidi_json_frames_with_subject",
    ):
        offset = production.find(token)
        if offset >= 0:
            violations.append(
                f"{rel}:{production.count(chr(10), 0, offset) + 1}: public targeted local invocation must use explicit tuple helpers or LocalDaemonSystemAbilityIssuer"
            )

remote = "src/daemon/invocation/routing/remote_invoke.rs"
remote_text = read(remote)
for token, detail in (
    ("pub(crate) enum RemoteInvocationSubject", "named remote subject derivation state"),
    ("CallerDeclared", "public caller-declared remote subject provenance"),
    ("DaemonTargetOwned", "daemon target-owned remote subject provenance"),
    ("no public subject omission, callee substitution, or descriptor substitution", "remote subject omission exclusion"),
    ("pub(crate) enum RemoteInvocationNonce", "explicit remote nonce state"),
    ("RemoteInvocationNonce::Explicit", "public explicit nonce state"),
    ("pub(crate) struct RemoteInvocationTuplePlan", "inspectable remote tuple plan"),
    ("pub(crate) fn public_explicit", "public explicit tuple constructor"),
    ("pub(crate) struct RemoteSystemInvocationIssuer", "daemon-system remote issuer"),
    ("pub(crate) fn target_owned_root_plan", "daemon-system target-owned remote root issuer constructor"),
    ("pub(crate) struct RemoteChildInvocationIssuer", "runtime child remote issuer"),
    ("pub(crate) fn child_plan", "runtime child remote issuer constructor"),
    ("InvocationCausalContext::daemon_system_root()", "shared daemon-system causal policy"),
):
    if token not in remote_text:
        violations.append(f"{remote}: missing {detail}: {token}")

for token in (
    "PairedOwnerDerived",
    "pub(crate) fn public_root",
    "pub(crate) fn public_with_causal_context",
    "pub(crate) fn product_policy_with_causal_context",
    "pub(crate) fn daemon_system_root",
    "InvocationCausalContext::public_root_derived()",
):
    if token in production_source(remote_text):
        violations.append(f"{remote}: public remote ingress default remains: {token}")

for path in (
    "src/cli/commands/invoke.rs",
    "src/cli/daemon_client/remote_system_ability.rs",
):
    production = production_source(read(path))
    for token in (
        "RemoteInvocationRequest::new",
        "axon_sdk::invocation::fresh_nonce()",
        "axon_sdk::invocation::CausalContext::None",
    ):
        if token in production:
            violations.append(
                f"{path}: public/daemon remote ingress may not use anonymous tuple default: {token}"
            )

local_daemon_system = "src/support/platform/local_daemon_grpc.rs"
local_text = read(local_daemon_system)
for token, detail in (
    ("struct LocalDaemonSystemTuplePlan", "inspectable local daemon-system tuple plan"),
    ("enum LocalDaemonSystemDerivationPolicy", "named local daemon-system derivation state"),
    ("targeted_explicit_causal", "local explicit-causal tuple constructor"),
    ("local_daemon_system_invocation_from_tuple_plan", "single local daemon-system lowering helper"),
):
    if token not in local_text:
        violations.append(f"{local_daemon_system}: missing {detail}: {token}")

local_invoke = "src/support/platform/local_invoke.rs"
local_invoke_text = read(local_invoke)
for token, detail in (
    ("pub struct LocalDaemonSystemAbilityIssuer", "named local daemon-system ability issuer"),
    ("invoke_root_for_subject", "daemon-system local root subject issuer"),
    ("invoke_root_for_subject_timeout", "daemon-system local root subject timeout issuer"),
    ("invoke_target_root_timeout", "daemon-system unary target issuer"),
    ("stream_target_root", "daemon-system stream target issuer"),
    ("invoke_local_target_explicit_root_timeout", "public local unary explicit tuple helper"),
    ("invoke_local_target_stream_explicit_root", "public local stream explicit tuple helper"),
    ("invoke_local_target_bidi_json_frames_explicit_root", "public local bidi explicit tuple helper"),
    ("invoke_local_target_with_invocation_meta", "public local metadata-verifying tuple helper"),
    ("invoke_local_target_with_hosted_agent_delegation", "public local hosted-agent delegation tuple helper"),
):
    if token not in local_invoke_text:
        violations.append(f"{local_invoke}: missing {detail}: {token}")

local_production = production_source(local_text)
for token in (
    "LocalDaemonSubjectPolicy",
    "local_daemon_system_invocation_from_subject_policy",
    "CallerDeclaredSubject",
    "targeted_root_with_declared_subject",
    "explicit_or_caller_declared",
):
    if token in local_production:
        violations.append(f"{local_daemon_system}: obsolete local daemon-system subject-only policy remains: {token}")

for match in re.finditer(
    r"invoke_local_daemon_system_ability_targeted_(?:root_timeout|stream_root)\s*\([^)]*subject\s*:\s*Option\s*<\s*String\s*>",
    local_production,
    re.S,
):
    violations.append(
        f"{local_daemon_system}:{local_production.count(chr(10), 0, match.start()) + 1}: targeted daemon-system local invoke must require explicit subject_ura"
    )

local_invoke_production = production_source(local_invoke_text)
for token in (
    "pub fn invoke_local_ability_with_subject",
    "pub fn invoke_local_ability_with_subject_timeout",
):
    if token in local_invoke_production:
        violations.append(
            f"{local_invoke}: generic subject-bearing local invoke facade is retired: {token}"
        )

for token in (
    "struct LocalDaemonAbilityClient",
    "local_root_with_subject",
    "targeted_root_with_subject",
    "pub(crate) fn invoke_local_daemon_ability_with_subject",
    "pub(crate) fn invoke_local_daemon_ability_with_subject_timeout",
    "fn invoke_with_subject(",
    "fn invoke_with_subject_and_timeout(",
):
    if token in local_production:
        violations.append(
            f"{local_daemon_system}: generic subject-bearing transport facade is retired: {token}"
        )

for path, tokens in (
    (
        "tests/resolve_before_invoke_e2e.rs",
        (
            "fn invoke_with_subject(",
        ),
    ),
    (
        "src/daemon/ability/builtins/governance/teach.rs",
        (
            "caller_env_with_subject",
        ),
    ),
    (
        "src/daemon/keyring/abilities.rs",
        (
            "handle_with_subject_and_signing_key",
        ),
    ),
):
    source = read(path)
    for token in tokens:
        if token in source:
            violations.append(
                f"{path}: non-SDK subject helper must name explicit tuple/bound subject semantics: {token}"
            )

for match in re.finditer(
    r"(?:invoke_target_root_timeout|stream_target_root)\s*\([^)]*subject\s*:\s*Option\s*<\s*String\s*>",
    local_invoke_production,
    re.S,
):
    violations.append(
        f"{local_invoke}:{local_invoke_production.count(chr(10), 0, match.start()) + 1}: LocalDaemonSystemAbilityIssuer must not accept optional subject fallback"
    )

for match in re.finditer(r"LocalDaemonSystemInvocation::from_target\s*\(", local_production):
    if enclosing_function_name(local_production, match.start()) != "into_invocation":
        violations.append(
            f"{local_daemon_system}:{local_production.count(chr(10), 0, match.start()) + 1}: local daemon-system ingress must lower through LocalDaemonSystemTuplePlan"
        )

for match in re.finditer(r"InvocationDerivationPolicy::FreshRoot", local_production):
    if enclosing_function_name(local_production, match.start()) != "as_axon":
        violations.append(
            f"{local_daemon_system}:{local_production.count(chr(10), 0, match.start()) + 1}: anonymous FreshRoot derivation must be owned by LocalDaemonSystemDerivationPolicy"
        )

bidi_dispatcher = "src/daemon/invocation/bidi/bidi_dispatcher.rs"
bidi_text = read(bidi_dispatcher)
for token, detail in (
    ("enum SessionControlRequestKind", "closed JSON session-control inventory"),
    ("struct SessionControlRequest", "typed JSON session-control request"),
    ("struct SessionControlLifecycle", "explicit JSON session-control lifecycle"),
    ("SessionControlScheduling", "explicit JSON session-control scheduling state"),
    ("session_control_kind_for_hub", "hub-owned session-control classifier"),
    ("session_control_lifecycle_from_wire", "single JSON request classification boundary"),
    ("dispatch_canonical_session_invoke", "canonical ReverseDispatchCall invoke path"),
    ("product invocations must use canonical ReverseDispatchCall", "JSON request product-bypass rejection"),
):
    if token not in bidi_text:
        violations.append(f"{bidi_dispatcher}: missing {detail}: {token}")

bidi_production = production_source(bidi_text)
for token in (
    "dispatch_checked_session_request",
    "dispatch_session_request_named_for_caller",
    "dispatch_session_request_from_session",
    "is_inline_session_request",
):
    if token in bidi_production:
        violations.append(
            f"{bidi_dispatcher}: obsolete procedural JSON session request dispatcher remains: {token}"
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

dispatch_adapter = "src/daemon/axon_bridge/descriptor_bound_dispatch.rs"
for token, detail in (
    ("descriptor_bound_from_wire_parts", "wire tuple reassembly through Axon descriptor-bound parser"),
    ("LocalRuntimeRequestFactory::request_for", "external signed request factory"),
    ("SystemInvocationIssuer::request_for_complete_envelope", "trusted-local system issuer"),
    ("invoke_descriptor_bound_request_async(prepared.request)", "RPC LocalRuntime descriptor-bound dispatch"),
    ("invoke_descriptor_bound_stream_request_async(prepared.request)", "stream LocalRuntime descriptor-bound dispatch"),
    ("invoke_descriptor_bound_bidi_request_async(prepared.request)", "bidi LocalRuntime descriptor-bound dispatch"),
):
    require(dispatch_adapter, token, detail)

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
