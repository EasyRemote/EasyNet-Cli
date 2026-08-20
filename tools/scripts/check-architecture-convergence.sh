#!/usr/bin/env bash
set -euo pipefail

python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/generate-runtime-governance-routes.py" --check

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SELF_DIR/../.." && pwd)"
AXON_ROOT="${EASYNET_AXON_ROOT:-$ROOT/../EasyNet-Axon}"

usage() {
  printf 'usage: %s [--root EASYNET_CLI_ROOT] [--axon-root EASYNET_AXON_ROOT]\n' "$0" >&2
}

while (($#)); do
  case "$1" in
    --root)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      ROOT="$2"
      shift 2
      ;;
    --axon-root)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      AXON_ROOT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

python3 - "$ROOT" "$AXON_ROOT" <<'PY'
from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path


cli_root = Path(sys.argv[1]).resolve()
axon_root = Path(sys.argv[2]).resolve()

required_roots = (
    cli_root / "src/eal/interpreter",
    cli_root / "src/daemon/ability/builtins/automation",
    cli_root / "src/daemon/ability/builtins/agents/chat.rs",
    cli_root / "sdk",
    axon_root / "core/runtime-rs/src",
    axon_root / "sdk/rust/src",
)
missing = [str(path) for path in required_roots if not path.exists()]
if missing:
    for path in missing:
        print(f"architecture-convergence: missing production root: {path}", file=sys.stderr)
    raise SystemExit(2)


@dataclass(frozen=True, order=True)
class Violation:
    rule: str
    path: str
    line: int
    detail: str


violations: set[Violation] = set()
ignored_parts = {
    ".git",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "target",
    "tests",
    "vendor",
}
source_suffixes = {".rs", ".go", ".py", ".js", ".ts", ".java", ".swift"}

PROFILE_PROJECTION_RULES = {
    "R106_TERMINAL_SYSTEM_AGENT_OWNER": (
        "TERMINAL_SYSTEM_AGENT_ID",
        "OwnerKind::terminal_system",
    ),
    "R107_LOCOMOTION_SYSTEM_AGENT_OWNER": (
        "LOCOMOTION_SYSTEM_AGENT_ID",
        "OwnerKind::locomotion_system",
    ),
    "R108_SKILL_SYSTEM_AGENT_OWNER": (
        "SKILL_MANAGEMENT_SYSTEM_AGENT_ID",
        "OwnerKind::skill_management_system",
    ),
    "R109_CONTEXT_SYSTEM_AGENT_OWNER": (
        "CONTEXT_SYSTEM_AGENT_ID",
        "OwnerKind::context_system",
    ),
    "R110_MEDIA_SYSTEM_AGENT_OWNER": (
        "MEDIA_SYSTEM_AGENT_ID",
        "OwnerKind::media_system",
    ),
    "R111_PLUGIN_SYSTEM_AGENT_OWNER": (
        "PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID",
        "OwnerKind::plugin_management_system",
    ),
    "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY": (
        "KEYRING_MANAGEMENT_SYSTEM_AGENT_ID",
        "OwnerKind::keyring_management_system",
    ),
    "R115_AUTOMATION_SYSTEM_AGENT_OWNER": (
        "AUTOMATION_SYSTEM_AGENT_ID",
        "OwnerKind::automation_system",
    ),
    "R116_SESSION_SYSTEM_AGENT_OWNER": (
        "SESSION_SYSTEM_AGENT_ID",
        "OwnerKind::session_system",
    ),
    "R117_RUNTIME_GOVERNANCE_SYSTEM_AGENT_OWNER": (
        "RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID",
        "OwnerKind::runtime_governance_system",
    ),
    "R118_ABILITY_MANAGEMENT_SYSTEM_AGENT_OWNER": (
        "ABILITY_MANAGEMENT_SYSTEM_AGENT_ID",
        "OwnerKind::ability_management_system",
    ),
    "R119_OPENAI_COMPAT_SYSTEM_AGENT_OWNER": (
        "OPENAI_COMPAT_SYSTEM_AGENT_ID",
        "OwnerKind::openai_compat_system",
    ),
    "R120_API_KEY_SYSTEM_AGENT_OWNER": (
        "API_KEY_MANAGEMENT_SYSTEM_AGENT_ID",
        "OwnerKind::api_key_management_system",
    ),
    "R121_A2A_INTEGRATION_SYSTEM_AGENT_OWNER": (
        "A2A_INTEGRATION_SYSTEM_AGENT_ID",
        "OwnerKind::a2a_integration_system",
    ),
    "R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER": (
        "RUNTIME_HEALTH_SYSTEM_AGENT_ID",
        "OwnerKind::runtime_health_system",
    ),
    "R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER": (
        "NODE_MANAGEMENT_SYSTEM_AGENT_ID",
        "OwnerKind::node_management_system",
    ),
    "R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER": (
        "RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID",
        "OwnerKind::runtime_introspection_system",
    ),
    "R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER": (
        "DESCRIPTOR_TRANSFER_SYSTEM_AGENT_ID",
        "OwnerKind::descriptor_transfer_system",
    ),
}


def display(path: Path) -> str:
    for label, root in (("EasyNet-Cli", cli_root), ("EasyNet-Axon", axon_root)):
        try:
            return f"{label}/{path.relative_to(root)}"
        except ValueError:
            pass
    return str(path)


def profiles_registry_satisfies(rule: str, path: Path) -> bool:
    if rule not in PROFILE_PROJECTION_RULES:
        return False
    if path.as_posix().split("/")[-5:] != [
        "daemon",
        "ability",
        "catalog",
        "profiles",
        "mod.rs",
    ]:
        return False
    text = path.read_text(encoding="utf-8", errors="replace")
    system_agent_id, owner_constructor = PROFILE_PROJECTION_RULES[rule]
    return all(
        token in text
        for token in (
            "SYSTEM_AGENT_DESCRIPTOR_PROJECTIONS",
            "fn system_agent_descriptor_projections_for_device(",
            "system_agent_descriptor_projections_for_device(device_ura)",
            "system_agent_descriptors_for_device(",
            system_agent_id,
            owner_constructor,
        )
    )


def add(rule: str, path: Path, line: int, detail: str) -> None:
    if (
        path.name == "mod.rs"
        and (
            "host profile aggregation must include" in detail
            or "SystemAgent descriptor projection must be explicit" in detail
        )
        and profiles_registry_satisfies(rule, path)
    ):
        return
    violations.add(Violation(rule, display(path), line, detail))


def production_files(root: Path, suffixes: set[str] = source_suffixes) -> list[Path]:
    if root.is_file():
        return [root] if root.suffix in suffixes else []
    result: list[Path] = []
    for path in root.rglob("*"):
        if not path.is_file() or path.suffix not in suffixes:
            continue
        relative = path.relative_to(root)
        if any(
            part in ignored_parts
            or part.lower().endswith("_tests")
            or part.lower() in {"internal/axonpb", "_axon_pb"}
            for part in relative.parts
        ):
            continue
        relative_text = relative.as_posix()
        if "/internal/axonpb/" in f"/{relative_text}/" or "/_axon_pb/" in f"/{relative_text}/":
            continue
        name = path.name.lower()
        if (
            name.endswith("_test.go")
            or name.endswith("_tests.rs")
            or name == "tests.rs"
            or name.startswith("test_")
            or ".test." in name
        ):
            continue
        result.append(path)
    return sorted(result)


def strip_comments_and_tests(text: str, suffix: str) -> str:
    # Keep line count stable so diagnostics point at the production source.
    if suffix == ".py":
        lines = [line.split("#", 1)[0] for line in text.splitlines()]
        return "\n".join(lines)
    text = re.sub(r"/\*.*?\*/", lambda m: "\n" * m.group(0).count("\n"), text, flags=re.S)
    lines = [line.split("//", 1)[0] for line in text.splitlines()]
    if suffix != ".rs":
        return "\n".join(lines)

    # Remove each cfg(test)-annotated Rust item, preserving all production
    # items that may follow a test-only import/helper near the top of a file.
    result = list(lines)
    index = 0
    cfg_test = re.compile(
        r"\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]"
    )
    while index < len(lines):
        if not cfg_test.match(lines[index]):
            index += 1
            continue
        start = index
        index += 1
        saw_brace = False
        depth = 0
        while index < len(lines):
            structural = re.sub(r'"(?:\\.|[^"\\])*"', '""', lines[index])
            if "{" in structural:
                saw_brace = True
            depth += structural.count("{") - structural.count("}")
            index += 1
            if saw_brace and depth <= 0:
                break
            if not saw_brace and ";" in structural:
                break
        for skipped in range(start, index):
            result[skipped] = ""
    return "\n".join(result)


def source(path: Path) -> str:
    return strip_comments_and_tests(
        path.read_text(encoding="utf-8", errors="replace"), path.suffix
    )


def line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


# Rule 0a: the consent ability family is the governed public catalog surface.
# The execution broker may keep permission-domain nouns for local approval
# decisions, but catalog assembly/metadata/profiles must not preserve the old
# permission_ability compatibility alias or stale follow-up rename language.
consent_catalog_files = (
    cli_root / "src/daemon/ability/catalog/build.rs",
    cli_root / "src/daemon/ability/catalog/catalog_metadata.rs",
    cli_root / "src/daemon/ability/catalog/profiles/consent.rs",
)
for consent_catalog_file in consent_catalog_files:
    if not consent_catalog_file.exists():
        continue
    raw_text = consent_catalog_file.read_text(encoding="utf-8", errors="replace")
    production_text = source(consent_catalog_file)
    for token in (
        "consent as permission_ability",
        "permission_ability::",
    ):
        pos = production_text.find(token)
        if pos >= 0:
            add(
                "R0A_CONSENT_CATALOG_NAMING_CUTOVER",
                consent_catalog_file,
                line_number(production_text, pos),
                f"catalog-owned consent surface must not use retired permission ability alias `{token}`",
            )
    for token in (
        "agents/permission_ability.rs",
        "renames to\n//! consent_ability.rs in a follow-up cleanup",
    ):
        pos = raw_text.find(token)
        if pos >= 0:
            add(
                "R0A_CONSENT_CATALOG_NAMING_CUTOVER",
                consent_catalog_file,
                line_number(raw_text, pos),
                f"consent profile must not preserve stale permission ability migration language `{token}`",
            )


# Rule 0b: Device authority-row projection is not permission to open Device
# product state. Combined Device+RealmAuthority descriptor inventories may
# need Device rows without owning the local ability deployment store, MCP
# config, plugin package state, or hosted-agent replay lifecycle.
ability_dispatch = cli_root / "src/daemon/ability/dispatch.rs"
catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
if ability_dispatch.exists():
    text = source(ability_dispatch)
    if "fn owns_device_product_state(&self) -> bool" not in text:
        add(
            "R0B_DEVICE_PRODUCT_STATE_OWNER_FORK",
            ability_dispatch,
            1,
            "AbilityAuthorityContext must expose an explicit Device product-state ownership predicate",
        )
    if "DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots" not in text:
        add(
            "R0B_DEVICE_PRODUCT_STATE_OWNER_FORK",
            ability_dispatch,
            1,
            "Device product-state ownership must be bound to explicit hosted-agent authority inventory",
        )
if catalog_build.exists():
    text = source(catalog_build)
    requirements = (
        (
            "let owns_device_product_state = authority_context.owns_device_product_state();",
            "catalog assembly must derive Device product-state ownership from authority context",
        ),
        (
            "let stateful_device_runtime = owns_device_product_state && daemon_runtime_assembly;",
            "catalog assembly must name the stateful Device runtime transition explicitly",
        ),
        (
            "let plugin_registry_mode = match (owns_device_product_state, plugin_registry_mode)",
            "catalog assembly must not open daemon plugin package state from DeviceScoped descriptor inventories",
        ),
        (
            "if stateful_device_runtime {\n        ability_deployment_registrar_cell",
            "ability deployment registrar must be initialized only for stateful Device runtime assembly",
        ),
        (
            "let mcp_svc = if stateful_device_runtime",
            "MCP client config must be opened only for stateful Device runtime assembly",
        ),
        (
            "let reflection_plan = if stateful_device_runtime",
            "MCP reflection lifecycle must be planned only for stateful Device runtime assembly",
        ),
    )
    for token, detail in requirements:
        if token not in text:
            add("R0B_DEVICE_PRODUCT_STATE_OWNER_FORK", catalog_build, 1, detail)
    for token in (
        "hosts_device_authority && daemon_runtime_assembly",
        "hosts_device_authority && assembly_mode.replays_hosted_agent_runtime()",
    ):
        pos = text.find(token)
        if pos >= 0:
            add(
                "R0B_DEVICE_PRODUCT_STATE_OWNER_FORK",
                catalog_build,
                line_number(text, pos),
                f"stateful Device lifecycle must not be gated by authority-row presence alone: `{token}`",
            )


def rust_method_body(text: str, name: str) -> tuple[int, str] | None:
    match = re.search(
        rf"(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+{re.escape(name)}\s*\([^)]*\)\s*(?:->\s*[^\{{]+)?\{{",
        text,
    )
    if not match:
        return None
    index = match.end()
    depth = 1
    while index < len(text):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return match.start(), text[match.end():index]
        index += 1
    return match.start(), text[match.end():]


def rust_struct_body(text: str, name: str) -> tuple[int, str] | None:
    match = re.search(
        rf"(?:pub(?:\([^)]*\))?\s+)?struct\s+{re.escape(name)}(?:\s*<[^>]+>)?\s*\{{",
        text,
    )
    if not match:
        return None
    index = match.end()
    depth = 1
    while index < len(text):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return match.start(), text[match.end():index]
        index += 1
    return match.start(), text[match.end():]


def rust_enum_body(text: str, name: str) -> tuple[int, str] | None:
    match = re.search(
        rf"(?:pub(?:\([^)]*\))?\s+)?enum\s+{re.escape(name)}(?:\s*<[^>]+>)?\s*\{{",
        text,
    )
    if not match:
        return None
    index = match.end()
    depth = 1
    while index < len(text):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return match.start(), text[match.end():index]
        index += 1
    return match.start(), text[match.end():]


def brace_function_body(text: str, signature_pattern: str) -> tuple[int, str] | None:
    match = re.search(signature_pattern, text)
    if not match:
        return None
    brace = text.find("{", match.end())
    if brace < 0:
        return None
    index = brace + 1
    depth = 1
    while index < len(text):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return match.start(), text[brace + 1:index]
        index += 1
    return match.start(), text[brace + 1:]


def brace_block_from(text: str, start: int) -> str:
    brace = text.find("{", start)
    if brace < 0:
        return ""
    index = brace + 1
    depth = 1
    while index < len(text):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[brace + 1:index]
        index += 1
    return text[brace + 1:]


def python_function_body(text: str, name: str) -> tuple[int, str] | None:
    pattern = rf"^def\s+{re.escape(name)}\s*\("
    match = re.search(pattern, text, flags=re.M)
    if not match:
        return None
    next_top_level = re.search(r"^(?:def|class)\s+", text[match.end():], flags=re.M)
    end = match.end() + next_top_level.start() if next_top_level else len(text)
    return match.start(), text[match.start():end]


# Rule 1: product orchestration may enter through a registered Invocation
# handler, but child EAL/Mission/Chat calls must not execute handlers or
# implementation bindings directly.
families = {
    "EAL": production_files(cli_root / "src/eal/interpreter", {".rs"}),
    "Mission": production_files(
        cli_root / "src/daemon/ability/builtins/automation", {".rs"}
    ),
    "Chat": [cli_root / "src/daemon/ability/builtins/agents/chat.rs"],
}
family_anchors = {
    "EAL": re.compile(
        r"\bMissionInvocationGateway\b|"
        r"\.invoke_step\s*\("
    ),
    "Mission": re.compile(r"register_(?:rpc|stream|bidi)[A-Za-z0-9_]*\s*\("),
    "Chat": re.compile(r"register_(?:rpc|stream|bidi)[A-Za-z0-9_]*\s*\("),
}
for family, paths in families.items():
    combined = "\n".join(source(path) for path in paths)
    if not family_anchors[family].search(combined):
        add(
            "R1_MISSING_INVOCATION_ANCHOR",
            paths[0],
            1,
            f"{family} production path has no canonical Invocation entry/dispatch anchor",
        )

direct_call_patterns = (
    ("direct catalog handler dispatch", re.compile(r"\.invoke_(?:rpc|stream|bidi)_json\s*\(")),
    ("direct chat implementation dispatch", re.compile(r"\binvoke_direct_with_progress\s*\(")),
    (
        "direct executor dispatch",
        re.compile(r"\brun_(?:shell|http|eal|mcp)_exec\s*\("),
    ),
    (
        "direct agent driver dispatch",
        re.compile(r"\b(?:send_to_agent(?:_with_[A-Za-z0-9_]+)?|send_external(?:_with_[A-Za-z0-9_]+)?)\s*\("),
    ),
)

# Chat owns implementation bindings after Invocation admission. Calls inside
# its handler factory are not bypasses; callers reaching that implementation
# directly from EAL/Mission are.
for family in ("EAL", "Mission"):
    for path in families[family]:
        text = source(path)
        for label, pattern in direct_call_patterns:
            for match in pattern.finditer(text):
                prefix = text[max(0, match.start() - 48) : match.start()]
                if re.search(r"\bfn\s+$", prefix):
                    continue
                add(
                    "R1_INVOCATION_BYPASS",
                    path,
                    line_number(text, match.start()),
                    f"{family} uses {label}",
                )

mission_gateway = cli_root / "src/daemon/execution/mission/invocation_gateway.rs"
mission_gateway_text = source(mission_gateway)
if re.search(r"\bCatalogMissionInvocationGateway\b", mission_gateway_text):
    match = re.search(r"\bCatalogMissionInvocationGateway\b", mission_gateway_text)
    add(
        "R1_MISSION_CATALOG_GATEWAY_PRODUCTION",
        mission_gateway,
        line_number(mission_gateway_text, match.start()),
        "Mission direct catalog gateway must remain a cfg(test)-only seam",
    )


# Rule 2: the canonical SDK LocalRuntime owns terminal finalization. CLI and
# transport adapters may project finalized receipts, but may not mint them.
owner = axon_root / "sdk/rust/src/invocation/handle.rs"
owner_text = source(owner) if owner.exists() else ""
owner_contract = (
    re.search(r"struct\s+InvocationCore\b", owner_text),
    re.search(r"fn\s+emit_with\s*\(", owner_text),
    re.search(r"ExecutionTerminal::new\s*\(", owner_text),
    re.search(r"append_signed_receipt\s*\(", owner_text),
    re.search(r"fn\s+complete_runtime_finalization\s*\(", owner_text),
)
if not all(owner_contract):
    add(
        "R2_TERMINAL_OWNER_MISSING",
        owner,
        1,
        "InvocationCore must own proof emission and terminal side effects",
    )

axon_invocation_candidates = (
    axon_root / "core/runtime-rs/src/services/invocation",
    axon_root / "core/runtime-rs/src/services",
)
receipt_factory = axon_root / "sdk/rust/src/invocation/receipt_provider.rs"
terminal_scan_files = production_files(cli_root / "src/daemon/invocation", {".rs"})
for axon_invocation_root in axon_invocation_candidates:
    if axon_invocation_root.exists():
        terminal_scan_files += production_files(axon_invocation_root, {".rs"})
        break
writer_name = re.compile(
    r"\bfn\s+(?:build|emit|mint|write|persist|commit)_[A-Za-z0-9_]*terminal[A-Za-z0-9_]*receipt[A-Za-z0-9_]*\s*\("
)
terminal_literal = re.compile(r"(?:[A-Za-z0-9_:]+::)?InvocationReceipt[ \t]*\{")
terminal_state = re.compile(r"\b(?:Completed|Failed|TimedOut|Cancelled)\b")
for path in terminal_scan_files:
    text = source(path)
    if path not in {owner, receipt_factory}:
        for match in writer_name.finditer(text):
            add(
                "R2_TERMINAL_WRITER_OUTSIDE_OWNER",
                path,
                line_number(text, match.start()),
                "terminal receipt builder/emitter exists outside TerminalFinalizationService",
            )
    if path not in {owner, receipt_factory}:
        for match in terminal_literal.finditer(text):
            line_start = text.rfind("\n", 0, match.start()) + 1
            if "->" in text[line_start : match.start()]:
                continue
            window = text[match.start() : match.start() + 700]
            if terminal_state.search(window):
                add(
                    "R2_TERMINAL_WRITER_OUTSIDE_OWNER",
                    path,
                    line_number(text, match.start()),
                    "terminal InvocationReceipt is constructed outside the canonical owner",
                )
    if path != owner:
        for match in re.finditer(r"\.emit_terminal_receipt_from_admission\s*\(", text):
            add(
                "R2_TERMINAL_WRITER_OUTSIDE_OWNER",
                path,
                line_number(text, match.start()),
                "terminal receipt factory is called outside TerminalFinalizationService",
            )

# The daemon may query ledger projections but cannot own a second writable
# terminal-record path. Axon's LedgerSink is the sole production adapter that
# persists a finalized receipt chain.
daemon_invocation_files = production_files(cli_root / "src/daemon/invocation", {".rs"})
manual_ledger_writer = re.compile(
    r"\b(?:fn\s+record_unary_invocation|"
    r"InvocationLedgerRecordBuilder\b|"
    r"Arc\s*<\s*InvocationLedger\s*>|"
    r"\.ledger\s*\.put\s*\()"
)
for path in daemon_invocation_files:
    text = source(path)
    for match in manual_ledger_writer.finditer(text):
        add(
            "R2_LEDGER_WRITER_OUTSIDE_AXON_SINK",
            path,
            line_number(text, match.start()),
            "daemon owns writable ledger state or a manual unary terminal writer",
        )

# LedgerSink adapters must preserve descriptor/binding ownership. A sink
# resolver may fail fast when Axon's canonical URA helpers cannot derive a
# record or route; it must not silently remap an unowned invocation into a
# synthetic `_system` receipt owner.
runtime_factory = cli_root / "src/daemon/axon_bridge/runtime_factory.rs"
if runtime_factory.exists():
    text = source(runtime_factory)
    if "LedgerSink cannot derive invocation record URA from binding" not in text:
        add(
            "R65_LEDGER_SINK_SYSTEM_FALLBACK",
            runtime_factory,
            1,
            "LedgerSink invocation resolver must reject unowned bindings instead of writing a system fallback",
        )
    if "LedgerSink cannot derive ability URA from binding" not in text:
        add(
            "R65_LEDGER_SINK_SYSTEM_FALLBACK",
            runtime_factory,
            1,
            "LedgerSink route resolver must reject unowned bindings instead of writing a system fallback",
        )
    for pattern in (
        r"\binvocation_history_resource_ura\s*\(",
        r"\bhub_ability_ura\s*\(\s*\"_system\"",
        r"format!\s*\(\s*\"system\.\{ability_name\}\"",
    ):
        for match in re.finditer(pattern, text):
            add(
                "R65_LEDGER_SINK_SYSTEM_FALLBACK",
                runtime_factory,
                line_number(text, match.start()),
                "LedgerSink resolver must not synthesize a `_system` receipt owner fallback",
            )
    if "pub struct RuntimePersistenceConfig" not in text:
        add(
            "R103_RUNTIME_PERSISTENCE_EXPLICIT_DEPENDENCY",
            runtime_factory,
            1,
            "daemon runtime factory must expose an explicit runtime persistence dependency",
        )
    for token in (
        "crate::daemon::persistence::config::state_dir",
        "persistence::config::state_dir",
        "config::state_dir",
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R103_RUNTIME_PERSISTENCE_EXPLICIT_DEPENDENCY",
                runtime_factory,
                line_number(text, offset),
                "daemon runtime factory must not derive Axon replay persistence from global state_dir",
            )

# Rule 12: mission run lifecycle owns one aggregate writer. `meta.json` is a
# projection of MissionRunAggregate, not a mutable DTO that every caller may
# rewrite directly.
mission_orchestration = cli_root / "src/daemon/execution/mission/orchestration.rs"
if mission_orchestration.exists():
    text = source(mission_orchestration)
    aggregate_contract = (
        re.search(r"\bstruct\s+MissionRunAggregate\b", text),
        re.search(r"\bfn\s+apply_terminal\s*\(", text),
        re.search(r"\bfn\s+cancel\s*\(", text),
        re.search(r"\bfn\s+write_mission_meta\s*\(", text),
    )
    if not all(aggregate_contract):
        add(
            "R12_MISSION_RUN_AGGREGATE_MISSING",
            mission_orchestration,
            1,
            "mission run meta.json must be owned by MissionRunAggregate transitions",
        )
    for match in re.finditer(r"\bpub\s+fn\s+write_meta\s*\(", text):
        add(
            "R12_MISSION_RUN_META_WRITER_FORK",
            mission_orchestration,
            line_number(text, match.start()),
            "MissionRunDir must not expose direct meta.json writers",
        )
    direct_meta_write = re.compile(r"\bfs::write\s*\([^;\n]*meta\.json")
    for match in direct_meta_write.finditer(text):
        prefix = text[max(0, match.start() - 300) : match.start()]
        if re.search(r"\bfn\s+write_mission_meta\s*\([^{}]*$", prefix, re.S):
            continue
        add(
            "R12_MISSION_RUN_META_WRITER_FORK",
            mission_orchestration,
            line_number(text, match.start()),
            "mission meta.json writes must go through write_mission_meta",
        )

# Daemon/application policy cannot depend on CLI presentation or command
# orchestration. Test-only HomeGuard imports are removed by source().
for path in production_files(cli_root / "src/daemon", {".rs"}):
    text = source(path)
    for match in re.finditer(r"\bcrate::cli::", text):
        add(
            "R5_DAEMON_DEPENDS_ON_CLI",
            path,
            line_number(text, match.start()),
            "daemon production code depends on the CLI layer",
        )


# Product protocol projections belong to their downstream product provider.
# Declaring them in Axon makes the canonical runtime model depend on one
# consumer and recreates a second ownership center.
product_protocol_projection_types = {
    "RemoteDesktopBackendStatus",
    "RemoteDesktopContractError",
    "RemoteDesktopMediaBackendContract",
    "RemoteDesktopQualityTargets",
    "RemoteDesktopSessionState",
    "RemoteDesktopTransportKind",
    "VoiceCallState",
    "VoiceContractError",
    "VoiceEndReason",
    "VoiceEventType",
    "VoiceNetworkMetrics",
}
protocol_projection_decl = re.compile(
    r"^\s*(?:(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|type))\s+"
    r"([A-Za-z_][A-Za-z0-9_]*)",
    re.M,
)
protocol_projection_roots = [axon_root / "sdk/rust/src", axon_root / "core/runtime-rs/src"]
for projection_root in protocol_projection_roots:
    if not projection_root.exists():
        continue
    for path in production_files(projection_root, {".rs"}):
        text = source(path)
        for match in protocol_projection_decl.finditer(text):
            name = match.group(1)
            if name in product_protocol_projection_types:
                add(
                    "R6_PRODUCT_PROTOCOL_IN_AXON",
                    path,
                    line_number(text, match.start()),
                    f"canonical Axon runtime declares product protocol projection {name!r}",
                )


# Runtime facades must expose a product-neutral object model. Axon's Rust SDK
# owns generic runtime protocol grammar; product protocols stay downstream.
runtime_facade_files: list[Path] = []
for sdk_subroot in (
    cli_root / "sdk/go",
    cli_root / "sdk/python/easynet_sdk",
    cli_root / "sdk/node",
    cli_root / "sdk/java/src/main",
    cli_root / "sdk/swift/Sources",
):
    if sdk_subroot.exists():
        runtime_facade_files.extend(production_files(sdk_subroot))
runtime_facade_files = sorted(set(runtime_facade_files))

go_runtime_lifecycle = cli_root / "sdk/go/runtime_lifecycle.go"
if go_runtime_lifecycle.exists():
    text = source(go_runtime_lifecycle)
    forbidden_lifecycle_vocabulary = re.compile(
        r"daemon (?:discover|start|attach|status|open-runtime|open runtime|"
        r"stop|detach|transport|control|handle|invocation endpoint|"
        r"endpoints|status JSON|lifecycle state)|daemon control|"
        r"daemon handle|decode daemon|invalid daemon|runtime-ready daemon"
    )
    for match in forbidden_lifecycle_vocabulary.finditer(text):
        add(
            "R3_SDK_RUNTIME_LIFECYCLE_NEUTRALITY",
            go_runtime_lifecycle,
            line_number(text, match.start()),
            "Go SDK canonical runtime lifecycle must use runtime-host vocabulary; daemon binding belongs in provider/easynet",
        )
    for token in (
        "runtime host discover transport function is required",
        "runtime host detach transport function is required",
        "runtime lifecycle transport is required",
        "runtime host lifecycle is not initialized",
        "runtime invocation endpoint is not ready",
        "runtime control endpoint is ready but invocation endpoint is not ready",
        "decode runtime host endpoints JSON",
        "invalid runtime lifecycle state",
    ):
        if token not in text:
            add(
                "R3_SDK_RUNTIME_LIFECYCLE_NEUTRALITY",
                go_runtime_lifecycle,
                1,
                f"Go SDK canonical runtime lifecycle is missing neutral diagnostic {token!r}",
            )

descriptor_catalog_scope_files = {
    cli_root / "src/daemon/ability/builtins/governance/meta.rs": (
        '"owner_ura"',
        '"ability_ura"',
    ),
    cli_root / "src/cli/daemon_client/ability_catalog.rs": (
        "AbilityCatalogQuery::new(owner_ura, ability_ura)",
        "self.inner.owner_ura()",
        "self.inner.ability_ura()",
    ),
    cli_root / "sdk/go/ability_descriptor.go": (
        'args["owner_ura"] = ownerURA',
        'args["ability_ura"] = abilityURA',
    ),
    cli_root / "sdk/python/easynet_sdk/ability_descriptor.py": (
        'args["owner_ura"] = request.owner_ura.strip()',
        'args["ability_ura"] = request.ability_ura.strip()',
    ),
}
for path, required_tokens in descriptor_catalog_scope_files.items():
    if not path.exists():
        continue
    text = source(path)
    production_text = text.split("\n#[cfg(test)]", 1)[0].split("\nmod tests {", 1)[0]
    for token in required_tokens:
        if token not in production_text:
            add(
                "R3_RUNTIME_DESCRIPTOR_CATALOG_SCOPE",
                path,
                1,
                f"runtime descriptor catalog scope must retain canonical field {token}",
            )
    for token in (
        'args["agent_ura"]',
        'args["subject_ura"]',
        '"agent_ura".to_string()',
        '"subject_ura".to_string()',
        '"scope" | "agent_ura" | "subject_ura"',
        "AbilitySubjectScope",
        "merge_owner_scope(",
    ):
        index = production_text.find(token)
        if index >= 0:
            add(
                "R3_RUNTIME_DESCRIPTOR_CATALOG_SCOPE",
                path,
                line_number(text, index),
                "runtime descriptor catalog scope must not lower canonical owner/ability filters to retired agent_ura/subject_ura fields",
            )

axon_protocol_root = axon_root / "sdk/rust/src"
axon_protocol_files = production_files(axon_protocol_root)
ura_vocabulary_files = sorted(set(runtime_facade_files + axon_protocol_files))

axon_adapter_modules = {
    "audio",
    "mcp",
    "presets",
}
axon_lib = axon_root / "sdk/rust/src/lib.rs"
lib_text = source(axon_lib)
for module in sorted(axon_adapter_modules):
    module_file = axon_protocol_root / f"{module}.rs"
    module_dir = axon_protocol_root / module
    if not module_file.exists() and not module_dir.exists():
        continue
    declaration = re.search(
        rf"^\s*(?:pub\s+)?mod\s+{re.escape(module)}\s*;", lib_text, re.M
    )
    location = declaration.start() if declaration else 0
    add(
        "R3_AXON_ADAPTER_MODULE",
        axon_lib if declaration else module_file if module_file.exists() else module_dir,
        line_number(lib_text, location) if declaration else 1,
        f"Axon protocol SDK contains product adapter module {module!r}",
    )

axon_adapter_operations = re.compile(
    r"^(?:build_deploy_package|deploy|deploy_ability_package|deploy_package|"
    r"deploy_to_node|deploy_to_value|disconnect_device|discover_nodes|"
    r"export_ability|install_plugin|uninstall_ability)$"
)
axon_public_decl = re.compile(
    r"^\s*pub(?:\([^)]*\))?\s+(?:(?:async|unsafe)\s+)?"
    r"(?:struct|enum|trait|type|fn)\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.M,
)
for path in axon_protocol_files:
    text = source(path)
    for match in axon_public_decl.finditer(text):
        name = match.group(1)
        if axon_adapter_operations.fullmatch(name):
            add(
                "R3_AXON_ADAPTER_OPERATION",
                path,
                line_number(text, match.start()),
                f"Axon protocol SDK exposes product adapter operation {name!r}",
            )

public_decl_patterns = {
    ".rs": re.compile(
        r"^\s*pub(?:\([^)]*\))?\s+(?:(?:async|unsafe)\s+)?"
        r"(?:struct|enum|trait|type|fn)\s+([A-Za-z_][A-Za-z0-9_]*)",
        re.M,
    ),
    ".go": re.compile(
        r"^(?:type\s+([A-Z][A-Za-z0-9_]*)\s+(?:struct|interface)|"
        r"func\s+(?:\([^)]*\)\s*)?([A-Z][A-Za-z0-9_]*)\s*\()",
        re.M,
    ),
    ".py": re.compile(r"^(?:class|(?:async\s+)?def)\s+([A-Za-z][A-Za-z0-9_]*)", re.M),
    ".js": re.compile(
        r"^\s*export\s+(?:class|function|const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)",
        re.M,
    ),
    ".ts": re.compile(
        r"^\s*export\s+(?:declare\s+)?(?:class|interface|type|function|const)\s+([A-Za-z_][A-Za-z0-9_]*)",
        re.M,
    ),
    ".java": re.compile(
        r"\bpublic\s+(?:final\s+)?(?:class|interface|record|enum)\s+([A-Za-z_][A-Za-z0-9_]*)"
    ),
    ".swift": re.compile(
        r"^\s*public\s+(?:struct|class|protocol|enum|func|typealias)\s+([A-Za-z_][A-Za-z0-9_]*)",
        re.M,
    ),
}
runtime_product_model = re.compile(
    r"^(?:Admin|Audio|Browser|Chat|Companion|Eal|Gateway|HostBinding|Mcp|Media|"
    r"Mic|Mission|OpenAI|Page|Plugin|Publication|RemoteControl|RemoteDesktop|"
    r"Skill|Surface|Voice)"
)
runtime_product_operation = re.compile(
    r"^(?:build_deploy_package|capture_utterance|deploy|deploy_ability_package|"
    r"deploy_package|deploy_to_node|deploy_to_value|disconnect_device|discover_nodes|"
    r"export_ability|install_plugin|open_mic|run_mission|start_easynet_daemon|"
    r"uninstall_ability)$"
)
for path in runtime_facade_files:
    text = source(path)
    pattern = public_decl_patterns.get(path.suffix)
    if pattern is None:
        continue
    for match in pattern.finditer(text):
        name = next(group for group in match.groups() if group is not None)
        if name.startswith("_"):
            continue
        if runtime_product_model.search(name):
            add(
                "R3_RUNTIME_PRODUCT_MODEL_PUBLIC",
                path,
                line_number(text, match.start()),
                f"Runtime facade publicly declares product model {name!r}",
            )
        if runtime_product_operation.search(name):
            add(
                "R3_RUNTIME_PRODUCT_OPERATION_PUBLIC",
                path,
                line_number(text, match.start()),
                f"Runtime facade publicly declares product operation {name!r}",
            )


semantic_entity = (
    r"(?:ability|agent|callee|caller|device|invocation|owner|principal|receipt|resource|subject)"
)
alternate_address = re.compile(
    rf"\b{semantic_entity}[A-Za-z0-9_]*(?:address|uri|url)\b|"
    rf"\b(?:address|uri|url)[A-Za-z0-9_]*{semantic_entity}\b",
    re.I,
)
transport_locator_type = re.compile(
    r"\b(?:hyper::Uri|http::Uri|tonic::transport::Uri|url::Url|URL|URI)\b"
)
semantic_name = re.compile(semantic_entity, re.I)
ura_name = re.compile(r"ura", re.I)
http_literal = re.compile(r"[\"'](?:https?|grpc)://", re.I)
ura_factory_bound_to_uri = re.compile(
    r"\b(?:let|var|const)\s+(?:mut\s+)?(?:[A-Za-z_][A-Za-z0-9_]*_)?uri\b[^\n=]*="
    r"(?:[^\n]*\n){0,4}?[^\n]*\b[A-Za-z_][A-Za-z0-9_]*_ura\s*\(",
    re.I,
)

semantic_ura_factory_roots = (
    cli_root / "src",
    cli_root / "sdk/go",
    cli_root / "sdk/python/easynet_sdk",
    cli_root / "sdk/node",
    cli_root / "sdk/java/src/main",
    cli_root / "sdk/swift/Sources",
    axon_root / "sdk/rust/src",
)
semantic_ura_factory_files: set[Path] = set()
for root in semantic_ura_factory_roots:
    if root.exists():
        semantic_ura_factory_files.update(production_files(root))

for path in ura_vocabulary_files:
    text = source(path)
    lines = text.splitlines()
    raw_lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    for index, line in enumerate(lines, start=1):
        # Alternate address vocabulary is checked only on declarations, not
        # comments, local transport variables, or method calls.
        declaration = re.search(
            r"\b(?:class|struct|record|interface|type|typealias|let|var|const|"
            r"pub|public|export|def|func)\b",
            line,
        )
        match = alternate_address.search(line) if declaration else None
        compatibility_alias = (
            index <= len(raw_lines)
            and "REQ-PROD-5 compatibility alias" in raw_lines[index - 1]
        )
        if match and not ura_name.search(match.group(0)) and not compatibility_alias:
            add(
                "R4_NON_URA_SEMANTIC_ADDRESS",
                path,
                index,
                f"semantic address {match.group(0)!r} must be named and modeled as URA",
            )
        if transport_locator_type.search(line) and semantic_name.search(line):
            add(
                "R4_TRANSPORT_LOCATOR_AS_SEMANTIC_URA",
                path,
                index,
                "semantic identity/URA uses a transport locator type",
            )
        if http_literal.search(line) and re.search(rf"{semantic_entity}[A-Za-z0-9_]*ura", line, re.I):
            add(
                "R4_TRANSPORT_LOCATOR_AS_SEMANTIC_URA",
                path,
                index,
                "semantic *_ura is populated with an HTTP/gRPC transport locator",
            )

for path in sorted(semantic_ura_factory_files):
    text = source(path)
    for match in ura_factory_bound_to_uri.finditer(text):
        add(
            "R4_URA_FACTORY_BOUND_TO_URI_NAME",
            path,
            line_number(text, match.start()),
            "semantic URA factory result must not be bound to a URI-named variable",
        )

local_daemon_grpc_transport = cli_root / "src/support/platform/local_daemon_grpc.rs"
if local_daemon_grpc_transport.exists():
    text = source(local_daemon_grpc_transport)
    direct_locator = "tonic::transport::Uri"
    if direct_locator in text:
        add(
            "R4_LOCAL_DAEMON_TRANSPORT_LOCATOR_TERMINOLOGY",
            local_daemon_grpc_transport,
            line_number(text, text.find(direct_locator)),
            "local daemon transport must alias tonic endpoint locators instead of exposing URI terminology",
        )
    if "Uri as GrpcEndpointLocator" not in text:
        add(
            "R4_LOCAL_DAEMON_TRANSPORT_LOCATOR_TERMINOLOGY",
            local_daemon_grpc_transport,
            1,
            "local daemon transport must name tonic connector endpoint placeholders as GrpcEndpointLocator",
        )

current_identity_docs = (
    cli_root / "docs/AXON-RFC-006-stateful-easynet.tex",
    cli_root / "docs/rfc/AXON-RFC-003-invokebidi-protocol.md",
    cli_root / "docs/rfc/AXON-RFC-006-stateful-easynet.tex",
    cli_root / "docs/rfc/AXON-RFC-006-stateful-easynet.md",
    cli_root / "docs/rfc/AXON-RFC-006-A-easynet-pages.tex",
    cli_root / "docs/rfc/AXON-RFC-006-A-easynet-pages.zh-CN.tex",
    cli_root / "docs/PAGES_AND_LLM_API.md",
)
current_identity_uri = re.compile(
    r"\bURIs?\b|caller-uri|principal-URI|Capability-URI|caller URI|agent URI"
)
for identity_doc in current_identity_docs:
    if identity_doc.exists():
        text = identity_doc.read_text(encoding="utf-8", errors="replace")
        stale_uri = current_identity_uri.search(text)
        if stale_uri:
            add(
                "R4_CURRENT_DOC_IDENTITY_URI_TERMINOLOGY",
                identity_doc,
                line_number(text, stale_uri.start()),
                "current identity/address docs must use URA, not URI",
            )


# Rule 7: unary InvocationResult uses one canonical terminal receipt projection.
# Stream and bidi frames may carry event-level `receipt` payloads; this rule is
# deliberately scoped to the unary result adapters that previously accepted or
# emitted a compatibility alias beside `terminal_receipt`.
unary_result_alias_patterns = (
    (
        cli_root / "src/ffi/invocation/mod.rs",
        re.compile(r'"receipt"\s*:\s*terminal_receipt|terminal_receipt\.clone\(\)\.map'),
    ),
    (
        cli_root / "sdk/go/runtime.go",
        re.compile(r'json:"receipt"|normalizeTerminalReceipt\s*\('),
    ),
    (
        cli_root / "sdk/python/easynet_sdk/runtime.py",
        re.compile(r'decoded\.get\("receipt"\)'),
    ),
    (
        cli_root / "sdk/python/easynet_sdk/providers/runtime/direct.py",
        re.compile(r'"receipt"\s*:\s*terminal_receipt'),
    ),
)
for path, pattern in unary_result_alias_patterns:
    if not path.exists():
        continue
    text = source(path)
    for match in pattern.finditer(text):
        add(
            "R7_UNARY_RESULT_RECEIPT_ALIAS",
            path,
            line_number(text, match.start()),
            "unary InvocationResult must use terminal_receipt instead of the retired receipt alias",
        )

dispatch_client = cli_root / "src/daemon/invocation/dispatch/client.rs"
if dispatch_client.exists():
    raw_text = dispatch_client.read_text(encoding="utf-8", errors="replace")
    raw_production_text = raw_text.split("#[cfg(test)]", 1)[0]
    for retired, detail in (
        (
            "source-compatible terminal result DTO",
            "InvocationOutcome::result must be documented as canonical terminal projection",
        ),
        (
            "source-compatible result DTO",
            "InvocationOutcome::into_result must be documented as canonical terminal projection",
        ),
    ):
        if retired in raw_text:
            add(
                "R7B_INVOCATION_OUTCOME_TERMINAL_RESULT_MODEL",
                dispatch_client,
                line_number(raw_text, raw_text.index(retired)),
                detail,
            )
    for required, detail in (
        (
            "Read the canonical terminal-result projection.",
            "InvocationOutcome::result canonical terminal projection doc is missing",
        ),
        (
            "return its canonical terminal-result projection",
            "InvocationOutcome::into_result canonical terminal projection doc is missing",
        ),
    ):
        if required not in raw_production_text:
            add(
                "R7B_INVOCATION_OUTCOME_TERMINAL_RESULT_MODEL",
                dispatch_client,
                1,
                detail,
            )


# Rule 7b: FFI complete-invocation ingress validates public tuple authority
# semantics before daemon I/O. The FFI may not rely on late daemon admission to
# discover all-zero placeholders or contradictory session/delegation subjects.
ffi_invocation = cli_root / "src/ffi/invocation/mod.rs"
if ffi_invocation.exists():
    raw_text = ffi_invocation.read_text(encoding="utf-8", errors="replace")
    text = source(ffi_invocation)
    production = text.split("\n#[cfg(all(test, feature = \"axon-pb\"))]\nmod tests", 1)[0]
    required_ffi_tuple_gate_tokens = (
        "fn validate_public_invocation_tuple(",
        "validate_public_invocation_tuple(",
        "&descriptor_ref,",
        "validate_public_invocation_descriptor_ref(descriptor_ref)?",
        "ReceiptHistoryReadDescriptor",
        "project_invocation_authority_metadata_shape(metadata)",
        "session_authority_admits_subject(&payload, subject_ura)",
        "AuthoritySubjectMismatch",
        "AllZeroPrincipal",
    )
    for token in required_ffi_tuple_gate_tokens:
        if token not in production:
            add(
                "R7B_FFI_PUBLIC_TUPLE_AUTHORITY_GATE_MISSING",
                ffi_invocation,
                1,
                f"FFI public invocation ingress must reject invalid tuple/authority metadata before daemon I/O: missing {token}",
            )
    for test_name in (
        "parse_invocation_json_rejects_all_zero_subject_before_daemon_io",
        "parse_invocation_json_rejects_receipt_history_descriptor_before_daemon_io",
        "parse_invocation_json_rejects_session_authority_subject_mismatch_before_daemon_io",
        "parse_invocation_json_rejects_device_caller_for_ordinary_ability_before_daemon_io",
        "builder_rejects_receipt_history_descriptor_before_daemon_io",
    ):
        if test_name not in raw_text:
            add(
                "R7B_FFI_PUBLIC_TUPLE_AUTHORITY_TEST_MISSING",
                ffi_invocation,
                1,
                f"FFI public tuple authority gate must keep failure-path test {test_name}",
            )
    if "validate_public_invocation_caller_ura(caller_ura, descriptor_ref)?" not in production:
        add(
            "R7C_FFI_DEVICE_CALLER_BOUNDARY",
            ffi_invocation,
            1,
            "FFI public invocation caller validation must receive descriptor_ref so Device custody calls cannot become a role-only allow",
        )
    device_unconditional_allow = re.search(
        r"URAKind::Device\s*=>\s*Ok\s*\(\s*\(\s*\)\s*\)", production
    )
    if device_unconditional_allow:
        add(
            "R7C_FFI_DEVICE_CALLER_BOUNDARY",
            ffi_invocation,
            line_number(production, device_unconditional_allow.start()),
            "FFI public invocation must not unconditionally allow Device caller; Device is substrate/custody, not an ordinary actor",
        )

device_caller_types = cli_root / "src/daemon/invocation/admission/device_caller_types.rs"
device_caller_classifier = cli_root / "src/daemon/invocation/admission/device_caller.rs"
policy_gate = cli_root / "src/daemon/invocation/admission/policy_gate.rs"
if device_caller_types.exists():
    types_text = source(device_caller_types)
    for token, detail in (
        (
            "pub(crate) enum DeviceCallerPurpose",
            "Device caller exceptions must be represented by a typed purpose enum",
        ),
        (
            "pub(crate) struct VerifiedDeviceInvocationPurpose",
            "Device caller classifier must expose an opaque verified-purpose proof type",
        ),
        (
            "pub(in crate::daemon::invocation::admission) purpose",
            "verified Device-caller purpose construction must remain scoped to admission",
        ),
        (
            "pub(in crate::daemon::invocation::admission) invocation_binding",
            "verified Device-caller purpose proof must bind the exact invocation geometry",
        ),
        (
            "Bootstrap",
            "Device caller classifier must name bootstrap as a Device-caller purpose",
        ),
        (
            "Pairing",
            "Device caller classifier must name pairing as a Device-caller purpose",
        ),
        (
            "PublicationCustody",
            "Device caller classifier must name publication custody as a Device-caller purpose",
        ),
        (
            "AbilityCatalogDiff",
            "Device caller classifier must name heartbeat liveness as a bounded Device-caller purpose",
        ),
        (
            "DeviceSelfSession",
            "Device caller classifier must name session control as a Device-caller purpose",
        ),
    ):
        if token not in types_text:
            add("R134_DEVICE_CALLER_CLASSIFIER", device_caller_types, 1, detail)
else:
    add(
        "R134_DEVICE_CALLER_CLASSIFIER",
        cli_root / "src/daemon/invocation/admission",
        1,
        "daemon admission must own an always-on Device-caller purpose vocabulary module",
    )

if device_caller_classifier.exists():
    text = source(device_caller_classifier)
    raw_text = device_caller_classifier.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "classify_public_invocation_caller_kind(",
            "public invocation caller validation must use the shared Device-caller classifier",
        ),
        (
            "admission::device_caller_types",
            "Device caller classifier must reuse the always-on policy vocabulary instead of redefining purpose types",
        ),
        (
            "verify_device_invocation_purpose(",
            "Device-caller ability exceptions must live in the shared classifier, not FFI",
        ),
        (
            "DEVICE_CALLER_RULES",
            "Device-caller purpose classification and policy admission must share one static rule table",
        ),
        (
            "admitted_device_policy_purpose(",
            "policy gate Device-caller exceptions must use the shared Device-caller rule table",
        ),
        (
            "AuthorityOwnerProjection",
            "Device publication custody must model owner-projection geometry separately from Device self geometry",
        ),
        (
            "ABILITY_FEDERATION_RESOLVE_KEY",
            "Device pairing must retain its bounded Hub key-resolution purpose after federation.join",
        ),
    ):
        if token not in text:
            add("R134_DEVICE_CALLER_CLASSIFIER", device_caller_classifier, 1, detail)
    for token, detail in (
        (
            '"ability.publish"',
            "ability-management abilities are owned by a device-sponsored SystemAgent and must not be Device-caller exceptions",
        ),
        (
            '"ability.unpublish"',
            "ability-management abilities are owned by a device-sponsored SystemAgent and must not be Device-caller exceptions",
        ),
        (
            '"ability.deploy"',
            "dynamic ability deployment uses target_ura as host selection; Device must not be the public caller exception",
        ),
        (
            '"ability.uninstall"',
            "dynamic ability uninstall uses target_ura as host selection; Device must not be the public caller exception",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R134_DEVICE_CALLER_CLASSIFIER",
                device_caller_classifier,
                line_number(text, offset),
                detail,
            )
    for token, detail in (
        (
            "classifies_public_device_caller_ability_surface",
            "classifier tests must prove all named Device-caller purposes",
        ),
        (
            "rejects_public_device_caller_for_ordinary_ability",
            "classifier tests must reject ordinary Device-caller abilities",
        ),
        (
            "rejects_device_caller_for_ability_management_system_agent_abilities",
            "classifier tests must prove ability-management SystemAgent abilities are not Device-caller exceptions",
        ),
        (
            "admits_policy_scope_only_for_exact_publication_geometry",
            "classifier tests must prove Device policy scope geometry is exact",
        ),
        (
            "Device custody must carry a hosted Agent's owner projection",
            "classifier tests must prove Device custody can publish a hosted Agent without becoming its owner",
        ),
    ):
        if token not in raw_text:
            add("R134_DEVICE_CALLER_CLASSIFIER", device_caller_classifier, 1, detail)
    for token, detail in (
        (
            "URAKind::Device | URAKind::Agent | URAKind::Authority",
            "Device publication custody must not broadly admit Device/Agent/Authority owners; DeviceProfileProjection is same-device only and Authority self-publication uses the Authority path",
        ),
        (
            "URAKind::Authority => true",
            "Device publication custody must not admit Authority owner projection; Authority self-publication uses the Authority caller path",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R134_DEVICE_CALLER_CLASSIFIER",
                device_caller_classifier,
                line_number(text, offset),
                detail,
            )
else:
    add(
        "R134_DEVICE_CALLER_CLASSIFIER",
        cli_root / "src/daemon/invocation/admission",
        1,
        "daemon admission must own a shared Device-caller classifier module",
    )

if ffi_invocation.exists():
    raw_text = ffi_invocation.read_text(encoding="utf-8", errors="replace")
    text = source(ffi_invocation)
    production = text.split("\n#[cfg(all(test, feature = \"axon-pb\"))]\nmod tests", 1)[0]
    for token, detail in (
        (
            "classify_public_invocation_caller(",
            "FFI public invocation caller validation must delegate Device caller semantics to admission::device_caller",
        ),
        (
            "DeviceCallerAdmissionError::DeviceCallerNotAllowed",
            "FFI must surface ordinary Device caller rejection from the shared classifier",
        ),
        (
            "parse_invocation_json_allows_device_caller_for_explicit_session_control",
            "FFI tests must prove the explicit session-control Device caller exception",
        ),
    ):
        haystack = raw_text if "tests must prove" in detail else production
        if token not in haystack:
            add("R134_DEVICE_CALLER_CLASSIFIER", ffi_invocation, 1, detail)
    for token, detail in (
        (
            "fn public_device_caller_ability_allowed(",
            "FFI must not keep a local Device caller allow-list",
        ),
        (
            "ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY\n            |",
            "FFI must not enumerate Device caller exception abilities locally",
        ),
    ):
        offset = production.find(token)
        if offset != -1:
            add(
                "R134_DEVICE_CALLER_CLASSIFIER",
                ffi_invocation,
                line_number(production, offset),
                detail,
            )

if policy_gate.exists():
    text = source(policy_gate)
    if "admitted_device_policy_purpose(" not in text or "DeviceCallerPurpose" not in text:
        add(
            "R134_DEVICE_CALLER_CLASSIFIER",
            policy_gate,
            1,
            "policy gate Device publication/session exceptions must delegate to admission::device_caller",
        )
    for function_name in (
        "device_publication_custody_manage_scope",
        "device_self_session_stream_scope",
    ):
        body = rust_method_body(text, function_name)
        if body is None:
            continue
        start, function_body = body
        for token, detail in (
            (
                "caller.kind != URAKind::Device",
                "policy gate must not reimplement Device caller kind checks inside Device exception rules",
            ),
            (
                "owner_ability_ura(",
                "policy gate must not reimplement Device caller ability matching inside Device exception rules",
            ),
        ):
            offset = function_body.find(token)
            if offset != -1:
                add(
                    "R134_DEVICE_CALLER_CLASSIFIER",
                    policy_gate,
                    line_number(text, start + offset),
                    detail,
                )

owner_projection_publication = (
    cli_root / "src/daemon/invocation/admission/owner_projection_publication.rs"
)
if owner_projection_publication.exists():
    text = source(owner_projection_publication)
    if "admits_owner_projection_publication_host(" not in text:
        add(
            "R134_DEVICE_CALLER_CLASSIFIER",
            owner_projection_publication,
            1,
            "owner-projection publication authority must use the shared Device publication-custody classifier",
        )
    body = rust_method_body(text, "verify_uras")
    if body is not None:
        start, function_body = body
        marker = "match caller.kind"
        offset = function_body.find(marker)
        if offset != -1:
            add(
                "R134_DEVICE_CALLER_CLASSIFIER",
                owner_projection_publication,
                line_number(text, start + offset),
                "owner-projection publication must not branch directly on caller.kind without the shared classifier",
            )

# Rule 11: stream and bidi SDK facades must not revive the retired frame-level
# receipt alias. Public accessors may project terminal_receipt for compatibility,
# but wire decode/encode must use admission_receipt and terminal_receipt only.
frame_receipt_alias_patterns = (
    (
        cli_root / "sdk/go/stream.go",
        re.compile(r'json:"receipt"|receiptJSON|event\.receipt|dto\.Receipt'),
    ),
    (
        cli_root / "sdk/go/bidi.go",
        re.compile(r'json:"receipt"|receiptJSON|frame\.receipt|dto\.Receipt'),
    ),
    (
        cli_root / "sdk/python/easynet_sdk/stream.py",
        re.compile(r'decoded\.get\("receipt"\)|else\s+event\.receipt'),
    ),
    (
        cli_root / "sdk/python/easynet_sdk/bidi.py",
        re.compile(r'decoded\.get\("receipt"\)|frame\.receipt'),
    ),
    (
        cli_root / "sdk/python/easynet_sdk/providers/runtime/direct.py",
        re.compile(r'event\["receipt"\]|event\["payload_json"\]\s*=\s*\{\s*"receipt"'),
    ),
)
for path, pattern in frame_receipt_alias_patterns:
    if not path.exists():
        continue
    text = source(path)
    for match in pattern.finditer(text):
        add(
            "R11_STREAM_BIDI_RECEIPT_ALIAS",
            path,
            line_number(text, match.start()),
            "stream/bidi SDK wire models must use terminal_receipt, not the retired receipt alias",
        )

retired_receipt_alias_rejection_markers = (
    (
        cli_root / "sdk/go/runtime.go",
        'rejectRetiredTopLevelReceiptAlias(raw, "invocation result")',
    ),
    (
        cli_root / "sdk/go/stream.go",
        'rejectRetiredTopLevelReceiptAlias(raw, "stream event")',
    ),
    (
        cli_root / "sdk/go/bidi.go",
        'rejectRetiredTopLevelReceiptAlias(raw, "bidi frame")',
    ),
    (
        cli_root / "sdk/python/easynet_sdk/runtime.py",
        'reject_retired_top_level_receipt_alias(',
    ),
    (
        cli_root / "sdk/python/easynet_sdk/stream.py",
        'reject_retired_top_level_receipt_alias(',
    ),
    (
        cli_root / "sdk/python/easynet_sdk/bidi.py",
        'reject_retired_top_level_receipt_alias(',
    ),
)
for path, marker in retired_receipt_alias_rejection_markers:
    if not path.exists():
        continue
    text = source(path)
    if marker not in text:
        add(
            "R64_SDK_RETIRED_RECEIPT_ALIAS_REJECTION",
            path,
            1,
            "SDK runtime result/frame decoders must reject the retired top-level receipt alias",
        )

python_receipt_projection = cli_root / "sdk/python/easynet_sdk/_receipt_projection.py"
if not python_receipt_projection.exists():
    add(
        "R64_SDK_RETIRED_RECEIPT_ALIAS_REJECTION",
        python_receipt_projection,
        1,
        "Python SDK receipt alias rejection must be owned by one shared wire projection guard",
    )
else:
    text = source(python_receipt_projection)
    for marker in (
        "def reject_retired_top_level_receipt_alias(",
        "retired receipt alias is not accepted",
        "stage=stage",
    ):
        if marker not in text:
            add(
                "R64_SDK_RETIRED_RECEIPT_ALIAS_REJECTION",
                python_receipt_projection,
                1,
                "Python SDK receipt alias rejection must be owned by one shared wire projection guard",
            )
    for path in (
        cli_root / "sdk/python/easynet_sdk/runtime.py",
        cli_root / "sdk/python/easynet_sdk/stream.py",
        cli_root / "sdk/python/easynet_sdk/bidi.py",
    ):
        text = source(path)
        if "def _reject_retired_top_level_receipt_alias(" in text:
            add(
                "R64_SDK_RETIRED_RECEIPT_ALIAS_REJECTION",
                path,
                1,
                "Python SDK facades must not duplicate the retired receipt alias guard",
            )

runtime_failure_extension_markers = (
    (
        cli_root / "sdk/go/errors.go",
        (
            "func runtimeFailureCode(",
            "isCanonicalExtensionErrorCode(code)",
            'return ErrorCode(code)',
            'case "DAEMON_DOWN", "DAEMON_OFFLINE":',
        ),
    ),
    (
        cli_root / "sdk/python/easynet_sdk/errors.py",
        (
            "def canonical_failure_code(",
            "_is_canonical_extension_error_code(code)",
            "return code",
            'code in {"DAEMON_DOWN", "DAEMON_OFFLINE"}',
        ),
    ),
)
for path, markers in runtime_failure_extension_markers:
    if not path.exists():
        continue
    text = source(path)
    for marker in markers:
        if marker not in text:
            add(
                "R65_SDK_RUNTIME_FAILURE_EXTENSION_CODE_PARITY",
                path,
                1,
                "SDK runtime failure classifiers must preserve canonical extension codes and reject retired aliases",
            )
            break


# Rule 30: C ABI stream/bidi callback JSON has one owner and one DTO shape.
# Rust FFI emits canonical callback fields; Go/Python C ABI adapters may order
# and normalize state/error objects, but must not repair retired callback names.
callback_projection_methods = (
    (
        cli_root / "src/ffi/invocation/backpressure.rs",
        "stream_callback_backpressure_event",
        ('"event"', '"content_type"', '"data_base64"', '"kind": "binary_chunk"'),
    ),
    (
        cli_root / "src/ffi/invocation/backpressure.rs",
        "bidi_callback_backpressure_frame",
        ('"event"', '"content_type"', '"data_base64"', '"kind": "binary_chunk"'),
    ),
    (
        cli_root / "src/ffi/invocation/mod.rs",
        "stream_chunk_json",
        ('"event"', '"content_type"', '"data_base64"', '"kind": "binary_chunk"'),
    ),
    (
        cli_root / "src/ffi/invocation/mod.rs",
        "stream_status_error_json",
        ('"event"', '"content_type"', '"data_base64"', '"kind": "binary_chunk"'),
    ),
    (
        cli_root / "src/ffi/invocation/mod.rs",
        "bidi_down_frame_json",
        ('"event"', '"content_type"', '"data_base64"', '"kind": "binary_chunk"'),
    ),
)
for path, method, retired_tokens in callback_projection_methods:
    if not path.exists():
        continue
    text = source(path)
    body = rust_method_body(text, method)
    if body is None:
        continue
    offset, method_body = body
    for token in retired_tokens:
        if token in method_body:
            add(
                "R30_SDK_STREAM_BIDI_CALLBACK_ALIAS",
                path,
                line_number(text, offset),
                f"C ABI callback projection `{method}` must not emit retired callback field {token}",
            )

go_cabi_runtime = cli_root / "sdk/go/cabi_runtime.go"
if go_cabi_runtime.exists():
    text = source(go_cabi_runtime)
    body = brace_function_body(
        text,
        r"func\s+projectCABIOrderedEvent\s*\(",
    )
    if body is not None:
        offset, function_body = body
        for token, detail in (
            (
                '"data_base64"',
                "Go C ABI event projector must not copy retired data_base64 into payload_base64",
            ),
            (
                '"event"',
                "Go C ABI event projector must not synthesize kind from retired event",
            ),
            (
                '"binary_chunk"',
                "Go C ABI event projector must not translate retired binary_chunk/chunk kinds",
            ),
        ):
            if token in function_body:
                add(
                    "R30_SDK_STREAM_BIDI_CALLBACK_ALIAS",
                    go_cabi_runtime,
                    line_number(text, offset),
                    detail,
                )

python_cabi = cli_root / "sdk/python/easynet_sdk/_cabi.py"
if python_cabi.exists():
    text = source(python_cabi)
    body = python_function_body(text, "_project_cabi_ordered_event")
    if body is not None:
        offset, function_body = body
        for token, detail in (
            (
                '"data_base64"',
                "Python C ABI event projector must not copy retired data_base64 into payload_base64",
            ),
            (
                '"event"',
                "Python C ABI event projector must not synthesize kind from retired event",
            ),
            (
                '"binary_chunk"',
                "Python C ABI event projector must not translate retired binary_chunk/chunk kinds",
            ),
        ):
            if token in function_body:
                add(
                    "R30_SDK_STREAM_BIDI_CALLBACK_ALIAS",
                    python_cabi,
                    line_number(text, offset),
                    detail,
                )


# Rule 31: file-resource ownership has one authority split. Host filesystem
# abilities keep the legacy local `fs.*` public names but are owned by the
# device-sponsored locomotion SystemAgent; daemon-local blob resources are
# owner-local `files.*` abilities owned by the sponsoring Device's Files
# SystemAgent. User-scoped Resource URAs remain unchanged. The OpenAI facade
# resolves the committed Files owner instead of rebuilding a hosted Agent URA.
files_store = cli_root / "src/daemon/ability/builtins/resources/files_store/mod.rs"
if files_store.exists():
    text = source(files_store)
    for token, detail in (
        (
            "OwnerKind::files_system()",
            "files.put/get/list must be owned by the Device-sponsored Files SystemAgent",
        ),
        (
            '"files.put"',
            "Files resource surface must register owner-local files.put",
        ),
        (
            '"files.get"',
            "Files resource surface must register owner-local files.get",
        ),
        (
            '"files.list"',
            "Files resource surface must register owner-local files.list",
        ),
    ):
        if token not in text:
            add("R31_FILE_RESOURCE_OWNERSHIP_FORK", files_store, 1, detail)
    match = re.search(r"\bOwnerKind::Device\b", text)
    if match:
        add(
            "R31_FILE_RESOURCE_OWNERSHIP_FORK",
            files_store,
            line_number(text, match.start()),
            "user blob files must not be registered as Device-owned system abilities",
        )
    for retired in ('OwnerKind::Agent("files".to_string())', "management_agent_ura"):
        pos = text.find(retired)
        if pos >= 0:
            add(
                "R31_FILE_RESOURCE_OWNERSHIP_FORK",
                files_store,
                line_number(text, pos),
                "Files must not revive a shared user-hosted Agent executor",
            )

device_files = cli_root / "src/daemon/ability/builtins/device_control/files.rs"
if device_files.exists():
    text = source(device_files)
    for token, detail in (
        ('"fs.read"', "Device-sponsored locomotion SystemAgent must expose fs.read"),
        ('"fs.write"', "Device-sponsored locomotion SystemAgent must expose fs.write"),
        ('"fs.stat"', "Device-sponsored locomotion SystemAgent must expose fs.stat"),
        ('"fs.list"', "Device-sponsored locomotion SystemAgent must expose fs.list"),
        ("OwnerKind::locomotion_system()", "Filesystem surface must bind through the locomotion SystemAgent owner"),
    ):
        if token not in text:
            add("R31_FILE_RESOURCE_OWNERSHIP_FORK", device_files, 1, detail)
    match = re.search(r"\bOwnerKind::Device\b", text)
    if match:
        add(
            "R31_FILE_RESOURCE_OWNERSHIP_FORK",
            device_files,
            line_number(text, match.start()),
            "Device filesystem surface must not register public fs.* abilities as direct Device-owned descriptors",
        )
    for match in re.finditer(r'"files\.(?:put|get|list)"|management_agent_ura', text):
        add(
            "R31_FILE_RESOURCE_OWNERSHIP_FORK",
            device_files,
            line_number(text, match.start()),
            "Device filesystem module must not own user blob files.* abilities",
        )

openai_compat = cli_root / "src/daemon/ability/builtins/integrations/openai_compat.rs"
if openai_compat.exists():
    text = source(openai_compat)
    for method, required_tokens in (
        (
            "handle_file_upload_with_context",
            ("OwnerKind::files_system()", '"files.put"', "invoke_resource_service_rpc"),
        ),
        (
            "handle_file_retrieve_with_context",
            ("OwnerKind::files_system()", '"files.get"', "invoke_resource_service_rpc"),
        ),
        (
            "deref_to_data_url",
            ("OwnerKind::files_system()", '"files.get"', "invoke_resource_service_rpc"),
        ),
    ):
        body = rust_method_body(text, method)
        if body is None:
            add(
                "R31_FILE_RESOURCE_OWNERSHIP_FORK",
                openai_compat,
                1,
                f"OpenAI compatibility must keep `{method}` as an explicit files authority adapter",
            )
            continue
        offset, method_body = body
        for token in required_tokens:
            if token not in method_body:
                add(
                    "R31_FILE_RESOURCE_OWNERSHIP_FORK",
                    openai_compat,
                    line_number(text, offset),
                    f"`{method}` must invoke owner-local files abilities through the files executor root",
                )
    legacy_files_dispatch = re.compile(
        r'["\'][A-Za-z0-9_{}.-]+\.files\.(?:put|get|list)["\']|'
        r'format!\s*\([^)]*\.files\.(?:put|get|list)'
    )
    for match in legacy_files_dispatch.finditer(text):
        add(
            "R31_FILE_RESOURCE_OWNERSHIP_FORK",
            openai_compat,
            line_number(text, match.start()),
            "OpenAI compatibility must not revive legacy <user>.files.* dispatch names",
        )
    chat_contract_tokens = (
        (
            "fn extract_chat_reply_text(",
            "OpenAI chat projection must centralize explicit provider response extraction",
        ),
        (
            "chat-base ability response must be a string or object with string reply, message, or content",
            "OpenAI chat projection must fail closed for unknown provider response shapes",
        ),
    )
    for token, detail in chat_contract_tokens:
        if token not in text:
            add("R96_OPENAI_CHAT_RESPONSE_FALLBACK", openai_compat, 1, detail)
    for token in (
        "fallback: stringify",
        "serde_json::to_string(&dispatch_result)",
        "serde_json::to_string(dispatch_result)",
    ):
        pos = text.find(token)
        if pos >= 0:
            add(
                "R96_OPENAI_CHAT_RESPONSE_FALLBACK",
                openai_compat,
                line_number(text, pos),
                f"OpenAI chat projection must not preserve fallback response stringification token `{token}`",
            )

catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
if catalog_build.exists():
    text = source(catalog_build)
    retired = text.find("declare_daemon_native_agent_authorities")
    if retired >= 0:
        add(
            "R31_FILE_RESOURCE_OWNERSHIP_FORK",
            catalog_build,
            line_number(text, retired),
            "daemon-native services must use SystemAgent authority, not declared hosted-Agent roots",
        )


# Rule 8: public FFI error JSON must not expose migration/legacy state.
ffi_root = cli_root / "src/ffi"
if ffi_root.exists():
    for path in production_files(ffi_root, {".rs"}):
        text = source(path)
        for match in re.finditer(r'\blegacy_untyped\b', text):
            add(
                "R8_FFI_LEGACY_ERROR_DETAIL",
                path,
                line_number(text, match.start()),
                "public FFI error JSON must expose typed ABI metadata, not legacy migration state",
            )


# Rule 9: realm-wide voice signaling has one realm Authority. Publishing the
# same state machine under Device and realm Authority creates duplicate descriptor truth.
voice_catalog = cli_root / "src/daemon/ability/builtins/resources/voice.rs"
if voice_catalog.exists():
    text = source(voice_catalog)
    device_owner = re.search(r"\bOwnerKind::Device\b", text)
    realm_authority_owner = re.search(r"\bOwnerKind::RealmAuthority\b", text)
    if device_owner:
        add(
            "R9_VOICE_OWNER_FORK",
            voice_catalog,
            line_number(text, device_owner.start()),
            "voice signaling must not mirror its realm Authority state machine under Device authority",
        )
    if not realm_authority_owner:
        add(
            "R9_VOICE_HUB_OWNER_MISSING",
            voice_catalog,
            1,
            "voice signaling must publish its realm-wide state under realm Authority",
        )

# An Authority-owned realm aggregate cannot be backed by a daemon-local filesystem
# adapter. Production assembly must receive an explicitly realm-shared
# repository provider; tests use a cfg(test)-only in-memory contract fake.
local_voice_repository = re.compile(
    r"LocalFileVoiceCallRepository|"
    r"(?:config::state_dir|state_dir\(\)|home_dir\(\)|\.easynet)"
    r"[\s\S]{0,160}(?:voice[_-]?calls?|calls)\.json|"
    r"(?:voice[_-]?calls?|calls)\.json"
    r"[\s\S]{0,160}(?:config::state_dir|state_dir\(\)|home_dir\(\)|\.easynet)"
)
for path in production_files(cli_root / "src", {".rs"}):
    text = source(path)
    for match in local_voice_repository.finditer(text):
        add(
            "R9_VOICE_LOCAL_STATE_FORK",
            path,
            line_number(text, match.start()),
            "Authority voice aggregate must use an injected realm-shared repository",
        )


# Rule 10: session presence and forwarded finalization cannot admit/read the
# retired JSON/v0 carrier. Wire fields may remain for schema compatibility,
# but production product code must neither construct nor consume them.
carrier_files = (
    cli_root / "src/daemon/invocation/bidi/state/presence.rs",
    cli_root / "src/daemon/invocation/bidi/bidi_dispatcher.rs",
)
carrier_fallback = re.compile(
    r"SessionContract::legacy\s*\(|"
    r"SessionContract::canonical\s*\(|"
    r"terminal_receipt\s*\.\s*or\s*\(|"
    r"receipt\s*:\s*legacy_terminal_receipt"
)
for path in carrier_files:
    if not path.exists():
        continue
    text = source(path)
    for match in carrier_fallback.finditer(text):
        add(
            "R10_RETIRED_CARRIER_FALLBACK",
            path,
            line_number(text, match.start()),
            "canonical session/finalization code consumes a retired v0 receipt carrier",
        )
    if path.name == "presence.rs":
        for token, detail in (
            (
                "enum PresenceSlot",
                "presence registry must model dispatch and resolve-only slots explicitly",
            ),
            (
                "pub fn insert_resolve_only(",
                "resolve-only presence must use an explicit registration path",
            ),
            (
                "fn validate_dispatch_session_contract(contract: &SessionContract) -> Result<(), String>",
                "dispatch session registration must validate carrier contract facts",
            ),
            (
                "contract.claimant_boot_nonce.len() != 16",
                "dispatch session registration must require a 16-byte claimant fingerprint",
            ),
            (
                "Self::ResolveOnly(_) => None",
                "resolve-only presence must not expose a dispatch session",
            ),
        ):
            if token not in text:
                add(
                    "R10_RETIRED_CARRIER_FALLBACK",
                    path,
                    1,
                    detail,
                )

for path in (
    axon_root / "core/proto/axon/v1/invoke.proto",
    axon_root / "core/runtime-rs/client-sdk/proto/axon/v1/invoke.proto",
    axon_root / "sdk/rust/proto/axon/v1/invoke.proto",
):
    if not path.exists():
        continue
    text = path.read_text(encoding="utf-8", errors="replace")
    match = re.search(r"(?:absent|zero)[^\n]*legacy|legacy JSON device", text, re.I)
    if match:
        add(
            "R10_RETIRED_CARRIER_CONTRACT",
            path,
            line_number(text, match.start()),
            "Axon schema still documents absent/v0 as a legacy carrier fallback",
        )


# Rule 12: per-agent state has one canonical directory. The old
# `workspaces/` name is retired; runtime readers, writers, and boot paths must
# not keep a directory migration or registry-prefix rewrite.
legacy_agent_root_pattern = re.compile(
    r'(?:state_dir\s*\(\s*\)|\.easynet)[^\n]{0,80}["\']workspaces["\']|'
    r'\.easynet/workspaces(?:/|\b)|'
    r'\blegacy_agents_root\s*\('
)
legacy_agent_root_operational_files = production_files(cli_root / "src", {".rs"})
legacy_agent_root_operational_files += production_files(
    cli_root / "skills", {".md"}
)
legacy_agent_root_operational_files += production_files(
    cli_root / "examples", {".md", ".sh"}
)
for path in sorted(set(legacy_agent_root_operational_files)):
    text = source(path)
    for match in legacy_agent_root_pattern.finditer(text):
        add(
            "R12_AGENT_ROOT_FORK",
            path,
            line_number(text, match.start()),
            "runtime agent state must use agents_root; workspaces migration is retired",
        )


# Rule 15: post-load agent registry rows own their canonical root path.
# Fresh creation and registry migration may derive `agents_root()/name`; steady
# state readers must call AgentEntry::required_root_path and fail closed when a
# row lacks `root_path`.
agent_root_fallback_allowed = {
    cli_root / "src/daemon/ability/builtins/agents/lifecycle.rs",
}
agent_root_fallback_pattern = re.compile(
    r"root_path[\s\S]{0,160}unwrap_or_else\s*\([\s\S]{0,160}agents_root\s*\(\s*\)\s*\.join"
)
for path in production_files(cli_root / "src", {".rs"}):
    if path in agent_root_fallback_allowed:
        continue
    text = source(path)
    for match in agent_root_fallback_pattern.finditer(text):
        add(
            "R15_AGENT_ROOTPATH_FALLBACK",
            path,
            line_number(text, match.start()),
            "post-load registry readers must use AgentEntry::required_root_path, not rebuild agents_root/name",
        )


# Rule 13: forwarded terminal data is untrusted until Axon's canonical wire
# parser and cryptographic checkpoint verifier authenticate both receipts.
forwarded_finalization = (
    cli_root / "src/daemon/invocation/dispatch/forwarded_finalization.rs"
)
if forwarded_finalization.exists():
    text = source(forwarded_finalization)
    projection = cli_root / "src/daemon/invocation/receipts/finalization_projection.rs"
    if projection.exists() and "finalization_projection" in text:
        text = text + "\n" + source(projection)
    for required, detail in (
        (
            "try_receipt_from_wire",
            "forwarded receipts must be parsed into Axon's canonical receipt domain",
        ),
        (
            "FinalizationCheckpointVerifier",
            "forwarded finalization must verify self-hashes and callee/host signatures",
        ),
        (
            "KeyResolver",
            "forwarded finalization must resolve the canonical receipt signer key",
        ),
    ):
        if required not in text:
            add("R13_FORWARDED_RECEIPT_UNVERIFIED", forwarded_finalization, 1, detail)

# Rule 14: daemon receipt proof primitives have one adapter owner. Product
# projections may consume finalization_projection, but must not recreate the
# Axon wire parser or finalization verifier pipeline in parallel.
receipt_projection = cli_root / "src/daemon/invocation/receipts/finalization_projection.rs"
receipt_proof_primitives = (
    (
        "try_receipt_from_wire",
        "receipt wire canonicalization must enter through finalization_projection",
    ),
    (
        "FinalizationCheckpointVerifier",
        "receipt finalization proof must enter through finalization_projection",
    ),
)
for path in production_files(cli_root / "src", {".rs"}):
    if path == receipt_projection:
        continue
    text = source(path)
    for primitive, detail in receipt_proof_primitives:
        offset = text.find(primitive)
        if offset >= 0:
            add(
                "R14_RECEIPT_PROOF_OWNER_FORK",
                path,
                line_number(text, offset),
                detail,
            )

# Rule 16: exact daemon unary, server-stream, and bidi routes must enter Axon
# LocalRuntime through one adapter owner. The tonic service may classify
# transport ingress, and dispatchers may own product behavior behind provider
# objects, but neither may reintroduce a direct exact-route execution table
# outside DaemonRouteRuntimeAdapter.
daemon_service = cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service.rs"
unary_dispatcher = cli_root / "src/daemon/invocation/dispatch/unary_dispatcher.rs"
stream_dispatcher = cli_root / "src/daemon/invocation/streams/stream_dispatcher.rs"
bidi_dispatcher = cli_root / "src/daemon/invocation/bidi/bidi_dispatcher.rs"
daemon_route_runtime = cli_root / "src/daemon/invocation/dispatch/daemon_route_runtime.rs"
boot_invocation_routes = cli_root / "src/daemon/boot/invocation/mod.rs"
descriptor_bound_dispatch = cli_root / "src/daemon/axon_bridge/descriptor_bound_dispatch.rs"
runtime_admin_contracts = (
    cli_root / "src/daemon/ability/catalog/runtime_admin_contracts.rs"
)
ability_catalog_dispatch = cli_root / "src/daemon/ability/dispatch.rs"
session_open_envelope = (
    cli_root / "src/daemon/invocation/bidi/session_initiator/envelope.rs"
)
retired_outer_admission_roots = (
    "verify_invoke(",
    "verify_invoke_stream(",
    "verify_envelope_for_bidi(",
)
for path in production_files(cli_root / "src/daemon/invocation", {".rs"}):
    text = source(path)
    for token in retired_outer_admission_roots:
        if token in text:
            add(
                "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                path,
                line_number(text, text.find(token)),
                f"retired outer admission root remains: `{token}`",
            )
if daemon_service.exists():
    service_text = source(daemon_service)
    service_requirements = (
        (
            "register_daemon_unary_routes",
            "daemon service must expose exact route registration",
        ),
        (
            "DaemonRouteRuntimeAdapter::new",
            "exact route registration must construct the runtime adapter",
        ),
        (
            ".dispatch_daemon_route_runtime(",
            "exact route dispatch must delegate to the LocalRuntime route path",
        ),
        (
            "register_daemon_stream_routes",
            "daemon service must expose exact stream route registration",
        ),
        (
            ".register_streams(",
            "exact stream route registration must install routes into LocalRuntime",
        ),
        (
            "streams.dispatch_daemon_route_runtime(route, &inner).await",
            "exact stream route dispatch must delegate to the LocalRuntime route path",
        ),
        (
            "pub(crate) enum DaemonBidiRoute",
            "exact bidi routes must be part of the daemon Invocation typed route inventory",
        ),
        (
            "DAEMON_INVOCATION_BIDI_ROUTES",
            "daemon Invocation route inventory must include bidi routes",
        ),
        (
            "register_daemon_bidi_routes",
            "daemon service must expose exact bidi route registration",
        ),
        (
            ".register_bidis(",
            "exact bidi route registration must install provider-bound routes into LocalRuntime",
        ),
        (
            ".dispatch_daemon_route_runtime(route, envelope_open, up)",
            "tonic exact bidi dispatch must open the descriptor-bound runtime route",
        ),
    )
    for token, detail in service_requirements:
        if token not in service_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", daemon_service, 1, detail)
    for token, detail in (
        (
            "fn validate_daemon_route_authority_owner(",
            "daemon exact route registration must use one Authority-only owner validator",
        ),
        (
            "parsed.kind != crate::core::ura::URAKind::Authority",
            "daemon exact route owner validator must reject every non-Authority owner kind",
        ),
        (
            "Device is only an execution host/custody root",
            "daemon exact route owner rejection must document that Device is a host/custody root, not public route owner",
        ),
        (
            "validate_daemon_route_authority_owner(owner_ura.trim(), \"stream\")",
            "stream exact-route registration must share the Authority-only owner validator",
        ),
        (
            "validate_daemon_route_authority_owner(owner_ura.trim(), \"bidi\")",
            "bidi exact-route registration must share the Authority-only owner validator",
        ),
    ):
        if token not in service_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", daemon_service, 1, detail)
    route_owner_validator = rust_method_body(service_text, "validate_daemon_route_authority_owner")
    if route_owner_validator is not None:
        offset, body = route_owner_validator
        for token, detail in (
            (
                "URAKind::Device",
                "daemon exact route owner validation must not allow Device owners",
            ),
            (
                "OwnerKind::Device",
                "daemon exact route owner validation must not project Device descriptor owners",
            ),
        ):
            local = body.find(token)
            if local != -1:
                add(
                    "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                    daemon_service,
                    line_number(service_text, offset + local),
                    detail,
                )
    if not any(
        token in service_text
        for token in (".register(owner_ura", ".register_for_owners(")
    ):
        add(
            "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
            daemon_service,
            1,
            "exact route registration must install every authority root into LocalRuntime",
        )
    direct_stream_calls = (
        "dispatch_subscribe_directory_initial(",
        "dispatch_subscribe_directory_v2(",
    )
    for token in direct_stream_calls:
        if token in service_text:
            add(
                "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                daemon_service,
                line_number(service_text, service_text.find(token)),
                f"daemon service must not call direct exact stream helper `{token}`",
            )

if bidi_dispatcher.exists():
    dispatcher_text = source(bidi_dispatcher)
    dispatcher_requirements = (
        (
            "RUNTIME_ADMIN_BIDI_ROUTES: &[DaemonBidiRoute]",
            "runtime-admin bidi conformance must consume the typed daemon bidi route inventory",
        ),
        (
            "DaemonBidiRoute::from_function(ability_name)",
            "BidiDispatcher exact-route classification must use the typed daemon bidi route owner",
        ),
        (
            "DaemonBidiRoute::SessionOpen =>",
            "the exact bidi provider must exhaustively own session.open product behavior",
        ),
        (
            "pub(crate) struct DaemonBidiRouteProvider",
            "exact bidi inventory must be behind a route provider object",
        ),
        (
            "fn dispatch_daemon_route_runtime",
            "BidiDispatcher must expose only the daemon bidi route runtime adapter path",
        ),
        (
            ".open_bidi(route, envelope_open, up)",
            "BidiDispatcher exact route path must open through the runtime adapter",
        ),
        (
            "struct SessionOpenProvider",
            "session.open must have a cohesive product lifecycle provider",
        ),
        (
            "impl SessionOpenProvider",
            "the session.open lifecycle must be implemented by its product provider",
        ),
        (
            "struct SessionOpenPolicy",
            "session.open product policy must have an explicit provider-owned boundary",
        ),
        (
            "policy: SessionOpenPolicy",
            "SessionOpenProvider must depend on the narrow session policy boundary",
        ),
        (
            "DaemonBidiRoute::SessionOpen => self.session_open.invoke(context).await",
            "the exact route provider must delegate session.open to its product owner",
        ),
    )
    for token, detail in dispatcher_requirements:
        if token not in dispatcher_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", bidi_dispatcher, 1, detail)
    if "match ability_name" in dispatcher_text:
        add(
            "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
            bidi_dispatcher,
            line_number(dispatcher_text, dispatcher_text.find("match ability_name")),
            "BidiDispatcher must not hand-match exact ability names outside the typed route owner",
            )
    session_provider_start = dispatcher_text.find("struct SessionOpenProvider {")
    session_provider_end = dispatcher_text.find("}\n", session_provider_start)
    if session_provider_start >= 0 and session_provider_end >= 0:
        session_provider_block = dispatcher_text[
            session_provider_start:session_provider_end
        ]
        for token, detail in (
            (
                "AdmissionFacade",
                "SessionOpenProvider must not retain the transport admission facade",
            ),
            (
                "session_realm:",
                "SessionOpenProvider must delegate realm admission to SessionOpenPolicy",
            ),
        ):
            if token in session_provider_block:
                add(
                    "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                    bidi_dispatcher,
                    line_number(
                        dispatcher_text,
                        session_provider_start + session_provider_block.find(token),
                    ),
                    detail,
                )
    for token in (
        "dispatch_self_session_accept",
        "self.dispatcher.run_session_open",
        "register_many(",
        "register_daemon_bidi_routes(",
    ):
        if token in dispatcher_text:
            add(
                "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                bidi_dispatcher,
                line_number(dispatcher_text, dispatcher_text.find(token)),
                f"bidi product provider must not retain direct or dynamic registration path `{token}`",
            )

if unary_dispatcher.exists():
    dispatcher_text = source(unary_dispatcher)
    dispatcher_requirements = (
        (
            "fn dispatch_daemon_route_runtime",
            "UnaryDispatcher must expose only the daemon route runtime adapter path",
        ),
        (
            "DaemonRouteRuntimeAdapter::new",
            "UnaryDispatcher exact route path must construct the runtime adapter",
        ),
        (
            ".dispatch(route, request, ingress)",
            "UnaryDispatcher exact route path must dispatch through the runtime adapter",
        ),
    )
    for token, detail in dispatcher_requirements:
        if token not in dispatcher_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", unary_dispatcher, 1, detail)

if stream_dispatcher.exists():
    dispatcher_text = source(stream_dispatcher)
    dispatcher_requirements = (
        (
            "fn dispatch_daemon_route_runtime",
            "StreamDispatcher must expose only the daemon stream route runtime adapter path",
        ),
        (
            "DaemonRouteRuntimeAdapter::new",
            "StreamDispatcher exact route path must construct the runtime adapter",
        ),
        (
            ".open_stream(route, request, local_system_ingress)",
            "StreamDispatcher exact route path must open streams through the runtime adapter",
        ),
        (
            "pub(crate) struct DaemonStreamRouteProvider",
            "exact stream product behavior must be behind a route provider object",
        ),
    )
    for token, detail in dispatcher_requirements:
        if token not in dispatcher_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", stream_dispatcher, 1, detail)
    obsolete_helpers = (
        "fn dispatch_subscribe_directory_initial",
        "fn dispatch_subscribe_directory_v2",
    )
    for token in obsolete_helpers:
        if token in dispatcher_text:
            add(
                "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                stream_dispatcher,
                line_number(dispatcher_text, dispatcher_text.find(token)),
                f"obsolete exact stream direct helper remains: `{token}`",
            )

if daemon_route_runtime.exists():
    adapter_text = source(daemon_route_runtime)
    adapter_requirements = (
        (
            "pub(crate) struct DaemonRouteRuntimeAdapter",
            "exact route runtime adapter owner is missing",
        ),
        ("Arc<LocalRuntime>", "daemon route runtime adapter must own LocalRuntime"),
        (
            "register_many(registrations)",
            "exact route adapter must atomically install route registrations",
        ),
        (
            "dispatch_rpc_admitted",
            "exact route adapter must drain Axon's admitted runtime path",
        ),
        (
            "register_streams",
            "exact route adapter must install stream route registrations",
        ),
        (
            "stream_env_ability_with_options",
            "exact stream routes must register as Axon stream-mode abilities",
        ),
        (
            "open_stream_admitted",
            "exact stream route adapter must open Axon's admitted stream path",
        ),
        (
            "register_bidis",
            "exact route adapter must install bidi route registrations",
        ),
        (
            "AbilityOptions::bidi()",
            "exact bidi routes must use canonical bounded Bidi registration options",
        ),
        (
            "open_bidi_external_signed",
            "exact bidi transport must construct an externally signed runtime request",
        ),
        (
            "project_registered_finalized_bidi_receipt",
            "exact bidi transport must project Axon's terminal receipt through the registered lifecycle owner",
        ),
    )
    for token, detail in adapter_requirements:
        if token not in adapter_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", daemon_route_runtime, 1, detail)
elif daemon_service.exists() or unary_dispatcher.exists():
    add(
        "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
        daemon_route_runtime,
        1,
        "exact daemon routes require a dedicated LocalRuntime adapter owner",
    )

if boot_invocation_routes.exists():
    boot_text = source(boot_invocation_routes)
    boot_requirements = (
        (
            "register_daemon_unary_routes(daemon_route_owner)",
            "boot must register exact unary routes before exposing listeners",
        ),
        (
            "register_daemon_stream_routes(daemon_route_owner)",
            "boot must register exact stream routes before exposing listeners",
        ),
        (
            "register_daemon_bidi_routes(daemon_route_owner)",
            "Hub boot must register exact bidi routes before exposing listeners",
        ),
    )
    for token, detail in boot_requirements:
        if token not in boot_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", boot_invocation_routes, 1, detail)
    hub_runtime_offsets = [
        match.start()
        for match in re.finditer(r"\bif\s+capabilities\.hub_runtime\s*\{", boot_text)
    ]
    if not hub_runtime_offsets:
        add(
            "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
            boot_invocation_routes,
            1,
            "daemon exact routes must be registered only inside the Hub runtime capability branch",
        )
    else:
        route_blocks = [
            (offset, brace_block_from(boot_text, offset))
            for offset in hub_runtime_offsets
            if any(token in brace_block_from(boot_text, offset) for token, _ in boot_requirements)
        ]
        if not route_blocks:
            add(
                "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                boot_invocation_routes,
                1,
                "daemon exact routes must be registered inside a Hub runtime capability branch",
            )
            route_blocks = [(hub_runtime_offsets[0], "")]
        hub_runtime_offset, hub_runtime_block = route_blocks[0]
        for token, detail in boot_requirements:
            if token not in hub_runtime_block:
                add(
                    "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                    boot_invocation_routes,
                    line_number(boot_text, hub_runtime_offset),
                    f"{detail}; Device-only boot must not install Hub daemon routes",
                )
    listener_offsets = [
        offset
        for token in ("spawn_uds_listener(", "spawn_tcp_tls_listener(")
        if (offset := boot_text.find(token)) >= 0
    ]
    stream_registration_offset = boot_text.find(
        "register_daemon_stream_routes(daemon_route_owner)"
    )
    bidi_registration_offset = boot_text.find(
        "register_daemon_bidi_routes(daemon_route_owner)"
    )
    for registration_offset, family in (
        (stream_registration_offset, "stream"),
        (bidi_registration_offset, "bidi"),
    ):
        if (
            listener_offsets
            and registration_offset >= 0
            and registration_offset > min(listener_offsets)
        ):
            add(
                "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                boot_invocation_routes,
                line_number(boot_text, registration_offset),
                f"exact {family} routes must be registered before invocation listeners are spawned",
            )

if descriptor_bound_dispatch.exists():
    dispatch_text = source(descriptor_bound_dispatch)
    for token, detail in (
        (
            "pub async fn open_bidi_external_signed",
            "the route adapter requires one externally signed bidi ingress seam",
        ),
        (
            "invoke_descriptor_bound_bidi_request_async",
            "the bidi ingress seam must open Axon's descriptor-bound runtime request",
        ),
    ):
        if token not in dispatch_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", descriptor_bound_dispatch, 1, detail)

if runtime_admin_contracts.exists():
    contract_text = source(runtime_admin_contracts)
    for token, detail in (
        (
            "register_control_plane_descriptor_with_owner",
            "session.open must use an explicit canonical owner registration",
        ),
        (
            "&OwnerKind::RealmAuthority",
            "session.open descriptor ownership must converge on the realm Authority",
        ),
    ):
        if token not in contract_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", runtime_admin_contracts, 1, detail)
    if "SESSION_OPEN_TEMPLATE_DEVICE_URA" in contract_text:
        add(
            "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
            runtime_admin_contracts,
            line_number(contract_text, contract_text.find("SESSION_OPEN_TEMPLATE_DEVICE_URA")),
            "session.open must not retain a Device-owned template authority",
        )

if ability_catalog_dispatch.exists():
    catalog_text = source(ability_catalog_dispatch)
    obsolete_scope_registration = "register_control_plane_descriptor_with_scope"
    if obsolete_scope_registration in catalog_text:
        add(
            "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
            ability_catalog_dispatch,
            line_number(
                catalog_text,
                catalog_text.find(obsolete_scope_registration),
            ),
            "descriptor-only contracts must resolve their canonical owner instead of accepting an explicit authority scope",
        )

if session_open_envelope.exists():
    envelope_text = source(session_open_envelope)
    for token, detail in (
        (
            "let hub_ura = crate::core::ura::hub_ura",
            "session.open must derive its canonical Hub callee from the caller realm",
        ),
        (
            "ProtoEnvelope::from_target(",
            "session.open signed tuple must use the canonical envelope builder",
        ),
        (
            "&hub_ura",
            "session.open signed descriptor reference must be Hub-owned",
        ),
        (
            "signed_descriptor_ref_invoke_request_with_signer(",
            "session.open must use the descriptor-bound canonical signing path",
        ),
    ):
        if token not in envelope_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", session_open_envelope, 1, detail)

session_open_initiator = cli_root / "src/daemon/invocation/bidi/session_initiator.rs"
session_open_warmup = cli_root / "src/daemon/invocation/bidi/session_initiator/warmup.rs"
if session_open_warmup.exists():
    add(
        "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
        session_open_warmup,
        1,
        "session.open must not keep a REST credential warmup/backstop before signed gRPC admission",
    )
if session_open_initiator.exists():
    initiator_text = source(session_open_initiator)
    for token, detail in (
        (
            "warm_device_credential_for_session",
            "session.open must not call a REST credential warmup before the canonical signed session",
        ),
        (
            "CredentialWarmupOutcome",
            "session.open must not maintain a parallel warmup lifecycle state",
        ),
        (
            "verify-credential",
            "session.open must not repair trust through the REST verify-credential route",
        ),
    ):
        if token in initiator_text:
            add(
                "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
                session_open_initiator,
                line_number(initiator_text, initiator_text.find(token)),
                detail,
            )

# Rule 16b: local daemon system request construction is daemon Invocation
# wire ownership, not support-layer transport ownership. The support adapter
# may resolve target policy and carry requests over tonic, but it must not
# define a second request object or rebuild Axon envelopes directly.
local_daemon_grpc = cli_root / "src/support/platform/local_daemon_grpc.rs"
invocation_wire = cli_root / "src/daemon/invocation/dispatch/invocation_wire.rs"
if local_daemon_grpc.exists():
    support_text = source(local_daemon_grpc)
    for token, detail in (
        (
            "struct LocalDaemonSystemInvocation",
            "support/platform must not define the local daemon-system request owner",
        ),
        (
            "ProtoEnvelope::targeted(",
            "support/platform must not assemble local daemon-system Axon envelopes directly",
        ),
        (
            "InvokeRequest {",
            "support/platform must not construct local daemon-system unary request protobufs directly",
        ),
        (
            "InvokeServerStreamRequest {",
            "support/platform must not construct local daemon-system stream request protobufs directly",
        ),
    ):
        offset = support_text.find(token)
        if offset >= 0:
            add(
                "R16B_LOCAL_DAEMON_SYSTEM_INVOCATION_OWNER_FORK",
                local_daemon_grpc,
                line_number(support_text, offset),
                detail,
            )
    if "LocalDaemonSystemInvocation" in support_text and not invocation_wire.exists():
        add(
            "R16B_LOCAL_DAEMON_SYSTEM_INVOCATION_OWNER_FORK",
            invocation_wire,
            1,
            "daemon invocation wire owner for local daemon-system requests is missing",
        )

if invocation_wire.exists():
    wire_text = source(invocation_wire)
    for token, detail in (
        (
            "pub(crate) struct LocalDaemonSystemInvocation",
            "daemon invocation wire module must own the local daemon-system request object",
        ),
        (
            "pub(crate) fn invoke_request",
            "local daemon-system owner must build unary InvokeRequest projections",
        ),
        (
            "pub(crate) fn stream_request",
            "local daemon-system owner must build stream InvokeServerStreamRequest projections",
        ),
        (
            "pub(crate) fn envelope",
            "local daemon-system owner must build the Axon envelope projection",
        ),
        (
            "derivation_policy: InvocationDerivationPolicy",
            "local daemon-system owner must require an explicit canonical derivation policy",
        ),
        (
            "pub(crate) fn with_trace_id",
            "local daemon-system owner must preserve trace-id projection",
        ),
        (
            "ProtoEnvelope::from_target(",
            "local daemon-system owner must use the daemon Invocation wire envelope builder",
        ),
    ):
        if token not in wire_text:
            add("R16B_LOCAL_DAEMON_SYSTEM_INVOCATION_OWNER_FORK", invocation_wire, 1, detail)
    forbidden_policy_mutator = "pub(crate) fn with_causal_context"
    offset = wire_text.find(forbidden_policy_mutator)
    if offset >= 0:
        add(
            "R16B_LOCAL_DAEMON_SYSTEM_INVOCATION_OWNER_FORK",
            invocation_wire,
            line_number(wire_text, offset),
            "causal derivation policy must be selected once at construction, not overwritten later",
        )

# Rule 16c: daemon-local system invocation construction has one named issuer.
# Transport shims may resolve descriptor refs and select call mode, but they
# must not mint the `_system.local` envelope fields or root causal policy in
# each RPC/stream/bidi helper.
descriptor_bound_dispatch = cli_root / "src/daemon/axon_bridge/descriptor_bound_dispatch.rs"
local_runtime_request = cli_root / "src/daemon/axon_bridge/local_runtime_request.rs"
wire_descriptor = cli_root / "src/daemon/axon_bridge/wire_descriptor.rs"
kernel_runtime = cli_root / "src/daemon/boot/kernel/mod.rs"
if local_runtime_request.exists():
    request_text = source(local_runtime_request)
    for token, detail in (
        (
            "pub(crate) struct SystemInvocationIssuer",
            "daemon-local system invocation requires a named issuer",
        ),
        (
            "request_for_descriptor_ref",
            "SystemInvocationIssuer must expose one descriptor-ref request constructor",
        ),
        (
            "request_for_complete_envelope",
            "SystemInvocationIssuer must accept only already-complete descriptor-bound envelopes",
        ),
        (
            "LocalRuntimeRequestFactory::request_for_local_system",
            "SystemInvocationIssuer must be the sole caller of the private system-signing factory",
        ),
        (
            "sign_system_canonical(&descriptor_bound_canonical_bytes(&envelope))",
            "system signing must use Axon's descriptor-bound draft owner inside the LocalRuntime request factory",
        ),
    ):
        if token not in request_text:
            add("R16C_SYSTEM_INVOCATION_ISSUER_FORK", local_runtime_request, 1, detail)
    if "pub(crate) fn request_for_local_system" in request_text:
        add(
            "R16C_SYSTEM_INVOCATION_ISSUER_FORK",
            local_runtime_request,
            line_number(
                request_text, request_text.find("pub(crate) fn request_for_local_system")
            ),
            "the system-signing factory must remain private to SystemInvocationIssuer",
        )

if wire_descriptor.exists():
    wire_descriptor_text = source(wire_descriptor)
    for token, detail in (
        (
            "require_descriptor_ref_for_wire",
            "wire adapter must resolve the product route to one canonical descriptor ref",
        ),
        (
            "wire::try_descriptor_bound_envelope_from_wire_parts",
            "Axon must own complete wire tuple reassembly and validation",
        ),
    ):
        if token not in wire_descriptor_text:
            add("R16C_SYSTEM_INVOCATION_ISSUER_FORK", wire_descriptor, 1, detail)
    for token, detail in (
        (
            "DescriptorBoundEnvelopeParts",
            "CLI wire adapter must not reconstruct canonical envelope parts",
        ),
        (
            "wire::try_agent_identity_from_wire",
            "CLI wire adapter must not parse canonical caller/callee identities independently",
        ),
        (
            "wire::try_subject_identity_from_wire",
            "CLI wire adapter must not parse the canonical subject independently",
        ),
        (
            "wire::try_invocation_nonce",
            "CLI wire adapter must not parse canonical freshness independently",
        ),
        (
            "wire::causal_context_from_wire",
            "CLI wire adapter must not parse canonical causal context independently",
        ),
        (
            "WireCallerIdentity",
            "wire reassembly must not select a caller-synthesis policy",
        ),
        (
            "system_agent_identity",
            "wire reassembly must not mint the local system caller",
        ),
        (
            "fresh_nonce",
            "wire reassembly must not replace a missing or invalid nonce",
        ),
        (
            "SubjectIdentity::from_callee",
            "wire reassembly must not replace a missing subject with the callee",
        ),
    ):
        offset = wire_descriptor_text.find(token)
        if offset >= 0:
            add(
                "R16C_SYSTEM_INVOCATION_ISSUER_FORK",
                wire_descriptor,
                line_number(wire_descriptor_text, offset),
                detail,
            )
if descriptor_bound_dispatch.exists():
    dispatch_text = source(descriptor_bound_dispatch)
    production_text = dispatch_text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "pub(crate) fn local_system_from_wire_parts",
            "trusted-local system dispatch construction must not be public SDK surface",
        ),
        (
            "then_some(LocalSystemAuthority)",
            "trusted-local classification must mint an unforgeable local-system authority seal",
        ),
        (
            "local_system_authority.ok_or_else",
            "local-system request conversion must require the trusted-local authority seal",
        ),
        (
            "SystemInvocationIssuer::request_for_descriptor_ref",
            "daemon local RPC/stream/bidi helpers must delegate to SystemInvocationIssuer",
        ),
        (
            "SystemInvocationIssuer::request_for_complete_envelope",
            "trusted local wire dispatch must delegate complete-envelope signing to SystemInvocationIssuer",
        ),
        (
            "open_stream_local_explicit_subject",
            "daemon local stream helper must name explicit subject issuer semantics",
        ),
        (
            "open_bidi_local_explicit_subject",
            "daemon local bidi helper must name explicit subject issuer semantics",
        ),
        (
            "dispatch_rpc_local_explicit_subject",
            "daemon local RPC helper must name explicit subject issuer semantics",
        ),
        (
            "DescriptorBoundEnvelopeParts",
            "descriptor-bound dispatch production code must not construct local system envelope parts",
        ),
        (
            "system_agent_identity()",
            "descriptor-bound dispatch production code must not mint the local system caller directly",
        ),
        (
            "open_stream_local_with_subject",
            "descriptor-bound dispatch must not preserve retired with_subject stream vocabulary",
        ),
        (
            "open_bidi_local_with_subject",
            "descriptor-bound dispatch must not preserve retired with_subject bidi vocabulary",
        ),
        (
            "dispatch_rpc_local_with_subject",
            "descriptor-bound dispatch must not preserve retired with_subject RPC vocabulary",
        ),
    ):
        if token in (
            "pub(crate) fn local_system_from_wire_parts",
            "then_some(LocalSystemAuthority)",
            "local_system_authority.ok_or_else",
            "SystemInvocationIssuer::request_for_descriptor_ref",
            "SystemInvocationIssuer::request_for_complete_envelope",
            "open_stream_local_explicit_subject",
            "open_bidi_local_explicit_subject",
            "dispatch_rpc_local_explicit_subject",
        ):
            if token not in production_text:
                add("R16C_SYSTEM_INVOCATION_ISSUER_FORK", descriptor_bound_dispatch, 1, detail)
            continue
        offset = production_text.find(token)
        if offset >= 0:
            add(
                "R16C_SYSTEM_INVOCATION_ISSUER_FORK",
                descriptor_bound_dispatch,
                line_number(production_text, offset),
                detail,
            )

if kernel_runtime.exists():
    kernel_text = source(kernel_runtime)
    if "SystemInvocationIssuer::request_for_descriptor_ref" not in kernel_text:
        add(
            "R16C_SYSTEM_INVOCATION_ISSUER_FORK",
            kernel_runtime,
            1,
            "kernel local-system request preparation must enter through SystemInvocationIssuer",
        )

for path in production_files(cli_root / "src", {".rs"}):
    text = source(path)
    offset = text.find("LocalRuntimeIngress::LocalSystem")
    if offset >= 0:
        add(
            "R16C_SYSTEM_INVOCATION_ISSUER_FORK",
            path,
            line_number(text, offset),
            "no caller may access a local-system request-factory ingress directly",
        )

local_runtime_invoker = cli_root / "src/daemon/invocation/dispatch/local_runtime_invoker.rs"
if local_runtime_invoker.exists():
    invoker_text = source(local_runtime_invoker)
    production_text = invoker_text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "SystemInvocationIssuer::request_for_descriptor_ref",
            "daemon LocalRuntime invoker must delegate local system tuple construction to SystemInvocationIssuer",
        ),
        (
            "DescriptorBoundEnvelopeParts",
            "daemon LocalRuntime invoker production code must not construct local system envelope parts",
        ),
        (
            "system_agent_identity()",
            "daemon LocalRuntime invoker production code must not mint the local system caller directly",
        ),
        (
            "fresh_nonce()",
            "daemon LocalRuntime invoker production code must not mint local system invocation nonces directly",
        ),
    ):
        if token == "SystemInvocationIssuer::request_for_descriptor_ref":
            if token not in production_text:
                add("R16C_SYSTEM_INVOCATION_ISSUER_FORK", local_runtime_invoker, 1, detail)
            continue
        offset = production_text.find(token)
        if offset >= 0:
            add(
                "R16C_SYSTEM_INVOCATION_ISSUER_FORK",
                local_runtime_invoker,
                line_number(production_text, offset),
                detail,
            )

# Rule 16d: Axon is the sole canonical Invocation tuple assembler. Product
# adapters may select typed values and transport metadata, but production CLI
# code must not restate protobuf or domain envelope fields as literals or call
# the low-level parts constructor.
manual_tuple_patterns = (
    (
        re.compile(r"(?:=\s*|Some\(\s*|return\s+)(?:[A-Za-z_][A-Za-z0-9_]*::)*Envelope\s*\{"),
        "production daemon code manually assembles a protobuf Invocation envelope",
    ),
    (
        re.compile(r"\bInvocationEnvelope\s*\{"),
        "production daemon code manually assembles a canonical Invocation tuple",
    ),
    (
        re.compile(r"\bDescriptorBoundEnvelope::from_parts\s*\("),
        "production daemon code bypasses Axon's canonical envelope builder",
    ),
    (
        re.compile(r"\bDescriptorBoundEnvelopeParts\b"),
        "production daemon code owns canonical descriptor-bound envelope parts",
    ),
)
for path in production_files(cli_root / "src/daemon", {".rs"}):
    text = source(path)
    for pattern, detail in manual_tuple_patterns:
        for match in pattern.finditer(text):
            add(
                "R16D_CANONICAL_ENVELOPE_OWNER_FORK",
                path,
                line_number(text, match.start()),
                detail,
            )

# Rule 23: CLI command modules may not own target-owned remote system ability
# routing. They map user input into payloads; the CLI daemon-client facade owns
# remote device/hub selector projection and caller identity selection. The
# descriptor-bound `ability invoke --node` path remains separate because it
# carries explicit origin-proof and subject semantics.
remote_system_ability_facade = (
    cli_root / "src/cli/daemon_client/remote_system_ability.rs"
)
descriptor_bound_remote_invoke_command = cli_root / "src/cli/commands/invoke.rs"
cli_remote_system_fork_patterns = (
    re.compile(r"RemoteAbilityInvocationTarget::for_target_owned_selector"),
    re.compile(r"remote_invoke::invoke_remote_target\s*\("),
    re.compile(r"daemon::invocation::routing::remote_invoke::invoke_remote_target\s*\("),
)
cli_root_dir = cli_root / "src/cli"
if cli_root_dir.exists():
    for path in production_files(cli_root_dir, {".rs"}):
        if path in (
            remote_system_ability_facade,
            descriptor_bound_remote_invoke_command,
        ):
            continue
        text = source(path)
        for pattern in cli_remote_system_fork_patterns:
            for match in pattern.finditer(text):
                add(
                    "R23_CLI_REMOTE_SYSTEM_ABILITY_FACADE_FORK",
                    path,
                    line_number(text, match.start()),
                    "CLI target-owned remote system ability dispatch must route through cli::daemon_client::remote_system_ability",
                )


# Rule 17: device-capable daemon modes must declare exactly one purge
# publication recovery owner before transport boot can perform lifecycle
# recovery or expose device-owned mutation routes. `both` is explicitly
# unsupported until it has a real publication/session owner; treating it like a
# device or hub silently strands committed purge outbox work.
boot_invocation = cli_root / "src/daemon/boot/invocation/mod.rs"
if boot_invocation.exists():
    text = source(boot_invocation)
    boot_requirements = (
        (
            "enum PublicationRecoveryOwner",
            "transport boot must model purge publication recovery ownership",
        ),
        (
            "struct InvocationModeCapabilities",
            "daemon mode capabilities must be explicit before boot wiring",
        ),
        (
            "fn for_mode(mode: DaemonMode) -> Self",
            "daemon mode capabilities must be derived in one owner",
        ),
        (
            "fn validate(self, mode: DaemonMode) -> anyhow::Result<()>",
            "unsupported device-capable modes must fail closed before mutation",
        ),
        (
            "fn owns_upstream_session(self) -> bool",
            "session-owned publication recovery must be a typed capability",
        ),
        (
            "register_purge_recovery_on_outbox_ready",
            "session readiness must redrive the durable purge publication outbox",
        ),
    )
    for token, detail in boot_requirements:
        if token not in text:
            add("R17_PURGE_PUBLICATION_MODE_OWNER_FORK", boot_invocation, 1, detail)

    mode_contracts = (
        (
            r"DaemonMode::Device\s*=>\s*Self\s*\{(?:(?!DaemonMode::).)*"
            r"publication_recovery\s*:\s*PublicationRecoveryOwner::UpstreamSession",
            "device mode must own purge publication recovery through the upstream session",
        ),
        (
            r"DaemonMode::Hub\s*=>\s*Self\s*\{(?:(?!DaemonMode::).)*"
            r"publication_recovery\s*:\s*PublicationRecoveryOwner::None",
            "hub mode must not pretend to own a device purge publication outbox",
        ),
        (
            r"DaemonMode::Both\s*=>\s*Self\s*\{(?:(?!DaemonMode::).)*"
            r"publication_recovery\s*:\s*PublicationRecoveryOwner::Unsupported",
            "both mode must fail closed until it owns a purge publication recovery path",
        ),
    )
    for pattern, detail in mode_contracts:
        match = re.search(pattern, text, re.S)
        if not match:
            add("R17_PURGE_PUBLICATION_MODE_OWNER_FORK", boot_invocation, 1, detail)

    validate_offset = text.find("capabilities.validate(config.mode())?")
    recovery_offset = text.find("recover_pending_purge_on_boot")
    if validate_offset < 0:
        add(
            "R17_PURGE_PUBLICATION_MODE_OWNER_FORK",
            boot_invocation,
            1,
            "transport boot must validate mode capabilities before lifecycle recovery",
        )
    elif recovery_offset >= 0 and validate_offset > recovery_offset:
        add(
            "R17_PURGE_PUBLICATION_MODE_OWNER_FORK",
            boot_invocation,
            line_number(text, validate_offset),
            "mode capability validation must happen before purge lifecycle recovery",
        )


# Rule 18: audited access-control mutations must be URA-only at the public
# boundary. A missing actor_ura must fail before store mutation; it must never
# be inferred from scalar owner_user_id or any nested grant/request DTO.
access_control = cli_root / "src/daemon/ability/builtins/governance/access_control.rs"
if access_control.exists():
    text = source(access_control)
    access_requirements = (
        (
            "fn require_actor_ura(actor_ura: &str) -> anyhow::Result<&str>",
            "access-control mutations need one actor URA validator",
        ),
        (
            "actor_ura is required for an audited mutation",
            "missing actor_ura must fail closed before mutation",
        ),
        (
            "actor_ura must be a canonical URA",
            "scalar actor IDs must not be persisted as audit URAs",
        ),
        (
            "let actor_ura = require_actor_ura(request.actor_ura.as_str())?",
            "revoke must validate actor_ura through the shared boundary validator",
        ),
    )
    for token, detail in access_requirements:
        if token not in text:
            add("R18_ACCESS_CONTROL_ACTOR_URA_FORK", access_control, 1, detail)

    if not re.search(
        r"store\.revoke_grant\(\s*&request\.grant_id,\s*&owner_user_ura,\s*actor_ura",
        text,
        re.S,
    ):
        add(
            "R18_ACCESS_CONTROL_ACTOR_URA_FORK",
            access_control,
            1,
            "revoke audit must persist the validated actor_ura against the canonical owner_user_ura",
        )

    revoke_match = re.search(r"struct\s+RevokeRequest\s*\{(?P<body>[^}]*)\}", text)
    if revoke_match is None:
        add(
            "R18_ACCESS_CONTROL_ACTOR_URA_FORK",
            access_control,
            1,
            "access-control revoke must have a typed request boundary",
        )
    else:
        prefix = text[max(0, revoke_match.start() - 160) : revoke_match.start()]
        if "deny_unknown_fields" not in prefix:
            add(
                "R18_ACCESS_CONTROL_ACTOR_URA_FORK",
                access_control,
                line_number(text, revoke_match.start()),
                "revoke request must reject scalar compatibility fields",
            )
        if re.search(r"\bowner_user_id\b|\bactor_user_id\b", revoke_match.group("body")):
            add(
                "R18_ACCESS_CONTROL_ACTOR_URA_FORK",
                access_control,
                line_number(text, revoke_match.start()),
                "revoke request must not expose scalar identity fields",
            )
        for field in ("owner_ura", "actor_ura"):
            if re.search(rf"#\[serde\(default\)\]\s*{field}\s*:\s*Option<", revoke_match.group("body")):
                add(
                    "R18_ACCESS_CONTROL_ACTOR_URA_FORK",
                    access_control,
                    line_number(text, revoke_match.start()),
                    f"revoke request must require {field} at typed ingress",
                )

    for pattern in (
        r"request\.actor_ura[\s\S]{0,120}(?:unwrap_or|unwrap_or_else|or_else)\s*\(",
        r"actor_ura[\s\S]{0,120}(?:unwrap_or|unwrap_or_else|or_else)\s*\([^)]*owner_user_id",
        r"actor_ura[\s\S]{0,120}(?:unwrap_or|unwrap_or_else|or_else)\s*\([^)]*request\.owner_ura",
    ):
        match = re.search(pattern, text, re.S)
        if match:
            add(
                "R18_ACCESS_CONTROL_ACTOR_URA_FORK",
                access_control,
                line_number(text, match.start()),
                "audited actor_ura must not fall back to scalar owner_user_id",
            )

# Rule 18b: SDK provider facades must mirror the daemon audited mutation
# boundary. Revoke is a mutation; actor_ura is not optional transport metadata.
# It must be required and parsed before provider wire dispatch in each SDK.
sdk_access_control_checks = (
    (
        cli_root / "sdk/go/access_control.go",
        "go",
        "func\\s+accessControlRevokeArgs\\s*\\(\\s*request\\s+AccessControlRevokeRequest\\s*\\)",
        (
            ("actorURA := strings.TrimSpace(request.ActorURA)", "Go revoke must normalize actor_ura before dispatch"),
            ("actorURA == \"\"", "Go revoke must reject missing actor_ura before dispatch"),
            ("ParseURAParts(actorURA)", "Go revoke must parse actor_ura as a canonical URA"),
            ("actor_ura must be canonical", "Go revoke must expose a canonical actor_ura error"),
            ("\"actor_ura\": actorURA", "Go revoke wire args must carry the validated actor_ura"),
        ),
        (
            r"_optional\s*\([^)]*actor_ura",
            r"if\s+actor\s*:=\s*strings\.TrimSpace\(request\.ActorURA\)\s*;\s*actor\s*!=",
        ),
    ),
    (
        cli_root / "sdk/python/easynet_sdk/access_control.py",
        "python",
        "_revoke_args",
        (
            ("actor_ura = _required_text(request.actor_ura, \"actor_ura\")", "Python revoke must require actor_ura before dispatch"),
            ("parse_ura(actor_ura)", "Python revoke must parse actor_ura as a canonical URA"),
            ("\"actor_ura\": actor_ura", "Python revoke wire args must carry the validated actor_ura"),
        ),
        (
            r"_optional\s*\(\s*args\s*,\s*[\"']actor_ura[\"']",
        ),
    ),
)
for path, language, function, requirements, forbidden_patterns in sdk_access_control_checks:
    if not path.exists():
        continue
    text = source(path)
    if language == "go":
        body = brace_function_body(text, function)
    else:
        body = python_function_body(text, function)
    if body is None:
        add(
            "R18B_SDK_ACCESS_CONTROL_REVOKE_ACTOR_URA_FORK",
            path,
            1,
            f"{language} SDK access-control provider must own revoke request lowering",
        )
        continue
    start, body_text = body
    for token, detail in requirements:
        if token not in body_text:
            add(
                "R18B_SDK_ACCESS_CONTROL_REVOKE_ACTOR_URA_FORK",
                path,
                line_number(text, start),
                detail,
            )
    for pattern in forbidden_patterns:
        match = re.search(pattern, body_text)
        if match:
            add(
                "R18B_SDK_ACCESS_CONTROL_REVOKE_ACTOR_URA_FORK",
                path,
                line_number(text, start + match.start()),
                "SDK access-control revoke must not serialize actor_ura as an optional field",
            )


# Rule 33: PrincipalLifecycle CLI may preserve source-compatible subject-self
# behavior, but actor source must be an explicit command-state boundary before
# JSON construction. The command serializer must not choose a missing actor_ura
# fallback itself; otherwise audit actor authority becomes a hidden convenience
# branch in the CLI facade.
principal_cli = cli_root / "src/cli/commands/groups/principal.rs"
if principal_cli.exists():
    text = source(principal_cli)
    principal_actor_requirements = (
        (
            "enum PrincipalCommandActor",
            "PrincipalLifecycle CLI actor source must be a typed boundary",
        ),
        (
            "fn supplied_or_subject_self(actor_ura: Option<&'a str>, principal_ura: &'a str) -> Self",
            "source-compatible subject-self behavior must be named at actor selection",
        ),
        (
            "fn subject_self(principal_ura: &'a str) -> Self",
            "internal subject-self commands must be explicit",
        ),
        (
            "fn actor_ura(self) -> &'a str",
            "actor projection must be owned by the typed actor boundary",
        ),
        (
            "fn principal_command(\n    actor: PrincipalCommandActor<'_>,",
            "principal command serializer must receive a selected actor state",
        ),
    )
    for token, detail in principal_actor_requirements:
        if token not in text:
            add("R33_PRINCIPAL_COMMAND_ACTOR_FALLBACK", principal_cli, 1, detail)

    old_signature = re.search(
        r"fn\s+principal_command\s*\(\s*actor_ura\s*:\s*Option\s*<\s*&str\s*>\s*,"
        r"\s*principal_ura\s*:\s*&str",
        text,
    )
    if old_signature:
        add(
            "R33_PRINCIPAL_COMMAND_ACTOR_FALLBACK",
            principal_cli,
            line_number(text, old_signature.start()),
            "principal command serializer must not accept actor_ura Option plus principal fallback source",
        )

    hidden_fallback = re.search(
        r"fn\s+principal_command[\s\S]{0,900}"
        r"(?:unwrap_or|unwrap_or_else)\s*\([\s\S]{0,120}principal_ura\s*\.trim",
        text,
        re.S,
    )
    if hidden_fallback:
        add(
            "R33_PRINCIPAL_COMMAND_ACTOR_FALLBACK",
            principal_cli,
            line_number(text, hidden_fallback.start()),
            "principal command serializer must not fall back from actor_ura to principal_ura",
        )


# Rule 34: remote hub routing has one authority boundary. Static
# `federated_peers` are operator intent and the only peer-hub dispatch endpoint
# authority. Federated-directory endpoints are observed read-model facts and
# must never synthesize Invocation dispatch routes.
hub_resolver = cli_root / "src/daemon/invocation/routing/hub_resolver.rs"
route_resolver = cli_root / "src/daemon/invocation/routing/route_resolver.rs"
daemon_config = cli_root / "src/daemon/persistence/daemon_config.rs"
if hub_resolver.exists():
    text = source(hub_resolver)
    hub_requirements = (
        (
            "pub enum HubResolution",
            "remote hub routing must expose typed resolution state",
        ),
        (
            "Static { hub_endpoint: String }",
            "static operator route must be a distinct resolution variant",
        ),
        (
            "Offline",
            "static miss must resolve offline",
        ),
    )
    for token, detail in hub_requirements:
        if token not in text:
            add("R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK", hub_resolver, 1, detail)

    resolve_match = re.search(
        r"pub\s+fn\s+resolve\s*\([^)]*\)\s*->\s*HubResolution\s*\{([\s\S]{0,1200})\n    \}",
        text,
        re.S,
    )
    resolve_body = resolve_match.group(1) if resolve_match else ""
    static_lookup = resolve_body.find("let peers_snapshot = self.static_peers.snapshot();")
    offline = resolve_body.find("HubResolution::Offline")
    if not resolve_match or static_lookup < 0 or offline < 0:
        add(
            "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK",
            hub_resolver,
            1,
            "HubResolver must make static lookup and offline miss visible",
        )
    elif not (static_lookup < offline):
        add(
            "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK",
            hub_resolver,
            line_number(text, resolve_match.start(1) + offline),
            "operator static peer lookup must precede offline miss",
        )

    for token in (
        "DirectoryFallback",
        "allow_directory_fallback",
        "lookup_in_federated_view",
        "federated_directory",
    ):
        pos = text.find(token)
        if pos >= 0:
            add(
                "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK",
                hub_resolver,
                line_number(text, pos),
                f"HubResolver must not preserve directory auto-route authority token `{token}`",
            )
else:
    add(
        "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK",
        hub_resolver,
        1,
        "HubResolver routing authority owner must exist",
    )

if route_resolver.exists():
    text = source(route_resolver)
    route_requirements = (
        (
            "HubResolver::new(",
            "RouteResolver must delegate remote hub source ordering to HubResolver",
        ),
        (
            "HubResolution::Static { hub_endpoint }",
            "RouteResolver must preserve static route evidence",
        ),
        (
            'DelegatedPeerEndpoint::new(hub_endpoint, "federated_peers", None)',
            "static route evidence source must stay explicit",
        ),
    )
    for token, detail in route_requirements:
        if token not in text:
            add("R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK", route_resolver, 1, detail)
    for token in (
        "peer_source.allow_directory_auto_route",
        "HubResolution::DirectoryFallback",
        '"federated_directory"',
        "allow_directory_auto_route",
        "lookup_in_federated_view",
    ):
        pos = text.find(token)
        if pos >= 0:
            add(
                "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK",
                route_resolver,
                line_number(text, pos),
                f"RouteResolver must not preserve directory auto-route authority token `{token}`",
            )
else:
    add(
        "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK",
        route_resolver,
        1,
        "RouteResolver must consume HubResolver route authority",
    )

if daemon_config.exists():
    text = daemon_config.read_text(encoding="utf-8", errors="replace")
    config_requirements = (
        (
            "no cross-realm dispatch endpoint is configured",
            "DaemonConfig must model empty federated_peers as typed no-route state",
        ),
        (
            "Federated-directory snapshots",
            "DaemonConfig must keep federated directory out of dispatch endpoint authority",
        ),
        (
            "do not synthesize dispatch endpoints",
            "DaemonConfig must reject directory-backed route synthesis semantics",
        ),
    )
    for token, detail in config_requirements:
        if token not in text:
            add("R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK", daemon_config, 1, detail)
    for token in (
        "target_online: false",
        "auto-discovered cross-realm directory",
        "directory route fallback",
        "allow_federated_directory_route_fallback",
        "allow_directory_fallback",
        "fallback to the legacy",
        "falls back to the legacy",
    ):
        pos = text.find(token)
        if pos >= 0:
            add(
                "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK",
                daemon_config,
                line_number(text, pos),
                f"DaemonConfig must not preserve retired route fallback token `{token}`",
            )


# Rule 32: Agent destructive lifecycle has one public boundary. Agent
# management is hosted by the device-sponsored SystemAgent, not by the Device
# owner plane directly. `agent.stop` remains a non-destructive row/authority
# removal; `agent.purge` is the only destructive root-removal entry point and
# must be projected as such by catalog metadata. Regressing this boundary
# revives either the old stop/purge semantic fork or the old Device-as-agent
# authority seam.
agent_lifecycle = cli_root / "src/daemon/ability/builtins/agents/lifecycle.rs"
catalog_metadata = cli_root / "src/daemon/ability/catalog/catalog_metadata.rs"
agent_purge_descriptor = (
    cli_root / "ability-descriptors/system/agents/agent.purge.ability.toml"
)
agent_stop_descriptor = (
    cli_root / "ability-descriptors/system/agents/agent.stop.ability.toml"
)
if agent_lifecycle.exists():
    text = source(agent_lifecycle)
    production_text = text.split("#[cfg(test)]", 1)[0]
    lifecycle_requirements = (
        (
            "pub const ABILITY_PURGE_AGENT",
            "Agent purge must have a named public ability constant",
        ),
        (
            "let owner = OwnerKind::agent_management_system();",
            "Agent lifecycle must bind to the device-sponsored agent-management SystemAgent owner",
        ),
        (
            "reg.register_rpc_with_owner(\n        ABILITY_PURGE_AGENT,\n        owner.clone()",
            "Agent purge must register through the agent-management SystemAgent owner variable",
        ),
        (
            "fn purge_agent_handler(",
            "Agent purge must have a dedicated handler separate from stop",
        ),
        (
            "struct PlatformTreeDeletion;",
            "Agent purge platform deletion must have one named owner",
        ),
        (
            "PlatformTreeDeletion::require_supported()?",
            "Agent purge must check platform deletion support before mutation",
        ),
        (
            "PlatformTreeDeletion::remove_quarantined_directory_identity_bound(",
            "Agent purge finalization must delete quarantine through the platform deletion owner",
        ),
        (
            "fn purge_agent_input_schema() -> Value",
            "Agent purge must publish a distinct schema owner",
        ),
        (
            "fn purge_agent_description() -> &'static str",
            "Agent purge must publish a distinct descriptor description",
        ),
        (
            "Requires Manage authority",
            "Agent purge descriptor text must advertise Manage authority",
        ),
        (
            'if args.get("purge").is_some()',
            "Agent stop must reject destructive purge input",
        ),
        (
            "invoke `agent.purge`",
            "Agent stop rejection must direct callers to the destructive boundary",
        ),
    )
    for token, detail in lifecycle_requirements:
        if token not in production_text:
            add("R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK", agent_lifecycle, 1, detail)
    if "ensure_identity_bound_purge_supported" in production_text:
        add(
            "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK",
            agent_lifecycle,
            1,
            "Agent purge support probing must not regress to a standalone helper",
        )
    if re.search(r"(?m)^fn\s+remove_quarantined_directory_identity_bound\s*\(", production_text):
        add(
            "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK",
            agent_lifecycle,
            1,
            "Agent purge quarantine deletion must remain owned by PlatformTreeDeletion",
        )
if catalog_metadata.exists():
    text = source(catalog_metadata)
    metadata_requirements = (
        (
            "destructive: is_destructive_public_ability(&public_name)",
            "catalog hints must route destructive risk through the central public-ability classifier",
        ),
        (
            "pub(crate) fn is_destructive_public_ability(public_name: &str) -> bool",
            "catalog destructive risk classifier must be an explicit named boundary",
        ),
        (
            "agent_names::AGENT_PURGE",
            "catalog destructive risk classifier must include agent.purge",
        ),
        (
            "agent_names::AGENT_PURGE => agent_lifecycle_ability::purge_agent_description()",
            "catalog descriptions must route agent.purge to the purge descriptor owner",
        ),
        (
            "agent_names::AGENT_PURGE => agent_lifecycle_ability::purge_agent_input_schema()",
            "catalog schemas must route agent.purge to the purge schema owner",
        ),
        (
            "agent_names::AGENT_PURGE_RECONCILE",
            "catalog metadata must model purge reconciliation as a named ability",
        ),
    )
    for token, detail in metadata_requirements:
        if token not in text:
            add("R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK", catalog_metadata, 1, detail)
if agent_purge_descriptor.exists():
    text = source(agent_purge_descriptor)
    descriptor_requirements = (
        ('name = "agent.purge"', "purge descriptor must name the purge ability"),
        (
            'admission_action = "manage"',
            "purge descriptor must require Manage authority",
        ),
        (
            '\\"destructive\\":true',
            "purge descriptor hints must mark destructive=true",
        ),
        (
            "Requires Manage authority",
            "purge descriptor description must state the Manage boundary",
        ),
    )
    for token, detail in descriptor_requirements:
        if token not in text:
            add("R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK", agent_purge_descriptor, 1, detail)
else:
    add(
        "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK",
        agent_purge_descriptor,
        1,
        "purge descriptor TOML must exist",
    )
if agent_stop_descriptor.exists():
    text = source(agent_stop_descriptor)
    descriptor_requirements = (
        ('name = "agent.stop"', "stop descriptor must name the stop ability"),
        (
            'admission_action = "manage"',
            "stop descriptor must require Manage authority for row removal",
        ),
        (
            '\\"destructive\\":false',
            "stop descriptor hints must remain destructive=false",
        ),
        (
            "registered root directory is always preserved",
            "stop descriptor description must preserve root-retention semantics",
        ),
    )
    for token, detail in descriptor_requirements:
        if token not in text:
            add("R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK", agent_stop_descriptor, 1, detail)
else:
    add(
        "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK",
        agent_stop_descriptor,
        1,
        "stop descriptor TOML must exist",
    )


# Rule 19: voice call signaling is Authority-owned realm state. Static descriptor
# contracts may exist without live handlers, but production route registration
# must require a qualified realm-shared repository provider. A process-local
# map, test repository, or daemon-local state file must never become the source
# of voice aggregate truth.
voice_contract = cli_root / "src/daemon/ability/builtins/resources/voice_contract.rs"
voice_repository = cli_root / "src/daemon/persistence/voice_calls.rs"
catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
voice_handler = cli_root / "src/daemon/ability/builtins/resources/voice.rs"

if voice_contract.exists():
    text = source(voice_contract)
    raw_text = voice_contract.read_text(encoding="utf-8", errors="replace")
    contract_requirements = (
        (
            "pub struct VoiceCallProviderAssembly",
            "voice live-route assembly must have a typed provider boundary",
        ),
        (
            "pub fn try_new(repository: Arc<dyn VoiceCallRepository>) -> anyhow::Result<Self>",
            "voice provider assembly must validate raw repositories before registration",
        ),
        (
            "qualification.validate_production()?",
            "voice provider assembly must require production qualification",
        ),
        (
            "fn qualification(&self) -> VoiceCallRepositoryQualification",
            "voice repositories must expose provider qualification facts",
        ),
    )
    for token, detail in contract_requirements:
        if token not in text:
            add("R19_VOICE_PROVIDER_BOUNDARY_FORK", voice_contract, 1, detail)

    if "VoiceCallRepositoryQualification::unqualified" not in raw_text:
        add(
            "R19_VOICE_PROVIDER_BOUNDARY_FORK",
            voice_contract,
            1,
            "test repositories must remain visibly unqualified",
        )
    if "struct TestVoiceCallRepository" not in raw_text:
        add(
            "R19_VOICE_PROVIDER_BOUNDARY_FORK",
            voice_contract,
            1,
            "the in-memory voice repository must stay test-only",
        )

    test_repo = raw_text.find("struct TestVoiceCallRepository")
    if test_repo >= 0:
        prefix = raw_text[max(0, test_repo - 120) : test_repo]
        if "cfg(test)" not in prefix:
            add(
                "R19_VOICE_PROVIDER_BOUNDARY_FORK",
                voice_contract,
                line_number(raw_text, test_repo),
                "in-memory voice repository must be compiled only for tests",
            )

if voice_repository.exists():
    text = source(voice_repository)
    repository_requirements = (
        (
            'pub const VOICE_SHARED_ROOT_ENV: &str = "EASYNET_HUB_VOICE_SHARED_ROOT"',
            "production voice repository root must be explicit deployment configuration",
        ),
        (
            "pub fn from_env(realm: &str) -> anyhow::Result<Option<std::sync::Arc<Self>>>",
            "production voice repository must be optional unless the shared root is configured",
        ),
        (
            "std::env::var_os(VOICE_SHARED_ROOT_ENV)",
            "production voice repository must read only the explicit shared-root setting",
        ),
        (
            "if !root.is_absolute()",
            "production voice repository must reject relative shared roots",
        ),
        (
            "VoiceCallRepositoryQualification::production",
            "Authority voice repository must be production-qualified only after shared-root validation",
        ),
        (
            "ExclusiveFileLock::acquire_for_data_path",
            "Authority voice mutations require a cross-process write guard",
        ),
        (
            "SharedFileLock::acquire_for_data_path",
            "Authority voice reads require a shared repository guard",
        ),
    )
    for token, detail in repository_requirements:
        if token not in text:
            add("R19_VOICE_PROVIDER_BOUNDARY_FORK", voice_repository, 1, detail)

    local_state = re.search(r"\b(?:config::state_dir|state_dir\s*\(|home_dir\s*\(|\.easynet)\b", text)
    if local_state:
        add(
            "R19_VOICE_PROVIDER_BOUNDARY_FORK",
            voice_repository,
            line_number(text, local_state.start()),
            "production voice repository must not derive authority from daemon-local state",
        )

if catalog_build.exists():
    text = source(catalog_build)
    catalog_requirements = (
        (
            "with_voice_call_provider_assembly",
            "catalog assembly must accept voice through provider assembly only",
        ),
        (
            "let voice_provider_assembly = shared_stores.voice_calls.clone()",
            "catalog build must snapshot voice provider assembly once",
        ),
        (
            "if hosts_realm_authority",
            "voice handlers may register only for realm-authority-capable runtime",
        ),
        (
            "if let Some(provider) = voice_provider_assembly.as_ref()",
            "voice handlers may register only when a qualified provider exists",
        ),
        (
            "voice_call_ability::register(&mut reg, provider.clone())",
            "catalog build must pass provider assembly into voice route registration",
        ),
        (
            "repository_assembled: voice_provider_assembly.is_some()",
            "voice capability evidence must derive ProviderBacked from provider assembly",
        ),
    )
    for token, detail in catalog_requirements:
        if token not in text:
            add("R19_VOICE_PROVIDER_BOUNDARY_FORK", catalog_build, 1, detail)

if voice_handler.exists():
    text = source(voice_handler)
    if "pub fn register(reg: &mut AxonAbilityCatalog, provider: VoiceCallProviderAssembly)" not in text:
        add(
            "R19_VOICE_PROVIDER_BOUNDARY_FORK",
            voice_handler,
            1,
            "voice route registration must require VoiceCallProviderAssembly",
        )
    if re.search(r"pub\s+fn\s+register\s*\([^)]*Arc\s*<\s*dyn\s+VoiceCallRepository", text):
        add(
            "R19_VOICE_PROVIDER_BOUNDARY_FORK",
            voice_handler,
            1,
            "voice route registration must not accept raw repositories",
        )


# Rule 20: stream/bidi cancellation at the C ABI provider boundary submits one
# independently signed canonical lifecycle-control command and retains the
# original reader until a canonical terminal receipt is observed. Language SDK
# adapters may expose the request state, but must not synthesize lifecycle
# terminality for local stream or bidi cancellation.
ffi_v5_spec = cli_root / "docs/spec/ffi-abi-v7.md"
ffi_invocation = cli_root / "src/ffi/invocation/mod.rs"
go_cabi_runtime = cli_root / "sdk/go/cabi_runtime.go"
go_errors = cli_root / "sdk/go/errors.go"
python_errors = cli_root / "sdk/python/easynet_sdk/errors.py"
python_cabi_runtime = cli_root / "sdk/python/easynet_sdk/_cabi.py"
go_direct_runtime = cli_root / "sdk/go/direct_runtime.go"
python_direct_runtime = cli_root / "sdk/python/easynet_sdk/providers/runtime/direct.py"
go_stream_facade = cli_root / "sdk/go/stream.go"
go_bidi_facade = cli_root / "sdk/go/bidi.go"
python_stream_facade = cli_root / "sdk/python/easynet_sdk/stream.py"
python_bidi_facade = cli_root / "sdk/python/easynet_sdk/bidi.py"
python_transport_facade = cli_root / "sdk/python/easynet_sdk/transport.py"
go_stream_tests = cli_root / "sdk/go/stream_test.go"
go_bidi_tests = cli_root / "sdk/go/bidi_test.go"
go_errors_tests = cli_root / "sdk/go/errors_test.go"
go_direct_runtime_tests = cli_root / "sdk/go/direct_runtime_test.go"
python_stream_tests = cli_root / "sdk/python/tests/test_stream.py"
python_bidi_tests = cli_root / "sdk/python/tests/test_bidi.py"
python_direct_runtime_tests = cli_root / "sdk/python/tests/test_direct_runtime.py"
python_errors_tests = cli_root / "sdk/python/tests/test_errors.py"

if ffi_v5_spec.exists():
    text = ffi_v5_spec.read_text(encoding="utf-8", errors="replace")
    if "stream cancel and bidi cancel are cancel-request operations" not in text:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_v5_spec,
            1,
            "ABI v7 contract must name stream/bidi cancel as request state, not terminal proof",
        )
    if not re.search(r"must\s+not\s+claim\s+lifecycle\s+terminality", text):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_v5_spec,
            1,
            "ABI v7 contract must forbid local cancel from claiming runtime terminality",
        )
    if not re.search(r"submits\s+at\s+most\s+one\s+independently\s+signed", text):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_v5_spec,
            1,
            "ABI v7 contract must require one-shot independently signed canonical cancellation",
        )
    if not re.search(r"keeps\s+the\s+callback/reader\s+path\s+draining", text):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_v5_spec,
            1,
            "ABI v7 contract must preserve the original terminal drain path",
        )
    legacy_terminal_claim = re.search(
        r"stream\s+cancel/close\s+and\s+bidi\s+cancel/close\s+are\s+terminal",
        text,
        flags=re.I,
    )
    if legacy_terminal_claim:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_v5_spec,
            line_number(text, legacy_terminal_claim.start()),
            "ABI v7 must not define local stream/bidi cancel or close as lifecycle terminal",
        )
else:
    add(
        "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
        ffi_v5_spec,
        1,
        "ABI v7 contract is required for stream/bidi cancellation terminal authority",
    )

if ffi_invocation.exists():
    text = source(ffi_invocation)
    ffi_requirements = (
        (
            "request_stream_cancellation(",
            "stream cancel must submit through the provider cancellation authority",
        ),
        (
            "request_bidi_cancellation(",
            "bidi cancel must submit through the provider cancellation authority",
        ),
        (
            "request_cancel_signed(",
            "provider cancellation must build and sign a canonical lifecycle-control command",
        ),
        (
            "ProviderCancellationPhase::Rejected",
            "provider cancellation must memoize rejection instead of resubmitting",
        ),
    )
    for token, detail in ffi_requirements:
        if token not in text:
            add(
                "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
                ffi_invocation,
                1,
                detail,
            )

    stream_cancel = re.search(
        r"fn\s+runtime_invocation_stream_cancel\b.*?\n}\n",
        text,
        flags=re.S,
    )
    if stream_cancel and re.search(r'"terminal"\s*:\s*true|InvocationState::Cancelled', stream_cancel.group(0)):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_invocation,
            line_number(text, stream_cancel.start()),
            "C ABI stream cancel must not synthesize lifecycle terminality",
        )
    if stream_cancel and "release_stream_with_reader_cancel(" in stream_cancel.group(0):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_invocation,
            line_number(text, stream_cancel.start()),
            "C ABI stream cancel must not release the terminal drain path",
        )
    bidi_cancel = re.search(
        r"fn\s+runtime_invocation_bidi_cancel\b.*?\n}\n",
        text,
        flags=re.S,
    )
    if bidi_cancel and re.search(r'"terminal"\s*:\s*true|InvocationState::Cancelled', bidi_cancel.group(0)):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_invocation,
            line_number(text, bidi_cancel.start()),
            "C ABI bidi cancel must not synthesize lifecycle terminality",
        )
    if bidi_cancel and re.search(r"remove_bidi_for_handle|reader_cancel\.cancel\(\)", bidi_cancel.group(0)):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_invocation,
            line_number(text, bidi_cancel.start()),
            "C ABI bidi cancel must not release the terminal drain path",
        )

if go_cabi_runtime.exists():
    text = source(go_cabi_runtime)
    go_cancel_contracts = (
        (
            r"func\s+\(s\s+\*cabiStreamTransport\)\s+Cancel\b.*?CancelRequested.*?terminal\":false",
            "Go C ABI stream cancel must project CancelRequested with terminal=false",
        ),
        (
            r"func\s+\(b\s+\*cabiBidiTransport\)\s+Cancel\b.*?CancelRequested.*?terminal\":false",
            "Go C ABI bidi cancel must project CancelRequested with terminal=false",
        ),
    )
    for pattern, detail in go_cancel_contracts:
        if not re.search(pattern, text, flags=re.S):
            add(
                "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
                go_cabi_runtime,
                1,
                detail,
            )
    go_backpressure = re.search(
        r"func\s+cabiCallbackBackpressureFailure\s*\(\s*\)\s*\[\]byte\s*\{.*?\n\}",
        text,
        flags=re.S,
    )
    if "type cabiCallbackInbox struct" in text and go_backpressure is None:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            go_cabi_runtime,
            1,
            "Go callback inbox must own a transport-failure projection without terminal authority",
        )
    if "cabiCallbackBackpressureTerminal" in text:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            go_cabi_runtime,
            1,
            "Go callback backpressure must not retain obsolete terminal-authority naming",
        )
    if go_backpressure and (
        not re.search(r'"terminal"\s*:\s*false', go_backpressure.group(0))
        or not re.search(
            r'"transport_terminal"\s*:\s*true', go_backpressure.group(0)
        )
    ):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            go_cabi_runtime,
            line_number(text, go_backpressure.start()),
            "Go callback backpressure must be a transport failure and must not claim receipt-backed terminality",
        )

if python_cabi_runtime.exists():
    text = source(python_cabi_runtime)
    python_cancel_contracts = (
        (
            r"class\s+_CABIStreamTransport\b.*?def\s+cancel\b.*?\"state\"\s*:\s*\"CancelRequested\".*?\"terminal\"\s*:\s*False",
            "Python C ABI stream cancel must project CancelRequested with terminal=False",
        ),
        (
            r"class\s+_CABIBidiTransport\b.*?def\s+cancel\b.*?\"state\"\s*:\s*\"CancelRequested\".*?\"terminal\"\s*:\s*False",
            "Python C ABI bidi cancel must project CancelRequested with terminal=False",
        ),
    )
    for pattern, detail in python_cancel_contracts:
        if not re.search(pattern, text, flags=re.S):
            add(
                "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
                python_cabi_runtime,
                1,
                detail,
            )
    python_backpressure = re.search(
        r"def\s+_callback_backpressure_failure\s*\(\s*\)\s*->\s*bytes\s*:.*?(?=\n\ndef\s+|\Z)",
        text,
        flags=re.S,
    )
    if "class _CallbackInbox" in text and python_backpressure is None:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            python_cabi_runtime,
            1,
            "Python callback inbox must own a transport-failure projection without terminal authority",
        )
    if "_callback_backpressure_terminal" in text:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            python_cabi_runtime,
            1,
            "Python callback backpressure must not retain obsolete terminal-authority naming",
        )
    if python_backpressure and (
        not re.search(r'"terminal"\s*:\s*False', python_backpressure.group(0))
        or not re.search(
            r'"transport_terminal"\s*:\s*True', python_backpressure.group(0)
        )
    ):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            python_cabi_runtime,
            line_number(text, python_backpressure.start()),
            "Python callback backpressure must be a transport failure and must not claim receipt-backed terminality",
        )

if go_direct_runtime.exists():
    text = source(go_direct_runtime)
    go_direct_cancel_contracts = (
        (
            r"func\s+\(t\s+\*directRuntimeStreamTransport\)\s+Cancel\b.*?unsupportedDirectCancellation\(t\.endpoint,\s*t\.streamID,\s*\"stream_cancel\"\)",
            "Go direct runtime stream cancel must report the capability unsupported",
        ),
        (
            r"func\s+\(t\s+\*directRuntimeBidiTransport\)\s+Cancel\b.*?unsupportedDirectCancellation\(t\.endpoint,\s*t\.sessionID,\s*\"bidi_cancel\"\)",
            "Go direct runtime bidi cancel must report the capability unsupported",
        ),
    )
    for pattern, detail in go_direct_cancel_contracts:
        if not re.search(pattern, text, flags=re.S):
            add(
                "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
                go_direct_runtime,
                1,
                detail,
            )
    if "isDescriptorOwnerOfflineMessage" in text:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            go_direct_runtime,
            line_number(text, text.find("isDescriptorOwnerOfflineMessage")),
            "Go direct runtime must not infer descriptor owner-offline from route diagnostic text",
        )
    if "canonicalErrorCodeFromMessagePrefix(message)" not in text:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            go_direct_runtime,
            1,
            "Go direct runtime must consume canonical descriptor owner-offline code text",
        )
    if go_direct_runtime_tests.exists():
        tests = source(go_direct_runtime_tests)
        if "TestDirectRuntimeGRPCErrorDoesNotInferOwnerOfflineFromRouteText" not in tests:
            add(
                "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                go_direct_runtime_tests,
                1,
                "Go direct runtime must test that route diagnostic text is not a descriptor owner-offline state",
            )
if go_errors.exists():
    text = source(go_errors)
    if "isDescriptorOwnerOfflineMessage" in text:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            go_errors,
            line_number(text, text.find("isDescriptorOwnerOfflineMessage")),
            "Go error DTO decoding must not infer descriptor owner-offline from route diagnostic text",
        )
    if go_errors_tests.exists():
        tests = source(go_errors_tests)
        if "TestDecodeTransportErrorJSONDoesNotInferOwnerOfflineFromRouteText" not in tests:
            add(
                "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                go_errors_tests,
                1,
                "Go error DTO decoding must test that route diagnostic text is not a descriptor owner-offline state",
            )

if python_errors.exists():
    text = source(python_errors)
    if "is_descriptor_owner_offline_message" in text or "_is_descriptor_owner_offline_message" in text:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            python_errors,
            line_number(
                text,
                max(
                    text.find("is_descriptor_owner_offline_message"),
                    text.find("_is_descriptor_owner_offline_message"),
                ),
            ),
            "Python error DTO decoding must not infer descriptor owner-offline from route diagnostic text",
        )
    if python_errors_tests.exists():
        tests = source(python_errors_tests)
        if "test_from_json_does_not_infer_owner_offline_from_route_text" not in tests:
            add(
                "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                python_errors_tests,
                1,
                "Python error DTO decoding must test that route diagnostic text is not a descriptor owner-offline state",
            )

if python_direct_runtime.exists():
    text = source(python_direct_runtime)
    if "is_descriptor_owner_offline_message" in text:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            python_direct_runtime,
            line_number(text, text.find("is_descriptor_owner_offline_message")),
            "Python direct runtime must not infer descriptor owner-offline from route diagnostic text",
        )
    if "_canonical_error_code_from_message_prefix(message)" not in text:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            python_direct_runtime,
            1,
            "Python direct runtime must consume canonical descriptor owner-offline code text",
        )
    if python_direct_runtime_tests.exists():
        tests = source(python_direct_runtime_tests)
        if "test_direct_runtime_grpc_does_not_infer_owner_offline_from_route_text" not in tests:
            add(
                "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                python_direct_runtime_tests,
                1,
                "Python direct runtime must test that route diagnostic text is not a descriptor owner-offline state",
            )
    python_direct_cancel_contracts = (
        (
            r"class\s+DirectRuntimeStreamTransport\b.*?def\s+cancel\b.*?_unsupported_direct_cancellation\(.*?capability=\"stream_cancel\"",
            "Python direct runtime stream cancel must report the capability unsupported",
        ),
        (
            r"class\s+DirectRuntimeBidiTransport\b.*?def\s+cancel\b.*?_unsupported_direct_cancellation\(.*?capability=\"bidi_cancel\"",
            "Python direct runtime bidi cancel must report the capability unsupported",
        ),
    )
    for pattern, detail in python_direct_cancel_contracts:
        if not re.search(pattern, text, flags=re.S):
            add(
                "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
                python_direct_runtime,
                1,
                detail,
            )

direct_facade_contracts = (
    (
        go_stream_facade,
        r"func\s+\(s\s+\*StreamHandle\)\s+Cancel\b.*?cancel\.state\s*!=\s*StreamCancelRequested.*?cancel\.terminal.*?cancel\.cancelled.*?stream cancel transport must return CancelRequested with terminal=false",
        "Go stream facade must reject terminal or cancelled provider-local cancel outcomes",
    ),
    (
        go_bidi_facade,
        r"func\s+\(s\s+\*BidiSession\)\s+Cancel\b.*?outcome\.state\s*!=\s*BidiCancelRequested.*?outcome\.terminal.*?bidi cancel transport must return CancelRequested with terminal=false",
        "Go bidi facade must reject terminal provider-local cancel outcomes",
    ),
    (
        python_stream_facade,
        r"def\s+cancel\b.*?outcome\.state\s*!=\s*StreamState\.CANCEL_REQUESTED.*?outcome\.terminal.*?outcome\.cancelled.*?stream cancel transport must return CancelRequested with terminal=false",
        "Python stream facade must reject terminal or cancelled provider-local cancel outcomes",
    ),
    (
        python_bidi_facade,
        r"def\s+cancel\b.*?outcome\.state\s*!=\s*BidiState\.CANCEL_REQUESTED.*?outcome\.terminal.*?bidi cancel transport must return CancelRequested with terminal=false",
        "Python bidi facade must reject terminal provider-local cancel outcomes",
    ),
)
for path, pattern, detail in direct_facade_contracts:
    if not path.exists():
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            path,
            1,
            detail,
        )
        continue
    text = source(path)
    if not re.search(pattern, text, flags=re.S):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            path,
            1,
            detail,
        )

for path in (go_bidi_facade, python_bidi_facade):
    if path.exists():
        text = source(path)
        if "bidi session must be terminal before close" in text:
            add(
                "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
                path,
                1,
                "bidi close is local resource release and must not require canonical terminality",
            )

if python_transport_facade.exists():
    text = source(python_transport_facade)
    bidi_adapter = re.search(
        r"class\s+BidiSessionAdapter\b.*?(?=\nclass\s+|\Z)",
        text,
        flags=re.S,
    )
    bidi_adapter_close = (
        re.search(
            r"\n    def\s+close\b.*?(?=\n    def\s+|\Z)",
            bidi_adapter.group(0),
            flags=re.S,
        )
        if bidi_adapter
        else None
    )
    if bidi_adapter_close and ".cancel(" in bidi_adapter_close.group(0):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            python_transport_facade,
            line_number(text, bidi_adapter.start() + bidi_adapter_close.start()),
            "bidi close must release local resources without synthesizing a cancellation request",
        )
    bidi_adapter_cancel = (
        re.search(
            r"\n    def\s+cancel\b.*?(?=\n    def\s+|\Z)",
            bidi_adapter.group(0),
            flags=re.S,
        )
        if bidi_adapter
        else None
    )
    if bidi_adapter_cancel and "_terminal = True" in bidi_adapter_cancel.group(0):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            python_transport_facade,
            line_number(text, bidi_adapter.start() + bidi_adapter_cancel.start()),
            "bidi cancel request must keep receive alive until a canonical terminal frame",
        )

sdk_cancel_tests = (
    (
        go_stream_tests,
        "TestStreamHandleCancelIsNonTerminalRequest",
        "TestStreamHandleRejectsTerminalCancelOutcome",
        "Go stream facade tests must prove non-terminal request and terminal rejection",
    ),
    (
        go_bidi_tests,
        "TestBidiCancelIsNonTerminalRequest",
        "TestBidiCancelRejectsTerminalOutcome",
        "Go bidi facade tests must prove non-terminal request and terminal rejection",
    ),
    (
        python_stream_tests,
        "test_stream_cancel_is_non_terminal_request",
        "test_stream_cancel_rejects_terminal_outcome",
        "Python stream facade tests must prove non-terminal request and terminal rejection",
    ),
    (
        python_bidi_tests,
        "test_cancel_is_non_terminal_request",
        "test_cancel_rejects_terminal_outcome",
        "Python bidi facade tests must prove non-terminal request and terminal rejection",
    ),
    (
        go_direct_runtime_tests,
        "TestDirectRuntimeStreamCancelIsExplicitlyUnsupported",
        "TestDirectRuntimeBidiCancelIsExplicitlyUnsupported",
        "Go direct runtime tests must prove stream/bidi cancellation is explicitly unsupported",
    ),
    (
        python_direct_runtime_tests,
        "test_direct_runtime_stream_cancel_is_explicitly_unsupported",
        "test_direct_runtime_bidi_cancel_is_explicitly_unsupported",
        "Python direct runtime tests must prove stream/bidi cancellation is explicitly unsupported",
    ),
)
for path, first_test, second_test, detail in sdk_cancel_tests:
    if not path.exists():
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            path,
            1,
            detail,
        )
        continue
    text = source(path)
    if first_test not in text or second_test not in text:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            path,
            1,
            detail,
        )


# Rule 21: unary cancellation is an independently signed lifecycle-control
# Invocation, not a replay of the target Invocation with unsigned metadata.
# The target is identified by canonical lifecycle hash; the cancel command has
# its own nonce, descriptor, signer policy and admission/replay result.
cancel_domain = cli_root / "src/daemon/invocation/dispatch/cancellation.rs"
request_model = cli_root / "src/daemon/invocation/dispatch/request.rs"
runtime_client = cli_root / "src/daemon/invocation/dispatch/client.rs"
unary_admission_tests = (
    cli_root
    / "src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs"
)

if cancel_domain.exists():
    text = source(cancel_domain)
    cancel_domain_requirements = (
        (
            "pub const ABILITY_INVOCATION_CANCEL",
            "cancel ability must have one named descriptor owner",
        ),
        (
            "pub struct InvocationCancelCommand",
            "cancel must be a typed command, not unsigned transport metadata",
        ),
        (
            "pub target_lifecycle_hash: String",
            "cancel command must bind the target by lifecycle hash",
        ),
        (
            "#[serde(deny_unknown_fields)]",
            "cancel command must reject unsigned metadata extensions",
        ),
        (
            "invocation_lifecycle_hash(envelope: &DescriptorBoundEnvelope)",
            "target lifecycle hash must derive from descriptor-bound canonical bytes",
        ),
        (
            "struct RegisteredInvocationLifecycle",
            "daemon adapters must share one lifecycle registration owner",
        ),
        (
            "pub(crate) async fn finalized(&self)",
            "lifecycle registration owner must bind canonical finalization to terminal retention",
        ),
        (
            "pub(crate) async fn cancel_and_finalize(",
            "lifecycle registration owner must bind cancellation to canonical finalization",
        ),
    )
    raw_text = cancel_domain.read_text(encoding="utf-8", errors="replace")
    for token, detail in cancel_domain_requirements:
        haystack = raw_text if token.startswith("#[") else text
        if token not in haystack:
            add(
                "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
                cancel_domain,
                1,
                detail,
            )
    raw_registry_mutation = re.search(
        r"\bpub(?:\(crate\))?\s+fn\s+register\s*\(\s*&self,\s*envelope:"
        r"|\bpub(?:\(crate\))?\s+fn\s+mark_terminal\s*\(\s*&self,\s*key:",
        text,
    )
    if raw_registry_mutation:
        add(
            "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
            cancel_domain,
            line_number(text, raw_registry_mutation.start()),
            "raw lifecycle registration and terminal mutation must remain private to RegisteredInvocationLifecycle",
        )
else:
    add(
        "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
        cancel_domain,
        1,
        "unary cancellation command domain is required",
    )

if request_model.exists():
    text = source(request_model)
    raw_text = request_model.read_text(encoding="utf-8", errors="replace")
    prepare_cancel = re.search(
        r"fn\s+prepare_cancel_command\s*\([^)]*\)\s*->\s*Result<PreparedInvocation>\s*\{(?P<body>.*?)\n    \}",
        text,
        flags=re.S,
    )
    if not prepare_cancel:
        add(
            "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
            request_model,
            1,
            "SignedInvocation must expose prepare_cancel_command for independent cancel drafts",
        )
    else:
        body = prepare_cancel.group("body")
        prepare_requirements = (
            (
                "self.prepared.canonical_hash_hex()",
                "cancel command must bind target by canonical lifecycle hash",
            ),
            (
                "InvocationCancelCommand::new",
                "cancel command must be normalized through the typed command",
            ),
            (
                "ABILITY_INVOCATION_CANCEL",
                "cancel draft must target the invocation.cancel descriptor",
            ),
            (
                ".build_draft()?",
                "cancel must build a new invocation draft with a fresh nonce",
            ),
            (
                "policy_ref: Some(\"invocation.cancel.caller\".to_string())",
                "cancel draft must carry explicit cancel caller signer policy",
            ),
        )
        for token, detail in prepare_requirements:
            if token not in body:
                add(
                    "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
                    request_model,
                    line_number(text, prepare_cancel.start()),
                    detail,
                )
        if "self.clone()" in body or "self.into_daemon_invocation()" in body:
            add(
                "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
                request_model,
                line_number(text, prepare_cancel.start()),
                "prepare_cancel_command must not reuse the signed target invocation",
            )

    test_requirements = (
        "prepare independent cancel command",
        "assert_ne!(cancel.draft().invocation.nonce(), target_nonce)",
        "assert_eq!(command.target_lifecycle_hash, target_hash)",
    )
    for token in test_requirements:
        if token not in raw_text:
            add(
                "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
                request_model,
                1,
                f"missing cancel independence test evidence: {token}",
            )

if runtime_client.exists():
    text = source(runtime_client)
    request_cancel = re.search(
        r"pub\s+async\s+fn\s+request_cancel_signed\s*\([^)]*\)\s*->\s*Result<InvocationHandle>\s*\{(?P<body>.*?)\n    \}",
        text,
        flags=re.S,
    )
    if not request_cancel:
        add(
            "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
            runtime_client,
            1,
            "runtime client must expose request_cancel_signed",
        )
    else:
        body = request_cancel.group("body")
        client_requirements = (
            (
                "signed.prepare_cancel_command(reason)?",
                "request_cancel_signed must prepare an independent cancel command",
            ),
            (
                "self.cancellation_authority.as_ref()",
                "request_cancel_signed must require an explicitly bound cancellation authority",
            ),
            (
                "authority.sign(prepared).await?",
                "request_cancel_signed must sign through the bound owner authority",
            ),
            (
                ".invoke(signed_cancel)",
                "request_cancel_signed must submit the signed cancel command",
            ),
        )
        for token, detail in client_requirements:
            if token not in body:
                add(
                    "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
                    runtime_client,
                    line_number(text, request_cancel.start()),
                    detail,
                )
        if re.search(r"\bsigned\.into_daemon_invocation\s*\(", body):
            add(
                "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
                runtime_client,
                line_number(text, request_cancel.start()),
                "request_cancel_signed must not replay the original signed invocation",
            )
        if (
            "RuntimeSigningIdentity::load_default" in body
            or "KeyringClient::default_path" in body
        ):
            add(
                "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
                runtime_client,
                line_number(text, request_cancel.start()),
                "RuntimeClient must not infer a default signer; ingress must bind an explicit caller or daemon KeyService authority",
            )

if unary_admission_tests.exists():
    raw_text = unary_admission_tests.read_text(encoding="utf-8", errors="replace")
    if "signed_invocation_cancel_command_replay_is_rejected" not in raw_text:
        add(
            "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
            unary_admission_tests,
            1,
            "LocalRuntime admission tests must prove signed cancel command replay rejection",
        )
else:
    add(
        "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
        unary_admission_tests,
        1,
        "LocalRuntime unary admission tests are required",
    )

# Rule 22: Agent lifecycle owns paired durable projections through one
# lifecycle projection store. Start/stop/purge recovery may advance lifecycle
# state and purge-journal stages, but production code in the handler must not
# hand-assemble direct agents.json/local-agents.json writes outside the
# projection owner.
agent_lifecycle = cli_root / "src/daemon/ability/builtins/agents/lifecycle.rs"
if agent_lifecycle.exists():
    lifecycle_text = source(agent_lifecycle)
    production_text = lifecycle_text.split("#[cfg(test)]", 1)[0]
    lifecycle_requirements = (
        (
            "struct AgentLifecycleProjectionStore",
            "agent lifecycle durable projections require one projection-store owner",
        ),
        (
            "fn persist_registry(&self, registry: &AgentRegistry)",
            "projection store must own agents.json persistence",
        ),
        (
            "fn persist_identities(&self, identities: &local_agents::LocalAgentsFile)",
            "projection store must own local-agents.json persistence",
        ),
        (
            "fn restore_uncommitted_purge_snapshots(",
            "purge recovery must restore paired projections through the store",
        ),
        (
            "fn bootstrap_local_agent_projection(",
            "startup hosted identity projection must be owned by Agent lifecycle",
        ),
        (
            "agent.bootstrap: acquire lifecycle transaction",
            "startup hosted identity projection must acquire the lifecycle mutation guard",
        ),
        (
            ".persist_identities(&identities)",
            "startup hosted identity projection must persist through the projection store",
        ),
        (
            "persist_registry_projection(&registry)",
            "stop lifecycle must persist the registry through the transaction/store boundary",
        ),
        (
            "persist_identity_projection(&identities)",
            "stop lifecycle must persist identities through the transaction/store boundary",
        ),
    )
    for token, detail in lifecycle_requirements:
        if token not in production_text:
            add("R22_AGENT_LIFECYCLE_PROJECTION_OWNER_FORK", agent_lifecycle, 1, detail)
    if production_text.count("agents::save_agents") > 2:
        add(
            "R22_AGENT_LIFECYCLE_PROJECTION_OWNER_FORK",
            agent_lifecycle,
            1,
            "production lifecycle must call agents::save_agents only inside AgentLifecycleProjectionStore",
        )
    if production_text.count("local_agents::save") > 2:
        add(
            "R22_AGENT_LIFECYCLE_PROJECTION_OWNER_FORK",
            agent_lifecycle,
            1,
            "production lifecycle must call local_agents::save only inside AgentLifecycleProjectionStore",
        )

cli_start = cli_root / "src/cli/commands/start.rs"
if cli_start.exists():
    start_text = source(cli_start)
    production_text = start_text.split("#[cfg(test)]", 1)[0]
    if "lifecycle::bootstrap_local_agent_projection(&plan)" not in production_text:
        add(
            "R22_AGENT_LIFECYCLE_PROJECTION_OWNER_FORK",
            cli_start,
            1,
            "cli start must delegate hosted identity projection to the Agent lifecycle owner",
        )
    if "local_agents::save" in production_text:
        add(
            "R22_AGENT_LIFECYCLE_PROJECTION_OWNER_FORK",
            cli_start,
            1,
            "cli start must not write local-agents.json directly",
        )

# Rule 33: public Agent read surfaces that join agents.json with
# local-agents.json must use a named aggregate snapshot owner. The first
# migrated surface is `agent.list`; letting it independently load the identity
# index would keep the read-side Agent aggregate split across two persistence
# files at the call site.
agent_aggregate = cli_root / "src/daemon/persistence/agent_aggregate.rs"
agent_list = cli_root / "src/daemon/ability/builtins/agents/list.rs"
catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
if agent_aggregate.exists():
    text = source(agent_aggregate)
    aggregate_requirements = (
        (
            "struct AgentAggregateSnapshot",
            "Agent aggregate reads require an immutable snapshot type",
        ),
        (
            "struct AgentAggregateRepository",
            "Agent aggregate reads require one repository owner",
        ),
        (
            "fn load_snapshot() -> anyhow::Result<AgentAggregateSnapshot>",
            "Agent aggregate repository must own paired snapshot loading",
        ),
        (
            "enum AgentAggregateSnapshotLoadError",
            "Agent aggregate repository must preserve registry-vs-identity load sources",
        ),
        (
            "fn try_load_snapshot(",
            "Agent aggregate repository must expose source-preserving snapshot loading",
        ),
        (
            "fn has_registered_agent(&self, agent: &str) -> bool",
            "Agent aggregate snapshot must own durable registry membership checks",
        ),
        (
            "fn hosted_llm_agent_identity(&self, agent: &str)",
            "Agent aggregate snapshot must own hosted LLM identity lookup",
        ),
        (
            "fn has_hosted_llm_agent_identity(&self, agent: &str) -> bool",
            "Agent aggregate snapshot must own hosted LLM identity presence checks",
        ),
        (
            "struct AgentLocalTargetProjection",
            "Agent aggregate snapshot must own local target projection shape",
        ),
        (
            "struct HostedAgentTarget",
            "Agent aggregate snapshot must own hosted Agent target parsing",
        ),
        (
            "Result<AgentLocalTargetProjection, AgentLocalTargetProjectionError>",
            "Agent aggregate snapshot must expose fail-closed local target projection to admission",
        ),
        (
            "enum AgentLocalTargetProjectionError",
            "Agent aggregate snapshot must type aggregate target projection failures",
        ),
        (
            "enum AgentRegisteredIdentityProjectionError",
            "Agent aggregate snapshot must type malformed registered-Agent identity projection",
        ),
        (
            "enum HostedAgentIdentityProjectionError",
            "Agent aggregate snapshot must type malformed hosted-Agent identity projection",
        ),
        (
            "struct AgentHostedPlacementProjection",
            "Agent aggregate snapshot must own hosted placement projection shape",
        ),
        (
            "struct AgentHostedPlacement",
            "Agent aggregate snapshot must own hosted placement entries",
        ),
        (
            "Result<AgentHostedPlacementProjection, HostedAgentIdentityProjectionError>",
            "Agent aggregate snapshot must expose fail-closed hosted placement projection to routing",
        ),
        (
            "enum HostedAgentNameLookupError",
            "Agent aggregate snapshot must own hosted Agent display-name lookup errors",
        ),
        (
            "struct HostedAgentIdentityProjection",
            "Agent aggregate snapshot must own hosted Agent identity projection shape",
        ),
        (
            "fn registered_agent_surface_names(",
            "Agent aggregate snapshot must expose checked registered Agent surface names",
        ),
        (
            "fn registered_agent_registry_projection(&self) -> AgentRegistry",
            "Agent aggregate snapshot must expose registered registry projection for provider adapters",
        ),
        (
            "fn hosted_agent_ura_by_name(",
            "Agent aggregate snapshot must expose hosted Agent lookup by display name",
        ),
        (
            "fn hosted_agent_identity_by_name(",
            "Agent aggregate snapshot must expose hosted Agent identity lookup by display name",
        ),
        (
            "fn hosted_agent_identity_by_ura(",
            "Agent aggregate snapshot must expose hosted Agent identity lookup by Agent URA",
        ),
        (
            "agent_registry::load_agents()",
            "Agent aggregate repository must load the durable registry projection",
        ),
        (
            "local_agents::load_for_fresh_host_projection()",
            "Agent aggregate repository must load the hosted-Agent identity projection through the explicit first-boot projection boundary",
        ),
    )
    for token, detail in aggregate_requirements:
        if token not in text:
            add("R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK", agent_aggregate, 1, detail)
else:
    add(
        "R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK",
        agent_aggregate,
        1,
        "Agent aggregate snapshot repository must exist",
    )
if agent_list.exists():
    text = source(agent_list)
    production_text = text.split("#[cfg(test)]", 1)[0]
    list_requirements = (
        (
            "AgentAggregateSnapshot",
            "agent.list must receive an aggregate snapshot provider",
        ),
        (
            "Fn() -> anyhow::Result<AgentAggregateSnapshot>",
            "agent.list provider contract must be aggregate-snapshot based",
        ),
        (
            "agent_rows(&snapshot)",
            "agent.list projection must read from the aggregate snapshot",
        ),
    )
    for token, detail in list_requirements:
        if token not in production_text:
            add("R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK", agent_list, 1, detail)
    if "local_agents::load()" in production_text:
        add(
            "R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK",
            agent_list,
            1,
            "agent.list production path must not independently load local-agents.json",
        )
if catalog_build.exists():
    text = source(catalog_build)
    if "agent_list_ability::register" in text and "AgentAggregateRepository::load_snapshot()" not in text:
        add(
            "R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK",
            catalog_build,
            1,
            "catalog build must register agent.list with the Agent aggregate snapshot repository",
        )
    if "agent.list: load durable agent registry" in text:
        add(
            "R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK",
            catalog_build,
            1,
            "catalog build must not wire agent.list to a registry-only loader",
        )

# Rule 34: hosted Agent authority proofs consume the Agent aggregate snapshot.
# Enrollment and durable-removal revocation both need the same paired
# agents.json/local-agents.json lifecycle fact. Dispatch may map those facts to
# authority-domain errors, but it must not reassemble the paired persistence
# reads inside the proof path.
ability_dispatch = cli_root / "src/daemon/ability/dispatch.rs"
if ability_dispatch.exists():
    text = source(ability_dispatch)
    production_text = text.split("#[cfg(test)]", 1)[0]
    authority_requirements = (
        (
            "AgentAggregateRepository::try_load_snapshot()",
            "hot Agent authority proofs must load through the aggregate snapshot repository",
        ),
        (
            "fn hot_agent_authority_snapshot_error(",
            "hot Agent authority proofs must preserve aggregate load source classification",
        ),
        (
            "HostedLlmAgentIdentity::Present(identity)",
            "hot Agent authority enrollment must consume typed hosted identity lookup",
        ),
        (
            "let registry_key = crate::core::agent::id::AgentId::parse(agent)",
            "hot Agent authority enrollment must project the surface name to the canonical registry key",
        ),
        (
            "snapshot.has_registered_agent(&registry_key)",
            "hot Agent authority enrollment must verify durable registry membership by canonical registry key",
        ),
        (
            "snapshot.host_device_ura()",
            "hot Agent authority enrollment must verify host-device binding from the snapshot",
        ),
        (
            "snapshot.has_registered_agent(&enrollment.agent)",
            "hot Agent authority revocation must verify durable registry absence from the snapshot",
        ),
        (
            "snapshot.has_hosted_llm_agent_identity(&enrollment.agent)",
            "hot Agent authority revocation must verify hosted identity absence from the snapshot",
        ),
    )
    for token, detail in authority_requirements:
        if token not in production_text:
            add("R34_HOT_AUTHORITY_AGGREGATE_SNAPSHOT_FORK", ability_dispatch, 1, detail)
    persisted_start = production_text.find("impl PersistedHotAgentAuthority")
    persisted_end = production_text.find("/// Receipt proving", persisted_start)
    if persisted_start >= 0 and persisted_end > persisted_start:
        persisted_body = production_text[persisted_start:persisted_end]
        for token, detail in (
            (
                "agent_registry::load_agents",
                "hot Agent authority enrollment must not load agents.json directly",
            ),
            (
                "local_agents::load",
                "hot Agent authority enrollment must not load local-agents.json directly",
            ),
        ):
            if token in persisted_body:
                add("R34_HOT_AUTHORITY_AGGREGATE_SNAPSHOT_FORK", ability_dispatch, 1, detail)
    revoke_body = rust_method_body(production_text, "revoke_after_durable_removal")
    if revoke_body is not None:
        _, body = revoke_body
        for token, detail in (
            (
                "agent_registry::load_agents",
                "hot Agent authority revocation must not load agents.json directly",
            ),
            (
                "local_agents::load",
                "hot Agent authority revocation must not load local-agents.json directly",
            ),
        ):
            if token in body:
                add("R34_HOT_AUTHORITY_AGGREGATE_SNAPSHOT_FORK", ability_dispatch, 1, detail)

# Rule 34c: lifecycle registry persistence uses canonical AgentId keys. Public
# lifecycle selectors may remain shorthand (`alice`), but durable agents.json
# rows must be keyed by canonical `default/alice`-style AgentIds, and hosted
# Agent bootstrap must project those keys back to local hosted names explicitly.
agent_lifecycle = cli_root / "src/daemon/ability/builtins/agents/lifecycle.rs"
bootstrap_profile = cli_root / "src/daemon/ability/catalog/profiles/bootstrap.rs"
agent_aggregate = cli_root / "src/daemon/persistence/agent_aggregate.rs"
agent_list = cli_root / "src/daemon/ability/builtins/agents/list.rs"
if agent_lifecycle.exists():
    text = source(agent_lifecycle)
    for token, detail in (
        (
            "let registry_key = agent_id.to_string();",
            "agent.start must derive a canonical durable registry key from AgentId",
        ),
        (
            "registry.agents.insert(registry_key.clone(), entry.clone())",
            "agent.start must persist under the canonical registry key",
        ),
        (
            "fn canonical_agent_registry_key(",
            "agent lifecycle stop/purge/refresh must share canonical registry key derivation",
        ),
        (
            "registry.agents.remove(&registry_key)",
            "agent.stop must remove by canonical registry key",
        ),
    ):
        if token not in text:
            add("R34C_AGENT_LIFECYCLE_CANONICAL_REGISTRY_KEY", agent_lifecycle, 1, detail)
    for token, detail in (
        (
            "registry.agents.insert(name.clone(), entry.clone())",
            "agent.start must not persist the public selector as the durable registry key",
        ),
        (
            "original_registry.agents.get(&name).cloned()",
            "agent.stop/purge must not lookup durable rows by public selector",
        ),
        (
            "registry.agents.remove(&name)",
            "agent.stop must not remove durable rows by public selector",
        ),
    ):
        if token in text:
            add(
                "R34C_AGENT_LIFECYCLE_CANONICAL_REGISTRY_KEY",
                agent_lifecycle,
                line_number(text, text.find(token)),
                detail,
            )
if bootstrap_profile.exists():
    text = source(bootstrap_profile)
    for token, detail in (
        (
            "pub fn llm_sub_agents_from_registry(",
            "hosted-agent bootstrap must centralize canonical registry key projection",
        ),
        (
            "AgentId::parse(key)",
            "hosted-agent bootstrap must parse registry keys as canonical AgentIds",
        ),
    ):
        if token not in text:
            add("R34C_AGENT_LIFECYCLE_CANONICAL_REGISTRY_KEY", bootstrap_profile, 1, detail)
if agent_aggregate.exists():
    text = source(agent_aggregate)
    body = rust_method_body(text, "has_registered_agent")
    if body is None:
        add(
            "R34C_AGENT_LIFECYCLE_CANONICAL_REGISTRY_KEY",
            agent_aggregate,
            1,
            "AgentAggregateSnapshot must retain a canonicalizing registered-agent lookup",
        )
    else:
        _, fn_body = body
        for token, detail in (
            (
                "AgentId::parse(agent)",
                "registered-agent lookup must parse public selectors into canonical AgentIds",
            ),
            (
                "contains_key(&registry_key)",
                "registered-agent lookup must query durable storage by canonical registry key",
            ),
        ):
            if token not in fn_body:
                add("R34C_AGENT_LIFECYCLE_CANONICAL_REGISTRY_KEY", agent_aggregate, 1, detail)
if agent_list.exists():
    text = source(agent_list)
    for token, detail in (
        (
            "AgentId::parse(registry_key)",
            "agent.list must parse durable registry keys before projecting display names",
        ),
        (
            "let name = agent_id.name.as_str();",
            "agent.list must expose hosted local names, not durable registry keys",
        ),
    ):
        if token not in text:
            add("R34C_AGENT_LIFECYCLE_CANONICAL_REGISTRY_KEY", agent_list, 1, detail)

# Rule 34b: hot Agent runtime row materialization must consume the authority
# root allocated by the catalogue enrollment transaction. The registrar owns
# runtime transactions; it must not re-open local-agents.json display-name
# lookup as a second durable identity source.
hot_agent_registrar = cli_root / "src/daemon/axon_bridge/hot_agent_registrar.rs"
if hot_agent_registrar.exists():
    text = source(hot_agent_registrar)
    production_text = text.split("#[cfg(test)]", 1)[0]
    register_body = rust_method_body(production_text, "register_agent_replacing")
    if "struct HostedAgentRuntimeBinding" in production_text and register_body is None:
        add(
            "R34B_HOT_AGENT_RUNTIME_BINDING_AGGREGATE_FORK",
            hot_agent_registrar,
            1,
            "HotAgentRegistrar must retain an explicit authority-enrolled runtime binding path",
        )
    if register_body is not None:
        offset, body = register_body
        for token, detail in (
            (
                ".enroll_persisted_hot_agent_authority(name)",
                "hot Agent runtime binding must start from the catalogue authority enrollment",
            ),
            (
                "agent_ura: enrollment.authority_root().to_string()",
                "hot Agent runtime binding must use the enrolled authority root",
            ),
        ):
            if token not in body:
                add(
                    "R34B_HOT_AGENT_RUNTIME_BINDING_AGGREGATE_FORK",
                    hot_agent_registrar,
                    line_number(production_text, offset),
                    detail,
                )
        for token, detail in (
            (
                "local_agents::load",
                "hot Agent runtime binding must not load local-agents.json directly",
            ),
            (
                "lookup_hosted_ura",
                "hot Agent runtime binding must not bypass catalogue authority enrollment",
            ),
            (
                "lookup_hosted_agent_by_name",
                "hot Agent runtime binding must not bypass catalogue authority enrollment",
            ),
            (
                "AgentAggregateRepository::try_load_snapshot()",
                "hot Agent runtime binding must not perform a separate aggregate lookup after authority enrollment",
            ),
        ):
            if token in body:
                add(
                    "R34B_HOT_AGENT_RUNTIME_BINDING_AGGREGATE_FORK",
                    hot_agent_registrar,
                    line_number(production_text, offset),
                    detail,
                )

# Rule 35: admission Agent self-target locality consumes the Agent aggregate
# snapshot. TargetGate gates unary, stream, and bidi dispatch. If it
# independently loads agents.json and local-agents.json, an invocation can be
# accepted or rejected from a split source-of-truth view of hosted Agent
# identity.
target_gate = cli_root / "src/daemon/invocation/admission/target_gate.rs"
if target_gate.exists():
    text = source(target_gate)
    production_text = text.split("#[cfg(test)]", 1)[0]
    target_gate_requirements = (
        (
            "AgentAggregateRepository::try_load_snapshot()",
            "TargetGate Agent locality must load through the aggregate snapshot repository",
        ),
        (
            "enum LocalAgentTargetProjectionState",
            "TargetGate must model Agent projection availability explicitly",
        ),
        (
            "snapshot.local_target_projection()",
            "TargetGate must consume the aggregate-owned Agent locality projection",
        ),
        (
            "LocalAgentTargetProjectionState::Unavailable",
            "TargetGate must make aggregate fail-closed behavior observable",
        ),
        (
            "HostedAgentTarget::parse(target_ura)",
            "TargetGate must parse inbound Agent URAs through the aggregate-owned target identity",
        ),
    )
    for token, detail in target_gate_requirements:
        if token not in production_text:
            add("R35_TARGET_GATE_AGENT_AGGREGATE_FORK", target_gate, 1, detail)
    for token, detail in (
        (
            "agent_registry::load_agents",
            "TargetGate must not load agents.json directly for Agent locality",
        ),
        (
            "local_agents::load",
            "TargetGate must not load local-agents.json directly for Agent locality",
        ),
    ):
        if token in production_text:
            add("R35_TARGET_GATE_AGENT_AGGREGATE_FORK", target_gate, 1, detail)
    for token, detail in (
        (
            ".local_agents",
            "TargetGate must not inspect AgentAggregateSnapshot.local_agents directly",
        ),
        (
            ".registry",
            "TargetGate must not inspect AgentAggregateSnapshot.registry directly",
        ),
    ):
        if token in production_text:
            add("R35_TARGET_GATE_AGENT_AGGREGATE_FORK", target_gate, 1, detail)

# Rule 36: ability-health scans consume the Agent aggregate snapshot. Health
# metadata is public catalog state; the scan must not independently join
# agents.json and local-agents.json or it can stamp records from a split
# source-of-truth view.
ability_health = cli_root / "src/daemon/ability/health.rs"
if ability_health.exists():
    text = source(ability_health)
    production_text = text.split("#[cfg(test)]", 1)[0]
    health_requirements = (
        (
            "AgentAggregateRepository::try_load_snapshot()",
            "ability health scan must load through the Agent aggregate repository",
        ),
        (
            "health_scan_snapshot_error",
            "ability health scan must preserve aggregate load source classification",
        ),
        (
            "agent_snapshot.registered_agents()",
            "ability health scan must iterate registry rows through the aggregate snapshot",
        ),
        (
            "agent_snapshot.hosted_llm_agent_ura(agent_name)",
            "ability health scan must resolve hosted LLM owners through the aggregate snapshot",
        ),
    )
    for token, detail in health_requirements:
        if token not in production_text:
            add("R36_ABILITY_HEALTH_AGENT_AGGREGATE_FORK", ability_health, 1, detail)
    for token, detail in (
        (
            "agent_registry::load_agents",
            "ability health scan must not load agents.json directly",
        ),
        (
            "local_agents::load",
            "ability health scan must not load local-agents.json directly",
        ),
        (
            "lookup_hosted_ura",
            "ability health scan must not bypass aggregate hosted LLM owner resolution",
        ),
    ):
        if token in production_text:
            add("R36_ABILITY_HEALTH_AGENT_AGGREGATE_FORK", ability_health, 1, detail)

# Rule 37: namespace route hosted-Agent placement consumes the Agent aggregate
# projection. Route resolution is part of invocation proof selection; it must
# not own the local-agents.json file layout or silently revive a split
# hosted-Agent placement read model.
route_resolver = cli_root / "src/daemon/invocation/routing/route_resolver.rs"
if route_resolver.exists():
    text = source(route_resolver)
    production_text = text.split("\n#[cfg(test)]\nmod tests", 1)[0]
    route_placement_requirements = (
        (
            "AgentAggregateRepository::try_load_snapshot()",
            "route resolver hosted placement must load through the Agent aggregate repository",
        ),
        (
            "AgentHostedPlacementProjection",
            "route resolver hosted placement must consume the aggregate placement projection",
        ),
        (
            "fn from_projection(projection: AgentHostedPlacementProjection) -> Self",
            "route resolver hosted placement must convert from the aggregate projection",
        ),
        (
            "enum HostedPlacementProjectionState",
            "route resolver hosted placement must model projection availability explicitly",
        ),
        (
            "HostedPlacementProjectionState::Unavailable",
            "route resolver hosted placement must fail closed when projection is unavailable",
        ),
        (
            "snapshot.hosted_agent_placements()",
            "route resolver hosted placement must use the aggregate-owned placement projection",
        ),
    )
    for token, detail in route_placement_requirements:
        if token not in production_text:
            add("R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK", route_resolver, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            "route resolver must not load local-agents.json directly for hosted placement",
        ),
        (
            "LocalAgentsFile",
            "route resolver must not inspect the local-agents.json file shape",
        ),
        (
            "fn from_file(",
            "route resolver must not own hosted placement projection from persistence files",
        ),
    ):
        if token in production_text:
            add("R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK", route_resolver, 1, detail)

    # Descriptor-bound route selection must be a fail-closed parse state, not
    # an Option pipeline that collapses malformed descriptor refs into a
    # generic route-shape miss.
    selector_body = rust_method_body(production_text, "route_selector_from_query")
    if selector_body is None:
        add(
            "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
            route_resolver,
            1,
            "route selector must remain inspectable",
        )
    else:
        offset, body = selector_body
        signature = production_text[offset : production_text.find("{", offset)]
        if "Result<Option<RouteSelector>, ResolveRouteFailure>" not in signature:
            add(
                "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
                route_resolver,
                line_number(production_text, offset),
                "route selector must preserve descriptor-ref parse failures as typed route failures",
            )
        if "ability_selector_from_descriptor_ref(query_name)?" in body:
            add(
                "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
                route_resolver,
                line_number(production_text, offset),
                "descriptor-ref must not stand in for an owner query",
            )
        for token, detail in (
            (
                "looks_like_descriptor_ref(query_name)",
                "route selector must recognize descriptor-like owner queries explicitly",
            ),
            (
                "descriptor_ref cannot stand in for an owner query",
                "descriptor-like owner queries must fail closed instead of resolving as routes",
            ),
            (
                "route_selector_from_descriptor_ref(owner_ura, ability_name).map(Some)",
                "target-bound descriptor-ref parsing must still propagate selector failures",
            ),
        ):
            if token not in body:
                add(
                    "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
                    route_resolver,
                    line_number(production_text, offset),
                    detail,
                )
    descriptor_body = rust_method_body(production_text, "ability_selector_from_descriptor_ref")
    if descriptor_body is None:
        add(
            "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
            route_resolver,
            1,
            "descriptor-ref ability selector must remain inspectable",
        )
    else:
        offset, body = descriptor_body
        signature = production_text[offset : production_text.find("{", offset)]
        if "Result<crate::core::ura::AbilitySelector, ResolveRouteFailure>" not in signature:
            add(
                "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
                route_resolver,
                line_number(production_text, offset),
                "descriptor-ref selector must return a typed route failure",
            )
        descriptor_ref_legacy_patterns = (
            (
                "canonical_ability_descriptor_ref(descriptor_ref).ok()",
                "descriptor-ref canonicalization failures must not be collapsed into None",
            ),
            (
                "ability_ura_from_descriptor_ref(&descriptor_ref).ok()",
                "descriptor-ref ability extraction failures must not be collapsed into None",
            ),
            (
                ".ok()?",
                "descriptor-ref ability extraction failures must not be collapsed into None",
            ),
            (
                "AbilitySelector::parse(&ability_ura).ok()",
                "descriptor-ref ability selector parse failures must not be collapsed into None",
            ),
        )
        compact_body = re.sub(r"\s+", "", body)
        for pattern, detail in descriptor_ref_legacy_patterns:
            compact_pattern = re.sub(r"\s+", "", pattern)
            if pattern in body:
                add(
                    "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
                    route_resolver,
                    line_number(production_text, offset + body.find(pattern)),
                    detail,
                )
            elif compact_pattern in compact_body:
                add(
                    "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK",
                    route_resolver,
                    line_number(production_text, offset),
                    detail,
                )

# Rule 38: Mission child-target proof consumes Agent aggregate projections.
# Mission execution creates child Invocations; target proof must not duplicate
# the Agent persistence file layout while deciding whether EAL device targets
# collide with Agents or resolving hosted Agent callees.
mission_orchestration = cli_root / "src/daemon/execution/mission/orchestration.rs"
mission_gateway = cli_root / "src/daemon/execution/mission/invocation_gateway.rs"
if mission_orchestration.exists():
    text = source(mission_orchestration)
    production_text = text.split("#[cfg(test)]", 1)[0]
    orchestration_requirements = (
        (
            "AgentAggregateRepository::load_snapshot()",
            "Mission implicit-Agent fallback guard must load through the Agent aggregate repository",
        ),
        (
            "snapshot.registered_agent_surface_names()",
            "Mission implicit-Agent fallback guard must consume aggregate surface names",
        ),
    )
    for token, detail in orchestration_requirements:
        if token not in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", mission_orchestration, 1, detail)
    if "agent_registry::load_agents" in production_text:
        add(
            "R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK",
            mission_orchestration,
            1,
            "Mission orchestration must not load agents.json directly for target collision proof",
        )
if mission_gateway.exists():
    text = source(mission_gateway)
    production_text = text.split("#[cfg(test)]", 1)[0]
    gateway_requirements = (
        (
            "AgentAggregateRepository::try_load_snapshot()",
            "Mission child target resolver must load through the Agent aggregate repository",
        ),
        (
            "hosted_agent_ura_by_name(agent_name)",
            "Mission child target resolver must consume aggregate hosted Agent lookup",
        ),
        (
            "HostedAgentNameLookupError",
            "Mission child target resolver must preserve hosted Agent lookup error classification",
        ),
    )
    for token, detail in gateway_requirements:
        if token not in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", mission_gateway, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            "Mission child target resolver must not load local-agents.json directly",
        ),
        (
            "lookup_hosted_agent_by_name",
            "Mission child target resolver must not bypass aggregate hosted Agent lookup",
        ),
    ):
        if token in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", mission_gateway, 1, detail)

hosted_name_surfaces = (
    (
        cli_root / "src/cli/commands/teach.rs",
        "CLI teach learner resolution",
    ),
)
for hosted_name_surface, surface_label in hosted_name_surfaces:
    if not hosted_name_surface.exists():
        continue
    text = source(hosted_name_surface)
    production_text = text.split("#[cfg(test)]", 1)[0]
    hosted_name_requirements = (
        (
            "AgentAggregateRepository::try_load_snapshot()",
            f"{surface_label} must load through the Agent aggregate repository",
        ),
        (
            "hosted_agent_ura_by_name(",
            f"{surface_label} must use aggregate hosted Agent display-name lookup",
        ),
        (
            "HostedAgentNameLookupError",
            f"{surface_label} must preserve hosted Agent lookup error classification",
        ),
    )
    for token, detail in hosted_name_requirements:
        if token not in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", hosted_name_surface, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            f"{surface_label} must not load local-agents.json directly for display-name lookup",
        ),
        (
            "lookup_hosted_agent_by_name",
            f"{surface_label} must not bypass aggregate hosted Agent display-name lookup",
        ),
    ):
        if token in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", hosted_name_surface, 1, detail)

local_daemon_grpc = cli_root / "src/support/platform/local_daemon_grpc.rs"
if local_daemon_grpc.exists():
    text = source(local_daemon_grpc)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "hosted_agent_ura: &str",
            "local daemon hosted delegation must require an explicit canonical Agent URA",
        ),
        (
            "HostedAgentDelegationRequest::new(hosted_agent_ura)",
            "local daemon hosted delegation must validate the supplied canonical Agent URA",
        ),
    ):
        if token not in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", local_daemon_grpc, 1, detail)
    for token, detail in (
        (
            "AgentAggregateRepository",
            "local daemon transport must not load product Agent state to resolve a callee",
        ),
        (
            "hosted_agent_ura_by_name(",
            "local daemon transport must not resolve hosted Agent display names",
        ),
        (
            "HostedAgentNameLookupError",
            "local daemon transport must not own hosted Agent lookup error policy",
        ),
    ):
        if token in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", local_daemon_grpc, 1, detail)

governance_teach = cli_root / "src/daemon/ability/builtins/governance/teach.rs"
if governance_teach.exists():
    text = source(governance_teach)
    production_text = text.split("#[cfg(test)]", 1)[0]
    teach_requirements = (
        (
            "AgentAggregateRepository::load_snapshot()",
            "governance teach hosted identity authorization must load through the Agent aggregate repository",
        ),
        (
            "hosted_agent_identity_by_name(",
            "governance teach hosted identity authorization must use aggregate display-name identity lookup",
        ),
        (
            "hosted_agent_identity_by_ura(",
            "governance teach hosted identity authorization must use aggregate Agent URA membership lookup",
        ),
        (
            "HostedAgentNameLookupError",
            "governance teach hosted identity authorization must preserve hosted Agent lookup error classification",
        ),
    )
    for token, detail in teach_requirements:
        if token not in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", governance_teach, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            "governance teach hosted identity authorization must not load local-agents.json directly",
        ),
        (
            "lookup_hosted_agent_by_name",
            "governance teach hosted identity authorization must not bypass aggregate hosted Agent lookup",
        ),
        (
            "LocalAgentsFile",
            "governance teach hosted identity authorization must not inspect local-agents.json shape",
        ),
    ):
        if token in production_text:
            add("R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK", governance_teach, 1, detail)

local_agents_identity_file = cli_root / "src/daemon/persistence/local_agents.rs"
if local_agents_identity_file.exists():
    text = source(local_agents_identity_file)
    production_text = text.split("#[cfg(test)]", 1)[0]
    if "fn lookup_hosted_agent_by_name" in production_text:
        add(
            "R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK",
            local_agents_identity_file,
            line_number(production_text, production_text.find("fn lookup_hosted_agent_by_name")),
            "hosted Agent display-name lookup must live on AgentAggregateSnapshot, not local-agents file helpers",
        )

# Rule 39: chat hot-added Agent providers consume Agent aggregate projections.
# Chat is an invocation-facing surface: hot-added discover handlers and
# peer-skill hints must not revive registry-only reads after the Agent aggregate
# became the read owner for registered + hosted Agent state. The retired
# `<agent>.invoke` handler must not be reintroduced merely to satisfy this gate.
agent_chat = cli_root / "src/daemon/ability/builtins/agents/chat.rs"
if agent_chat.exists():
    text = source(agent_chat)
    production_text = text.split("#[cfg(test)]", 1)[0]
    chat_requirements = (
        (
            "AgentAggregateRepository::load_snapshot()",
            "Agent chat provider reads must load through the Agent aggregate repository",
        ),
        (
            "snapshot.registered_agents()",
            "Agent chat peer-skill enumeration must iterate aggregate registered Agents",
        ),
    )
    for token, detail in chat_requirements:
        if token not in production_text:
            add("R39_AGENT_CHAT_AGGREGATE_PROVIDER_FORK", agent_chat, 1, detail)
    if "agent_registry::load_agents" in production_text:
        add(
            "R39_AGENT_CHAT_AGGREGATE_PROVIDER_FORK",
            agent_chat,
            1,
            "Agent chat provider path must not load agents.json directly",
        )

# Rule 40: governance status surfaces consume Agent aggregate hosted-identity
# projections. Operator/status reads may expose hosted-Agent identity facts, but
# they must not learn the LocalAgentsFile persistence shape or create a second
# source-of-truth beside AgentAggregateRepository.
governance_status_surfaces = (
    (
        cli_root / "src/daemon/ability/builtins/governance/admin_status.rs",
        "admin.status",
    ),
    (
        cli_root / "src/daemon/ability/builtins/governance/network_health.rs",
        "observe.network_health",
    ),
    (
        cli_root / "src/daemon/ability/builtins/governance/meta.rs",
        "meta.describe",
    ),
    (
        cli_root / "src/daemon/ability/builtins/governance/invocation_history.rs",
        "invocation history",
    ),
)
if agent_aggregate.exists():
    text = source(agent_aggregate)
    aggregate_status_requirements = (
        (
            "struct AgentHostedIdentityStatus",
            "Agent aggregate must own hosted identity status projection shape",
        ),
        (
            "fn hosted_identity_status(&self) -> AgentHostedIdentityStatus",
            "Agent aggregate snapshot must expose hosted identity status projection",
        ),
        (
            "fn load_hosted_identity_status() -> anyhow::Result<AgentHostedIdentityStatus>",
            "Agent aggregate repository must expose hosted identity status reads",
        ),
        (
            "fn load_hosted_identity_projection() -> Result<LocalAgentsFile, AgentAggregateSnapshotLoadError>",
            "Agent aggregate repository must own hosted identity persistence loading",
        ),
    )
    for token, detail in aggregate_status_requirements:
        if token not in text:
            add("R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK", agent_aggregate, 1, detail)
for governance_status_surface, surface_label in governance_status_surfaces:
    if not governance_status_surface.exists():
        continue
    text = source(governance_status_surface)
    production_text = text.split("#[cfg(test)]", 1)[0]
    if (
        surface_label != "invocation history"
        and "AgentAggregateRepository::load_hosted_identity_status()" not in production_text
    ):
        add(
            "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
            governance_status_surface,
            1,
            f"{surface_label} must read hosted identity status through the Agent aggregate repository",
        )
    for token, detail in (
        (
            "local_agents::load",
            f"{surface_label} must not load local-agents.json directly",
        ),
        (
            "LocalAgentsFile",
            f"{surface_label} must not inspect local-agents.json shape",
        ),
        (
            ".host_device_ura.",
            f"{surface_label} must not inspect host-device Agent URA storage directly",
        ),
        (
            ".hosted_agents",
            f"{surface_label} must not inspect hosted-Agent storage rows directly",
        ),
    ):
        if token in production_text:
            add("R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK", governance_status_surface, 1, detail)
    if surface_label == "invocation history":
        for retired, detail in (
            (
                "AgentAggregateRepository",
                "invocation history must not infer ledger governance from product Agent state",
            ),
            (
                "load_hosted_identity_status",
                "invocation history must not read ambient identity state during request handling",
            ),
            (
                "host_device_ura",
                "invocation history must not bind a generic runtime ledger to a product identity shape",
            ),
        ):
            if retired in production_text:
                add(
                    "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                    governance_status_surface,
                    line_number(production_text, production_text.find(retired)),
                    detail,
                )
        if "ledger_governance_authority()" not in production_text:
            add(
                "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                governance_status_surface,
                1,
                "invocation history must register through one catalog-owned ledger governance authority",
            )
        if "local_runtime_owners()" in production_text:
            add(
                "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                governance_status_surface,
                1,
                "invocation history must not fan out one daemon ledger across all runtime owners",
            )
        ledger_body = rust_method_body(
            production_text, "ledger_resource_ura_from_runtime_owner_ura"
        )
        if ledger_body is None:
            add(
                "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                governance_status_surface,
                1,
                "invocation history ledger URA projection must accept the catalog runtime owner",
            )
        else:
            offset, body = ledger_body
            signature = production_text[offset : production_text.find("{", offset)]
            if "anyhow::Result<String>" not in signature:
                add(
                    "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                    governance_status_surface,
                    line_number(production_text, offset),
                    "invocation history ledger URA projection must require a canonical runtime owner",
                )
            if "AgentAggregateRepository" in body:
                add(
                    "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                    governance_status_surface,
                    line_number(
                        production_text,
                        offset + body.find("AgentAggregateRepository"),
                    ),
                    "invocation history ledger URA projection must not depend on product repositories",
                )
            if "parse_ura(" not in body:
                add(
                    "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                    governance_status_surface,
                    line_number(production_text, offset),
                    "invocation history ledger URA parsing must remain in the fallible projection helper",
                )
            if "invocation_ledger_resource_ura(" not in production_text:
                add(
                    "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK",
                    governance_status_surface,
                    line_number(production_text, offset),
                    "invocation history ledger URA projection must use Axon's canonical owner-kind projection",
                )

# Rule 41: EAL agent dispatch consumes the Agent repository-owned registered
# Agent registry projection. EAL member calls are execution paths; constructing
# their dispatcher from a direct registry file read revives a second read owner
# for Agent dispatch proof, while projecting registry load failures as empty
# state hides unavailable runtime authority.
eal_dispatch = cli_root / "src/eal/interpreter/dispatch.rs"
if eal_dispatch.exists():
    text = source(eal_dispatch)
    production_text = text.split("#[cfg(test)]", 1)[0]
    if "AgentAggregateRepository::load_registered_agent_registry_projection()" not in production_text:
        add(
            "R41_EAL_AGENT_DISPATCH_AGGREGATE_FORK",
            eal_dispatch,
            1,
            "EAL AgentAwareDispatcher must consume the repository-owned registered-Agent projection",
        )
    if "agent_registry::load_agents" in production_text:
        add(
            "R41_EAL_AGENT_DISPATCH_AGGREGATE_FORK",
            eal_dispatch,
            1,
            "EAL AgentAwareDispatcher must not load agents.json directly",
        )
    for token in ("unwrap_or_default()", "AgentRegistry::default()"):
        if token in production_text:
            add(
                "R41_EAL_AGENT_DISPATCH_AGGREGATE_FORK",
                eal_dispatch,
                1,
                "EAL AgentAwareDispatcher must not project registry load failures as empty registry",
            )

# Rule 42: hosted owner lookup surfaces consume Agent aggregate projections.
# CLI selector parsing resolves a user-provided Agent label through the Agent
# aggregate. Local discovery is different: committed descriptors already carry
# their canonical owner URA and must not rediscover it from authoring state.
hosted_owner_lookup_surfaces = (
    (
        cli_root / "src/cli/commands/abilities.rs",
        "CLI abilities --agent resolution",
        (
            "AgentAggregateRepository::load_hosted_identity_snapshot()",
            "hosted_llm_agent_ura(",
        ),
    ),
)
for hosted_owner_surface, surface_label, required_tokens in hosted_owner_lookup_surfaces:
    if not hosted_owner_surface.exists():
        continue
    text = source(hosted_owner_surface)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token in required_tokens:
        if token not in production_text:
            add(
                "R42_HOSTED_OWNER_LOOKUP_AGENT_AGGREGATE_FORK",
                hosted_owner_surface,
                1,
                f"{surface_label} must consume hosted owner identity from its Agent aggregate boundary ({token})",
            )
    for token, detail in (
        (
            "local_agents::load",
            f"{surface_label} must not load local-agents.json directly for hosted owner lookup",
        ),
        (
            "lookup_hosted_ura",
            f"{surface_label} must not bypass aggregate hosted owner lookup",
        ),
    ):
        if token in production_text:
            add("R42_HOSTED_OWNER_LOOKUP_AGENT_AGGREGATE_FORK", hosted_owner_surface, 1, detail)

discover_owner_surface = cli_root / "src/daemon/ability/builtins/agents/discover.rs"
if discover_owner_surface.exists():
    text = source(discover_owner_surface)
    production_text = text.split("#[cfg(test)]", 1)[0]
    if "let owner = row.descriptor.owner_ura.clone();" not in production_text:
        add(
            "R42_HOSTED_OWNER_LOOKUP_AGENT_AGGREGATE_FORK",
            discover_owner_surface,
            1,
            "local discover owner projection must consume the committed descriptor owner_ura",
        )
    if "hosted_llm_agent_ura(peer_name)" in production_text:
        add(
            "R42_HOSTED_OWNER_LOOKUP_AGENT_AGGREGATE_FORK",
            discover_owner_surface,
            1,
            "local discover must not re-resolve committed descriptor owners through authoring state",
        )

# Rule 43: host descriptor catalog identity consumes the Agent aggregate
# hosted-identity projection. The descriptor catalog may validate descriptor
# owners, but it must not own the local-agents.json file layout while deriving
# device, MCP, or LLM profile owners. Consent is daemon-native governance
# owned by the consent-management SystemAgent, not a hosted Agent profile.
host_descriptor_catalog = cli_root / "src/daemon/ability/catalog/profiles/mod.rs"
if agent_aggregate.exists():
    text = source(agent_aggregate)
    aggregate_requirements = (
        (
            "struct AgentHostDescriptorIdentityProjection",
            "Agent aggregate must own the host descriptor identity projection shape",
        ),
        (
            "fn host_descriptor_identity_projection(",
            "hosted identity snapshot must expose descriptor identity projection",
        ),
        (
            "fn llm_agent_uras(&self) -> &[(String, String)]",
            "Agent aggregate projection must own hosted LLM owner enumeration",
        ),
    )
    for token, detail in aggregate_requirements:
        if token not in text:
            add("R43_HOST_DESCRIPTOR_IDENTITY_AGGREGATE_FORK", agent_aggregate, 1, detail)
if host_descriptor_catalog.exists():
    text = source(host_descriptor_catalog)
    production_text = text.split("#[cfg(test)]", 1)[0]
    catalog_requirements = (
        (
            "AgentAggregateRepository::load_hosted_identity_snapshot()",
            "host descriptor catalog must load hosted identity state through the Agent aggregate repository",
        ),
        (
            "host_descriptor_identity_projection()",
            "host descriptor catalog must consume the aggregate descriptor identity projection",
        ),
    )
    for token, detail in catalog_requirements:
        if token not in production_text:
            add("R43_HOST_DESCRIPTOR_IDENTITY_AGGREGATE_FORK", host_descriptor_catalog, 1, detail)
    projection_method_requirements = (
        (
            r"projection\s*\.\s*host_device_ura\s*\(",
            "host descriptor catalog must read host device owner through the aggregate projection",
        ),
        (
            r"projection\s*\.\s*llm_agent_uras\s*\(",
            "host descriptor catalog must enumerate LLM owners through the aggregate projection",
        ),
    )
    for pattern, detail in projection_method_requirements:
        if not re.search(pattern, production_text):
            add("R43_HOST_DESCRIPTOR_IDENTITY_AGGREGATE_FORK", host_descriptor_catalog, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            "host descriptor catalog must not load local-agents.json directly",
        ),
        (
            "lookup_hosted_ura",
            "host descriptor catalog must not bypass aggregate profile owner lookup",
        ),
        (
            "LocalAgentsFile",
            "host descriptor catalog must not inspect hosted identity file shape",
        ),
        (
            "local.hosted_agents",
            "host descriptor catalog must not inspect hosted identity rows directly",
        ),
        (
            "local.host_device_ura",
            "host descriptor catalog must not inspect host-device storage directly",
        ),
    ):
        if token in production_text:
            add("R43_HOST_DESCRIPTOR_IDENTITY_AGGREGATE_FORK", host_descriptor_catalog, 1, detail)

# Rule 44: host device URA read surfaces consume the Agent aggregate
# hosted-identity status. These callers only need the persisted host device URA;
# they must not open local-agents.json or inspect its storage field directly.
local_device_ura_surfaces = (
    (
        cli_root / "src/daemon/identity/local_invocation.rs",
        "daemon local invocation identity",
    ),
    (
        cli_root / "src/daemon/resources/context/clipboard_tracker.rs",
        "clipboard context tracker",
    ),
)
for local_device_surface, surface_label in local_device_ura_surfaces:
    if not local_device_surface.exists():
        continue
    text = source(local_device_surface)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "AgentAggregateRepository::load_hosted_identity_status()",
            f"{surface_label} must load host device URA through the hosted identity aggregate status",
        ),
        (
            "host_device_ura()",
            f"{surface_label} must consume the aggregate host device URA projection",
        ),
    ):
        if token not in production_text:
            add("R44_LOCAL_DEVICE_URA_AGENT_AGGREGATE_FORK", local_device_surface, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            f"{surface_label} must not load local-agents.json directly for host device URA lookup",
        ),
        (
            "LocalAgentsFile",
            f"{surface_label} must not inspect hosted identity file shape",
        ),
        (
            "local.host_device_ura",
            f"{surface_label} must not inspect host-device storage directly",
        ),
        (
            "file.host_device_ura",
            f"{surface_label} must not inspect host-device storage directly",
        ),
    ):
        if token in production_text:
            add("R44_LOCAL_DEVICE_URA_AGENT_AGGREGATE_FORK", local_device_surface, 1, detail)

# Rule 45: hosted authority-root enumeration consumes the Agent aggregate
# hosted-identity projection. Ability authority contexts need hosted Agent URAs,
# but the public persistence facade must not reopen local-agents.json or own
# its row layout.
persistence_facade = cli_root / "src/daemon/persistence/mod.rs"
if agent_aggregate.exists():
    text = source(agent_aggregate)
    if "fn hosted_agent_authority_roots(&self) -> anyhow::Result<Vec<String>>" not in text:
        add(
            "R45_HOSTED_AUTHORITY_ROOTS_AGENT_AGGREGATE_FORK",
            agent_aggregate,
            1,
            "Agent hosted identity snapshot must expose hosted authority roots",
        )
if persistence_facade.exists():
    text = source(persistence_facade)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "pub fn hosted_agent_authority_roots() -> anyhow::Result<Vec<String>>",
            "public hosted authority roots facade must remain available",
        ),
        (
            "AgentAggregateRepository::load_hosted_identity_snapshot()",
            "hosted authority roots facade must load through the Agent aggregate hosted identity snapshot",
        ),
        (
            ".hosted_agent_authority_roots()",
            "hosted authority roots facade must consume the aggregate projection method",
        ),
    ):
        if token not in production_text:
            add("R45_HOSTED_AUTHORITY_ROOTS_AGENT_AGGREGATE_FORK", persistence_facade, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            "hosted authority roots facade must not load local-agents.json directly",
        ),
        (
            "LocalAgentsFile",
            "hosted authority roots facade must not inspect hosted identity file shape",
        ),
        (
            ".hosted_agents",
            "hosted authority roots facade must not inspect hosted identity rows directly",
        ),
    ):
        if token in production_text:
            add("R45_HOSTED_AUTHORITY_ROOTS_AGENT_AGGREGATE_FORK", persistence_facade, 1, detail)

# Rule 46: agent.list row projection consumes Agent aggregate rows. The ability
# may format the public JSON row, but it must not split the aggregate back into
# raw registry and local-agents file shapes for hosted owner lookup.
agent_list_ability = cli_root / "src/daemon/ability/builtins/agents/list.rs"
if agent_list_ability.exists():
    text = source(agent_list_ability)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "fn agent_rows(snapshot: &AgentAggregateSnapshot)",
            "agent.list rows must take the Agent aggregate snapshot as the row contract",
        ),
        (
            ".registered_agents()",
            "agent.list rows must enumerate registry rows through the aggregate projection",
        ),
        (
            ".hosted_llm_agent_ura(",
            "agent.list rows must resolve hosted LLM owner URAs through the aggregate projection",
        ),
    ):
        if token not in production_text:
            add("R46_AGENT_LIST_AGGREGATE_ROW_FORK", agent_list_ability, 1, detail)
    for token, detail in (
        (
            "LocalAgentsFile",
            "agent.list production code must not mention the hosted identity file shape",
        ),
        (
            "lookup_hosted_ura",
            "agent.list production code must not bypass aggregate hosted owner lookup",
        ),
        (
            "snapshot.registry",
            "agent.list production code must not split registry out of the aggregate row contract",
        ),
        (
            "snapshot.local_agents",
            "agent.list production code must not split hosted identity out of the aggregate row contract",
        ),
    ):
        if token in production_text:
            add("R46_AGENT_LIST_AGGREGATE_ROW_FORK", agent_list_ability, 1, detail)

# Rule 47: hosted-Agent session advertisement uses the Agent aggregate hosted
# identity projection. The bidi session prelude may publish entries, but it must
# not know the local-agents.json shape or rebuild hosted-agent rows from storage.
session_prelude = cli_root / "src/daemon/invocation/bidi/session_initiator/prelude.rs"
if agent_aggregate.exists():
    text = source(agent_aggregate)
    for token, detail in (
        (
            "struct AgentHostedAdvertiseEntry",
            "Agent aggregate must own the hosted-agent advertise entry type",
        ),
        (
            "fn hosted_advertise_entries(",
            "hosted identity snapshot must expose hosted-agent advertise entries",
        ),
        (
            "fn short_label(&self) -> &str",
            "hosted-agent advertise entries must expose presentation labels without leaking storage rows",
        ),
    ):
        if token not in text:
            add("R47_HOSTED_ADVERTISE_PRELUDE_AGENT_AGGREGATE_FORK", agent_aggregate, 1, detail)

if session_prelude.exists():
    text = source(session_prelude)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "AgentAggregateRepository::load_hosted_identity_snapshot()",
            "hosted-agent advertise prelude must load hosted identity through the Agent aggregate",
        ),
        (
            ".hosted_advertise_entries(&realm, &user_segment)",
            "hosted-agent advertise prelude must consume aggregate advertise entries",
        ),
        (
            "AgentHostedAdvertiseEntry",
            "hosted-agent advertise prelude must use the aggregate advertise entry type",
        ),
    ):
        if token not in production_text:
            add("R47_HOSTED_ADVERTISE_PRELUDE_AGENT_AGGREGATE_FORK", session_prelude, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            "hosted-agent advertise prelude must not load local-agents.json directly",
        ),
        (
            "LocalAgentsFile",
            "hosted-agent advertise prelude must not mention the hosted identity file shape",
        ),
        (
            ".hosted_agents",
            "hosted-agent advertise prelude must not inspect hosted identity rows directly",
        ),
        (
            "collect_advertise_entries",
            "hosted-agent advertise row collection must live on the Agent aggregate",
        ),
    ):
        if token in production_text:
            add("R47_HOSTED_ADVERTISE_PRELUDE_AGENT_AGGREGATE_FORK", session_prelude, 1, detail)

# Rule 48: skill.list hosted owner scope consumes an Agent aggregate projection.
# The ability owns skill inventory traversal and wire shaping, but it must not
# load or inspect hosted identity persistence rows.
skill_list_ability = cli_root / "src/daemon/ability/builtins/resources/skills/list.rs"
if agent_aggregate.exists():
    text = source(agent_aggregate)
    for token, detail in (
        (
            "struct AgentHostedSkillOwnerProjection",
            "Agent aggregate must expose a hosted skill owner projection type",
        ),
        (
            "fn hosted_skill_owner_projection(&self) -> AgentHostedSkillOwnerProjection",
            "Agent aggregate snapshot must expose the hosted skill owner projection",
        ),
        (
            "fn hosted_ura_for(&self, agent_name: &str) -> Option<&str>",
            "hosted skill owner projection must resolve owner name to Agent URA",
        ),
        (
            "fn owner_name_for_agent_ura(&self, agent_ura: &str) -> Option<&str>",
            "hosted skill owner projection must resolve Agent URA to owner name",
        ),
    ):
        if token not in text:
            add("R48_SKILL_LIST_AGENT_AGGREGATE_IDENTITY_FORK", agent_aggregate, 1, detail)
if skill_list_ability.exists():
    text = source(skill_list_ability)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "AgentAggregateRepository::load_snapshot()",
            "skill.list must load registry and hosted identity through the Agent aggregate snapshot",
        ),
        (
            ".hosted_skill_owner_projection()",
            "skill.list must consume the aggregate hosted skill owner projection",
        ),
        (
            "AgentHostedSkillOwnerProjection",
            "skill.list scope helpers must depend on the aggregate projection interface",
        ),
        (
            ".owner_name_for_agent_ura(",
            "skill.list must resolve Agent URA scopes through the aggregate projection",
        ),
        (
            ".hosted_ura_for(",
            "skill.list resource URA derivation must resolve hosted owners through the aggregate projection",
        ),
    ):
        if token not in production_text:
            add("R48_SKILL_LIST_AGENT_AGGREGATE_IDENTITY_FORK", skill_list_ability, 1, detail)
    for token, detail in (
        (
            "local_agents::load",
            "skill.list production code must not load local-agents.json directly",
        ),
        (
            "LocalAgentsFile",
            "skill.list production code must not mention the hosted identity file shape",
        ),
        (
            ".hosted_agents",
            "skill.list production code must not inspect hosted identity rows directly",
        ),
        (
            "from_local_agents",
            "skill.list production code must not rebuild hosted identity projections locally",
        ),
        (
            "agent_registry::AgentType",
            "skill.list production code must not reintroduce retired registry AgentType",
        ),
        (
            "agents::AgentType",
            "skill.list production code must not reintroduce retired registry AgentType",
        ),
        (
            "snapshot.registry",
            "skill.list production code must not iterate raw AgentRegistry rows",
        ),
    ):
        if token in production_text:
            add("R48_SKILL_LIST_AGENT_AGGREGATE_IDENTITY_FORK", skill_list_ability, 1, detail)

# Rule 48b: Claude Code runtime plugin discovery must consume only the
# runtime-owned project-local skill root. The historical agent-private
# `<cwd>/skills/` tree may exist for public skill APIs, but it must not remain
# a driver launch fallback.
claude_code_driver = cli_root / "src/daemon/execution/mission/drivers/claude_code.rs"
if claude_code_driver.exists():
    raw_text = claude_code_driver.read_text(encoding="utf-8", errors="replace")
    raw_production_text = raw_text.split("#[cfg(test)]", 1)[0]
    production_text = source(claude_code_driver).split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            'cwd.join("skills")',
            "Claude Code driver must not scan legacy <cwd>/skills as a plugin discovery fallback",
        ),
        (
            "<cwd>/skills/` — legacy",
            "Claude Code driver must not document legacy <cwd>/skills as an active runtime discovery root",
        ),
        (
            "backward compatibility with skills installed",
            "Claude Code driver must not preserve pre-cutover skill discovery compatibility",
        ),
    ):
        if token in raw_production_text:
            add(
                "R48B_CLAUDE_PLUGIN_DISCOVERY_CANONICAL_ROOT",
                claude_code_driver,
                line_number(raw_text, raw_text.index(token)),
                detail,
            )
    for token, detail in (
        (
            "fn append_claude_workspace_plugin_dirs(",
            "Claude Code driver must expose a focused plugin discovery helper",
        ),
        (
            'cwd.join(".claude").join("skills")',
            "Claude Code driver must consume the canonical .claude/skills runtime root",
        ),
        (
            "looks_like_claude_plugin_dir",
            "Claude Code plugin discovery must keep plugin-shape filtering at the driver boundary",
        ),
    ):
        if token not in production_text:
            add(
                "R48B_CLAUDE_PLUGIN_DISCOVERY_CANONICAL_ROOT",
                claude_code_driver,
                1,
                detail,
            )

# Rule 49: skill package owner resolution consumes the Agent aggregate
# registered-workspace projection. Package surfaces own package path layout,
# but they must not reopen agents.json or inspect AgentRegistry rows.
skill_publish_ability = cli_root / "src/daemon/ability/builtins/resources/skills/publish.rs"
skill_store = cli_root / "src/daemon/resources/skills/store.rs"
if agent_aggregate.exists():
    text = source(agent_aggregate)
    for token, detail in (
        (
            "struct AgentRegisteredWorkspace",
            "Agent aggregate must expose a registered workspace projection type",
        ),
        (
            "enum AgentRegisteredWorkspaceLookupError",
            "Agent aggregate must classify missing and invalid registered workspaces",
        ),
        (
            "fn load_registered_agent_workspace(",
            "Agent aggregate repository must own registered workspace lookup",
        ),
        (
            "fn root_path(&self) -> &Path",
            "registered workspace projection must expose the canonical root path",
        ),
        (
            "enum AgentSkillLayout",
            "Agent aggregate must expose a skill-layout projection instead of raw registry row type",
        ),
        (
            "fn skill_layout(&self) -> AgentSkillLayout",
            "registered workspace projection must expose the skill-layout selector",
        ),
    ):
        if token not in text:
            add("R49_SKILL_PUBLISH_AGENT_AGGREGATE_OWNER_FORK", agent_aggregate, 1, detail)

if skill_publish_ability.exists():
    text = source(skill_publish_ability)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "AgentAggregateRepository::load_registered_agent_workspace(",
            "skill package owner resolution must load through the registry-only Agent workspace resolver",
        ),
        (
            "owner_id, \"skill.publish\"",
            "skill package owner resolution must declare its registered workspace operation",
        ),
        (
            "AgentSkillLayout",
            "skill package owner resolution must consume aggregate skill-layout projection",
        ),
    ):
        if token not in production_text:
            add("R49_SKILL_PUBLISH_AGENT_AGGREGATE_OWNER_FORK", skill_publish_ability, 1, detail)
    for token, detail in (
        (
            "agents::load_agents",
            "skill package owner resolution must not load agents.json directly",
        ),
        (
            "agent_registry::load_agents",
            "skill package owner resolution must not bypass the Agent aggregate repository",
        ),
        (
            "registry.agents",
            "skill package owner resolution must not inspect AgentRegistry rows directly",
        ),
        (
            "agent_registry::AgentType",
            "skill package owner resolution must not reintroduce retired registry AgentType",
        ),
        (
            "agents::AgentType",
            "skill package owner resolution must not reintroduce retired registry AgentType",
        ),
    ):
        if token in production_text:
            add("R49_SKILL_PUBLISH_AGENT_AGGREGATE_OWNER_FORK", skill_publish_ability, 1, detail)

if skill_store.exists():
    text = source(skill_store)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "fn resolve_skill_agent_workspace(",
            "shared skill mutations must centralize registered workspace and layout resolution",
        ),
        (
            "AgentAggregateRepository::load_registered_agent_workspace(",
            "shared skill mutations must load through the registry-only Agent workspace resolver",
        ),
        (
            "agent, mutation.operation()",
            "shared skill mutations must declare their registered workspace operation",
        ),
        (
            "AgentSkillLayout",
            "shared skill package helpers must consume aggregate skill-layout projection",
        ),
    ):
        if token not in production_text:
            add("R49_SKILL_PUBLISH_AGENT_AGGREGATE_OWNER_FORK", skill_store, 1, detail)
    for token, detail in (
        (
            "agents::load_agents",
            "shared skill mutations must not load agents.json directly",
        ),
        (
            "agent_registry::load_agents",
            "shared skill mutations must not bypass the Agent aggregate repository",
        ),
        (
            "registry.agents",
            "shared skill mutations must not inspect AgentRegistry rows directly",
        ),
        (
            "agent_registry::AgentType",
            "shared skill mutations must not reintroduce retired registry AgentType",
        ),
        (
            "agents::AgentType",
            "shared skill mutations must not reintroduce retired registry AgentType",
        ),
    ):
        if token in production_text:
            add("R49_SKILL_PUBLISH_AGENT_AGGREGATE_OWNER_FORK", skill_store, 1, detail)

# Rule 50: boot-time discovery/A2A providers consume the Agent aggregate
# registry projection. Discovery handlers still accept a registry-shaped
# adapter, but production boot must not inject a raw agents.json loader.
catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
if catalog_build.exists():
    text = source(catalog_build)
    production_text = text.split("#[cfg(test)]", 1)[0]
    provider_blocks = (
        (
            "discover_ability::register_device_aggregate_with_resolver(",
            "agent.discover boot provider",
        ),
        (
            "a2a_bridge_ability::register(",
            "A2A bridge boot provider",
        ),
    )
    for token, label in provider_blocks:
        start = production_text.find(token)
        if start < 0:
            add(
                "R50_BOOT_DISCOVERY_AGENT_AGGREGATE_PROVIDER_FORK",
                catalog_build,
                1,
                f"{label} registration block is missing",
            )
            continue
        block_end = start + 650
        if token == "discover_ability::register_device_aggregate_with_resolver(":
            next_registration = production_text.find(
                "a2a_bridge_ability::register(", start + len(token)
            )
            if next_registration >= 0:
                block_end = next_registration
        block = production_text[start:block_end]
        required_tokens = [
            (
                "AgentAggregateRepository::load_snapshot()",
                f"{label} must load through the Agent aggregate snapshot",
            )
        ]
        if token == "a2a_bridge_ability::register(":
            required_tokens.append(
                (
                    ".registered_agent_registry_projection()",
                    f"{label} must project registry rows from the Agent aggregate",
                )
            )
        for required, detail in required_tokens:
            if required not in block:
                add(
                    "R50_BOOT_DISCOVERY_AGENT_AGGREGATE_PROVIDER_FORK",
                    catalog_build,
                    line_number(production_text, start),
                    detail,
                )
        for forbidden, detail in (
            (
                "agent_registry::load_agents",
                f"{label} must not inject a raw agents.json provider",
            ),
            (
                "agents::load_agents",
                f"{label} must not inject a raw agents.json provider",
            ),
        ):
            if forbidden in block:
                add(
                    "R50_BOOT_DISCOVERY_AGENT_AGGREGATE_PROVIDER_FORK",
                    catalog_build,
                    line_number(production_text, start),
                    detail,
                )
        if (
            token == "discover_ability::register_device_aggregate_with_resolver("
            and ".registered_agent_registry_projection()" in block
        ):
            add(
                "R50_BOOT_DISCOVERY_AGENT_AGGREGATE_PROVIDER_FORK",
                catalog_build,
                line_number(production_text, start),
                "agent.discover must receive the complete Agent aggregate instead of discarding hosted identity before dispatch",
            )

# Rule 51: daemon-native Hub URA join credentials must not be forced through
# backend HTTP token verification. Token-paired credentials still use the
# backend verifier; federation-native credentials are proven by join lineage and
# Hub trust material persisted by `federation.join`.
cli_start = cli_root / "src/cli/commands/start.rs"
if cli_start.exists():
    text = source(cli_start)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "fn has_daemon_native_join_lineage(",
            "CLI start must classify daemon-native Hub URA join credentials explicitly",
        ),
        (
            "creds.credential_token.trim().is_empty()",
            "daemon-native join lineage must require tokenless credentials",
        ),
        (
            "join_receipt_hash",
            "daemon-native join lineage must require the federation join receipt hash",
        ),
        (
            "hub_pubkey_b64",
            "daemon-native join lineage must require pinned Hub trust material",
        ),
    ):
        if token not in production_text:
            add("R51_DAEMON_NATIVE_JOIN_CREDENTIAL_VERIFICATION_FORK", cli_start, 1, detail)
    body = brace_function_body(
        production_text,
        r"fn\s+load_and_verify_credentials_with(?:<[^>]+>)?\s*\(",
    )
    if body is None:
        add(
            "R51_DAEMON_NATIVE_JOIN_CREDENTIAL_VERIFICATION_FORK",
            cli_start,
            1,
            "CLI start must keep credential verification in load_and_verify_credentials_with",
        )
    else:
        offset, method_body = body
        lineage_index = method_body.find("has_daemon_native_join_lineage(&creds)")
        verify_index = method_body.find("verify(&creds)")
        if lineage_index < 0:
            add(
                "R51_DAEMON_NATIVE_JOIN_CREDENTIAL_VERIFICATION_FORK",
                cli_start,
                line_number(production_text, offset),
                "load_and_verify_credentials_with must check daemon-native join lineage",
            )
        if verify_index < 0:
            add(
                "R51_DAEMON_NATIVE_JOIN_CREDENTIAL_VERIFICATION_FORK",
                cli_start,
                line_number(production_text, offset),
                "token-paired credentials must still use the backend verifier",
            )
        if lineage_index >= 0 and verify_index >= 0 and lineage_index > verify_index:
            add(
                "R51_DAEMON_NATIVE_JOIN_CREDENTIAL_VERIFICATION_FORK",
                cli_start,
                line_number(production_text, offset),
                "daemon-native join lineage must short-circuit before backend verifier execution",
            )

# Rule 52: ability manifest publication resolves registered owner workspaces
# through the registry-only Agent resolver, never by reopening agents.json.
ability_publish = cli_root / "src/daemon/ability/builtins/device_control/ability_management/publish.rs"
if ability_publish.exists():
    text = source(ability_publish)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "AgentAggregateRepository::load_registered_agent_workspace(",
            "ability publication must load registered owners through the registry-only workspace resolver",
        ),
        (
            "\"ability.publish\"",
            "ability publication must declare its registered workspace operation",
        ),
    ):
        if token not in production_text:
            add("R52_ABILITY_PUBLISH_AGENT_AGGREGATE_WORKSPACE_FORK", ability_publish, 1, detail)
    for token, detail in (
        ("agents::load_agents", "ability publication must not load agents.json directly"),
        ("agent_registry::load_agents", "ability publication must not bypass the Agent resolver"),
        ("registry.agents", "ability publication must not inspect AgentRegistry rows directly"),
    ):
        if token in production_text:
            add("R52_ABILITY_PUBLISH_AGENT_AGGREGATE_WORKSPACE_FORK", ability_publish, 1, detail)

# Rule 53: transactional agent ability authoring uses both the registered
# runtime entry and workspace projection, sourced through the registry-only resolver.
agent_authoring = cli_root / "src/daemon/ability/builtins/agents/authoring.rs"
if agent_authoring.exists():
    text = source(agent_authoring)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "AgentAggregateRepository::load_registered_agent(",
            "agent ability authoring must load its runtime entry through the registry-only resolver",
        ),
        (
            "\"agent.ability.put\"",
            "agent ability authoring must declare its registered runtime operation",
        ),
        ("registered.entry()", "agent ability authoring must use the runtime entry projection"),
        ("registered.workspace()", "agent ability authoring must use the workspace projection"),
    ):
        if token not in production_text:
            add("R53_AGENT_ABILITY_AUTHORING_REGISTRY_ONLY_WORKSPACE_FORK", agent_authoring, 1, detail)
    for token, detail in (
        ("agents::load_agents", "agent ability authoring must not load agents.json directly"),
        ("registry.agents", "agent ability authoring must not inspect AgentRegistry rows directly"),
    ):
        if token in production_text:
            add("R53_AGENT_ABILITY_AUTHORING_REGISTRY_ONLY_WORKSPACE_FORK", agent_authoring, 1, detail)

# Rule 54: registry-only consumers must acquire full Agent registry state from
# the Agent aggregate repository. These consumers intentionally do not join
# registered metadata with hosted identity state, so `load_snapshot` would add
# an incorrect local-agents.json dependency.
registry_bootstrap = cli_root / "src/daemon/ability/catalog/profiles/bootstrap.rs"
think_ability = cli_root / "src/daemon/ability/builtins/automation/think.rs"
registry_projection_call = "AgentAggregateRepository::load_registered_agent_registry_projection()"
if agent_aggregate.exists():
    text = source(agent_aggregate)
    if "enum AgentRegistryProjectionLoadError" not in text:
        add(
            "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK",
            agent_aggregate,
            1,
            "Agent aggregate must classify registry-only projection failures",
        )
    projection_body = brace_function_body(
        text,
        r"fn\s+load_registered_agent_registry_projection\s*\(",
    )
    if projection_body is None:
        add(
            "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK",
            agent_aggregate,
            1,
            "Agent aggregate must own the full registry-only projection loader",
        )
    else:
        offset, body = projection_body
        if "agent_registry::load_agents()" not in body:
            add(
                "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK",
                agent_aggregate,
                line_number(text, offset),
                "registry-only projection loader must delegate to registry persistence",
            )
        if "load_snapshot" in body or "local_agents" in body:
            add(
                "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK",
                agent_aggregate,
                line_number(text, offset),
                "registry-only projection loader must not depend on hosted identity state",
            )

for registry_consumer, label, minimum_calls in (
    (registry_bootstrap, "bootstrap plan", 1),
    (think_ability, "curator catalog", 1),
    (catalog_build, "daemon catalog boot and purge replay", 2),
):
    if not registry_consumer.exists():
        continue
    text = source(registry_consumer)
    production_text = text.split("#[cfg(test)]", 1)[0]
    required_calls = minimum_calls
    if registry_consumer == catalog_build and "fn build_registry_for_daemon_result" not in production_text:
        required_calls = 0
    call_count = production_text.count(registry_projection_call)
    if call_count < required_calls:
        add(
            "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK",
            registry_consumer,
            1,
            f"{label} must load the full registry through the Agent aggregate projection ({required_calls} call(s) required)",
        )
    if "agent_registry::load_agents" in production_text or "agents::load_agents" in production_text:
        add(
            "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK",
            registry_consumer,
            1,
            f"{label} must not bypass the Agent registry projection owner",
        )

# Rule 55: meta.teach's manifest lookup and forget-recovery state machine read
# only the durable Agent registry. Their hosted identity checks remain paired
# snapshot reads elsewhere; these registry-only paths must not reopen
# agents.json or turn a malformed identity file into a recovery blocker.
if agent_aggregate.exists():
    text = source(agent_aggregate)
    for token, detail in (
        (
            "struct AgentRegisteredRuntimeProjection",
            "Agent aggregate must expose the registered runtime projection type for teach forget convergence",
        ),
        (
            "fn registered_agent_runtime_projection(",
            "Agent aggregate snapshot must own optional registered runtime lookup",
        ),
        (
            "fn ability_manifest_path(&self, ability: &str) -> Option<PathBuf>",
            "registered runtime projection must own teach ability manifest path derivation",
        ),
    ):
        if token not in text:
            add("R55_TEACH_REGISTRY_PROJECTION_OWNER_FORK", agent_aggregate, 1, detail)

teach_registry_owners = (
    (
        "resolve_owner_manifest",
        "owner manifest resolution",
    ),
    (
        "recover_forget_transactions",
        "forget transaction recovery",
    ),
)
if governance_teach.exists():
    text = source(governance_teach)
    production_text = text.split("#[cfg(test)]", 1)[0]
    if "snapshot.registry.agents" in production_text:
        add(
            "R55_TEACH_REGISTRY_PROJECTION_OWNER_FORK",
            governance_teach,
            line_number(production_text, production_text.index("snapshot.registry.agents")),
            "teach governance production code must consume Agent aggregate runtime projections instead of raw registry rows",
        )
    if any(f"fn {function_name}" in production_text for function_name, _ in teach_registry_owners):
        for function_name, label in teach_registry_owners:
            body = brace_function_body(production_text, rf"fn\s+{function_name}\s*\(")
            if body is None:
                add(
                    "R55_TEACH_REGISTRY_PROJECTION_OWNER_FORK",
                    governance_teach,
                    1,
                    f"{label} must remain a named registry-only read owner",
                )
                continue
            offset, function_body = body
            for token, detail in (
                (
                    "AgentAggregateRepository::load_registered_agent_registry_projection()",
                    f"{label} must load through the Agent registry projection owner",
                ),
                (
                    "AgentRegistryProjectionLoadError::into_source_or_self",
                    f"{label} must preserve raw registry persistence errors",
                ),
            ):
                if token not in function_body:
                    add(
                        "R55_TEACH_REGISTRY_PROJECTION_OWNER_FORK",
                        governance_teach,
                        line_number(production_text, offset),
                        detail,
                    )
            if "agent_registry::load_agents" in function_body or "agents::load_agents" in function_body:
                add(
                    "R55_TEACH_REGISTRY_PROJECTION_OWNER_FORK",
                    governance_teach,
                    line_number(production_text, offset),
                    f"{label} must not bypass the Agent registry projection owner",
                )

# Rule 56: KernelApi is a daemon entry boundary, not a second invocation
# lifecycle owner. Runtime-admitted calls must consume Axon's finalized proof
# directly instead of deriving or projecting a daemon receipt.
kernel_module = cli_root / "src/daemon/boot/kernel/mod.rs"
kernel_api = cli_root / "src/daemon/boot/kernel/api.rs"
if kernel_module.exists():
    text = source(kernel_module)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (".finalized()", "kernel runtime dispatch must consume Axon finalized invocation proof"),
        (
            "anyhow::Result<FinalizedInvocation>",
            "kernel must return Axon's finalized invocation proof directly",
        ),
    ):
        if token not in production_text:
            add("R56_KERNEL_CANONICAL_TERMINAL_PROJECTION_FORK", kernel_module, 1, detail)
    for token, detail in (
        ("KernelDispatchTerminal", "kernel must not maintain a parallel terminal state enum"),
        ("KernelDispatchOutcome", "kernel must not maintain a parallel receipt outcome"),
        ("TerminalState::", "kernel must not project Axon's terminal state into a CLI enum"),
        ("terminal_receipt_hash:", "kernel must not project Axon's canonical receipt into a CLI receipt"),
        ("handle.wait().await", "kernel must not infer terminal state by waiting outside Axon finalization"),
        ("events.iter().rev().find(|e| e.state.is_terminal())", "kernel must not derive terminality from an event snapshot"),
    ):
        if token in production_text:
            add("R56_KERNEL_CANONICAL_TERMINAL_PROJECTION_FORK", kernel_module, 1, detail)
if kernel_api.exists():
    api_text = source(kernel_api)
    for token, detail in (
        (
            "DescriptorBoundInvocationRequest",
            "KernelApi must accept Axon's canonical descriptor-bound request",
        ),
        (
            "FinalizedInvocation",
            "KernelApi must return Axon's canonical finalization",
        ),
    ):
        if token not in api_text:
            add("R56_KERNEL_CANONICAL_TERMINAL_PROJECTION_FORK", kernel_api, 1, detail)

# Rule 57: daemon-local RPC dispatch is a terminal presentation adapter, not
# an alternate lifecycle owner. It must consume Axon's finalized proof instead
# of searching invocation events after waiting for a terminal state.
local_runtime_invoker = cli_root / "src/daemon/invocation/dispatch/local_runtime_invoker.rs"
if local_runtime_invoker.exists():
    text = source(local_runtime_invoker)
    production_text = text.split("#[cfg(test)]", 1)[0]
    rpc_projection = brace_function_body(production_text, r"pub\s+async\s+fn\s+rpc_value_from_handle\s*\(")
    if rpc_projection is None:
        add(
            "R57_LOCAL_RUNTIME_RPC_CANONICAL_TERMINAL_PROJECTION_FORK",
            local_runtime_invoker,
            1,
            "local RPC adapter must retain one named canonical terminal projection",
        )
    else:
        offset, rpc_projection_body = rpc_projection
        if ".finalized()" not in rpc_projection_body:
            add(
                "R57_LOCAL_RUNTIME_RPC_CANONICAL_TERMINAL_PROJECTION_FORK",
                local_runtime_invoker,
                line_number(production_text, offset),
                "local RPC adapter must consume Axon finalized invocation proof",
            )
        for token, detail in (
            ("handle.wait().await", "local RPC adapter must not infer terminal state by waiting outside Axon finalization"),
            ("events.iter().rev().find(|event| event.state.is_terminal())", "local RPC adapter must not derive terminality from an event snapshot"),
        ):
            if token in rpc_projection_body:
                add(
                    "R57_LOCAL_RUNTIME_RPC_CANONICAL_TERMINAL_PROJECTION_FORK",
                    local_runtime_invoker,
                    line_number(production_text, offset),
                    detail,
                )

# Rule 58: daemon runtime consumers must use Axon's canonical terminal state
# and finalized receipt directly. A CLI receipt or terminal-state projection is
# a second authority even when it preserves all variants.
runtime_record = cli_root / "src/daemon/invocation/receipts/runtime_record.rs"
loop_instance = cli_root / "src/daemon/execution/loop_instance/mod.rs"
if runtime_record.exists():
    add(
        "R58_TERMINAL_TIMEOUT_PROJECTION_FORK",
        runtime_record,
        1,
        "obsolete CLI runtime invocation/receipt authority remains; consume Axon finalized receipts directly",
    )
if loop_instance.exists():
    text = source(loop_instance)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "InvocationState::TimedOut",
            "loop terminal consumer must handle Axon timeout distinctly",
        ),
        (
            "InvocationState::Failed",
            "loop terminal consumer must handle Axon failure distinctly",
        ),
        (
            "finalized.terminal_receipt.reason()",
            "loop terminal diagnostics must come from the canonical terminal receipt",
        ),
    ):
        if token not in production_text:
            add("R58_TERMINAL_TIMEOUT_PROJECTION_FORK", loop_instance, 1, detail)
    if "TerminalState::" in production_text:
        add(
            "R58_TERMINAL_TIMEOUT_PROJECTION_FORK",
            loop_instance,
            line_number(production_text, production_text.find("TerminalState::")),
            "loop must not consume a CLI terminal-state projection",
        )

# Rule 59: Go and Python RuntimeAbility facades share one descriptor binding
# owner. Addressing may project URAs and subjects, but only RuntimeClient may
# select a registered descriptor version, hash, action, and call mode.
go_runtime_ability = cli_root / "sdk/go/runtime_ability.go"
python_runtime_ability = cli_root / "sdk/python/easynet_sdk/runtime_ability.py"
if go_runtime_ability.exists():
    text = source(go_runtime_ability)
    if "c.runtime.ResolveDescriptorRef(" not in text:
        add(
            "R59_SDK_RUNTIME_DESCRIPTOR_OWNER_FORK",
            go_runtime_ability,
            1,
            "Go runtime ability lowering must resolve descriptors through RuntimeClient",
        )
if python_runtime_ability.exists():
    text = source(python_runtime_ability)
    if "self._runtime.resolve_descriptor_ref(" not in text:
        add(
            "R59_SDK_RUNTIME_DESCRIPTOR_OWNER_FORK",
            python_runtime_ability,
            1,
            "Python runtime ability lowering must resolve descriptors through RuntimeClient",
        )
    if "owner_ability_descriptor_ref(" in text:
        add(
            "R59_SDK_RUNTIME_DESCRIPTOR_OWNER_FORK",
            python_runtime_ability,
            1,
            "Python runtime ability lowering must not mint descriptor refs through Addressing",
        )
    if 'call_mode="stream"' not in text:
        add(
            "R59_SDK_RUNTIME_DESCRIPTOR_OWNER_FORK",
            python_runtime_ability,
            1,
            "Python stream lowering must resolve a stream-mode descriptor",
        )

# Rule 60: generic descriptor resolver ingress must not default tuple
# selector state. Higher-level convenience APIs may choose RPC explicitly, but
# RuntimeClient / C ABI descriptor resolvers must require `call_mode` before
# crossing the provider seam so there is no second implicit selector authority.
python_runtime = cli_root / "sdk/python/easynet_sdk/runtime.py"
python_cabi = cli_root / "sdk/python/easynet_sdk/_cabi.py"
go_runtime = cli_root / "sdk/go/runtime.go"
go_cabi_runtime = cli_root / "sdk/go/cabi_runtime.go"
rust_ffi_invocation = cli_root / "src/ffi/invocation/mod.rs"
if python_runtime.exists():
    text = source(python_runtime)
    for token, detail in (
        (
            'call_mode: str = "rpc"',
            "Python RuntimeClient.resolve_descriptor_ref must require explicit call_mode",
        ),
        (
            'call_mode = call_mode.strip() or "rpc"',
            "Python RuntimeClient must not normalize blank descriptor call_mode to rpc",
        ),
    ):
        if token in text:
            add("R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK", python_runtime, 1, detail)
    if 'raise _invalid_runtime_client("descriptor_ref call_mode is required")' not in text:
        add(
            "R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK",
            python_runtime,
            1,
            "Python RuntimeClient must reject missing descriptor call_mode before provider resolution",
        )
if python_cabi.exists():
    text = source(python_cabi)
    if "_resolve_descriptor_ref_from_diagnostics" in text:
        for token, detail in (
            (
                'request.get("call_mode") or "rpc"',
                "Python C ABI diagnostics resolver must not default request call_mode to rpc",
            ),
            (
                'entry.get("call_mode") or "rpc"',
                "Python C ABI diagnostics resolver must not default catalog entry call_mode to rpc",
            ),
        ):
            if token in text:
                add("R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK", python_cabi, 1, detail)
        if '_required_string(request, "call_mode")' not in text:
            add(
                "R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK",
                python_cabi,
                1,
                "Python C ABI diagnostics resolver must require request call_mode",
            )
if go_runtime.exists():
    text = source(go_runtime)
    for token, detail in (
        (
            'CallMode   string `json:"call_mode,omitempty"`',
            "Go RuntimeDescriptorRefRequest must not omit call_mode at the generic resolver seam",
        ),
        (
            'req.CallMode = "rpc"',
            "Go RuntimeClient must not default blank descriptor call_mode to rpc",
        ),
    ):
        if token in text:
            add("R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK", go_runtime, 1, detail)
    if 'invalidRuntimeClient("descriptor_ref call_mode is required")' not in text:
        add(
            "R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK",
            go_runtime,
            1,
            "Go RuntimeClient must reject missing descriptor call_mode before provider resolution",
        )
if go_cabi_runtime.exists():
    text = source(go_cabi_runtime)
    if "resolveDescriptorRefFromDiagnostics" in text:
        for token, detail in (
            (
                'callMode = "rpc"',
                "Go C ABI diagnostics resolver must not default request call_mode to rpc",
            ),
            (
                'entryCallMode = "rpc"',
                "Go C ABI diagnostics resolver must not default catalog entry call_mode to rpc",
            ),
        ):
            if token in text:
                add("R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK", go_cabi_runtime, 1, detail)
        if 'invalidRuntimePayload("call_mode is required for descriptor_ref resolution", nil)' not in text:
            add(
                "R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK",
                go_cabi_runtime,
                1,
                "Go C ABI diagnostics resolver must require request call_mode",
            )
runtime_descriptor_provider = cli_root / "src/daemon/axon_bridge/runtime_descriptor_provider.rs"
if rust_ffi_invocation.exists() or runtime_descriptor_provider.exists():
    text = source(rust_ffi_invocation) if rust_ffi_invocation.exists() else ""
    provider_text = source(runtime_descriptor_provider) if runtime_descriptor_provider.exists() else ""
    combined = text + "\n" + provider_text
    for token, detail in (
        (
            'descriptor_ref request missing call_mode',
            "Rust FFI descriptor resolver must require request call_mode",
        ),
    ):
        if token not in combined:
            add(
                "R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK",
                runtime_descriptor_provider if runtime_descriptor_provider.exists() else rust_ffi_invocation,
                1,
                detail,
            )
    if '.unwrap_or("rpc")' in combined:
        add(
            "R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK",
            runtime_descriptor_provider if runtime_descriptor_provider.exists() else rust_ffi_invocation,
            1,
            "Rust FFI descriptor resolver/catalog path must not default missing call_mode to rpc",
        )

# Rule 61: direct gRPC transports own only unary/stream/bidi wire dispatch.
# Prepare, signed submit, and invocation-handle lifecycle belong to the
# explicitly configured Runtime transport. Go and Python must both fail closed
# when that owner is absent rather than inventing local prepared or terminal
# handle state.
go_direct_runtime = cli_root / "sdk/go/direct_runtime.go"
python_direct_runtime = cli_root / "sdk/python/easynet_sdk/providers/runtime/direct.py"
if go_direct_runtime.exists():
    text = source(go_direct_runtime)
    if "func (t *directRuntimeTransport) requireHandleTransport(" not in text:
        add(
            "R61_DIRECT_RUNTIME_HANDLE_OWNER_FORK",
            go_direct_runtime,
            1,
            "Go direct runtime must centralize handle-owner acquisition",
        )
    for token, detail in (
        ("Code:      ErrNotImplemented", "Go direct runtime must fail closed without a handle transport"),
        ("return handle.Prepare(ctx, projectedJSON, optionsJSON)", "Go direct prepare must delegate its projected draft to the handle owner"),
        ("return handle.SubmitSigned(ctx, signedJSON)", "Go direct signed submit must delegate to the handle owner"),
        ("return handle.AwaitHandle(ctx, control)", "Go direct await must delegate to the handle owner"),
    ):
        if token not in text:
            add("R61_DIRECT_RUNTIME_HANDLE_OWNER_FORK", go_direct_runtime, 1, detail)
    for token, detail in (
        ("directRuntimePrepare", "Go direct runtime must not retain a local prepare fallback"),
        ("directSignedInvocationDraftJSON", "Go direct runtime must not submit signed calls through direct gRPC"),
        ("directRuntimeHandleSnapshot", "Go direct runtime must not own synthetic handle state"),
        ("storeDirectHandle", "Go direct runtime must not mint local invocation handles"),
    ):
        if token in text:
            add("R61_DIRECT_RUNTIME_HANDLE_OWNER_FORK", go_direct_runtime, 1, detail)
if python_direct_runtime.exists():
    text = source(python_direct_runtime)
    for token, detail in (
        ("def _require_handle_transport(self) -> RuntimeTransport:", "Python direct runtime must centralize handle-owner acquisition"),
        ("raise _unsupported(\"direct runtime handle transport is not configured\")", "Python direct runtime must fail closed without a handle transport"),
        ("return handle_transport.prepare(", "Python direct prepare must delegate to the handle owner"),
        ("return self._require_handle_transport().submit_signed(signed_json)", "Python direct signed submit must delegate to the handle owner"),
    ):
        if token not in text:
            add("R61_DIRECT_RUNTIME_HANDLE_OWNER_FORK", python_direct_runtime, 1, detail)

# Rule 23: MCP stdio frame ownership must enforce declared bounds before
# retaining arbitrarily long lines or allocating Content-Length bodies. The
# daemon MCP stdio owner may drain oversized input, but it must not revive the
# old read_line architecture where the OS peer controlled allocation size.
mcp_client = cli_root / "src/daemon/execution/mcp/mod.rs"
mcp_stdio_server = cli_root / "src/daemon/execution/mcp/stdio.rs"
mcp_stdio_requirements = (
    (
        mcp_client,
        (
            (
                "const MAX_CHILD_STDIO_LINE_BYTES",
                "child MCP stdout must declare a bounded line limit",
            ),
            (
                "const MAX_CHILD_STDIO_FRAME_BYTES",
                "child MCP Content-Length frames must declare a bounded body limit",
            ),
            (
                "read_bounded_child_stdio_line",
                "child MCP stdout must enter through the bounded line reader",
            ),
            (
                "len > MAX_CHILD_STDIO_FRAME_BYTES",
                "Content-Length must be rejected before body allocation",
            ),
        ),
    ),
    (
        mcp_stdio_server,
        (
            (
                "const MAX_LINE_LENGTH",
                "MCP stdio server stdin must declare a bounded line limit",
            ),
            (
                "fn read_bounded_line",
                "MCP stdio server stdin must enter through the bounded line reader",
            ),
            (
                "read_bounded_line(&mut input",
                "MCP stdio server run loop must use the bounded reader",
            ),
        ),
    ),
)
for path, requirements in mcp_stdio_requirements:
    if not path.exists():
        continue
    text = source(path)
    production_text = text.split("#[cfg(test)]", 1)[0]
    for token, detail in requirements:
        if token not in production_text:
            add("R23_MCP_STDIO_UNBOUNDED_FRAME_READER", path, 1, detail)
    if re.search(r"\bread_line\s*\(", production_text):
        add(
            "R23_MCP_STDIO_UNBOUNDED_FRAME_READER",
            path,
            line_number(production_text, re.search(r"\bread_line\s*\(", production_text).start()),
            "MCP stdio production readers must not use unbounded read_line",
        )

# Rule 23b: mission recursion/child-invocation evidence must be executable.
# A guard that depends on local Claude/Codex auth and is permanently ignored is
# documentation, not architecture evidence; canonical Mission child proof lives
# in runtime-level tests that do not spawn external agents.
mission_dispatch = cli_root / "src/daemon/execution/mission/dispatch.rs"
if mission_dispatch.exists():
    text = source(mission_dispatch)
    production_text = text.split("#[cfg(test)]", 1)[0]
    ignored_recursion = re.search(
        r"#\s*\[\s*ignore[^\]]*\]\s*(?:\n\s*(?://.*)?)*\n\s*fn\s+agent_send_desugar_e2e\b",
        text,
    )
    if ignored_recursion:
        add(
            "R23B_MISSION_RECURSION_IGNORED_EVIDENCE",
            mission_dispatch,
            line_number(text, ignored_recursion.start()),
            "mission recursion/agent-send architecture evidence must not be a permanently ignored external-CLI test",
        )
    runtime_config_entry_fallback_patterns = (
        r"resolve_model_with_overrides[\s\S]{0,220}\bentry_model\b",
        r"resolve_model\s*\([^)]*\bentry_model\b",
        r"\.or\s*\(\s*entry_model\s*\)",
        r"resolve_model_with_overrides\s*\([\s\S]{0,220}entry\.model\.clone\s*\(",
        r"resolve_timeout\s*\([^)]*\bentry_timeout_secs\b",
        r"resolve_timeout\s*\([\s\S]{0,160}entry\.timeout_secs",
        r"unwrap_or\s*\(\s*entry_timeout_secs\s*\)",
    )
    for pattern in runtime_config_entry_fallback_patterns:
        match = re.search(pattern, production_text)
        if match:
            add(
                "R66_MISSION_RUNTIME_CONFIG_ENTRY_FALLBACK",
                mission_dispatch,
                line_number(production_text, match.start()),
                "mission dispatch runtime config must come from per-call override, agent.toml, or canonical defaults, not registry entry fallback",
            )

# Rule 24: cancellation terminal retention is an idempotent lifecycle index.
# Repeated observation of the same terminal invocation must not enqueue
# duplicate eviction tokens that can later remove the current terminal map row.
cancellation_registry = cli_root / "src/daemon/invocation/dispatch/cancellation.rs"
if cancellation_registry.exists():
    text = source(cancellation_registry)
    production_text = text.split("#[cfg(test)]", 1)[0]
    cancellation_requirements = (
        (
            "fn retain_terminal_key(&mut self, key: &str)",
            "cancellation terminal retention requires one idempotent queue owner",
        ),
        (
            "if !self.terminal_order.iter().any(|retained| retained == key)",
            "terminal retention must check for an existing key before enqueue",
        ),
        (
            "state.retain_terminal_key(key)",
            "mark_terminal must retain terminal keys through the idempotent helper",
        ),
    )
    for token, detail in cancellation_requirements:
        if token not in text:
            add("R24_CANCEL_RETENTION_IDEMPOTENCY_FORK", cancellation_registry, 1, detail)
    if production_text.count("terminal_order.push_back") != 1:
        add(
            "R24_CANCEL_RETENTION_IDEMPOTENCY_FORK",
            cancellation_registry,
            1,
            "terminal_order.push_back must exist only inside retain_terminal_key",
        )

# Rule 25: hosted Agent authority leases are invalidated by a monotonic
# generation and incarnation state machine. Dynamic enroll/rollback/revoke may
# not use wrapping counters because wraparound can make stale leases or rollback
# receipts indistinguishable from current authority state.
ability_dispatch = cli_root / "src/daemon/ability/dispatch.rs"
if ability_dispatch.exists():
    raw_text = ability_dispatch.read_text(encoding="utf-8", errors="replace")
    text = source(ability_dispatch)
    production_text = text.split("#[cfg(test)]", 1)[0]
    hot_authority_requirements = (
        (
            "fn allocate_incarnation(",
            "hosted Agent authority inventory must own incarnation allocation",
        ),
        (
            "fn advance_generation(",
            "hosted Agent authority inventory must own generation advancement",
        ),
        (
            "HotAgentAuthorityInventoryError::CounterOverflow",
            "hosted Agent authority counter overflow must fail closed",
        ),
        (
            ".checked_add(1)",
            "hosted Agent authority counters must use checked arithmetic",
        ),
        (
            "state.allocate_incarnation(agent)?",
            "hot enrollment must allocate incarnations through the inventory state owner",
        ),
        (
            "state.advance_generation(agent)?",
            "hot enrollment must advance generation through the inventory state owner",
        ),
        (
            "state.advance_generation(&enrollment.agent)?",
            "rollback/revoke must advance generation through the inventory state owner",
        ),
    )
    for token, detail in hot_authority_requirements:
        if token not in production_text:
            add("R25_HOT_AUTHORITY_GENERATION_WRAP", ability_dispatch, 1, detail)
    for pattern, detail in (
        (
            r"state\.generation\s*=\s*state\.generation\.wrapping_add\s*\(",
            "hosted Agent authority generation must not use wrapping arithmetic",
        ),
        (
            r"state\.next_incarnation\s*=\s*state\.next_incarnation\.wrapping_add\s*\(",
            "hosted Agent authority incarnation must not use wrapping arithmetic",
        ),
    ):
        match = re.search(pattern, production_text)
        if match:
            add(
                "R25_HOT_AUTHORITY_GENERATION_WRAP",
                ability_dispatch,
                line_number(production_text, match.start()),
                detail,
            )

    # Rule 25b: ability routeability publication is a catalogue transaction
    # boundary. Public `has_*` checks must require the committed control-plane
    # mode row plus the execution-index handler; `LocalRuntime` ability options
    # are proof/dispatch state, not a catalogue source of truth.
    routeability_helper = rust_method_body(production_text, "routeable_mode_registered")
    if routeability_helper is None:
        add(
            "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
            ability_dispatch,
            1,
            "AxonAbilityCatalog must centralize has_* routeability in routeable_mode_registered",
        )
    else:
        helper_offset, helper_body = routeability_helper
        for token, detail in (
            (
                "control_plane_record_for_mode",
                "routeability must require a committed control-plane mode record",
            ),
            (
                ".has_mode(ability, call_mode)",
                "routeability must require the execution-index handler for the same mode",
            ),
        ):
            if token not in helper_body:
                add(
                    "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                    ability_dispatch,
                    line_number(production_text, helper_offset),
                    detail,
                )

    for method in ("has_rpc", "has_stream", "has_bidi"):
        body = rust_method_body(production_text, method)
        if body is None:
            add(
                "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                ability_dispatch,
                1,
                f"missing AxonAbilityCatalog::{method}",
            )
            continue
        method_offset, method_body = body
        if "routeable_mode_registered" not in method_body:
            add(
                "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                ability_dispatch,
                line_number(production_text, method_offset),
                f"AxonAbilityCatalog::{method} must delegate to routeable_mode_registered",
            )
        for retired in ("ability_options", "runtime_ability_key_for_mode"):
            if retired in method_body:
                add(
                    "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                    ability_dispatch,
                    line_number(production_text, method_offset + method_body.find(retired)),
                    f"AxonAbilityCatalog::{method} must not query LocalRuntime as catalogue fallback",
                )

    if "fn runtime_ability_key_for_mode" in production_text:
        match = re.search(r"fn\s+runtime_ability_key_for_mode", production_text)
        add(
            "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
            ability_dispatch,
            line_number(production_text, match.start() if match else 0),
            "retired runtime-key helper must not remain as a routeability fallback surface",
        )
    for retired, detail in (
        (
            "fn handlers_for_ability",
            "execution index must not synthesize handler sets by merging authority-scoped rows by ability name",
        ),
        (
            "fn fill_missing_from",
            "runtime handler sets must not be assembled through missing-slot fallback from another authority row",
        ),
        (
            "fn list_dynamic_abilities",
            "dynamic ability-name list read model must not remain as publication/diagnostic compatibility surface",
        ),
        (
            "union dynamic with static",
            "catalogue publication must not be documented as a dynamic/static execution-row union",
        ),
        (
            "fall-through paths",
            "routeability comments must not describe retired fall-through handler projection semantics",
        ),
    ):
        if retired in production_text:
            match = re.search(re.escape(retired), production_text)
            add(
                "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                ability_dispatch,
                line_number(production_text, match.start() if match else 0),
                detail,
            )
    for required, detail in (
        (
            "fn unique_handler_slot",
            "ability-name handler lookup must use a named unique slot projection",
        ),
        (
            "fn unique_mode_registered",
            "routeability must require a unique execution row for the requested mode",
        ),
        (
            "fn runtime_handlers_for_key",
            "runtime sync must read handlers by exact authority-scoped execution key",
        ),
    ):
        if required not in production_text:
            add(
                "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                ability_dispatch,
                1,
                detail,
            )
    if "fn control_plane_authority_root" in production_text:
        match = re.search(r"fn\s+control_plane_authority_root", production_text)
        add(
            "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
            ability_dispatch,
            line_number(production_text, match.start() if match else 0),
            "runtime key derivation must not collapse control-plane records by ability name",
        )
    runtime_key_verifier = rust_method_body(production_text, "verify_execution_key_control_plane_modes")
    if runtime_key_verifier is None:
        add(
            "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
            ability_dispatch,
            1,
            "AxonAbilityCatalog must verify runtime keys against exact authority/mode control-plane records",
        )
    else:
        offset, body = runtime_key_verifier
        for token, detail in (
            (
                "control_plane_record_for_authority_mode",
                "runtime key verifier must query exact authority/mode control-plane rows",
            ),
            (
                "key.authority_root()",
                "runtime key verifier must bind the execution authority root",
            ),
            (
                "slot.call_mode()",
                "runtime key verifier must bind every installed handler mode",
            ),
        ):
            if token not in body:
                add(
                    "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                    ability_dispatch,
                    line_number(production_text, offset),
                    detail,
                )
    for token in (
        "static_runtime_key_validates_exact_authority_mode_record",
        "static_runtime_key_rejects_unrelated_authority_record_as_rescue_path",
        "dynamic_runtime_key_validates_exact_authority_mode_record",
        "ability_name_handler_projection_rejects_multi_authority_same_slot",
        "ability_name_handler_projection_does_not_synthesize_cross_authority_runtime_set",
        "dynamic execution row remains present after adding a second mode",
    ):
        if token not in raw_text:
            add(
                "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                ability_dispatch,
                1,
                f"missing exact authority/mode runtime-key test: {token}",
            )
    control_plane_source = cli_root / "src/daemon/ability/control_plane.rs"
    if control_plane_source.exists():
        control_plane_text = source(control_plane_source)
        if "authority_roots_for_ability" in control_plane_text:
            add(
                "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                control_plane_source,
                1,
                "retired ability-level authority-root collapse query must not remain in control-plane registry",
            )

    list_rpc_names = rust_method_body(production_text, "list_rpc_names")
    if list_rpc_names is None:
        add(
            "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
            ability_dispatch,
            1,
            "missing AxonAbilityCatalog::list_rpc_names",
        )
    else:
        offset, body = list_rpc_names
        for token, detail in (
            ("control_plane", "RPC-name publication must read control-plane records"),
            (".records()", "RPC-name publication must project committed control-plane rows"),
            (
                "DescriptorCallMode::Rpc",
                "RPC-name publication must filter committed RPC-mode records",
            ),
        ):
            if token not in body:
                add(
                    "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                    ability_dispatch,
                    line_number(production_text, offset),
                    detail,
                )
        for retired in ("execution_index", "extend_rpc_names"):
            if retired in body:
                add(
                    "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK",
                    ability_dispatch,
                    line_number(production_text, offset + body.find(retired)),
                    "RPC-name publication must not enumerate the execution index",
                )


# Rule 26: MCP tools/list is a unary tools/call catalogue. Until the MCP
# provider owns a real stream/bidi invocation path, non-RPC descriptors may
# remain in the daemon ability catalogue but must not be published as callable
# MCP tools or routed through the unary fallback.
mcp_profile = cli_root / "src/daemon/ability/catalog/profiles/mcp.rs"
if mcp_profile.exists():
    raw_text = mcp_profile.read_text(encoding="utf-8")
    text = source(mcp_profile)
    production_text = text.split("#[cfg(test)]", 1)[0]
    mcp_geometry_requirements = (
        (
            "fn descriptor_is_mcp_callable(",
            "MCP callable geometry must be owned by a named descriptor predicate",
        ),
        (
            "descriptor.call_mode() == crate::daemon::ability::descriptors::CallMode::Rpc",
            "current MCP provider may advertise only RPC descriptors",
        ),
        (
            "if !descriptor_is_mcp_callable(descriptor)",
            "MCP route table must filter through the callable geometry predicate",
        ),
        (
            "provider_excludes_geometries_it_cannot_invoke",
            "MCP callable geometry must have stream/bidi exclusion coverage",
        ),
    )
    for token, detail in mcp_geometry_requirements:
        search_text = raw_text if token.startswith("provider_") else production_text
        if token not in search_text:
            add("R26_MCP_CALLABLE_GEOMETRY_FORK", mcp_profile, 1, detail)
    route_builder_match = re.search(
        r"for\s*\([^)]*descriptor[^)]*\)\s+in\s+descriptors\.iter\(\)\.enumerate\(\)\s*\{(?P<body>.*?)routes\.push",
        production_text,
        re.S,
    )
    if route_builder_match and "descriptor_is_mcp_callable(descriptor)" not in route_builder_match.group("body"):
        add(
            "R26_MCP_CALLABLE_GEOMETRY_FORK",
            mcp_profile,
            line_number(production_text, route_builder_match.start()),
            "MCP route construction must reject non-callable descriptor geometries before push",
        )


# Rule 27: local self admission is a transport-boundary state, not a boolean
# loopback bypass. The daemon serves local IPC and off-box TCP/TLS with the same
# invocation service shape, so the TCP/TLS clone must be explicitly OffBoxStrict
# and no production code may retain a generic loopback_trusted compatibility API.
admission_transport = cli_root / "src/daemon/invocation/admission/admission_facade.rs"
daemon_service_transport = cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service.rs"
boot_invocation_transport = cli_root / "src/daemon/boot/invocation/mod.rs"
if admission_transport.exists():
    raw_text = admission_transport.read_text(encoding="utf-8", errors="replace")
    text = source(admission_transport)
    production_text = text.split("#[cfg(test)]", 1)[0]
    admission_boundary_requirements = (
        (
            "pub enum AdmissionTransportBoundary",
            "admission transport policy must be an explicit state",
        ),
        (
            "LocalOnlyIpc",
            "admission boundary must name the local-only IPC state",
        ),
        (
            "OffBoxStrict",
            "admission boundary must name the off-box strict state",
        ),
        (
            "fn admits_local_self(self) -> bool",
            "local self admission predicate must live on the transport boundary",
        ),
        (
            "pub(crate) fn accepts_local_self_caller(",
            "transport boundary must own the full local self caller predicate",
        ),
        (
            "transport_boundary: AdmissionTransportBoundary",
            "AdmissionFacade must store the explicit transport boundary",
        ),
        (
            "pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self",
            "AdmissionFacade must expose explicit transport-boundary wiring",
        ),
        (
            "fn accepts_local_self_caller(&self, caller_ura: &str) -> bool",
            "AdmissionFacade must centralize local self caller admission",
        ),
        (
            "self.transport_boundary\n            .accepts_local_self_caller",
            "AdmissionFacade local self admission must delegate to the transport boundary owner",
        ),
    )
    for token, detail in admission_boundary_requirements:
        if token not in production_text:
            add("R27_ADMISSION_TRANSPORT_BOUNDARY_FORK", admission_transport, 1, detail)
    for token in (
        "off_box_facade_does_not_accept_daemon_ura_spoof_as_local_self",
        "off_box_facade_does_not_accept_local_system_self_admission",
    ):
        if token not in raw_text:
            add(
                "R27_ADMISSION_TRANSPORT_BOUNDARY_FORK",
                admission_transport,
                1,
                f"missing off-box local-self rejection test: {token}",
            )
    for retired in ("with_loopback_trusted", "loopback_trusted"):
        if retired in production_text:
            add(
                "R27_ADMISSION_TRANSPORT_BOUNDARY_FORK",
                admission_transport,
                1,
                f"retired boolean loopback admission API remains: {retired}",
            )

if daemon_service_transport.exists():
    text = source(daemon_service_transport)
    service_requirements = (
        (
            "AdmissionTransportBoundary",
            "DaemonInvocationService must expose the typed admission transport boundary",
        ),
        (
            "pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self",
            "DaemonInvocationService must wire typed transport boundaries",
        ),
        (
            "struct RuntimeAdmissionPlane",
            "DaemonInvocationService must own admission through a runtime admission plane",
        ),
        (
            "admission_plane: RuntimeAdmissionPlane",
            "DaemonInvocationService must store the runtime admission plane, not a raw AdmissionFacade field",
        ),
        (
            "self.admission_plane = self.admission_plane.with_transport_boundary(boundary)",
            "DaemonInvocationService must delegate the boundary through RuntimeAdmissionPlane",
        ),
    )
    for token, detail in service_requirements:
        if token not in text:
            add("R27_ADMISSION_TRANSPORT_BOUNDARY_FORK", daemon_service_transport, 1, detail)
    if "with_loopback_trusted" in text or "loopback_trusted" in text:
        add(
            "R27_ADMISSION_TRANSPORT_BOUNDARY_FORK",
            daemon_service_transport,
            1,
            "DaemonInvocationService must not retain boolean loopback admission wiring",
        )
    for retired in (
        "legacy generic-route transport policy facade",
        "Transport policy gate retained by route families",
        "    admission: AdmissionFacade,",
        "self.admission.with_transport_boundary(boundary)",
    ):
        if retired in text:
            add(
                "R27_ADMISSION_TRANSPORT_BOUNDARY_FORK",
                daemon_service_transport,
                1,
                f"DaemonInvocationService retained retired admission ownership shape: {retired}",
            )

if boot_invocation_transport.exists():
    text = source(boot_invocation_transport)
    if "with_transport_boundary" not in text or "AdmissionTransportBoundary::OffBoxStrict" not in text:
        add(
            "R27_ADMISSION_TRANSPORT_BOUNDARY_FORK",
            boot_invocation_transport,
            1,
            "TCP/TLS invocation listener must be wired with AdmissionTransportBoundary::OffBoxStrict",
        )
    if "with_loopback_trusted" in text:
        add(
            "R27_ADMISSION_TRANSPORT_BOUNDARY_FORK",
            boot_invocation_transport,
            1,
            "boot must not configure off-box admission with the retired boolean loopback API",
        )

# Rule 28: identity trust-row writers must not own a second local-self model.
# Transport admission owns the local/off-box boundary; IdentityWriteGate may
# project that state for trust-row policy, but it must not revive a separate
# loopback flag or predicate.
identity_write_gate = cli_root / "src/daemon/invocation/admission/identity_write_gate.rs"
unary_dispatcher = cli_root / "src/daemon/invocation/dispatch/unary_dispatcher.rs"
if identity_write_gate.exists():
    raw_text = identity_write_gate.read_text(encoding="utf-8", errors="replace")
    text = source(identity_write_gate)
    production_text = text.split("#[cfg(test)]", 1)[0]
    identity_gate_requirements = (
        (
            "use crate::daemon::invocation::admission::admission_facade::AdmissionTransportBoundary;",
            "IdentityWriteGate must consume the admission transport-boundary type",
        ),
        (
            "transport_boundary: AdmissionTransportBoundary",
            "IdentityWriteGate must store the explicit boundary projection",
        ),
        (
            "fn is_local_self(&self, caller_ura: &str) -> bool",
            "IdentityWriteGate must name local self admission explicitly",
        ),
        (
            ".accepts_local_self_caller(self.daemon_ura.as_deref(), caller_ura)",
            "IdentityWriteGate local self checks must delegate to AdmissionTransportBoundary",
        ),
        (
            "local_self: bool",
            "authorized identity-write caller state must use local_self, not loopback",
        ),
    )
    for token, detail in identity_gate_requirements:
        if token not in production_text:
            add("R28_IDENTITY_WRITE_LOCAL_SELF_BOUNDARY_FORK", identity_write_gate, 1, detail)
    for token in (
        "off_box_boundary_rejects_daemon_ura_spoof_without_anchor_entry",
        "local_self_can_bootstrap_backend_row_without_anchor_entry",
    ):
        if token not in raw_text:
            add(
                "R28_IDENTITY_WRITE_LOCAL_SELF_BOUNDARY_FORK",
                identity_write_gate,
                1,
                f"missing identity-write local-self boundary test: {token}",
            )
    for retired in ("loopback: bool", "caller.loopback", "fn is_loopback("):
        if retired in production_text:
            add(
                "R28_IDENTITY_WRITE_LOCAL_SELF_BOUNDARY_FORK",
                identity_write_gate,
                1,
                f"retired identity-write loopback state remains: {retired}",
            )

if unary_dispatcher.exists():
    text = source(unary_dispatcher)
    if "self.admission.transport_boundary()" not in text:
        add(
            "R28_IDENTITY_WRITE_LOCAL_SELF_BOUNDARY_FORK",
            unary_dispatcher,
            1,
            "UnaryDispatcher must pass AdmissionFacade transport boundary into IdentityWriteGate",
        )


# Rule 29: resolver-selected dispatch binds descriptor refs from the selected
# route's live control-plane publication, not from LocalRuntime options alone.
# LocalRuntime remains the execution-installation check; descriptor
# version/hash/action facts must come from the catalog row that made the route
# callable.
descriptor_binding = cli_root / "src/daemon/invocation/dispatch/descriptor_binding.rs"
stream_dispatcher = cli_root / "src/daemon/invocation/streams/stream_dispatcher.rs"
bidi_dispatcher = cli_root / "src/daemon/invocation/bidi/bidi_dispatcher.rs"
if descriptor_binding.exists():
    raw_text = descriptor_binding.read_text(encoding="utf-8", errors="replace")
    text = source(descriptor_binding)
    from_selected = rust_method_body(text, "from_selected_route")
    if from_selected is None:
        add(
            "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
            descriptor_binding,
            1,
            "RuntimeBoundAbility must expose from_selected_route for resolver-selected dispatch",
        )
    else:
        offset, body = from_selected
        for token, detail in (
            (
                "selected_route_descriptor_ref_from_catalog",
                "selected route binding must derive descriptor ref from catalog proof",
            ),
            (
                "selected_route_descriptor_ref: Some",
                "selected route binding must store the catalog-derived descriptor ref",
            ),
        ):
            if token not in body:
                add(
                    "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
                    descriptor_binding,
                    line_number(text, offset),
                    detail,
                )
    for token, detail in (
        (
            "catalog: Option<&AxonAbilityCatalog>",
            "selected route binding must receive the live ability catalog",
        ),
        (
            "mode: CallMode",
            "selected route binding must receive the concrete dispatch call mode",
        ),
    ):
        if token not in text:
            add(
                "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
                descriptor_binding,
                1,
                detail,
            )
    catalog_helper = rust_method_body(text, "selected_route_descriptor_ref_from_catalog")
    if catalog_helper is None:
        add(
            "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
            descriptor_binding,
            1,
            "selected route descriptor proof must have one catalog helper",
        )
    else:
        offset, body = catalog_helper
        for token, detail in (
            (
                "control_plane_record_for_authority_mode",
                "selected route proof must read the live authority+mode control-plane row",
            ),
            (
                "options.proof_for_mode(mode)",
                "selected route proof must compare LocalRuntime installation proof for the same mode",
            ),
            (
                "proof.descriptor_version != descriptor.version.as_str()",
                "selected route proof must compare descriptor version",
            ),
            (
                "proof.descriptor_hash != expected_descriptor_hash",
                "selected route proof must compare descriptor hash",
            ),
            (
                "proof.schema_hash != expected_schema_hash",
                "selected route proof must compare schema hash",
            ),
            (
                "proof.impl_hash != expected_impl_hash",
                "selected route proof must compare implementation hash",
            ),
            (
                "proof.admission_action != descriptor.admission_action().as_str()",
                "selected route proof must compare admission action",
            ),
        ):
            if token not in body:
                add(
                    "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
                    descriptor_binding,
                    line_number(text, offset),
                    detail,
                )
    descriptor_ref_for_mode = rust_method_body(text, "descriptor_ref_for_mode")
    if descriptor_ref_for_mode is None or "selected_route_descriptor_ref" not in descriptor_ref_for_mode[1]:
        add(
            "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
            descriptor_binding,
            1,
            "descriptor_ref_for_mode must return selected-route catalog proof before runtime fallback",
        )
    wire_match = rust_method_body(text, "require_wire_target_matches")
    if wire_match is None:
        add(
            "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
            descriptor_binding,
            1,
            "RuntimeBoundAbility must verify request wire targets through a typed state boundary",
        )
    else:
        offset, body = wire_match
        for token, detail in (
            (
                "WireAbilityTarget::parse(surface, callee_ura, wire_target)?",
                "wire-target matching must parse DescriptorRef and OwnerLocal as explicit states",
            ),
            (
                "wire_target.ability_ura() != self.runtime_ability_ura",
                "wire-target matching must compare the typed target ability with the selected runtime ability",
            ),
            (
                "status_from_dispatch_key_mismatch",
                "wire-target mismatch must preserve dispatch-key mismatch semantics",
            ),
        ):
            if token not in body:
                add(
                    "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
                    descriptor_binding,
                    line_number(text, offset),
                    detail,
                )
    for token, detail in (
        ("enum WireAbilityTarget", "wire targets must be represented by an explicit state enum"),
        ("DescriptorRef {", "wire targets must distinguish descriptor-bound refs"),
        ("OwnerLocal {", "wire targets must distinguish owner-local public selectors"),
        (
            "fn is_descriptor_bound_wire_target(",
            "descriptor-like targets must be classified before owner-local normalization",
        ),
    ):
        if token not in text:
            add(
                "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
                descriptor_binding,
                1,
                detail,
            )
    if "historic forms" in raw_text:
        add(
            "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
            descriptor_binding,
            1,
            "runtime wire-target binding must not describe owner-local ingress as historic fallback forms",
        )
    for token in (
        "selected_route_descriptor_ref_comes_from_live_catalog_for_all_modes",
        "selected_route_rejects_missing_catalog_descriptor_proof",
        "selected_route_rejects_runtime_proof_that_drifted_from_catalog",
        "wire_target_match_accepts_owner_local_selector_explicitly",
        "wire_target_match_accepts_descriptor_bound_selector_explicitly",
        "wire_target_match_rejects_malformed_descriptor_like_target_without_owner_local_reinterpretation",
    ):
        if token not in raw_text:
            add(
                "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
                descriptor_binding,
                1,
                f"missing selected-route descriptor proof test: {token}",
            )

for path, surface in (
    (unary_dispatcher, "unary"),
    (stream_dispatcher, "stream"),
    (bidi_dispatcher, "bidi"),
):
    if not path.exists():
        continue
    text = source(path)
    if "RuntimeBoundAbility::from_selected_route" not in text:
        continue
    if "local_ability_catalog.as_deref()" not in text or "call_mode" not in text:
        add(
            "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK",
            path,
            1,
            f"{surface} selected-route dispatch must pass live catalog and call mode into RuntimeBoundAbility",
        )

# Rule 62: hosted-Agent session publication has one Hub-owned generation.
# The Device persists an opaque incarnation before transport, the Hub returns
# the generation assignment, and only the exact activated assignment may
# produce an ability projection. Device code must never send or mint a local
# generation in `federation.advertise_agent`.
session_prelude = cli_root / "src/daemon/invocation/bidi/session_initiator/prelude.rs"
if session_prelude.exists():
    text = source(session_prelude)
    production_text = text.split("#[cfg(test)]", 1)[0]
    advertise_entry = rust_method_body(production_text, "advertise_hosted_agent_entry")
    if advertise_entry is None:
        add(
            "R62_HOSTED_AGENT_PRELUDE_GENERATION_FORK",
            session_prelude,
            1,
            "hosted-agent prelude must have one publication entrypoint",
        )
    else:
        _, body = advertise_entry
        for token, detail in (
            (
                "HostedAgentPublicationPlan::begin",
                "hosted-agent prelude must persist an incarnation plan before identity advertisement",
            ),
            (
                "plan.identity_payload_bytes()?",
                "identity advertisement must carry the persisted incarnation rather than a Device-owned generation",
            ),
            (
                "let active = plan.activate(assignment)?;",
                "ability projection must activate the exact Hub generation assignment",
            ),
            (
                "advertise_hosted_agent_abilities(&mut advertise_ctx, entry, &active)",
                "ability projection must consume the activated publication plan",
            ),
        ):
            if token not in body:
                add(
                    "R62_HOSTED_AGENT_PRELUDE_GENERATION_FORK",
                    session_prelude,
                    line_number(production_text, advertise_entry[0]),
                    detail,
                )
    advertise_agent = rust_method_body(production_text, "send_advertise_agent_prelude")
    if advertise_agent is None:
        add(
            "R62_HOSTED_AGENT_PRELUDE_GENERATION_FORK",
            session_prelude,
            1,
            "hosted-agent identity serializer is missing",
        )
    else:
        offset, body = advertise_agent
        if "generation:" in body or re.search(r'"generation"\s*:', body):
            add(
                "R62_HOSTED_AGENT_PRELUDE_GENERATION_FORK",
                session_prelude,
                line_number(production_text, offset),
                "hosted-agent identity advertisement must not accept or serialize a Device-owned generation",
            )
        for token, detail in (
            (
                "signed_prelude_request(signer, agent_ura, \"federation.advertise_agent\", arguments)",
                "hosted-agent identity transport must consume the plan's incarnation payload",
            ),
            (
                "Ok(receipt.assignment)",
                "hosted-agent identity transport must return a typed Hub assignment",
            ),
            (
                "receipt.assignment.validate()",
                "hosted-agent identity transport must validate the Hub assignment",
            ),
        ):
            if token not in body:
                add(
                    "R62_HOSTED_AGENT_PRELUDE_GENERATION_FORK",
                    session_prelude,
                    line_number(production_text, offset),
                    detail,
                )
    if "HostedAgentGenerationAssignment" not in production_text:
        add(
            "R62_HOSTED_AGENT_PRELUDE_GENERATION_FORK",
            session_prelude,
            1,
            "hosted-agent prelude must consume a typed Hub generation assignment",
        )

# Rule 63: manifest-bound EAL exec has a real run-level timeout. The
# `[exec] kind = "eal"` timeout is a public ability SLA, not a local variable
# to compute and discard. The canonical mission runner owns the deadline and
# the interpreter clips child dispatch timeouts to the remaining run budget.
eal_executor = cli_root / "src/daemon/execution/mission/executors/eal.rs"
mission_orchestration = cli_root / "src/daemon/execution/mission/orchestration.rs"
eal_interpreter = cli_root / "src/eal/interpreter/mod.rs"
eal_retry = cli_root / "src/eal/interpreter/retry.rs"
if eal_executor.exists():
    text = source(eal_executor)
    production_text = text.split("#[cfg(test)]", 1)[0]
    discarded_timeout = re.search(r"let\s+_\s*=\s*timeout\s*\.", production_text)
    if discarded_timeout:
        add(
            "R63_EAL_EXEC_RUN_TIMEOUT_FORK",
            eal_executor,
            line_number(production_text, discarded_timeout.start()),
            "EAL executor must not discard the manifest timeout",
        )
    for token, detail in (
        (
            "let effective_timeout = timeout.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS))",
            "EAL executor must materialize one effective timeout",
        ),
        (
            "mission_run_opts(effective_timeout)",
            "EAL executor must pass the effective timeout into mission opts",
        ),
        (
            "run_timeout: Some(run_timeout)",
            "EAL mission opts must carry the manifest timeout as a run deadline",
        ),
    ):
        if token not in production_text:
            add("R63_EAL_EXEC_RUN_TIMEOUT_FORK", eal_executor, 1, detail)
if mission_orchestration.exists():
    text = source(mission_orchestration).split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "pub run_timeout: Option<std::time::Duration>",
            "MissionRunOpts must expose a run-level timeout for manifest executors",
        ),
        (
            "execute_with_gateway_for_trace_with_timeout",
            "canonical mission runner must pass run timeout into the interpreter",
        ),
        (
            "opts.run_timeout",
            "canonical mission runner must consume MissionRunOpts.run_timeout",
        ),
    ):
        if token not in text:
            add("R63_EAL_EXEC_RUN_TIMEOUT_FORK", mission_orchestration, 1, detail)
if eal_interpreter.exists():
    text = source(eal_interpreter).split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "pub deadline: Option<Instant>",
            "RunContext must carry the mission deadline to child dispatch",
        ),
        (
            "execute_with_dispatcher_for_trace_with_timeout",
            "interpreter must expose the deadline-aware execution path",
        ),
        (
            "Instant::now().checked_add(timeout)",
            "interpreter must convert run timeout into a fixed deadline once",
        ),
    ):
        if token not in text:
            add("R63_EAL_EXEC_RUN_TIMEOUT_FORK", eal_interpreter, 1, detail)
if eal_retry.exists():
    text = source(eal_retry).split("#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "effective_dispatch_timeout_ms(step_timeout_ms, run.deadline)",
            "retry layer must combine step timeout with remaining run deadline",
        ),
        (
            "EalError::DeadlineExceeded",
            "expired run deadline must fail closed as a typed timeout",
        ),
        (
            "step_timeout.min(remaining_ms)",
            "child dispatch timeout must be clipped to the remaining run deadline",
        ),
    ):
        if token not in text:
            add("R63_EAL_EXEC_RUN_TIMEOUT_FORK", eal_retry, 1, detail)

# Rule 64: session dispatch has one canonical invocation carrier. The JSON
# session envelope owns daemon control and bidi input only; protobuf
# DispatchCall/DispatchResult owns invocation open, progress, and completion.
# Stream/bidi lifecycle terminal frames are projected only from Axon's
# FinalizedInvocation and therefore always carry a terminal receipt.
session_wire = cli_root / "src/daemon/invocation/bidi/session_wire.rs"
local_session_dispatcher = (
    cli_root / "src/daemon/invocation/dispatch/local_session_dispatcher.rs"
)
bidi_dispatcher = cli_root / "src/daemon/invocation/bidi/bidi_dispatcher.rs"
if session_wire.exists():
    text = session_wire.read_text(encoding="utf-8", errors="replace")
    for pattern, detail in (
        (
            r"\bBidiOpen\s*\{",
            "JSON SessionDispatch must not own canonical bidi open",
        ),
        (
            r"\bResult\s*\{",
            "JSON SessionDispatch must not own invocation result or terminal state",
        ),
    ):
        match = re.search(pattern, text)
        if match:
            add(
                "R64_SESSION_CANONICAL_CARRIER_FORK",
                session_wire,
                line_number(text, match.start()),
                detail,
            )
if local_session_dispatcher.exists():
    text = local_session_dispatcher.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "SessionDispatch::Result",
            "local session dispatch must not emit the retired JSON invocation result",
        ),
        (
            "SessionDispatch::BidiOpen",
            "local session dispatch must not accept the retired JSON bidi open",
        ),
        (
            "session_error_result",
            "session control failure must not synthesize an invocation terminal result",
        ),
        (
            "send_bidi_result",
            "bidi output must use explicit admission, progress, control-failure, and terminal projections",
        ),
    ):
        if token in text:
            add("R64_SESSION_CANONICAL_CARRIER_FORK", local_session_dispatcher, 1, detail)
    for token, detail in (
        (
            "fn json_frame_error_reason",
            "JSON-frame bidi error projection must not derive default failure reasons from incomplete handler frames",
        ),
        (
            "file_transfer handler returned error",
            "file_transfer error projection must require explicit handler code/message facts",
        ),
        (
            "JSON-frame bidi handler returned error",
            "JSON-frame bidi error projection must require explicit handler code/message facts",
        ),
    ):
        if token in text:
            add("R64_SESSION_CANONICAL_CARRIER_FORK", local_session_dispatcher, 1, detail)
    for token, detail in (
        (
            "struct HandlerErrorFrame",
            "local session dispatcher must centralize handler error-frame schema validation",
        ),
        (
            "handler_error_frame_requires_code_and_message_before_failure_projection",
            "local session dispatcher tests must prove incomplete handler error frames fail closed",
        ),
        (
            "file_transfer_error_frame_rejects_missing_message_before_failure_projection",
            "file_transfer error-frame tests must prove missing message is not projected as default terminal failure",
        ),
    ):
        if token not in text:
            add("R64_SESSION_CANONICAL_CARRIER_FORK", local_session_dispatcher, 1, detail)

    control_failure = rust_method_body(text, "canonical_carrier_control_failure")
    if control_failure is None:
        add(
            "R64_SESSION_CANONICAL_CARRIER_FORK",
            local_session_dispatcher,
            1,
            "canonical session control failures need one named non-terminal projection",
        )
    else:
        offset, body = control_failure
        if "terminal: false" not in body or "terminal_receipt" in body:
            add(
                "R64_SESSION_CANONICAL_CARRIER_FORK",
                local_session_dispatcher,
                line_number(text, offset),
                "session control failure must be non-terminal and receipt-free",
            )

    terminal_projection = rust_method_body(text, "send_bidi_terminal")
    if terminal_projection is None:
        add(
            "R64_SESSION_CANONICAL_CARRIER_FORK",
            local_session_dispatcher,
            1,
            "canonical bidi terminal projection is missing",
        )
    else:
        offset, body = terminal_projection
        signature = text[offset : text.find("{", offset)]
        terminal_projection_body = body
        if "canonical_terminal_dispatch_result" in body:
            shared_terminal_projection = rust_method_body(
                text, "canonical_terminal_dispatch_result"
            )
            if shared_terminal_projection is not None:
                terminal_projection_body += shared_terminal_projection[1]
        for token, detail in (
            (
                "FinalizedInvocation",
                "bidi terminal projection must consume Axon's FinalizedInvocation",
            ),
            (
                "terminal: true",
                "bidi terminal projection must explicitly close the carrier lifecycle",
            ),
            (
                "terminal_receipt: Some(receipt_to_session_wire(",
                "bidi terminal projection must carry the canonical terminal receipt",
            ),
        ):
            haystack = (
                signature
                if token == "FinalizedInvocation"
                else terminal_projection_body
            )
            if token not in haystack:
                add(
                    "R64_SESSION_CANONICAL_CARRIER_FORK",
                    local_session_dispatcher,
                    line_number(text, offset),
                    detail,
                )
if bidi_dispatcher.exists():
    text = bidi_dispatcher.read_text(encoding="utf-8", errors="replace")
    if "SessionDispatch::Result" in text:
        add(
            "R64_SESSION_CANONICAL_CARRIER_FORK",
            bidi_dispatcher,
            1,
            "hub dispatch must not dual-read the retired JSON invocation result",
        )
    if "fn classify_canonical_carrier_result" not in text:
        add(
            "R64_SESSION_CANONICAL_CARRIER_FORK",
            bidi_dispatcher,
            1,
            "hub dispatch must retain one canonical result classifier",
        )
    else:
        for token, detail in (
            (
                "if result.terminal_receipt.is_none()",
                "stream terminal classification must reject a missing terminal receipt",
            ),
            (
                "CANONICAL_TERMINAL_RECEIPT_REQUIRED",
                "receiptless stream terminal rejection must remain a typed protocol failure",
            ),
        ):
            if token not in text:
                add(
                    "R64_SESSION_CANONICAL_CARRIER_FORK",
                    bidi_dispatcher,
                    1,
                    detail,
                )


# Rule 67: trust-anchor bare URA lookup must stay singleton-only.
# User-role trust rows are DEC-EU multi-key bindings; selecting one
# key from a user bucket by bare URA reintroduces a second signer
# resolution authority and can mask missing descriptor-bound signer
# material. Callers must use lookup_user_by_pubkey/lookup_user_all
# for user entries.
trust_anchor = cli_root / "src/daemon/trust/anchor.rs"
if trust_anchor.exists():
    text = trust_anchor.read_text(encoding="utf-8", errors="replace")
    lookup_match = re.search(r"\bpub\s+fn\s+lookup\s*\(\s*&self\s*,", text)
    if lookup_match is None:
        add(
            "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK",
            trust_anchor,
            1,
            "RealmTrustAnchor::lookup must exist as the singleton trust-anchor lookup",
        )
    else:
        offset = lookup_match.start()
        lookup_body = brace_block_from(text, offset)
        for token, detail in (
            (
                "self.users",
                "bare RealmTrustAnchor::lookup must not read user multi-key buckets",
            ),
            (
                "lookup_user_by_pubkey",
                "bare RealmTrustAnchor::lookup must not delegate to user-key lookup without a presented pubkey",
            ),
            (
                "lookup_user_all",
                "bare RealmTrustAnchor::lookup must not enumerate user keys",
            ),
        ):
            if token in lookup_body:
                add(
                    "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK",
                    trust_anchor,
                    line_number(text, offset),
                    detail,
                )
        if "self.by_ura.get(" not in lookup_body:
            add(
                "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK",
                trust_anchor,
                line_number(text, offset),
                "RealmTrustAnchor::lookup must resolve through the singleton by_ura map",
            )
    for pattern, detail in (
        (
            r"User bucket fallback",
            "trust anchor must not document or preserve user bucket fallback",
        ),
        (
            r"lex-smallest",
            "trust anchor must not select arbitrary user keys by deterministic ordering",
        ),
        (
            r"single-keypair fallback",
            "trust anchor must not preserve single-keypair user fallback semantics",
        ),
    ):
        match = re.search(pattern, text, flags=re.I)
        if match:
            add(
                "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK",
                trust_anchor,
                line_number(text, match.start()),
                detail,
            )

trust_key_resolver = cli_root / "src/daemon/trust/key_resolver.rs"
if trust_key_resolver.exists():
    text = source(trust_key_resolver)
    resolve_all = rust_method_body(text, "resolve_all")
    if resolve_all is None:
        add(
            "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK",
            trust_key_resolver,
            1,
            "RealmTrustAnchorKeyResolver::resolve_all must own user bucket key-material validation",
        )
    else:
        offset, body = resolve_all
        body_start = text.find("{", offset) + 1
        for token, detail in (
            (
                "filter_map",
                "user key bucket resolution must not skip corrupt key rows",
            ),
            (
                "decode_pubkey(&row.public_key_b64, agent_ura).ok()",
                "user key bucket resolution must fail closed on corrupt key material",
            ),
            (
                "decode_pubkey(&row.public_key_b64, caller_ura).ok()",
                "user key bucket resolution must fail closed on corrupt key material",
            ),
        ):
            if token in body:
                add(
                    "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK",
                    trust_key_resolver,
                    line_number(text, body_start + body.find(token)),
                    detail,
                )
        if (
            "decode_pubkey(&row.public_key_b64, agent_ura)?" not in body
            and "decode_pubkey(&row.public_key_b64, caller_ura)?" not in body
        ):
            add(
                "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK",
                trust_key_resolver,
                line_number(text, offset),
                "user key bucket resolution must propagate decode_pubkey errors",
            )


# Rule 68: API key credential store load must fail closed on malformed state.
# A missing api_keys.toml is a valid fresh-install empty state, but an existing
# malformed credential authority cannot be projected as "no keys"; doing so
# turns bearer-auth corruption into "token not recognized" and lets create
# overwrite the evidence.
api_key_store = cli_root / "src/daemon/ability/builtins/governance/api_key.rs"
if api_key_store.exists():
    text = source(api_key_store)
    body = rust_method_body(text, "load_store")
    if body is None:
        add(
            "R68_API_KEY_STORE_PARSE_FALLBACK",
            api_key_store,
            1,
            "api_key credential store must have one load_store authority",
        )
    else:
        offset, load_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset : body_start]
        if "anyhow::Result<ApiKeyStore>" not in signature:
            add(
                "R68_API_KEY_STORE_PARSE_FALLBACK",
                api_key_store,
                line_number(text, offset),
                "api_key load_store must return Result so malformed authority state can surface",
            )
        for token, detail in (
            (
                "toml::from_str(&text).unwrap_or_default()",
                "api_key load_store must not parse-fallback malformed TOML to an empty store",
            ),
            (
                "toml::from_str(&text).ok()",
                "api_key load_store must not hide malformed TOML behind Option",
            ),
        ):
            if token in load_body:
                add(
                    "R68_API_KEY_STORE_PARSE_FALLBACK",
                    api_key_store,
                    line_number(text, body_start + load_body.find(token)),
                    detail,
                )
        if "parse API key store" not in load_body:
            add(
                "R68_API_KEY_STORE_PARSE_FALLBACK",
                api_key_store,
                line_number(text, offset),
                "api_key load_store must attach a parse-store diagnostic",
            )


# Rule 69: Context clipboard JSONL is an append-only read model, not a cache.
# Missing clipboard.jsonl is an empty history; unreadable files and malformed
# rows must surface as unavailable/corrupt context state.
context_store = cli_root / "src/daemon/persistence/context_store.rs"
if context_store.exists():
    text = source(context_store)
    if "fn read_clipboard_log() -> anyhow::Result<Option<String>>" not in text:
        add(
            "R69_CONTEXT_CLIPBOARD_HISTORY_FALLBACK",
            context_store,
            1,
            "context clipboard history must have a fallible missing-vs-unavailable log loader",
        )
    for name, expected in (
        ("list_clips", "anyhow::Result<Vec<ClipEntry>>"),
        ("list_clip_summaries", "anyhow::Result<Vec<ClipListEntry>>"),
    ):
        body = rust_method_body(text, name)
        if body is None:
            add(
                "R69_CONTEXT_CLIPBOARD_HISTORY_FALLBACK",
                context_store,
                1,
                f"context clipboard {name} must remain an explicit read-model surface",
            )
            continue
        offset, load_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if expected not in signature:
            add(
                "R69_CONTEXT_CLIPBOARD_HISTORY_FALLBACK",
                context_store,
                line_number(text, offset),
                f"context clipboard {name} must return {expected}",
            )
        for token, detail in (
            (
                "return Vec::new()",
                f"context clipboard {name} must not project read failure as empty history",
            ),
            (
                "filter_map(|l| serde_json::from_str",
                f"context clipboard {name} must not skip malformed JSONL rows",
            ),
            (
                ".ok())",
                f"context clipboard {name} must not hide malformed JSONL rows behind Option",
            ),
        ):
            if token in load_body:
                add(
                    "R69_CONTEXT_CLIPBOARD_HISTORY_FALLBACK",
                    context_store,
                    line_number(text, body_start + load_body.find(token)),
                    detail,
                )
    remove_body = rust_method_body(text, "remove_clip")
    if remove_body is not None:
        offset, body = remove_body
        body_start = text.find("{", offset) + 1
        if "unwrap_or_default()" in body:
            add(
                "R69_CONTEXT_CLIPBOARD_HISTORY_FALLBACK",
                context_store,
                line_number(text, body_start + body.find("unwrap_or_default()")),
                "context clipboard remove must not project read failure as empty history",
            )
        if "parse context clipboard log" not in body:
            add(
                "R69_CONTEXT_CLIPBOARD_HISTORY_FALLBACK",
                context_store,
                line_number(text, offset),
                "context clipboard remove must validate JSONL rows before rewrite",
            )


# Rule 70: Node Runtime Core must validate typed authority metadata against the
# descriptor-bound invocation tuple before transport. Shape-only metadata
# validation lets product callers submit stale user/session authority for
# device-owned system abilities and pushes deterministic AUTHORITY_* failures
# into daemon admission.
node_sdk = cli_root / "sdk/node/index.js"
if node_sdk.exists():
    text = source(node_sdk)
    required_tokens = (
        (
            '"AUTHORITY_SUBJECT_MISMATCH"',
            "Node SDK must expose the canonical authority-subject mismatch error code",
        ),
        (
            "validateInvocationAuthorityBinding(this);",
            "InvocationDraft must validate typed authority metadata against the tuple",
        ),
        (
            "function validateInvocationAuthorityBinding(draft)",
            "Node SDK must own one invocation authority-binding validator",
        ),
        (
            "class InvocationAuthorityBindingValidator",
            "Node SDK authority-binding validation must stay in a cohesive validator object",
        ),
        (
            "function sessionAuthorityAdmitsSubject(authority, subjectURA)",
            "Node SDK must mirror daemon session subject-admission semantics",
        ),
        (
            "function validateSessionAuthoritySubjectBinding(",
            "Node SDK must validate session authority subject ownership before metadata is accepted",
        ),
        (
            "function canonicalAuthoritySubject(",
            "Node SDK must classify canonical user/session subjects before transport",
        ),
        (
            "function authorityScopesAdmit(patterns, ability)",
            "Node SDK must validate authority scopes against runtime ability projection",
        ),
        (
            "class RuntimeAbilityProjection",
            "Node SDK must own descriptor-to-scope projection as a canonical runtime object",
        ),
        (
            "RuntimeAbilityProjection.fromInvocation(draft)",
            "Node SDK authority validator must consume the canonical runtime ability projection",
        ),
        (
            "descriptor_ref must contain a canonical Ability URA",
            "Node SDK must reject authority-bound bare descriptor names",
        ),
    )
    for token, detail in required_tokens:
        if token not in text:
            add("R70_NODE_AUTHORITY_BINDING_PREFLIGHT", node_sdk, 1, detail)
    draft_offset = text.find("export class InvocationDraft")
    if draft_offset >= 0:
        next_class = text.find("\nexport class ", draft_offset + 1)
        draft_body = text[draft_offset : next_class if next_class >= 0 else len(text)]
        if "validateAuthorityMetadata(this.metadata);" in draft_body and (
            "validateInvocationAuthorityBinding(this);" not in draft_body
        ):
            add(
                "R70_NODE_AUTHORITY_BINDING_PREFLIGHT",
                node_sdk,
                line_number(text, draft_offset),
                "InvocationDraft must not stop at shape-only authority metadata validation",
            )
    if "authorityMetadataValue(value, SESSION_AUTHORITY_METADATA_KEY);" in text and (
        "SessionAuthority.fromMetadata(session)" not in text
    ):
        add(
            "R70_NODE_AUTHORITY_BINDING_PREFLIGHT",
            node_sdk,
            1,
            "Node SDK must decode session authority metadata before draft submission",
        )
    for forbidden, detail in (
        (
            "abilityViewForInvocation",
            "Node SDK must not retain retired ability-view descriptor fallback",
        ),
    ):
        if forbidden in text:
            add("R70_NODE_AUTHORITY_BINDING_PREFLIGHT", node_sdk, 1, detail)


# Rule 71: Node type-level SDK tests must exercise the generic runtime surface
# without importing downstream workflow/profile placeholders at runtime. A
# static ESM import fails before the test can assert SDK neutrality and
# pressures the SDK to restore non-canonical convenience symbols.
node_types_test = cli_root / "sdk/node/test/types.test.ts"
if node_types_test.exists():
    text = source(node_types_test)
    downstream_profile_symbols = (
        "WorkflowClient",
        "WorkflowTransport",
        "ApplicationLifecycleClient",
        "ApplicationDirectoryView",
        "ApplicationReceiptPage",
        "TranslationLayer",
        "ConvenienceWrapperClient",
        "ProfileBundle",
    )
    for symbol in downstream_profile_symbols:
        for token in (
            f"import {{ {symbol} }}",
            f"import {{ type {symbol} }}",
            f"import type {{ {symbol} }}",
            f"void {symbol}",
        ):
            if token in text:
                add(
                    "R71_NODE_PRODUCT_NEUTRAL_TYPES_TEST",
                    node_types_test,
                    line_number(text, text.find(token)),
                    f"Node type tests must not import downstream profile symbol {symbol}",
                )
    if "opaque-authority" in text:
        add(
            "R71_NODE_PRODUCT_NEUTRAL_TYPES_TEST",
            node_types_test,
            line_number(text, text.find("opaque-authority")),
            "Node type tests must use typed authority metadata, not opaque compatibility metadata",
        )
    for token, detail in (
        (
            "Object.hasOwn(sdk, symbol)",
            "Node type tests must assert downstream profile symbols are absent from runtime exports",
        ),
        (
            "declarations.includes(symbol)",
            "Node type tests must assert downstream profile symbols are absent from index.d.ts",
        ),
    ):
        if token not in text:
            add("R71_NODE_PRODUCT_NEUTRAL_TYPES_TEST", node_types_test, 1, detail)


# Rule 71B: Receipt proof-fact identity profiles are part of the canonical
# runtime evidence model. Language SDKs must not keep local legacy/opaque
# profile whitelists that diverge from the Go/Python Axon parser path.
sdk_receipt_profile_files = {
    "node": cli_root / "sdk/node/index.js",
    "swift": cli_root / "sdk/swift/Sources/RuntimeSDK/Runtime.swift",
    "java": cli_root / "sdk/java/src/main/java/run/runtime/sdk/RuntimeReceiptProofFacts.java",
}
for language, path in sdk_receipt_profile_files.items():
    if not path.exists():
        continue
    text = source(path)
    if "axon-legacy-v1" in text:
        add(
            "R71B_SDK_RECEIPT_PROFILE_CONVERGENCE",
            path,
            line_number(text, text.find("axon-legacy-v1")),
            f"{language} receipt proof-fact validator must not admit retired axon-legacy-v1 profiles",
        )
    for token in (
        '["axon-strict-v2", "axon-legacy-v1", "opaque"]',
        'case "axon-strict-v2", "axon-legacy-v1", "opaque"',
    ):
        if token in text:
            add(
                "R71B_SDK_RECEIPT_PROFILE_CONVERGENCE",
                path,
                line_number(text, text.find(token)),
                f"{language} receipt proof-fact validator must not use a local legacy/opaque profile whitelist",
            )
required_profile_markers = {
    "node": 'profile !== "axon-strict-v2"',
    "swift": 'profile == "axon-strict-v2"',
    "java": 'case "axon-strict-v2" -> {}',
}
for language, marker in required_profile_markers.items():
    path = sdk_receipt_profile_files[language]
    if path.exists() and marker not in source(path):
        add(
            "R71B_SDK_RECEIPT_PROFILE_CONVERGENCE",
            path,
            1,
            f"{language} receipt proof-fact validator must fail closed to axon-strict-v2 only",
        )

java_receipt = cli_root / "sdk/java/src/main/java/run/runtime/sdk/RuntimeReceipt.java"
java_proof = cli_root / "sdk/java/src/main/java/run/runtime/sdk/RuntimeReceiptProofFacts.java"
java_tests = cli_root / "sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
if java_receipt.exists() and java_proof.exists():
    receipt_text = source(java_receipt)
    proof_text = source(java_proof)
    proof_offset = receipt_text.find("RuntimeReceiptProofFacts.validate(this.raw)")
    summary_offset = receipt_text.find('requiredString(this.raw, "invocation_id")')
    if proof_offset < 0 or summary_offset < 0 or proof_offset > summary_offset:
        add(
            "R71C_JAVA_RECEIPT_PROOF_FACTS_MANDATORY",
            java_receipt,
            1,
            "Java RuntimeReceipt must validate mandatory proof facts before summary projection",
        )
    if "SDKError.receiptProofFactsMissing(" not in proof_text:
        add(
            "R71C_JAVA_RECEIPT_PROOF_FACTS_MANDATORY",
            java_proof,
            1,
            "Java missing receipt proof facts must use RECEIPT_PROOF_FACTS_MISSING",
        )
if java_tests.exists():
    tests_text = source(java_tests)
    for marker, detail in (
        (
            "mandatoryRuntimeReceiptProofFactFields",
            "Java tests must cover the complete mandatory receipt proof-fact omission matrix",
        ),
        (
            "RECEIPT_PROOF_FACTS_MISSING",
            "Java tests must assert the canonical missing-proof-facts error code",
        ),
    ):
        if marker not in tests_text:
            add("R71C_JAVA_RECEIPT_PROOF_FACTS_MANDATORY", java_tests, 1, detail)


# Rule 72: Sidecar stderr is plugin failure evidence. The host must preserve
# binary stderr lossily and capture reader failures explicitly; it must not
# collapse read errors, UTF-8 errors, or reader panics into an empty diagnostic.
sidecar_io = cli_root / "src/daemon/plugins/sidecar/io.rs"
if sidecar_io.exists():
    text = source(sidecar_io)
    required_tokens = (
        (
            "fn capture_stderr_diagnostics",
            "sidecar stderr capture must have one diagnostic-preserving helper",
        ),
        (
            "String::from_utf8_lossy(&bytes)",
            "sidecar stderr capture must preserve binary stderr lossily",
        ),
        (
            "sidecar stderr capture failed",
            "sidecar stderr read failures must be surfaced in diagnostics",
        ),
        (
            "sidecar stderr reader panicked",
            "sidecar stderr reader panics must be surfaced in diagnostics",
        ),
    )
    for token, detail in required_tokens:
        if token not in text:
            add("R72_PLUGIN_SIDECAR_STDERR_DIAGNOSTICS", sidecar_io, 1, detail)
    collect_body = rust_method_body(text, "collect_stderr")
    if collect_body is not None:
        offset, body = collect_body
        body_start = text.find("{", offset) + 1
        for token, detail in (
            (
                "unwrap_or_default()",
                "sidecar stderr collection must not default reader failure to empty diagnostics",
            ),
            (
                ".join().ok()",
                "sidecar stderr collection must not hide reader panics behind Option",
            ),
        ):
            if token in body:
                add(
                    "R72_PLUGIN_SIDECAR_STDERR_DIAGNOSTICS",
                    sidecar_io,
                    line_number(text, body_start + body.find(token)),
                    detail,
                )
    spawn_body = rust_method_body(text, "spawn_stderr_reader")
    if spawn_body is not None:
        offset, body = spawn_body
        body_start = text.find("{", offset) + 1
        for token, detail in (
            (
                "read_to_string",
                "sidecar stderr capture must not use UTF-8-only read_to_string",
            ),
            (
                "let _ = reader.read",
                "sidecar stderr capture must not ignore reader failures",
            ),
        ):
            if token in body:
                add(
                    "R72_PLUGIN_SIDECAR_STDERR_DIAGNOSTICS",
                    sidecar_io,
                    line_number(text, body_start + body.find(token)),
                    detail,
                )


# Rule 75: Plugin bidi wire profiles are a runtime-state projection. Daemon
# boot must consume the same plugin runtime manager that populated the ability
# catalog; it must not hide package-state failures by falling back to a
# core-only wire registry.
plugin_runtime_manager = cli_root / "src/daemon/plugins/runtime_manager.rs"
if plugin_runtime_manager.exists():
    text = source(plugin_runtime_manager)
    if "pub fn new() -> Result<Self>" not in text:
        add(
            "R75_PLUGIN_WIRE_PROFILE_FAIL_CLOSED",
            plugin_runtime_manager,
            1,
            "PluginRuntimeManager::new must be fallible so default package-state failure cannot boot as core-only wire registry",
        )
    for token, detail in (
        (
            "unwrap_or_else(|_| AbilityWireRegistry::core())",
            "plugin runtime manager must not downgrade package-state failure to core-only wire registry",
        ),
        (
            "RwLock::new(loaded.map_err",
            "plugin runtime manager must not retain an unavailable default state while exposing a live core-only wire registry",
        ),
    ):
        for match in re.finditer(re.escape(token), text):
            add(
                "R75_PLUGIN_WIRE_PROFILE_FAIL_CLOSED",
                plugin_runtime_manager,
                line_number(text, match.start()),
                detail,
            )

invocation_boot = cli_root / "src/daemon/boot/invocation/mod.rs"
if invocation_boot.exists():
    text = source(invocation_boot)
    for token, detail in (
        (
            "ability_wire_registry_load_failed",
            "invocation transport must fail assembly on plugin wire registry load failure instead of warning and continuing",
        ),
        (
            "daemon will use core bidi wire profiles only",
            "invocation transport must not continue with a core-only plugin wire registry",
        ),
        (
            "AbilityWireRegistry::load_default_profile()",
            "invocation transport must not independently reload plugin wire profiles; consume the catalog-owned manager",
        ),
    ):
        for match in re.finditer(re.escape(token), text):
            add(
                "R75_PLUGIN_WIRE_PROFILE_FAIL_CLOSED",
                invocation_boot,
                line_number(text, match.start()),
                detail,
            )

ability_wire = cli_root / "src/daemon/ability/wire/mod.rs"
if ability_wire.exists():
    text = source(ability_wire)
    for pattern, detail in (
        (
            r"load_default_profile\(\)[\s\S]{0,120}\.ok\(\)",
            "ability wire helpers must not swallow default profile load errors",
        ),
        (
            r"(?m)^pub\s+fn\s+bidi_wire_kind_for\s*\(",
            "ability wire lookup must require an explicit registry handle, not a process-global default profile read",
        ),
        (
            r"(?m)^pub\s+fn\s+is_bidi_wire_ability\s*\(",
            "ability wire predicates must require an explicit registry handle, not a process-global default profile read",
        ),
    ):
        match = re.search(pattern, text)
        if match:
            add(
                "R75_PLUGIN_WIRE_PROFILE_FAIL_CLOSED",
                ability_wire,
                line_number(text, match.start()),
                detail,
            )


# Rule 76: Cross-Hub peer envelopes must carry an explicit subject source.
# Missing caller envelopes are not target-self invocations; fresh daemon-owned
# peer requests must declare their subject before crossing the peer transport.
peer_envelope_signer = cli_root / "src/daemon/invocation/admission/peer_envelope_signer.rs"
if peer_envelope_signer.exists():
    text = source(peer_envelope_signer)
    for token, detail in (
        (
            "enum PeerInvocationSubject",
            "peer envelope signer must model subject source as an explicit state machine",
        ),
        (
            "ForwardedCaller(&'a Envelope)",
            "peer envelope signer must preserve forwarded caller provenance explicitly",
        ),
        (
            "ExplicitSubject(&'a str)",
            "peer envelope signer must support fresh peer requests through explicit subject input",
        ),
    ):
        if token not in text:
            add("R76_PEER_ENVELOPE_EXPLICIT_SUBJECT", peer_envelope_signer, 1, detail)
    for token, detail in (
        (
            "caller_envelope.cloned().unwrap_or_default()",
            "peer envelope signer must not default a missing caller envelope into an empty envelope",
        ),
        (
            "unwrap_or_else(|| target_ura.trim().to_string())",
            "peer envelope signer must not default missing subject provenance to target_ura",
        ),
        (
            "caller_envelope: Option<&",
            "PeerInvokeRequest must require a PeerInvocationSubject instead of optional caller envelope input",
        ),
    ):
        for match in re.finditer(re.escape(token), text):
            add(
                "R76_PEER_ENVELOPE_EXPLICIT_SUBJECT",
                peer_envelope_signer,
                line_number(text, match.start()),
                detail,
            )


# Rule 77: InvokeBidi down-frame projection has one support-layer owner, and
# receipt payload facts must fail closed when they are not declared JSON or do
# not parse as JSON. BinaryChunk data may stay lossless `data_b64`; receipt
# payloads must not be converted to opaque bytes after JSON parse failure.
local_invoke = cli_root / "src/support/platform/local_invoke.rs"
if local_invoke.exists():
    text = source(local_invoke)
    for token, detail in (
        (
            "pub fn project_invoke_bidi_down_frame",
            "LocalBidiFrame owner must provide the single InvokeBidiDown projection helper",
        ),
        (
            "fn project_receipt_payload_json",
            "InvokeBidi receipt payload JSON validation must be centralized",
        ),
        (
            "InvokeBidi receipt payload declares non-JSON content_type",
            "receipt payload projection must reject non-JSON content types",
        ),
        (
            "InvokeBidi receipt payload is not valid JSON",
            "receipt payload projection must reject malformed JSON",
        ),
    ):
        if token not in text:
            add("R77_BIDI_RECEIPT_PAYLOAD_PROJECTION", local_invoke, 1, detail)
    helper_body = rust_method_body(text, "project_invoke_bidi_down_frame")
    if helper_body is not None:
        offset, body = helper_body
        body_start = text.find("{", offset) + 1
        receipt_match = re.search(
            r"DownPayload::Receipt[\s\S]{0,900}data_b64",
            body,
        )
        if receipt_match:
            add(
                "R77_BIDI_RECEIPT_PAYLOAD_PROJECTION",
                local_invoke,
                line_number(text, body_start + receipt_match.start()),
                "receipt payload projection must not wrap malformed receipt payload bytes as data_b64",
            )

for path in (
    cli_root / "src/support/platform/local_daemon_grpc.rs",
    cli_root / "src/daemon/invocation/routing/remote_invoke.rs",
):
    if not path.exists():
        continue
    text = source(path)
    owns_bidi_drain_projection = (
        "invoke_local_daemon_ability_bidi_json_frames_with_tuple_plan" in text
        or "invoke_remote_target_bidi_json_frames" in text
        or "DownPayload::Receipt" in text
    )
    if owns_bidi_drain_projection and "project_invoke_bidi_down_frame(frame)" not in text:
        add(
            "R77_BIDI_RECEIPT_PAYLOAD_PROJECTION",
            path,
            1,
            "bidi drain transport must delegate down-frame projection to local_invoke",
        )
    receipt_match = re.search(r"DownPayload::Receipt[\s\S]{0,900}data_b64", text)
    if receipt_match:
        add(
            "R77_BIDI_RECEIPT_PAYLOAD_PROJECTION",
            path,
            line_number(text, receipt_match.start()),
            "transport-local bidi drain must not synthesize data_b64 receipt payload fallbacks",
        )


# Rule 73: Device trust sync consumes hub resolve_key responses as
# schema-bound trust evidence. It must not repair missing `public_keys_b64`
# from legacy `public_key_b64`, and it must not skip malformed key rows.
device_trust_sync = cli_root / "src/daemon/invocation/admission/device_trust_sync.rs"
if device_trust_sync.exists():
    text = source(device_trust_sync)
    parse_body = rust_method_body(text, "parse_resolved_caller_trust")
    if parse_body is None:
        add(
            "R73_DEVICE_TRUST_SYNC_RESOLVE_KEY_SCHEMA",
            device_trust_sync,
            1,
            "device trust sync must own a schema-bound resolve_key response parser",
        )
    else:
        offset, body = parse_body
        body_start = text.find("{", offset) + 1
        for token, detail in (
            (
                '.get("public_key_b64")',
                "device trust sync must not repair resolve_key responses from legacy public_key_b64",
            ),
            (
                "filter_map",
                "device trust sync must not skip malformed public_keys_b64 rows",
            ),
            (
                "unwrap_or_default()",
                "device trust sync must not default missing public_keys_b64 to an empty key set",
            ),
        ):
            if token in body:
                add(
                    "R73_DEVICE_TRUST_SYNC_RESOLVE_KEY_SCHEMA",
                    device_trust_sync,
                    line_number(text, body_start + body.find(token)),
                    detail,
                )
        if "parse_public_keys_b64_field(&response)?" not in body:
            add(
                "R73_DEVICE_TRUST_SYNC_RESOLVE_KEY_SCHEMA",
                device_trust_sync,
                line_number(text, offset),
                "device trust sync parser must delegate public_keys_b64 validation to a fallible helper",
            )
    helper_body = rust_method_body(text, "parse_public_keys_b64_field")
    if helper_body is None:
        add(
            "R73_DEVICE_TRUST_SYNC_RESOLVE_KEY_SCHEMA",
            device_trust_sync,
            1,
            "device trust sync must validate public_keys_b64 with a dedicated helper",
        )
    else:
        offset, body = helper_body
        for token, detail in (
            (
                "resolve_key_response_missing_public_keys_b64",
                "missing public_keys_b64 must be a typed parse error",
            ),
            (
                "resolve_key_response_public_keys_b64_not_array",
                "non-array public_keys_b64 must be a typed parse error",
            ),
            (
                "_not_string",
                "non-string public_keys_b64 rows must be typed parse errors",
            ),
            (
                "_empty",
                "empty public_keys_b64 rows must be typed parse errors",
            ),
        ):
            if token not in body:
                add("R73_DEVICE_TRUST_SYNC_RESOLVE_KEY_SCHEMA", device_trust_sync, line_number(text, offset), detail)


# Rule 93: Session prelude trust sync consumes the same hub
# federation.resolve_key evidence as admission. It must not repair legacy
# single-key responses or skip malformed public_keys_b64 rows while importing
# hub/user trust before session.open.
session_prelude = cli_root / "src/daemon/invocation/bidi/session_initiator/prelude.rs"
if session_prelude.exists():
    text = source(session_prelude)
    if "resolved_public_keys" in text or "sync_paired_user_trust_prelude" in text:
        parser_body = rust_method_body(text, "resolved_public_keys")
        if parser_body is None:
            add(
                "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                session_prelude,
                1,
                "session prelude trust sync must own a schema-bound resolve_key response parser",
            )
        else:
            offset, body = parser_body
            body_start = text.find("{", offset) + 1
            for token, detail in (
                (
                    '.get("public_key_b64")',
                    "session prelude must not repair resolve_key responses from legacy public_key_b64",
                ),
                (
                    "filter_map",
                    "session prelude must not skip malformed public_keys_b64 rows",
                ),
                (
                    "unwrap_or_default()",
                    "session prelude must not default missing public_keys_b64 to an empty key set",
                ),
                (
                    "serde_json::from_slice::<serde_json::Value>(result).ok()",
                    "session prelude must not ignore malformed resolve_key JSON",
                ),
            ):
                if token in body:
                    add(
                        "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                        session_prelude,
                        line_number(text, body_start + body.find(token)),
                        detail,
                    )
            if "fn resolved_public_keys(result: &[u8]) -> anyhow::Result<Vec<String>>" not in text:
                add(
                    "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                    session_prelude,
                    line_number(text, offset),
                    "session prelude resolve_key parser must be fallible",
                )
            for token, detail in (
                (
                    "resolve_key_response_missing_public_keys_b64",
                    "missing public_keys_b64 must be a typed prelude parse error",
                ),
                (
                    "resolve_key_response_public_keys_b64_not_array",
                    "non-array public_keys_b64 must be a typed prelude parse error",
                ),
                (
                    "_not_string",
                    "non-string public_keys_b64 rows must be typed prelude parse errors",
                ),
                (
                    "_empty",
                    "empty public_keys_b64 rows must be typed prelude parse errors",
                ),
            ):
                if token not in body:
                    add(
                        "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                        session_prelude,
                        line_number(text, offset),
                        detail,
                    )
        sync_body = rust_method_body(text, "sync_paired_user_trust_prelude")
        if sync_body is not None:
            offset, body = sync_body
            for pattern, detail in (
                (
                    "signer: &dyn CanonicalSigner",
                    "paired user trust sync must not reuse the session Device signer",
                ),
                (
                    "publish_paired_user_keys_prelude(client, signer",
                    "paired user key publication must be signed by the paired User signer",
                ),
            ):
                if pattern in body:
                    add(
                        "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                        session_prelude,
                        line_number(text, offset + body.find(pattern)),
                        detail,
                    )
            for token, detail in (
                (
                    "sync.user_signer",
                    "paired user trust sync must load an explicit paired User signer source",
                ),
                (
                    "UserTrustBootstrapError::SignerUnavailable",
                    "missing paired User signer custody must be a typed prelude failure",
                ),
                (
                    "user_signer.as_ref()",
                    "paired user trust operations must sign with the paired User signer",
                ),
            ):
                if token not in body:
                    add(
                        "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                        session_prelude,
                        line_number(text, offset),
                        detail,
                    )
            if "load_runtime_caller_signer(user_ura)" not in text:
                add(
                    "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                    session_prelude,
                    line_number(text, offset),
                    "paired user signer source must load the runtime caller signer for the paired User URA",
                )
            if "paired_user_resolve_key_args(&user_ura, presented_pubkey_b64)" not in body:
                add(
                    "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                    session_prelude,
                    line_number(text, offset),
                    "paired user trust sync must pin resolve_key with the presented local user pubkey",
                )
            if "resolved_public_keys(&response.result).map_err" not in body:
                add(
                    "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                    session_prelude,
                    line_number(text, offset),
                    "paired user trust sync must propagate resolve_key response schema failures",
                )
            for pattern, detail in (
                (
                    "let Ok(creds)",
                    "paired user trust sync must not classify credential load errors as NotRequired",
                ),
                (
                    "let Ok(user_ura)",
                    "paired user trust sync must not classify invalid paired user identity as NotRequired",
                ),
                (
                    "load_credentials().ok()",
                    "paired user trust sync must not swallow credential load failures",
                ),
                (
                    "user_ura().ok()",
                    "paired user trust sync must not swallow paired user URA projection failures",
                ),
            ):
                if pattern in body:
                    add(
                        "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                        session_prelude,
                        line_number(text, offset + body.find(pattern)),
                        detail,
                    )
            for token, detail in (
                (
                    "load_credentials_optional()",
                    "paired user trust sync must distinguish missing credentials from corrupt credentials",
                ),
                (
                    "UserTrustBootstrapError::CredentialsUnavailable",
                    "paired user trust sync must expose corrupt credentials as a prelude failure state",
                ),
            ):
                if token not in text:
                    add(
                        "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                        session_prelude,
                        line_number(text, offset),
                        detail,
                    )

        session_initiator = cli_root / "src/daemon/invocation/bidi/session_initiator.rs"
        if session_initiator.exists():
            initiator_text = session_initiator.read_text(encoding="utf-8", errors="replace")
            for token, detail in (
                (
                    "__caller_ura",
                    "session prelude tests must capture the signed envelope caller",
                ),
                (
                    "prelude must publish the paired user key as the paired User",
                    "session prelude tests must prove identity.register_pubkey is signed as the paired User",
                ),
                (
                    "paired user resolve_key must pin the presented public key as the paired User",
                    "session prelude tests must prove federation.resolve_key is signed as the paired User",
                ),
            ):
                if token not in initiator_text:
                    add(
                        "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA",
                        session_initiator,
                        1,
                        detail,
                    )


# Rule 74: Pages serve adapter must consume page.fetch output as
# schema-bound resource evidence. Invalid/missing bytes or metadata must not
# be projected into a 200 response with empty bytes/default MIME/default sha.
old_pages_serve = cli_root / "src/daemon/resources/pages/pages_serve_ability.rs"
pages_serve = cli_root / "src/daemon/resources/pages/pages_http_projection.rs"
pages_listener = cli_root / "src/daemon/resources/pages/pages_listener.rs"
if old_pages_serve.exists():
    add(
        "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
        old_pages_serve,
        1,
        "Pages HTTP projection must not preserve the retired pages_serve_ability pseudo-ability module",
    )
if not pages_serve.exists():
    add(
        "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
        pages_serve,
        1,
        "Pages HTTP projection module is missing",
    )
if pages_serve.exists():
    text = source(pages_serve)
    for token, detail in (
        (
            "canonical_invoke",
            "Pages HTTP projection must not claim canonical Invocation dispatch",
        ),
        (
            "Phase 2",
            "Pages HTTP projection must not carry staged cutover compatibility prose",
        ),
        (
            "operational receipt",
            "Pages HTTP projection must not claim receipt emission",
        ),
        (
            "01HUB.pages.serve",
            "Pages HTTP projection must not preserve the retired pages.serve pseudo-ability name",
        ),
        (
            "pages.serve adapter ability",
            "Pages HTTP projection must not describe itself as an Ability implementation",
        ),
    ):
        if token in text:
            add(
                "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
                pages_serve,
                line_number(text, text.find(token)),
                detail,
            )
    bytes_body = rust_method_body(text, "bytes_from_value")
    if bytes_body is None:
        add(
            "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
            pages_serve,
            1,
            "Pages serve adapter must own a fallible page.fetch projection parser",
        )
    else:
        offset, body = bytes_body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "anyhow::Result<ServedBytes>" not in signature:
            add(
                "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
                pages_serve,
                line_number(text, offset),
                "bytes_from_value must return Result so malformed fetch output cannot become HTTP 200",
            )
        for token, detail in (
            (
                "unwrap_or_default()",
                "Pages serve adapter must not default invalid bytes_b64 to empty bytes",
            ),
            (
                'unwrap_or("application/octet-stream")',
                "Pages serve adapter must not default missing content_type",
            ),
            (
                'unwrap_or("")',
                "Pages serve adapter must not default missing bytes_b64 or sha256 to empty strings",
            ),
            (
                "unwrap_or(false)",
                "Pages serve adapter must not default missing force_attachment",
            ),
        ):
            if token in body:
                add(
                    "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
                    pages_serve,
                    line_number(text, body_start + body.find(token)),
                    detail,
                )
        for token, detail in (
            (
                'required_non_empty_string(&value, "bytes_b64")?',
                "bytes_from_value must require bytes_b64",
            ),
            (
                'required_non_empty_string(&value, "content_type")?',
                "bytes_from_value must require content_type",
            ),
            (
                'required_non_empty_string(&value, "sha256")?',
                "bytes_from_value must require sha256",
            ),
            (
                "sha256 != actual_sha256",
                "bytes_from_value must verify sha256 against decoded bytes",
            ),
        ):
            if token not in body:
                add("R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA", pages_serve, line_number(text, offset), detail)
if pages_listener.exists():
    listener_text = source(pages_listener)
    if "pages_serve_ability" in listener_text:
        add(
            "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
            pages_listener,
            line_number(listener_text, listener_text.find("pages_serve_ability")),
            "Pages listener must import the HTTP projection module, not the retired pseudo-ability module",
        )
    for token, detail in (
        (
            "01HUB.pages.serve",
            "Pages listener must not preserve the retired pages.serve pseudo-ability shape",
        ),
        (
            "cut-over (Phase 2)",
            "Pages listener must not carry staged cutover compatibility prose",
        ),
    ):
        if token in listener_text:
            add(
                "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA",
                pages_listener,
                line_number(listener_text, listener_text.find(token)),
                detail,
            )


# Rule 78: Authority metadata projection must fail closed when the runtime
# clock cannot be represented as Unix epoch milliseconds. It must not convert
# clock failure to epoch zero before session-authority expiry validation.
authority_metadata = cli_root / "src/daemon/invocation/admission/authority_metadata.rs"
if authority_metadata.exists():
    text = source(authority_metadata)
    if re.search(r"duration_since\s*\(\s*UNIX_EPOCH\s*\)\s*\.unwrap_or_default\s*\(", text, re.S):
        add(
            "R78_AUTHORITY_METADATA_CLOCK_FAIL_CLOSED",
            authority_metadata,
            1,
            "authority metadata projection must not default clock failure to epoch zero",
        )
    for token, detail in (
        (
            "REASON_AUTHORITY_CLOCK_UNAVAILABLE",
            "authority metadata must expose a named clock-unavailable state",
        ),
        (
            "fn current_unix_epoch_millis() -> Result<i64, AuthorityMetadataError>",
            "authority metadata must use a fallible current clock helper",
        ),
        (
            "fn unix_epoch_millis(now: SystemTime) -> Result<i64, AuthorityMetadataError>",
            "authority metadata must keep Unix epoch conversion fallible and testable",
        ),
    ):
        if token not in text:
            add("R78_AUTHORITY_METADATA_CLOCK_FAIL_CLOSED", authority_metadata, 1, detail)

    # Rule 90: Daemon authority metadata must reject all-zero principal
    # placeholders through the core identity guard. SDK guards are not
    # sufficient because product ingress can still submit raw metadata to the
    # daemon admission gate, but the sentinel ownership belongs in core
    # identity rather than a daemon-local duplicate.
    if authority_metadata.exists():
        text = source(authority_metadata)
        for token, detail in (
            (
                "crate::core::identity::contains_all_zero_principal_placeholder(value)",
                "authority metadata must use the core all-zero principal guard",
            ),
            (
                "fn reject_all_zero_authority_fields",
                "authority metadata must centralize all-zero field rejection",
            ),
        ):
            if token not in text:
                add("R90_AUTHORITY_METADATA_REJECTS_ALL_ZERO_PRINCIPAL", authority_metadata, 1, detail)
        for fn_name, detail in (
            (
                "validate_delegation_payload_shape",
                "delegation authority validation must reject all-zero principal placeholders",
            ),
            (
                "validate_session_authority_payload_shape",
                "session authority validation must reject all-zero principal placeholders",
            ),
        ):
            body_info = rust_method_body(text, fn_name)
            if body_info is None:
                add("R90_AUTHORITY_METADATA_REJECTS_ALL_ZERO_PRINCIPAL", authority_metadata, 1, f"{fn_name} must remain inspectable")
                continue
            offset, body = body_info
            if "reject_all_zero_authority_fields(" not in body:
                add(
                    "R90_AUTHORITY_METADATA_REJECTS_ALL_ZERO_PRINCIPAL",
                    authority_metadata,
                    line_number(text, offset),
                    detail,
                )

    # Rule 93: Canonical runtime authority metadata keys must be
    # product-neutral. EasyNet-hosted-agent delegation metadata is provider
    # policy and intentionally not part of this canonical SDK/admission set.
    authority_key_sources = [
        authority_metadata,
        cli_root / "sdk/go/authority.go",
        cli_root / "sdk/python/easynet_sdk/authority.py",
        cli_root / "sdk/node/index.js",
        cli_root / "sdk/node/index.d.ts",
        cli_root / "sdk/java/src/main/java/run/runtime/sdk/AuthoritySupport.java",
        cli_root / "sdk/swift/Sources/RuntimeSDK/Authority.swift",
        cli_root / "sdk/schemas/authority.schema.json",
        cli_root / "sdk/conformance/fixtures/authority-metadata.v4.json",
        cli_root / "sdk/conformance/cases/authority-mutual-exclusion.yaml",
    ]
    existing_authority_key_sources = [path for path in authority_key_sources if path.exists()]
    if len(existing_authority_key_sources) != len(authority_key_sources):
        for path in authority_key_sources:
            if not path.exists():
                add(
                    "R93_RUNTIME_AUTHORITY_METADATA_KEY_NEUTRALITY",
                    path,
                    1,
                    "runtime authority metadata key source is missing",
                )
    else:
        texts = [
            path.read_text(encoding="utf-8", errors="replace")
            for path in authority_key_sources
        ]
        combined = "\n".join(texts)
        for retired in ("x-easynet-delegation", "x-easynet-session-authority"):
            if retired in combined:
                add(
                    "R93_RUNTIME_AUTHORITY_METADATA_KEY_NEUTRALITY",
                    authority_metadata,
                    1,
                    f"canonical runtime authority metadata must not use product key {retired}",
                )
        for required in ("x-runtime-delegation", "x-runtime-session-authority"):
            for path, text in zip(authority_key_sources, texts):
                if (
                    path == authority_metadata
                    and required == "x-runtime-delegation"
                    and "RUNTIME_DELEGATION_METADATA_KEY" in text
                ):
                    # Daemon admission deliberately imports the canonical
                    # runtime constant instead of duplicating its wire value.
                    continue
                if required not in text:
                    add(
                        "R93_RUNTIME_AUTHORITY_METADATA_KEY_NEUTRALITY",
                        path,
                        1,
                        f"runtime authority metadata key {required} must be shared across SDK languages and daemon admission",
                    )


# Rule 94: Product device show/remove must not repair unavailable local
# pairing identity into empty route/caller facts. Non-local device product
# ingress needs the local device identity for self-target classification and
# remote signer/admission context; malformed credentials are unavailable
# product state, not permission to synthesize empty node/realm/local_ura.
device_group = cli_root / "src/cli/commands/groups/device.rs"
if device_group.exists():
    text = source(device_group)
    for token, detail in (
        (
            "struct DeviceLocalIdentity",
            "device product ingress must model local identity explicitly",
        ),
        (
            "fn load_local_device_identity",
            "device product ingress must centralize fallible local credential loading",
        ),
        (
            "fn from_credentials",
            "device local identity must be built from validated credential facts",
        ),
    ):
        if token not in text:
            add("R94_DEVICE_PRODUCT_INGRESS_REQUIRES_LOCAL_IDENTITY", device_group, 1, detail)

    for fn_name, detail in (
        (
            "run_remove",
            "device remove must require local device identity instead of defaulting missing credentials",
        ),
        (
            "describe_target",
            "device show remote/self target resolution must require local device identity",
        ),
    ):
        body_info = rust_method_body(text, fn_name)
        if body_info is None:
            add("R94_DEVICE_PRODUCT_INGRESS_REQUIRES_LOCAL_IDENTITY", device_group, 1, f"{fn_name} must remain inspectable")
            continue
        offset, body = body_info
        if "load_local_device_identity(" not in body:
            add(
                "R94_DEVICE_PRODUCT_INGRESS_REQUIRES_LOCAL_IDENTITY",
                device_group,
                line_number(text, offset),
                detail,
            )
        for pattern, reason in (
            (
                "load_credentials().ok()",
                "device product ingress must not swallow credential load failures",
            ),
            (
                "unwrap_or_default()",
                "device product ingress must not synthesize empty local identity facts",
            ),
            (
                "String::new()",
                "device product ingress must not synthesize empty local_ura",
            ),
            (
                "local_ura.is_empty()",
                "device product ingress must not branch on empty local_ura compatibility state",
            ),
        ):
            if pattern in body:
                add(
                    "R94_DEVICE_PRODUCT_INGRESS_REQUIRES_LOCAL_IDENTITY",
                    device_group,
                    line_number(text, offset + body.find(pattern)),
                    reason,
                )

    classify_body = rust_method_body(text, "classify_device_show_target")
    if classify_body is None:
        add(
            "R94_DEVICE_PRODUCT_INGRESS_REQUIRES_LOCAL_IDENTITY",
            device_group,
            1,
            "device show target classifier must remain inspectable",
        )
    else:
        offset, body = classify_body
        if "target == local_identity.device_ura()" not in body:
            add(
                "R94_DEVICE_PRODUCT_INGRESS_REQUIRES_LOCAL_IDENTITY",
                device_group,
                line_number(text, offset),
                "device show must classify the canonical self Device URA as local instead of routing it remotely",
            )


# Rule 95: Pages API HTTP ingress is descriptor-bound product input. Empty
# bodies may project to JSON null, but malformed JSON is invalid HTTP input and
# must not be repaired to null before ability dispatch.
pages_listener = cli_root / "src/daemon/resources/pages/pages_listener.rs"
if pages_listener.exists():
    text = source(pages_listener)
    for pattern, detail in (
        (
            r"serde_json::from_slice\s*\(\s*&?body_bytes\s*\)\s*\.unwrap_or\s*\(\s*serde_json::Value::Null\s*\)",
            "Pages API body parsing must not repair malformed JSON to null",
        ),
        (
            r"serde_json::from_slice\s*\(\s*&?body_bytes\s*\)\s*\.unwrap_or\s*\(\s*Value::Null\s*\)",
            "Pages API body parsing must not repair malformed JSON to null",
        ),
    ):
        match = re.search(pattern, text, re.S)
        if match:
            add(
                "R95_PAGES_API_BODY_FAIL_CLOSED",
                pages_listener,
                line_number(text, match.start()),
                detail,
            )
    if "fn parse_pages_api_body(" not in text:
        add(
            "R95_PAGES_API_BODY_FAIL_CLOSED",
            pages_listener,
            1,
            "Pages API body parsing must stay centralized and testable",
        )
    body_info = brace_function_body(text, r"fn\s+parse_pages_api_body\b")
    if body_info is not None:
        offset, body = body_info
        if "body_bytes.is_empty()" not in body or "serde_json::from_slice(body_bytes)" not in body:
            add(
                "R95_PAGES_API_BODY_FAIL_CLOSED",
                pages_listener,
                line_number(text, offset),
                "parse_pages_api_body must treat only absent body as null and parse non-empty bytes fallibly",
            )


# Rule 96: Runtime lifecycle projection loading must distinguish missing
# runtime.json from corrupt/unavailable runtime.json. Missing projection is a
# lifecycle state; corrupt existing projection is unavailable lifecycle input.
lifecycle_projection = cli_root / "src/daemon/boot/lifecycle/projection.rs"
lifecycle_service = cli_root / "src/daemon/boot/lifecycle/service.rs"
if lifecycle_projection.exists():
    text = source(lifecycle_projection)
    for token, detail in (
        (
            "pub fn load(&self) -> anyhow::Result<Option<RuntimeSessionProjection>>",
            "RuntimeProjectionStore::load must return Result<Option<_>> so projection load failures propagate",
        ),
        (
            "pub fn load_current() -> anyhow::Result<Option<Self>>",
            "RuntimeSessionProjection::load_current must return Result<Option<_>> so malformed runtime.json is not absence",
        ),
        (
            "config::load_optional_runtime_state()?",
            "RuntimeSessionProjection must use the optional runtime-state loader that distinguishes missing from corrupt",
        ),
    ):
        if token not in text:
            add("R96_RUNTIME_PROJECTION_LOAD_FAIL_CLOSED", lifecycle_projection, 1, detail)
    for pattern, detail in (
        (
            "config::load().ok()",
            "runtime lifecycle projection must not swallow config::load failures",
        ),
        (
            ".ok().map(Self::from_state)",
            "runtime lifecycle projection must not project load failure as missing projection",
        ),
    ):
        if pattern in text:
            add(
                "R96_RUNTIME_PROJECTION_LOAD_FAIL_CLOSED",
                lifecycle_projection,
                line_number(text, text.find(pattern)),
                detail,
            )

if lifecycle_service.exists():
    text = source(lifecycle_service)
    for token, detail in (
        (
            "pub fn status(&self) -> Result<RuntimeStatusReport, RuntimeLifecycleError>",
            "RuntimeLifecycleService::status must propagate projection load failures",
        ),
        (
            "RuntimeLifecycleError::ProjectionLoadFailed",
            "RuntimeLifecycleService must classify projection load failure as lifecycle boundary error",
        ),
        (
            "pub fn stop_plan(&self) -> Result<RuntimeStopPlan, RuntimeLifecycleError>",
            "RuntimeLifecycleService::stop_plan must propagate projection load failures before planning cleanup",
        ),
    ):
        if token not in text:
            add("R96_RUNTIME_PROJECTION_LOAD_FAIL_CLOSED", lifecycle_service, 1, detail)


# Rule 97: Device reset is a destructive product lifecycle command. It must
# consume the daemon lifecycle status report and must not reopen runtime.json
# as an optional local hint before deleting credentials.
reset_cmd = cli_root / "src/cli/commands/reset.rs"
if reset_cmd.exists():
    text = source(reset_cmd)
    for token, detail in (
        (
            "RuntimeLifecycleService::new().status()?",
            "device reset must consume lifecycle status and propagate projection load failures",
        ),
        (
            "fn reset_runtime_is_active(",
            "device reset must centralize active-runtime classification",
        ),
        (
            "RuntimeLifecycleStatus::ProjectionPresentProcessMissing",
            "device reset must clean only lifecycle-classified stale runtime projections",
        ),
        (
            "config::remove()?",
            "device reset must propagate stale runtime projection cleanup failures",
        ),
    ):
        if token not in text:
            add("R97_RESET_RUNTIME_PROJECTION_FAIL_CLOSED", reset_cmd, 1, detail)
    for pattern, detail in (
        (
            "config::load().ok()",
            "device reset must not swallow runtime projection load failures",
        ),
        (
            "config::remove().ok()",
            "device reset must not ignore stale runtime projection cleanup failures",
        ),
    ):
        if pattern in text:
            add(
                "R97_RESET_RUNTIME_PROJECTION_FAIL_CLOSED",
                reset_cmd,
                line_number(text, text.find(pattern)),
                detail,
            )


# Rule 98: MCP status is a product diagnostics surface. It must consume the
# lifecycle status report and must not collapse corrupt runtime.json into
# "runtime not running".
mcp_cmd = cli_root / "src/cli/commands/groups/mcp.rs"
if mcp_cmd.exists():
    text = source(mcp_cmd)
    for token, detail in (
        (
            "RuntimeLifecycleService::new().status()?",
            "MCP status must consume lifecycle status and propagate projection load failures",
        ),
        (
            "fn render_lifecycle_details(",
            "MCP status must centralize lifecycle detail rendering",
        ),
        (
            "report.daemon().has_daemon_fact()",
            "MCP status must distinguish missing projection with daemon facts from stopped runtime",
        ),
    ):
        if token not in text:
            add("R98_MCP_STATUS_RUNTIME_PROJECTION_FAIL_CLOSED", mcp_cmd, 1, detail)
    if "config::load().ok()" in text:
        add(
            "R98_MCP_STATUS_RUNTIME_PROJECTION_FAIL_CLOSED",
            mcp_cmd,
            line_number(text, text.find("config::load().ok()")),
            "MCP status must not swallow runtime projection load failures",
        )


# Rule 99: The top-level help banner is a product navigation surface. It cannot
# return Result, so corrupt runtime projection must render as explicit
# unavailable metadata rather than being collapsed into a stopped runtime.
banner_mod = cli_root / "src/cli/presentation/banner.rs"
if banner_mod.exists():
    text = source(banner_mod)
    for token, detail in (
        (
            "RuntimeLifecycleService::new().status()",
            "banner runtime status must consume lifecycle status",
        ),
        (
            "struct BannerDaemonObservation",
            "banner runtime status must centralize lifecycle-to-display projection",
        ),
        (
            "metadata unavailable",
            "banner must render lifecycle projection load failure explicitly",
        ),
        (
            "fn from_lifecycle_status(",
            "banner must render lifecycle state machine states through one mapper",
        ),
    ):
        if token not in text:
            add("R99_BANNER_RUNTIME_PROJECTION_FAIL_CLOSED", banner_mod, 1, detail)
    if "config::load().ok()" in text:
        add(
            "R99_BANNER_RUNTIME_PROJECTION_FAIL_CLOSED",
            banner_mod,
            line_number(text, text.find("config::load().ok()")),
            "banner must not swallow runtime projection load failures",
        )
    for retired, detail in (
        (
            "user_ura().ok()",
            "banner must not hide invalid paired user identity projection",
        ),
        (
            "if let Ok(user_ura)",
            "banner must render explicit paired user binding state instead of swallowing projection errors",
        ),
    ):
        if retired in text:
            add(
                "R99_BANNER_RUNTIME_PROJECTION_FAIL_CLOSED",
                banner_mod,
                line_number(text, text.find(retired)),
                detail,
            )


for identity_surface in (
    cli_root / "src/cli/commands/status.rs",
    cli_root / "src/cli/commands/auth.rs",
):
    if identity_surface.exists():
        text = source(identity_surface)
        for retired, detail in (
            (
                "user_ura().ok()",
                "CLI identity surfaces must not hide invalid paired user identity projection",
            ),
            (
                "if let Ok(user_ura)",
                "CLI identity surfaces must render explicit runtime user binding state",
            ),
        ):
            if retired in text:
                add(
                    "R99_CLI_IDENTITY_PROJECTION_FAIL_CLOSED",
                    identity_surface,
                    line_number(text, text.find(retired)),
                    detail,
                )


identity_projection = cli_root / "src/cli/presentation/identity.rs"
if identity_projection.exists():
    text = source(identity_projection)
    for token, detail in (
        (
            "pub enum RuntimeUserBindingDisplayState",
            "CLI identity projection must model bound/unbound/invalid states explicitly",
        ),
        (
            "pub fn runtime_user_binding_display(creds: &config::Credentials)",
            "CLI identity projection must centralize runtime user binding display",
        ),
        (
            "RuntimeUserBindingDisplayState::Invalid",
            "CLI identity projection must preserve invalid credential projection as an explicit state",
        ),
    ):
        if token not in text:
            add("R99_CLI_IDENTITY_PROJECTION_FAIL_CLOSED", identity_projection, 1, detail)
else:
    add(
        "R99_CLI_IDENTITY_PROJECTION_FAIL_CLOSED",
        identity_projection,
        1,
        "CLI identity projection shared projector is missing",
    )
for surface in (
    cli_root / "src/cli/commands/status.rs",
    cli_root / "src/cli/commands/auth.rs",
    cli_root / "src/cli/presentation/banner.rs",
):
    if surface.exists() and "runtime_user_binding_display" not in source(surface):
        add(
            "R99_CLI_IDENTITY_PROJECTION_FAIL_CLOSED",
            surface,
            1,
            "CLI identity surfaces must reuse the shared runtime user binding projector",
        )


# Rule 91: C ABI diagnostics descriptor fallback must keep descriptor
# resolution semantics. A catalog miss is DESCRIPTOR_NOT_FOUND, not generic
# NOT_FOUND/ABILITY_NOT_FOUND; otherwise products cannot distinguish absent
# descriptor refs from missing abilities or upstream signer/admission
# failures.
for path, missing_token, forbidden_pattern in (
    (
        cli_root / "sdk/go/cabi_runtime.go",
        "ErrDescriptorNotFound",
        r"Code\s*:\s*ErrNotFound",
    ),
    (
        cli_root / "sdk/python/easynet_sdk/_cabi.py",
        "ErrorCode.DESCRIPTOR_NOT_FOUND",
        r"code\s*=\s*ErrorCode\.NOT_FOUND",
    ),
):
    if not path.exists():
        continue
    text = source(path)
    if "resolveDescriptorRefFromDiagnostics" in text or "_resolve_descriptor_ref_from_diagnostics" in text:
        if missing_token not in text:
            add(
                "R91_CABI_DESCRIPTOR_RESOLVE_NOT_FOUND_TYPED",
                path,
                1,
                "C ABI diagnostics descriptor fallback must return DESCRIPTOR_NOT_FOUND",
            )
        match = re.search(forbidden_pattern, text)
        if match:
            add(
                "R91_CABI_DESCRIPTOR_RESOLVE_NOT_FOUND_TYPED",
                path,
                line_number(text, match.start()),
                "C ABI diagnostics descriptor fallback must not return generic NOT_FOUND",
            )


# Rule 94: FFI descriptor catalog projection must fail closed. Every provider,
# including governance providers, resolves through the attached daemon's
# committed catalog and the shared canonical row validator.
ffi_invocation = cli_root / "src/ffi/invocation/mod.rs"
runtime_descriptor_provider = cli_root / "src/daemon/axon_bridge/runtime_descriptor_provider.rs"
if ffi_invocation.exists() or runtime_descriptor_provider.exists():
    text = source(ffi_invocation) if ffi_invocation.exists() else ""
    provider_text = source(runtime_descriptor_provider) if runtime_descriptor_provider.exists() else ""
    production = text.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]
    provider_production = provider_text.split("\n#[cfg(test)]", 1)[0]
    for token, detail in (
        (
            "runtime_meta_descriptor_catalog_entries",
            "FFI descriptor resolver must not invoke meta.list_abilities as a hidden catalog provider",
        ),
        (
            "descriptor_catalog_entry_from_value",
            "FFI descriptor resolver must not keep a generic provider-row parser without an explicit provider seam",
        ),
    ):
        if token in production or token in provider_production:
            source_text = text if token in production else provider_text
            source_path = ffi_invocation if token in production else runtime_descriptor_provider
            add(
                "R94_FFI_DESCRIPTOR_CATALOG_FAIL_CLOSED",
                source_path,
                line_number(source_text, source_text.find(token)),
                detail,
            )
    for token, source_path, source_text, detail in (
        (
            "trait RuntimeDescriptorCatalogReader",
            runtime_descriptor_provider,
            provider_production,
            "runtime descriptor provider must expose an explicit committed-catalog reader seam",
        ),
        (
            "runtime_live_descriptor_catalog_entries",
            runtime_descriptor_provider,
            provider_production,
            "generic descriptor resolution must consume the daemon committed catalog",
        ),
        (
            "AbilityCatalogRow::parse",
            runtime_descriptor_provider,
            provider_production,
            "daemon committed catalog rows must be validated before selection",
        ),
        (
            "AttachedDaemonDescriptorCatalogReader",
            ffi_invocation,
            production,
            "FFI descriptor resolution must bind catalog reads to the attached runtime session",
        ),
        (
            "RemoteCatalogueReadIssuer::catalogue_read_plan",
            ffi_invocation,
            production,
            "FFI descriptor catalog reads must use the canonical signed catalogue-read issuer",
        ),
        (
            "invoke_remote_target_with_signer_at_endpoint",
            ffi_invocation,
            production,
            "FFI descriptor catalog reads must preserve the explicitly attached daemon endpoint",
        ),
        (
            "paired_user_ura",
            ffi_invocation,
            production,
            "remote FFI descriptor catalog reads must remain accountable to the paired User",
        ),
        (
            "enum AttachedDescriptorCatalogRoute",
            ffi_invocation,
            production,
            "FFI descriptor catalog transport must model local-internal and remote-User routes explicitly",
        ),
        (
            "AttachedDescriptorCatalogRoute::LocalRuntime",
            ffi_invocation,
            production,
            "local attached runtime catalogue reads must retain their internal daemon route",
        ),
        (
            "AttachedDescriptorCatalogRoute::RemoteRuntime",
            ffi_invocation,
            production,
            "remote attached runtime catalogue reads must use their paired-User route",
        ),
        (
            "execution_host_ura_for_device_sponsored_system_agent",
            ffi_invocation,
            production,
            "FFI descriptor catalog reads must route remote SystemAgent owners through their sponsor Device execution host",
        ),
        (
            "catalog_execution_target_ura",
            ffi_invocation,
            production,
            "FFI descriptor catalog reads must keep catalogue execution target separate from logical descriptor owner",
        ),
    ):
        if token not in source_text:
            add("R94_FFI_DESCRIPTOR_CATALOG_FAIL_CLOSED", source_path, 1, detail)
    if "runtime_daemon_native_agent_descriptor_catalog_entries" in provider_production:
        add(
            "R94_FFI_DESCRIPTOR_CATALOG_FAIL_CLOSED",
            runtime_descriptor_provider,
            line_number(provider_text, provider_text.find("runtime_daemon_native_agent_descriptor_catalog_entries")),
            "descriptor resolution must not retain agent-type or product-specific static catalog branches",
        )
    catalog_row_path = cli_root / "src/daemon/ability/catalog_row.rs"
    if not catalog_row_path.exists():
        add(
            "R94_FFI_DESCRIPTOR_CATALOG_FAIL_CLOSED",
            catalog_row_path,
            1,
            "AbilityCatalogRow must own committed catalog row validation",
        )
    else:
        catalog_row_text = source(catalog_row_path)
        catalog_row_raw_text = catalog_row_path.read_text(encoding="utf-8", errors="replace")
        for token in (
            '"ability_ura"',
            '"owner_ura"',
            '"descriptor_hash"',
            '"descriptor_ref"',
            "serde_json::from_value(value.clone())",
            "insert_catalog_descriptor",
            "WIRE_DESCRIPTOR_VERSION_ALIAS",
            "wire-adapter field",
            "It is never a second catalog identity",
            "descriptor_version_alias_is_wire_adapter_only",
        ):
            haystack = catalog_row_raw_text if token in {
                "wire-adapter field",
                "It is never a second catalog identity",
                "descriptor_version_alias_is_wire_adapter_only",
            } else catalog_row_text
            if token not in haystack:
                add(
                    "R94_FFI_DESCRIPTOR_CATALOG_FAIL_CLOSED",
                    catalog_row_path,
                    1,
                    f"shared canonical catalog row contract is missing {token}",
                )


# Rule 95: Descriptor resolution is one bounded committed-catalog lookup. The
# explicit reader selects either a local internal catalogue route or a remote
# paired-User signed catalogue route before lookup. It must not add probe/fallback
# execution paths after an exact catalog miss or classify failures by wording.
ffi_invocation = cli_root / "src/ffi/invocation/mod.rs"
runtime_descriptor_provider = cli_root / "src/daemon/axon_bridge/runtime_descriptor_provider.rs"
if ffi_invocation.exists() or runtime_descriptor_provider.exists():
    text = source(ffi_invocation) if ffi_invocation.exists() else ""
    provider_text = source(runtime_descriptor_provider) if runtime_descriptor_provider.exists() else ""
    production = text.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]
    provider_production = provider_text.split("\n#[cfg(test)]", 1)[0]
    combined_production = production + "\n" + provider_production
    if "fn descriptor_resolution_error_projection(" in production:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            ffi_invocation,
            line_number(text, production.find("fn descriptor_resolution_error_projection(")),
            "descriptor resolver FFI projection must use typed DescriptorResolutionError, not message-string classification",
        )
    for token, detail in (
        (
            "from_remote_probe_failure",
            "descriptor resolver must not classify remote probe failures from anyhow message text",
        ),
        (
            "lowered.contains",
            "descriptor resolver must not lowercase message text for state classification",
        ),
        (
            'contains("owner is not online")',
            "descriptor resolver must not depend on daemon owner-offline wording",
        ),
        (
            'contains("NEGATIVE_REASON_NXDOMAIN")',
            "descriptor resolver must not depend on daemon negative-route wording",
        ),
        (
            'contains("ROUTE_NEGATIVE")',
            "descriptor resolver must not depend on daemon route-negative wording",
        ),
        (
            'contains("requires a caller signer")',
            "descriptor resolver must not depend on signer error wording",
        ),
        (
            "RemoteDescriptorCatalogProbe",
            "descriptor resolver must not keep remote probe fallback state",
        ),
        (
            "DescriptorCatalogProbeSubject",
            "descriptor resolver must not keep probe-specific subject state",
        ),
        (
            "invoke_remote_target_with_caller_signer_typed(",
            "descriptor resolver must not invoke remote targets",
        ),
        (
            "runtime_meta_descriptor_catalog_entries",
            "descriptor resolver must not invoke meta.list_abilities as a hidden catalog probe",
        ),
        (
            "descriptor_catalog_entry_from_value",
            "descriptor resolver must not keep a generic provider-row parser without an explicit provider seam",
        ),
        (
            "RemoteInvocationFailure::",
            "descriptor resolver must not classify remote invocation failures",
        ),
        (
            "error.runtime_failure_kind()",
            "descriptor resolver ABI projection must be enum-driven, not message-classifier-driven",
        ),
        (
            "Self::CallerSignerUnavailable(",
            "descriptor resolver must not expose signer failures as catalog lookup state",
        ),
        (
            "Self::RuntimeOffline(",
            "descriptor resolver must not expose remote transport failures as catalog lookup state",
        ),
    ):
        if token in combined_production:
            source_text = text if token in production else provider_text
            source_path = ffi_invocation if token in production else runtime_descriptor_provider
            add(
                "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                source_path,
                line_number(source_text, source_text.find(token)),
                detail,
            )
    for token, detail in (
        (
            "enum DescriptorResolutionError",
            "descriptor resolver must expose typed failure states",
        ),
        (
            "OwnerOffline(String)",
            "descriptor resolver must expose owner offline as a typed provider state",
        ),
        (
            "fn descriptor_resolution_abi_projection(",
            "descriptor resolver typed failures must own ABI projection",
        ),
        (
            "descriptor_resolution_abi_projection(&error)",
            "FFI descriptor resolver must project from typed error variants",
        ),
        (
            "descriptor_ref not found in committed runtime catalog",
            "descriptor resolver must fail closed as a committed catalog miss",
        ),
        (
            "CatalogUnavailable(String)",
            "descriptor resolver must expose committed catalog unavailability as typed state",
        ),
    ):
        if token not in combined_production:
            add(
                "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                ffi_invocation if token == "descriptor_resolution_abi_projection(&error)" else runtime_descriptor_provider,
                1,
                detail,
            )
    resolve_body = rust_method_body(provider_text, "runtime_resolve_descriptor_ref_json")
    if resolve_body is not None:
        offset, body = resolve_body
        for token, detail in (
            (
                "runtime_owner_ura_from_session(session).ok()",
                "descriptor resolver must not collapse runtime owner resolution failure into remote probe fallback",
            ),
            (
                "runtime owner URA is unavailable",
                "descriptor resolver must not use runtime-owner fallback diagnostics for missing caller_ura",
            ),
            (
                ".unwrap_or_else(||",
                "descriptor resolver must not synthesize caller_ura from runtime_owner_ura",
            ),
        ):
            if token in body:
                add(
                    "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                    runtime_descriptor_provider,
                    line_number(provider_text, offset + body.find(token)),
                    detail,
                )
        if 'descriptor_ref_request_required_string(object, "caller_ura")' in body:
            add(
                "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
                runtime_descriptor_provider,
                line_number(provider_text, offset + body.find('descriptor_ref_request_required_string(object, "caller_ura")')),
                "descriptor resolver must not require caller_ura for hidden remote probe fallback",
            )
    elif "runtime_resolve_descriptor_ref_json" in combined_production:
        add(
            "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG",
            runtime_descriptor_provider if runtime_descriptor_provider.exists() else ffi_invocation,
            1,
            "runtime_resolve_descriptor_ref_json must remain inspectable",
        )

descriptor_dir = cli_root / "ability-descriptors/system/device_control"
if descriptor_dir.exists():
    for descriptor in sorted(descriptor_dir.glob("browser.*.ability.toml")):
        add(
            "R95B_RETIRED_BROWSER_DESCRIPTOR_SURFACE",
            descriptor,
            1,
            "retired browser mock descriptors must not remain in active system inventory",
        )
    for descriptor in sorted(descriptor_dir.glob("*.ability.toml")):
        descriptor_text = descriptor.read_text(encoding="utf-8", errors="replace")
        for token in (
            "browser.open_session",
            "browser.capture_viewport",
            "browser.send_input",
            "browser.close_session",
            "browser.attach_session",
            "V0 MOCK",
            "PLACEHOLDER",
        ):
            if token in descriptor_text:
                add(
                    "R95B_RETIRED_BROWSER_DESCRIPTOR_SURFACE",
                    descriptor,
                    line_number(descriptor_text, descriptor_text.find(token)),
                    "active system descriptors must not advertise retired browser mock surface",
                )

federation_descriptor_dir = cli_root / "ability-descriptors/system/federation"
if federation_descriptor_dir.exists():
    retired_v1 = federation_descriptor_dir / "federation.subscribe_directory.ability.toml"
    if retired_v1.exists():
        add(
            "R95C_RETIRED_FEDERATION_DIRECTORY_V1_DESCRIPTOR",
            retired_v1,
            1,
            "retired federation.subscribe_directory v1 descriptor must not remain in active system inventory",
        )
    for descriptor in sorted(federation_descriptor_dir.glob("*.ability.toml")):
        if descriptor.name == "federation.subscribe_directory_v2.ability.toml":
            continue
        descriptor_text = descriptor.read_text(encoding="utf-8", errors="replace")
        for token in (
            "federation.subscribe_directory",
            "legacy federation directory snapshots",
            "PresenceEventDelta",
            "SubscribeDirectoryInitial",
        ):
            if token in descriptor_text:
                add(
                    "R95C_RETIRED_FEDERATION_DIRECTORY_V1_DESCRIPTOR",
                    descriptor,
                    line_number(descriptor_text, descriptor_text.find(token)),
                    "active federation descriptors must not advertise retired v1 directory stream",
                )


# Rule 96: Authorized runtime history reads must bind the authority-bearing
# call tuple while keeping receipt filters as post-admission ledger predicates.
# Caller/callee filters may only narrow the authorized tuple. Subject filters
# are exact ledger predicates and must not be coupled to the authority subject:
# a user-session authority can authorize the history call while filtering the
# device-owned receipt subject that is being inspected.
for (
    path,
    list_name,
    request_validator,
    filter_validator,
    scope_resolver,
    scope_capability,
    canonical_session_admission,
    retired_exact_subject_helper,
    history_test,
    history_test_name,
    filter_subject_test_name,
) in (
    (
        cli_root / "sdk/go/authorized_runtime_session.go",
        "List",
        "validateSessionHistoryRequest(request, scope)",
        "validateSessionHistoryFilterBinding(request.Call, request.Filter)",
        "receiptHistoryListAuthorityScope(o.session.receipts)",
        "ReceiptHistoryAuthorityScopeProvider",
        "runtimeSessionAuthorityAdmitsSubject(authority, subjectURA)",
        "sessionHistoryAuthoritySubjectMatches(",
        cli_root / "sdk/go/authorized_runtime_session_test.go",
        "TestAuthorizedRuntimeSessionHistoryAllowsUserOwnedResourceSubjectBeforeReceiptProvider",
        "TestAuthorizedRuntimeSessionHistoryAllowsSessionAuthorityWithExactDeviceSubjectFilter",
    ),
    (
        cli_root / "sdk/python/easynet_sdk/authorized_runtime_session.py",
        "list",
        "validate_receipt_history_request(request.call, request.filter, required_scope)",
        "_validate_receipt_history_filter_binding(call, receipt_filter)",
        "_receipt_history_list_authority_scope(self._session._receipts)",
        "receipt_history_list_authority_scope",
        "session_authority_admits_subject(authority, subject_ura)",
        "_session_history_authority_subject_matches(",
        cli_root / "sdk/python/tests/test_authorized_runtime_session.py",
        "test_history_allows_user_owned_resource_subject_before_receipt_provider",
        "test_history_allows_session_authority_with_exact_device_subject_filter",
    ),
):
    if not path.exists():
        continue
    text = source(path)
    guard_text = ""
    if path.suffix == ".py":
        guard_path = cli_root / "sdk/python/easynet_sdk/_receipt_history_admission.py"
        if guard_path.exists():
            guard_text = source(guard_path)
    combined_text = text + "\n" + guard_text
    if "SessionHistoryOperations" not in text:
        continue
    if path.suffix == ".go":
        list_body = brace_function_body(
            text,
            r"func\s*\([^)]*\*SessionHistoryOperations[^)]*\)\s*List\s*\(",
        )
    elif path.suffix == ".py":
        match = re.search(
            r"class\s+SessionHistoryOperations\b.*?^\s*class\s+",
            text,
            re.DOTALL | re.MULTILINE,
        )
        list_body = (match.start(), match.group(0)) if match else None
    else:
        list_body = None
    if list_body is None:
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            path,
            1,
            "SessionHistoryOperations list method must remain inspectable",
        )
    else:
        offset, body = list_body
        if request_validator not in body:
            add(
                "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
                path,
                line_number(text, offset),
                "Session history list must validate the complete request, including filters",
            )
        if scope_resolver not in body:
            add(
                "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
                path,
                line_number(text, offset),
                "Session history list must resolve the provider-declared authority scope",
            )
    if request_validator not in text or filter_validator not in combined_text:
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            path,
            1,
            "Session history request validation must call filter tuple binding",
        )
    if scope_capability not in text:
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            path,
            1,
            "Session history validation must depend on a provider-declared authority-scope capability",
        )
    if canonical_session_admission not in combined_text:
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            path,
            1,
            "Session history authority subject must reuse canonical session authority admission",
        )
    if retired_exact_subject_helper in text:
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            path,
            1,
            "Retired history-only exact subject helper must not remain",
        )
    if path.suffix == ".py" and "def _session_authority_admits_subject(" in text:
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            path,
            1,
            "Authorized runtime session must call the canonical session authority admission helper directly",
        )
    if history_test.exists() and history_test_name not in source(history_test):
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            history_test,
            1,
            "Session history tests must admit user-owned resource subjects through canonical session authority admission",
        )
    if history_test.exists() and filter_subject_test_name not in source(history_test):
        add(
            "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
            history_test,
            1,
            "Session history tests must allow exact device subject filters under a session-authorized call",
        )
    for token, detail in (
        (
            "filter_caller_ura",
            "history filter caller_ura must be compared with call caller_ura",
        ),
        (
            "filter_callee_ura",
            "history filter callee_ura must be compared with call callee_ura",
        ),
    ):
        if token not in combined_text:
            add("R96_SDK_HISTORY_FILTER_TUPLE_BINDING", path, 1, detail)
    for retired in (
        "filter_subject_ura",
        "receipt filter subject_uras must be bound to receipt query subject_ura",
    ):
        if retired in text:
            add(
                "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
                path,
                1,
                "Session history subject filters must remain ledger predicates, not authority-subject aliases",
            )


# Node participates in the same canonical runtime model. Keep the history
# preflight at the SDK boundary so product callers cannot submit stale
# session-authority subjects and discover deterministic mismatches only after
# daemon admission.
node_history = cli_root / "sdk/node/index.js"
node_history_test = cli_root / "sdk/node/test/runtime-core.test.mjs"
if node_history.exists():
    text = source(node_history)
    for token, detail in (
        ("export class SessionHistoryOperations", "Node SDK must expose generic session history operations"),
        ("export class RuntimeCallContext", "Node SDK must model the authority-bearing runtime call tuple"),
        ("export class ReceiptListRequest", "Node SDK must model canonical receipt history requests"),
        ("function validateSessionHistoryRequest(request)", "Node SDK history list must validate complete requests"),
        ("function validateSessionHistoryFilterBinding(call, filter)", "Node SDK must keep receipt filters explicit"),
        ("function validateSessionHistorySessionBinding(", "Node SDK must validate session authority before provider I/O"),
        ("sessionAuthorityAdmitsSubject(authority, subjectURA)", "Node history must reuse canonical session subject admission"),
        ("session authority subject does not admit receipt query subject_ura", "Node history must surface typed subject mismatch"),
        ("receipt filter caller_ura does not match receipt query caller_ura", "Node history caller filter must only narrow the tuple"),
        ("receipt filter callee_ura does not match receipt query callee_ura", "Node history callee filter must only narrow the tuple"),
    ):
        if token not in text:
            add("R96_SDK_HISTORY_FILTER_TUPLE_BINDING", node_history, 1, detail)
    history_offset = text.find("export class SessionHistoryOperations")
    if history_offset >= 0:
        next_class = text.find("\nexport class ", history_offset + 1)
        body = text[history_offset : next_class if next_class >= 0 else len(text)]
        if "validateSessionHistoryRequest(payload);" not in body:
            add(
                "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
                node_history,
                line_number(text, history_offset),
                "Node session history list must validate before calling the receipt provider",
            )
        if ".receipts.list(payload)" not in body:
            add(
                "R96_SDK_HISTORY_FILTER_TUPLE_BINDING",
                node_history,
                line_number(text, history_offset),
                "Node session history list must delegate through one receipt provider boundary",
            )
    if node_history_test.exists():
        tests = source(node_history_test)
        for token, detail in (
            (
                "session history preflight rejects authority subject mismatch before receipt provider",
                "Node history subject mismatch preflight test is missing",
            ),
            (
                "session history keeps subject filters as ledger predicates",
                "Node history subject-filter ledger predicate test is missing",
            ),
            (
                "providerCalls, 0",
                "Node history mismatch test must prove provider I/O was not reached",
            ),
        ):
            if token not in tests:
                add("R96_SDK_HISTORY_FILTER_TUPLE_BINDING", node_history_test, 1, detail)


# Rule 92: Invocation attempt audit is the product-visible pre-runtime
# failure ledger. It must fail closed when unavailable or corrupt; otherwise
# descriptor/admission/route failures disappear from invocation.history.list
# and products see false empty history rows.
attempt_audit = cli_root / "src/daemon/invocation/dispatch/attempt_audit.rs"
if attempt_audit.exists():
    raw_text = attempt_audit.read_text(encoding="utf-8", errors="replace")
    text = source(attempt_audit)
    for pattern, detail in (
        (
            r"fn\s+disabled\s*\(",
            "invocation attempt audit must not expose a disabled handle",
        ),
        (
            r"let\s+Ok\s*\([^)]*\)\s*=\s*self\.writer\.lock\(\)\s*else\s*\{\s*return\s*;",
            "invocation attempt audit must not ignore a poisoned writer lock",
        ),
        (
            r"if\s+let\s+Ok\s*\([^)]*\)\s*=\s*serde_json::from_str::<InvocationAttemptRecord>",
            "invocation attempt audit must not skip corrupt ledger rows",
        ),
        (
            r"let\s+_\s*=\s*writeln!\(",
            "invocation attempt audit must not ignore append failures",
        ),
        (
            r"unwrap_or_else\s*\(\s*\|_\|\s*\"unknown\"\.to_string\(\)\s*\)",
            "runtime response state projection must fail closed instead of recording unknown",
        ),
    ):
        match = re.search(pattern, text, re.DOTALL)
        if match:
            add(
                "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED",
                attempt_audit,
                line_number(text, match.start()),
                detail,
            )
    for token, detail in (
        (
            "anyhow::Result<InvocationAttemptHandle>",
            "InvocationAttemptLedger::begin must report append failure",
        ),
        (
            "fn append(&self, record: &InvocationAttemptRecord) -> anyhow::Result<()>",
            "attempt ledger append must return a failure result",
        ),
        (
            "decode invocation attempt ledger row",
            "attempt ledger reads must fail closed on corrupt rows",
        ),
        (
            "struct RuntimeResponseStateProjection",
            "attempt audit must use a typed runtime response state projection",
        ),
        (
            "PROTOCOL_MISMATCH",
            "invalid runtime response state must become an explicit protocol mismatch",
        ),
    ):
        if token not in text:
            add(
                "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED",
                attempt_audit,
                1,
                detail,
            )
    if "invocation_attempt_audit_projects_invalid_runtime_state_as_protocol_mismatch" not in raw_text:
        add(
            "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED",
            attempt_audit,
            1,
            "attempt audit must test invalid runtime state projection",
        )

daemon_service = cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service.rs"
if daemon_service.exists():
    text = source(daemon_service)
    if "InvocationAttemptHandle::disabled" in text or "unwrap_or_else(missing_invocation_attempt_ledger)" in text:
        add(
            "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED",
            daemon_service,
            1,
            "daemon invocation service must fail closed when attempt audit is not wired",
        )
    for token, detail in (
        (
            "missing_invocation_attempt_ledger",
            "daemon invocation service must classify missing attempt audit as an internal boot wiring fault",
        ),
        (
            "invocation_attempt_audit_status",
            "daemon invocation service must propagate attempt audit write failures",
        ),
    ):
        if token not in text:
            add(
                "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED",
                daemon_service,
                1,
                detail,
            )

boot_invocation = cli_root / "src/daemon/boot/invocation/mod.rs"
if boot_invocation.exists():
    text = source(boot_invocation)
    if "invocation_attempt_ledger_disabled" in text:
        add(
            "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED",
            boot_invocation,
            line_number(text, text.index("invocation_attempt_ledger_disabled")),
            "daemon boot must not continue with invocation attempt audit disabled",
        )
    if "refusing to boot without" not in text or "InvocationAttemptLedger::open" not in text:
        add(
            "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED",
            boot_invocation,
            1,
            "daemon boot must open the required invocation attempt ledger fail-closed",
        )


# Rule 79: Invocation signing custody must be ownership/lease backed. A raw
# key-service signer capability must not imply descriptor-bound invocation
# authority by constructing a caller identity from the URA string.
receipt_signing = cli_root / "src/daemon/identity/receipt_signing.rs"
if receipt_signing.exists():
    text = source(receipt_signing)
    for match in re.finditer(r"strict_identity\s*\(\s*caller_ura\s*\)\s*\.ok\s*\(", text):
        add(
            "R79_INVOCATION_SIGNER_CUSTODY_AUTHORITY",
            receipt_signing,
            line_number(text, match.start()),
            "invocation signing must not construct authority from strict_identity(caller_ura).ok()",
        )
    resolve_body = rust_method_body(text, "resolve")
    if resolve_body is not None:
        offset, body = resolve_body
        if "self_signed" in body and ".callee_identity()" not in body:
            add(
                "R79_INVOCATION_SIGNER_CUSTODY_AUTHORITY",
                receipt_signing,
                line_number(text, offset),
                "self-signed invocation signing must project caller identity from owned receipt authority",
            )

self_identity = cli_root / "src/daemon/identity/self_identity.rs"
if self_identity.exists():
    text = source(self_identity)
    raw_text = self_identity.read_text(encoding="utf-8", errors="replace")
    runtime_owner_guard = "fn validate_runtime_owner_signing_ura(owner_ura: &str)"
    for token, detail, haystack in (
        (
            runtime_owner_guard,
            "RuntimeSigningIdentity must classify runtime-owner URAs before key-service lookup",
            text,
        ),
        (
            "runtime-owner signing identity does not manage User URAs; use managed user signing custody",
            "RuntimeSigningIdentity must fail closed for User URAs instead of probing runtime-owner keyring state",
            text,
        ),
        (
            "runtime_owner_signing_identity_rejects_user_before_keyring_lookup",
            "self_identity tests must prove User URAs do not reach the runtime-owner provider",
            raw_text,
        ),
    ):
        if token not in haystack:
            add("R79_INVOCATION_SIGNER_CUSTODY_AUTHORITY", self_identity, 1, detail)
    load_match = re.search(
        r"impl\s+RuntimeSigningIdentity\s*\{(?P<body>.*?)\n\}\n\n#\[async_trait::async_trait\]",
        text,
        re.DOTALL,
    )
    if load_match is None:
        add(
            "R79_INVOCATION_SIGNER_CUSTODY_AUTHORITY",
            self_identity,
            1,
            "RuntimeSigningIdentity impl must remain inspectable",
        )
    else:
        body = load_match.group("body")
        guard_index = body.find("validate_runtime_owner_signing_ura(owner_ura)?")
        lookup_index = body.find("provider.public_key(owner_ura)?")
        if guard_index < 0 or lookup_index < 0 or guard_index > lookup_index:
            add(
                "R79_INVOCATION_SIGNER_CUSTODY_AUTHORITY",
                self_identity,
                line_number(text, load_match.start()),
                "RuntimeSigningIdentity::load must validate runtime-owner custody before provider.public_key",
            )


# Rule 80: Ability catalogue assembly must receive a concrete authority
# context. Optional authority state plus unwrap/default lets daemon boot and
# deterministic snapshots silently publish the wrong owner plane.
catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
if catalog_build.exists():
    text = source(catalog_build)
    for match in re.finditer(r"authority_context\s*:\s*Option\s*<", text):
        add(
            "R80_CATALOG_AUTHORITY_CONTEXT_REQUIRED",
            catalog_build,
            line_number(text, match.start()),
            "catalog build authority_context must be concrete, not Option",
        )
    for match in re.finditer(r"authority_context\s*\.unwrap_or_default\s*\(", text):
        add(
            "R80_CATALOG_AUTHORITY_CONTEXT_REQUIRED",
            catalog_build,
            line_number(text, match.start()),
            "catalog build must not default missing authority context from local environment",
        )
    for match in re.finditer(r"authority_context\s*:\s*None\b", text):
        add(
            "R80_CATALOG_AUTHORITY_CONTEXT_REQUIRED",
            catalog_build,
            line_number(text, match.start()),
            "catalog build must not encode absent authority context as a valid assembly state",
        )


# Rule 81: Ability publication projection is route/catalog evidence, not a
# lossy render cache. Local publication and federation.resolve must fail closed
# on corrupt summaries instead of silently dropping rows and making product
# callers see empty route visibility.
ability_publication = cli_root / "src/daemon/ability/catalog/publication.rs"
if ability_publication.exists():
    text = source(ability_publication)
    body = rust_method_body(text, "owner_projection_values")
    if body is None:
        add(
            "R81_ABILITY_PUBLICATION_PROJECTION_FAIL_CLOSED",
            ability_publication,
            1,
            "LocalAbilityPublicationSnapshot must expose owner_projection_values",
        )
    else:
        offset, projection_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<Value>, String>" not in signature:
            add(
                "R81_ABILITY_PUBLICATION_PROJECTION_FAIL_CLOSED",
                ability_publication,
                line_number(text, offset),
                "owner_projection_values must return Result so corrupt local publication cannot become an empty catalogue",
            )
        for token, detail in (
            (
                "filter_map",
                "owner_projection_values must not skip corrupt descriptor summaries",
            ),
            (
                ".ok()",
                "owner_projection_values must not hide summary/JSON projection failures behind Option",
            ),
        ):
            if token in projection_body:
                add(
                    "R81_ABILITY_PUBLICATION_PROJECTION_FAIL_CLOSED",
                    ability_publication,
                    line_number(text, body_start + projection_body.find(token)),
                    detail,
                )

# Rule 82: owner projection cursors are canonical lifecycle facts, not
# arbitrary publication cache rows. Cursor persistence and publication integrity
# must share one owner/host URA binding state machine so malformed rows fail
# before heartbeat refresh, purge replay, or authority admission can see them as
# partial state.
owner_projection_store = cli_root / "src/daemon/persistence/owner_projections.rs"
owner_projection_read_model = cli_root / "src/daemon/federation/read_model/owner_projection.rs"
if owner_projection_store.exists():
    text = source(owner_projection_store)
    for token, detail in (
        (
            "fn validate_owner_projection_host_binding(",
            "owner projection cursor store must expose canonical owner/host binding validation",
        ),
        (
            "owner_ura must be a canonical owner URA",
            "owner projection cursor validation must reject malformed owner URAs",
        ),
        (
            "host_device_ura must be a canonical host URA",
            "owner projection cursor validation must reject malformed host URAs",
        ),
        (
            "Agent owner projections must be hosted by a Device URA",
            "owner projection cursor validation must bind Agent owners to Device hosts",
        ),
        (
            "DeviceProfileProjection migration cursors must be hosted by the same Device URA",
            "owner projection cursor validation must bind same-device DeviceProfileProjection migration cursors to themselves",
        ),
        (
            "Authority owner projections must be hosted by the same Authority URA",
            "owner projection cursor validation must bind Authority owners to themselves",
        ),
        (
            "same-device DeviceProfileProjection URA",
            "owner projection cursor store must describe Device rows as migration read-model cursors, not public actor ownership",
        ),
        (
            "validate_owner_projection_host_binding(&cursor.owner_ura, &cursor.host_device_ura)",
            "owner projection cursor file validation must call the shared binding guard",
        ),
    ):
        if token not in text:
            add(
                "R82_OWNER_PROJECTION_CURSOR_BINDING_FAIL_CLOSED",
                owner_projection_store,
                1,
                detail,
            )
if owner_projection_read_model.exists():
    text = source(owner_projection_read_model)
    integrity = rust_method_body(text, "validate_integrity")
    if integrity is None or "validate_owner_projection_host_binding" not in integrity[1]:
        add(
            "R82_OWNER_PROJECTION_CURSOR_BINDING_FAIL_CLOSED",
            owner_projection_read_model,
            1,
            "owner projection publication integrity must use the shared owner/host binding guard",
        )
    refresh = rust_method_body(text, "heartbeat_refresh_owner_uras_from_file")
    if refresh is not None:
        offset, refresh_body = refresh
        for token, detail in (
            (
                "trim().is_empty()",
                "heartbeat owner refresh must not skip corrupt cursor rows as empty state",
            ),
            (
                "return None",
                "heartbeat owner refresh must not silently drop active corrupt cursor rows",
            ),
        ):
            match = refresh_body.find(token)
            if match != -1:
                add(
                    "R82_OWNER_PROJECTION_CURSOR_BINDING_FAIL_CLOSED",
                    owner_projection_read_model,
                    line_number(text, offset + match),
                    detail,
                )

# Rule 83: builtin plugin provider projection must bind provider identity to the
# manifest entrypoint before creating a package binding. Package load still
# validates manifests, but the registry owns the provider -> manifest binding
# boundary and must not emit a partially-valid builtin binding.
plugin_provider_registry = cli_root / "src/daemon/plugins/provider_registry.rs"
if plugin_provider_registry.exists():
    text = source(plugin_provider_registry)
    body = rust_method_body(text, "binding_from_provider")
    if body is None:
        add(
            "R83_PLUGIN_PROVIDER_ENTRYPOINT_BINDING",
            plugin_provider_registry,
            1,
            "PluginProviderRegistry must own provider -> builtin binding projection",
        )
    else:
        _, projection_body = body
        for token, detail in (
            (
                "validate_builtin_entrypoint(&manifest, provider.expected_entrypoint())",
                "provider registry must validate manifest entrypoint against the compiled provider entrypoint",
            ),
            (
                "ProviderManifestMismatch",
                "provider registry must validate package id before entrypoint projection",
            ),
        ):
            if token not in projection_body:
                add(
                    "R83_PLUGIN_PROVIDER_ENTRYPOINT_BINDING",
                    plugin_provider_registry,
                    1,
                    detail,
                )
federation_wrappers = cli_root / "src/daemon/invocation/dispatch/federation_wrappers.rs"
if federation_wrappers.exists():
    text = source(federation_wrappers)
    for name, expected in (
        ("handle_resolve_at", "Result<ResolveResponse, String>"),
        ("resolved_owner_projection_values", "Result<Vec<serde_json::Value>, String>"),
    ):
        body = rust_method_body(text, name)
        if body is None:
            add(
                "R81_ABILITY_PUBLICATION_PROJECTION_FAIL_CLOSED",
                federation_wrappers,
                1,
                f"federation resolve must keep {name} as an explicit projection surface",
            )
            continue
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if expected not in signature:
            add(
                "R81_ABILITY_PUBLICATION_PROJECTION_FAIL_CLOSED",
                federation_wrappers,
                line_number(text, offset),
                f"{name} must return {expected} so projection failures cannot become empty route visibility",
            )
        for token, detail in (
            (
                "return;",
                f"{name} must not silently drop invalid ability summaries",
            ),
            (
                "summary_from_value(&summary).and_then",
                f"{name} must parse summary rows as required schema-bound input",
            ),
        ):
            if token in fn_body:
                add(
                    "R81_ABILITY_PUBLICATION_PROJECTION_FAIL_CLOSED",
                    federation_wrappers,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )


# Rule 82: Desktop companion status is a runtime observation, not an optional
# decoration. Lifecycle status and plugin package surfaces must preserve
# companion projection failures as explicit facts instead of collapsing broken
# manager/index/projection state into an empty companion list.
lifecycle_status = cli_root / "src/daemon/boot/lifecycle/status.rs"
if lifecycle_status.exists():
    text = source(lifecycle_status)
    if "pub struct DesktopCompanionStatusObservation" not in text:
        add(
            "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
            lifecycle_status,
            1,
            "desktop companion runtime status must use a named observation carrying statuses and errors",
        )
    if "desktop_companion_errors" not in text:
        add(
            "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
            lifecycle_status,
            1,
            "RuntimeStatusReport JSON must expose desktop_companion_errors",
        )
    body = rust_method_body(text, "desktop_companion_statuses")
    if body is None:
        add(
            "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
            lifecycle_status,
            1,
            "desktop_companion_statuses must remain the lifecycle companion observation collector",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "DesktopCompanionStatusObservation" not in signature:
            add(
                "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
                lifecycle_status,
                line_number(text, offset),
                "desktop_companion_statuses must return DesktopCompanionStatusObservation, not Vec<Value>",
            )
        for token, detail in (
            (
                "return Vec::new()",
                "desktop_companion_statuses must not project plugin-state failure as no companions",
            ),
            (
                "filter_map(|package| manager.status_json(package).ok())",
                "desktop_companion_statuses must not skip companion status projection errors",
            ),
            (
                "status_json(package).ok()",
                "desktop_companion_statuses must not hide companion status projection errors behind Option",
            ),
        ):
            if token in fn_body:
                add(
                    "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
                    lifecycle_status,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )

plugin_surface = cli_root / "src/daemon/plugins/surface.rs"
if plugin_surface.exists():
    text = source(plugin_surface)
    if "companion_error" not in text:
        add(
            "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
            plugin_surface,
            1,
            "plugin package surface must expose companion_error instead of omitting broken companion status",
        )
    body = rust_method_body(text, "project_packages_with_daemon")
    if body is not None:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        if "status_json(package).ok()" in fn_body:
            add(
                "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
                plugin_surface,
                line_number(text, body_start + fn_body.find("status_json(package).ok()")),
                "plugin package surface must preserve companion status projection errors",
            )

companion_manager = cli_root / "src/daemon/plugins/companion/mod.rs"
if companion_manager.exists():
    text = source(companion_manager)
    body = rust_method_body(text, "status_json")
    if body is None:
        add(
            "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
            companion_manager,
            1,
            "DesktopCompanionManager::status_json must remain the fallible companion DTO projector",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        for token, detail in (
            (
                "serde_json::to_value(status)\n            .ok()",
                "DesktopCompanionManager::status_json must preserve serialization failure causes",
            ),
            (
                "project_status(&value).ok()",
                "DesktopCompanionManager::status_json must preserve projection failure causes",
            ),
        ):
            if token in fn_body:
                add(
                    "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
                    companion_manager,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "companion status projection failed: {source}" not in fn_body:
            add(
                "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS",
                companion_manager,
                line_number(text, offset),
                "DesktopCompanionManager::status_json must report the projection source error",
            )


# Rule 83: mission.think curator catalog is authoring authority, not a
# best-effort prompt hint. Corrupt/unreadable Agent registry projection must
# fail the curator catalog stage instead of becoming an empty catalog that lets
# the curator author against false route visibility.
automation_think = cli_root / "src/daemon/ability/builtins/automation/think.rs"
if automation_think.exists():
    text = source(automation_think)
    body = rust_method_body(text, "collect_owner_catalog")
    if body is None:
        add(
            "R83_CURATOR_CATALOG_FAIL_CLOSED",
            automation_think,
            1,
            "mission.think must keep collect_owner_catalog as the curator catalog authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<CatalogEntry>, String>" not in signature:
            add(
                "R83_CURATOR_CATALOG_FAIL_CLOSED",
                automation_think,
                line_number(text, offset),
                "collect_owner_catalog must return Result so registry projection failure cannot become an empty catalog",
            )
        for token, detail in (
            (
                "Err(_) => return Vec::new()",
                "collect_owner_catalog must not hide registry projection failures behind an empty catalog",
            ),
            (
                "Err(error) => return Vec::new()",
                "collect_owner_catalog must not hide registry projection failures behind an empty catalog",
            ),
            (
                "Catalog gathering is best-effort",
                "curator catalog comments must not preserve best-effort catalog semantics",
            ),
        ):
            if token in fn_body or token in text[max(0, offset - 600):offset]:
                add(
                    "R83_CURATOR_CATALOG_FAIL_CLOSED",
                    automation_think,
                    line_number(text, body_start + fn_body.find(token))
                    if token in fn_body
                    else line_number(text, max(0, offset - 600) + text[max(0, offset - 600):offset].find(token)),
                    detail,
                )
    if '"stage": "catalog"' not in text:
        add(
            "R83_CURATOR_CATALOG_FAIL_CLOSED",
            automation_think,
            1,
            "mission.think curator outcome must expose catalog acquisition failures as stage=catalog",
        )
    if "owner ability catalog unavailable" not in text:
        add(
            "R83_CURATOR_CATALOG_FAIL_CLOSED",
            automation_think,
            1,
            "collect_owner_catalog must preserve an operator-facing owner catalog unavailable diagnostic",
        )


# Rule 83b: skill.publish provenance is explicit runtime publication state,
# not a Mission compatibility fallback. Missing mission_run_id preserves the
# public request contract but must record direct runtime publication instead of
# pretending the publish came from mission.think.
skill_publish = cli_root / "src/daemon/ability/builtins/resources/skills/publish.rs"
if skill_publish.exists():
    text = source(skill_publish)
    raw_text = skill_publish.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "enum SkillPublishProvenance",
            "skill.publish must keep provenance as an explicit state object",
        ),
        (
            "DirectPublish",
            "skill.publish must model missing mission_run_id as direct publication",
        ),
        (
            '"direct_publish"',
            "skill.publish must persist direct runtime publication provenance",
        ),
    ):
        if token not in text:
            add("R83B_SKILL_PUBLISH_EXPLICIT_PROVENANCE", skill_publish, 1, detail)
    if "publish_without_run_id_records_direct_publish_provenance" not in raw_text:
        add(
            "R83B_SKILL_PUBLISH_EXPLICIT_PROVENANCE",
            skill_publish,
            1,
            "skill.publish must test direct publication provenance",
        )
    body = rust_method_body(text, "publish_handler")
    if body is None:
        add(
            "R83B_SKILL_PUBLISH_EXPLICIT_PROVENANCE",
            skill_publish,
            1,
            "skill.publish must keep a named publish_handler for provenance auditing",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        for token, detail in (
            (
                'unwrap_or_else(|| "mission.think".to_string())',
                "skill.publish must not synthesize Mission provenance when mission_run_id is absent",
            ),
            (
                'kind: "curator".to_string()',
                "publish_handler must not inline curator provenance assembly outside SkillPublishProvenance",
            ),
        ):
            if token in fn_body:
                add(
                    "R83B_SKILL_PUBLISH_EXPLICIT_PROVENANCE",
                    skill_publish,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )


# Rule 84: schedule due selection and snapshot projection are runtime
# lifecycle state, not optional cache reads. A poisoned schedule cache or
# corrupt enabled cron row must fail the observation explicitly; it must never
# be projected as an empty due list/schedule list that makes operators believe
# no schedule exists or is ready.
schedule_mod = cli_root / "src/daemon/execution/schedule/mod.rs"
daemon_bin = cli_root / "src/bin/easynet-daemon.rs"
automation_schedule = cli_root / "src/daemon/ability/builtins/automation/schedule.rs"
schedule_loader = cli_root / "src/daemon/ability/builtins/resources/context/loaders/schedule.rs"
if schedule_mod.exists():
    text = source(schedule_mod)
    body = rust_method_body(text, "list")
    if body is None:
        add(
            "R84_SCHEDULE_DUE_FAIL_CLOSED",
            schedule_mod,
            1,
            "ScheduleService must keep list as the schedule snapshot authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<ScheduleEntry>>" not in signature:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_mod,
                line_number(text, offset),
                "ScheduleService::list must return Result so corrupt runtime state cannot become an empty schedule list",
            )
        for token, detail in (
            (
                "Err(_) => Vec::new()",
                "ScheduleService::list must not hide a poisoned cache behind an empty schedule list",
            ),
            (
                "Err(_) => return Vec::new()",
                "ScheduleService::list must not hide a poisoned cache behind an empty schedule list",
            ),
        ):
            if token in fn_body:
                add(
                    "R84_SCHEDULE_DUE_FAIL_CLOSED",
                    schedule_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "schedule list cache lock poisoned" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_mod,
                line_number(text, offset),
                "ScheduleService::list must preserve poisoned cache as explicit unavailable state",
            )

    body = rust_method_body(text, "next_fire_for_entry")
    if body is None:
        add(
            "R84_SCHEDULE_DUE_FAIL_CLOSED",
            schedule_mod,
            1,
            "ScheduleService must expose next_fire_for_entry so snapshot readers do not re-query and hide cron corruption",
        )
    else:
        offset, fn_body = body
        if "parse_entry_cron(entry)?" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_mod,
                line_number(text, offset),
                "next_fire_for_entry must use the schedule-entry cron validator",
            )
    body = rust_method_body(text, "parse_entry_cron")
    if body is None:
        add(
            "R84_SCHEDULE_DUE_FAIL_CLOSED",
            schedule_mod,
            1,
            "schedule core must keep parse_entry_cron as the shared corrupt-cron diagnostic boundary",
        )
    else:
        offset, fn_body = body
        if "has invalid cron" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_mod,
                line_number(text, offset),
                "parse_entry_cron must preserve corrupt enabled cron rows as explicit unavailable state",
            )

    body = rust_method_body(text, "due")
    if body is None:
        add(
            "R84_SCHEDULE_DUE_FAIL_CLOSED",
            schedule_mod,
            1,
            "ScheduleService must keep due selection as the tick lifecycle authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<DueFire>>" not in signature:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_mod,
                line_number(text, offset),
                "ScheduleService::due must return Result so corrupt runtime state cannot become an empty due list",
            )
        for token, detail in (
            (
                "Err(_) => return Vec::new()",
                "ScheduleService::due must not hide a poisoned cache behind an empty due list",
            ),
            (
                "Err(_) => continue",
                "ScheduleService::due must not skip corrupt enabled cron rows",
            ),
            (
                "Err(error) => continue",
                "ScheduleService::due must not skip corrupt enabled cron rows",
            ),
        ):
            if token in fn_body:
                add(
                    "R84_SCHEDULE_DUE_FAIL_CLOSED",
                    schedule_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "schedule due cache lock poisoned" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_mod,
                line_number(text, offset),
                "ScheduleService::due must preserve poisoned cache as explicit unavailable state",
            )
        if "parse_entry_cron(entry)?" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_mod,
                line_number(text, offset),
                "ScheduleService::due must use the shared schedule-entry cron validator",
            )

if daemon_bin.exists():
    text = source(daemon_bin)
    body = rust_method_body(text, "spawn_schedule_tick")
    if body is None:
        add(
            "R84_SCHEDULE_DUE_FAIL_CLOSED",
            daemon_bin,
            1,
            "schedule tick runner must own due-selection failure projection",
        )
    else:
        offset, fn_body = body
        if "schedule.due(" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                daemon_bin,
                line_number(text, offset),
                "schedule tick runner must consume ScheduleService::due",
            )
        if "due selection failed" not in fn_body or "Err(err)" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                daemon_bin,
                line_number(text, offset),
                "schedule tick runner must surface due-selection failure instead of treating it as no due fires",
            )
        if "schedule snapshot failed" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                daemon_bin,
                line_number(text, offset),
                "schedule tick runner must surface schedule snapshot failure instead of treating it as vanished schedules",
            )

if automation_schedule.exists():
    text = source(automation_schedule)
    body = rust_method_body(text, "list_handler_with_envelope")
    if body is None:
        add(
            "R84_SCHEDULE_DUE_FAIL_CLOSED",
            automation_schedule,
            1,
            "schedule.list handler must preserve schedule snapshot failures",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        if "svc.list_for_accountable_user(&owner)?" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                automation_schedule,
                line_number(text, offset),
                "schedule.list handler must propagate owner-scoped ScheduleService snapshot errors",
            )
        if "unwrap_or(Value::Null)" in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                automation_schedule,
                line_number(text, body_start + fn_body.find("unwrap_or(Value::Null)")),
                "schedule.list handler must not project schedule serialization failure as null rows",
            )

if schedule_loader.exists():
    text = source(schedule_loader)
    body = rust_method_body(text, "load")
    if body is None:
        add(
            "R84_SCHEDULE_DUE_FAIL_CLOSED",
            schedule_loader,
            1,
            "ScheduleLoader::load must preserve schedule context projection failures",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        if "ScheduleService::next_fire_for_entry(&entry, now)?" not in fn_body:
            add(
                "R84_SCHEDULE_DUE_FAIL_CLOSED",
                schedule_loader,
                line_number(text, offset),
                "ScheduleLoader must compute next fires from the coherent snapshot entry and propagate corrupt cron errors",
            )
        for token, detail in (
            (
                "Ok(None) | Err(_) => continue",
                "ScheduleLoader must not hide next-fire errors as absent schedule context",
            ),
            (
                "next_fire_after(&entry.id",
                "ScheduleLoader must not re-query schedule state after obtaining the snapshot entry",
            ),
        ):
            if token in fn_body:
                add(
                    "R84_SCHEDULE_DUE_FAIL_CLOSED",
                    schedule_loader,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )


# Rule 85: live session index is runtime state, not a best-effort
# discovery cache. A poisoned session index must surface as unavailable
# session state; it must not become an empty session list, an unknown-session
# attach snapshot, or null rows in session SystemAgent session.list.
session_mod = cli_root / "src/daemon/execution/session/mod.rs"
device_session = cli_root / "src/daemon/ability/builtins/device_control/session.rs"
kernel_mod = cli_root / "src/daemon/boot/kernel/mod.rs"
if session_mod.exists():
    text = source(session_mod)
    body = rust_method_body(text, "list_active")
    if body is None:
        add(
            "R85_SESSION_INDEX_FAIL_CLOSED",
            session_mod,
            1,
            "SessionService must keep list_active as the live session index snapshot authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<Session>>" not in signature:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                session_mod,
                line_number(text, offset),
                "SessionService::list_active must return Result so poisoned index state cannot become an empty session list",
            )
        for token, detail in (
            (
                "Err(_) => Vec::new()",
                "SessionService::list_active must not hide a poisoned index behind an empty session list",
            ),
            (
                "Err(_) => return Vec::new()",
                "SessionService::list_active must not hide a poisoned index behind an empty session list",
            ),
        ):
            if token in fn_body:
                add(
                    "R85_SESSION_INDEX_FAIL_CLOSED",
                    session_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "SessionService session index lock poisoned" not in fn_body:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                session_mod,
                line_number(text, offset),
                "SessionService::list_active must preserve poisoned index as explicit unavailable state",
            )

    body = rust_method_body(text, "get")
    if body is None:
        add(
            "R85_SESSION_INDEX_FAIL_CLOSED",
            session_mod,
            1,
            "SessionService must keep get as the session lookup authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Option<Session>>" not in signature:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                session_mod,
                line_number(text, offset),
                "SessionService::get must return Result so poisoned index state cannot become unknown session",
            )
        for token, detail in (
            (
                ".read()\n            .ok()",
                "SessionService::get must not erase poisoned index errors with .ok()",
            ),
            (
                "and_then(|g| g.get(id)",
                "SessionService::get must not collapse unavailable index state into None",
            ),
        ):
            if token in fn_body:
                add(
                    "R85_SESSION_INDEX_FAIL_CLOSED",
                    session_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "SessionService session index lock poisoned" not in fn_body:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                session_mod,
                line_number(text, offset),
                "SessionService::get must preserve poisoned index as explicit unavailable state",
            )

if device_session.exists():
    text = source(device_session)
    body = rust_method_body(text, "list_handler")
    if body is None:
        add(
            "R85_SESSION_INDEX_FAIL_CLOSED",
            device_session,
            1,
            "session SystemAgent session.list must preserve session index failures",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        if "svc.list_active()?" not in fn_body:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                device_session,
                line_number(text, offset),
                "session SystemAgent session.list must propagate SessionService::list_active errors",
            )
        if "unwrap_or(Value::Null)" in fn_body:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                device_session,
                line_number(text, body_start + fn_body.find("unwrap_or(Value::Null)")),
                "session SystemAgent session.list must not project session serialization failures as null rows",
            )

    body = rust_method_body(text, "attach_handler")
    if body is None:
        add(
            "R85_SESSION_INDEX_FAIL_CLOSED",
            device_session,
            1,
            "session SystemAgent session.attach must preserve session index failures",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        if "svc.get(&id)?.is_none()" not in fn_body:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                device_session,
                line_number(text, offset),
                "session SystemAgent session.attach must propagate SessionService::get errors before deciding unknown-session snapshot",
            )
        if "svc.get(&id).is_none()" in fn_body:
            add(
                "R85_SESSION_INDEX_FAIL_CLOSED",
                device_session,
                line_number(text, body_start + fn_body.find("svc.get(&id).is_none()")),
                "session SystemAgent session.attach must not collapse unavailable index state into an empty snapshot",
            )

if kernel_mod.exists():
    text = source(kernel_mod)
    for method_name, token, detail in (
        (
            "list_active_sessions",
            "self.session.list_active()",
            "Kernel::list_active_sessions must return SessionService::list_active directly",
        ),
        (
            "get_session",
            "self.session.get(id)",
            "Kernel::get_session must return SessionService::get directly",
        ),
    ):
        body = rust_method_body(text, method_name)
        if body is None:
            add("R85_SESSION_INDEX_FAIL_CLOSED", kernel_mod, 1, detail)
        else:
            offset, fn_body = body
            if token not in fn_body or "Ok(self.session." in fn_body:
                add(
                    "R85_SESSION_INDEX_FAIL_CLOSED",
                    kernel_mod,
                    line_number(text, offset),
                    detail,
                )


# Rule 86: discuss room registry is runtime state, not a best-effort
# discovery cache. A poisoned room registry must surface as unavailable
# room state; it must not become an empty room list at Kernel/product
# boundaries.
discuss_mod = cli_root / "src/daemon/execution/mission/discuss/mod.rs"
kernel_mod = cli_root / "src/daemon/boot/kernel/mod.rs"
if discuss_mod.exists():
    text = source(discuss_mod)
    body = rust_method_body(text, "list")
    if body is None:
        add(
            "R86_DISCUSS_ROOM_REGISTRY_FAIL_CLOSED",
            discuss_mod,
            1,
            "DiscussService must keep list as the room registry snapshot authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<DiscussRoom>>" not in signature:
            add(
                "R86_DISCUSS_ROOM_REGISTRY_FAIL_CLOSED",
                discuss_mod,
                line_number(text, offset),
                "DiscussService::list must return Result so poisoned room registry state cannot become an empty room list",
            )
        for token, detail in (
            (
                "Err(_) => Vec::new()",
                "DiscussService::list must not hide a poisoned room registry behind an empty room list",
            ),
            (
                "Err(_) => return Vec::new()",
                "DiscussService::list must not hide a poisoned room registry behind an empty room list",
            ),
        ):
            if token in fn_body:
                add(
                    "R86_DISCUSS_ROOM_REGISTRY_FAIL_CLOSED",
                    discuss_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "DiscussService room registry lock poisoned" not in fn_body:
            add(
                "R86_DISCUSS_ROOM_REGISTRY_FAIL_CLOSED",
                discuss_mod,
                line_number(text, offset),
                "DiscussService::list must preserve poisoned room registry as explicit unavailable state",
            )

if kernel_mod.exists():
    text = source(kernel_mod)
    body = rust_method_body(text, "list_discuss_rooms")
    if body is None:
        add(
            "R86_DISCUSS_ROOM_REGISTRY_FAIL_CLOSED",
            kernel_mod,
            1,
            "Kernel::list_discuss_rooms must preserve DiscussService::list failures",
        )
    else:
        offset, fn_body = body
        direct_call = "self.discuss.list()" in fn_body or "(*self.discuss).list()" in fn_body
        wraps_old_projection = (
            "Ok((*self.discuss).list())" in fn_body
            or "Ok(self.discuss.list())" in fn_body
        )
        if not direct_call or wraps_old_projection:
            add(
                "R86_DISCUSS_ROOM_REGISTRY_FAIL_CLOSED",
                kernel_mod,
                line_number(text, offset),
                "Kernel::list_discuss_rooms must return DiscussService::list directly and propagate registry failures",
            )


# Rule 87: loop cache is lifecycle state, not a best-effort status cache.
# A poisoned loop cache must surface as unavailable loop state; it must not
# become unknown-loop, empty-loop-list, empty resume work, or a zero-loop
# debug projection.
loop_mod = cli_root / "src/daemon/execution/loop_instance/mod.rs"
loop_ability = cli_root / "src/daemon/ability/builtins/automation/loop_ability.rs"
kernel_mod = cli_root / "src/daemon/boot/kernel/mod.rs"
if loop_mod.exists():
    text = source(loop_mod)
    body = rust_method_body(text, "status")
    if body is None:
        add(
            "R87_LOOP_CACHE_FAIL_CLOSED",
            loop_mod,
            1,
            "LoopService must keep status as the loop lookup authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Option<LoopInstance>>" not in signature:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                loop_mod,
                line_number(text, offset),
                "LoopService::status must return Result so poisoned cache state cannot become unknown-loop",
            )
        for token, detail in (
            (
                ".read().ok()",
                "LoopService::status must not erase poisoned cache errors with .ok()",
            ),
            (
                "and_then(|g| g.get(id)",
                "LoopService::status must not collapse unavailable cache state into None",
            ),
        ):
            if token in fn_body:
                add(
                    "R87_LOOP_CACHE_FAIL_CLOSED",
                    loop_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "LoopService cache lock poisoned" not in fn_body:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                loop_mod,
                line_number(text, offset),
                "LoopService::status must preserve poisoned cache as explicit unavailable state",
            )

    body = rust_method_body(text, "list")
    if body is None:
        add(
            "R87_LOOP_CACHE_FAIL_CLOSED",
            loop_mod,
            1,
            "LoopService must keep list as the loop cache snapshot authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<LoopInstance>>" not in signature:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                loop_mod,
                line_number(text, offset),
                "LoopService::list must return Result so poisoned cache state cannot become an empty loop list",
            )
        for token, detail in (
            (
                "Err(_) => Vec::new()",
                "LoopService::list must not hide a poisoned cache behind an empty loop list",
            ),
            (
                "Err(_) => return Vec::new()",
                "LoopService::list must not hide a poisoned cache behind an empty loop list",
            ),
        ):
            if token in fn_body:
                add(
                    "R87_LOOP_CACHE_FAIL_CLOSED",
                    loop_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "LoopService cache lock poisoned" not in fn_body:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                loop_mod,
                line_number(text, offset),
                "LoopService::list must preserve poisoned cache as explicit unavailable state",
            )

    body = rust_method_body(text, "resume_inflight")
    if body is None:
        add(
            "R87_LOOP_CACHE_FAIL_CLOSED",
            loop_mod,
            1,
            "LoopService::resume_inflight must preserve loop cache snapshot failures",
        )
    else:
        offset, fn_body = body
        if ".list()?" not in fn_body:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                loop_mod,
                line_number(text, offset),
                "LoopService::resume_inflight must propagate LoopService::list failures",
            )

    body = rust_method_body(text, "subscribe")
    if body is None:
        add(
            "R87_LOOP_CACHE_FAIL_CLOSED",
            loop_mod,
            1,
            "LoopService::subscribe must preserve loop cache lookup failures",
        )
    else:
        offset, fn_body = body
        if ".status(id)?" not in fn_body:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                loop_mod,
                line_number(text, offset),
                "LoopService::subscribe must propagate LoopService::status failures before deciding loop-not-found",
            )

    body = rust_method_body(text, "fmt")
    if body is not None:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        for token, detail in (
            (
                ".read().ok()",
                "LoopService Debug must not erase poisoned cache state with .ok()",
            ),
            (
                "unwrap_or(0)",
                "LoopService Debug must not project unavailable cache state as zero loops",
            ),
        ):
            if token in fn_body:
                add(
                    "R87_LOOP_CACHE_FAIL_CLOSED",
                    loop_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )

if loop_ability.exists():
    text = source(loop_ability)
    body = rust_method_body(text, "status_handler_with_envelope")
    if body is None:
        add(
            "R87_LOOP_CACHE_FAIL_CLOSED",
            loop_ability,
            1,
            "loop.status ability must preserve LoopService::status failures",
        )
    else:
        offset, fn_body = body
        if "svc.status_for_accountable_user(&LoopId::new(id), &owner)?" not in fn_body:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                loop_ability,
                line_number(text, offset),
                "loop.status ability must propagate owner-scoped LoopService errors before reporting not found",
            )

if kernel_mod.exists():
    text = source(kernel_mod)
    body = rust_method_body(text, "loop_status")
    if body is None:
        add(
            "R87_LOOP_CACHE_FAIL_CLOSED",
            kernel_mod,
            1,
            "Kernel::loop_status must preserve LoopService::status failures",
        )
    else:
        offset, fn_body = body
        if "self.loop_svc.status(id)" not in fn_body or "Ok(self.loop_svc.status(id))" in fn_body:
            add(
                "R87_LOOP_CACHE_FAIL_CLOSED",
                kernel_mod,
                line_number(text, offset),
                "Kernel::loop_status must return LoopService::status directly and propagate cache failures",
            )


# Rule 88: cross-agent chat ability discovery is route/context authority, not
# a best-effort prompt hint. Agent aggregate projection failures must stop chat
# context construction; they must not become an empty "other agents" ability
# list that makes products believe no peer ability exists.
agents_chat = cli_root / "src/daemon/ability/builtins/agents/chat.rs"
if agents_chat.exists():
    text = source(agents_chat)
    def chat_fn_body(name: str) -> tuple[int, str] | None:
        return rust_method_body(text, name) or brace_function_body(
            text, rf"(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+{re.escape(name)}\b"
        )

    if "enumerate_other_agent_specs" in text:
        body = chat_fn_body("enumerate_other_agent_specs")
        if body is None:
            add(
                "R88_CHAT_CROSS_AGENT_REGISTRY_FAIL_CLOSED",
                agents_chat,
                1,
                "agents.chat must keep enumerate_other_agent_specs as the cross-agent ability discovery authority",
            )
        else:
            offset, fn_body = body
            body_start = text.find("{", offset) + 1
            signature = text[offset:body_start]
            if "Result<Vec<" not in signature:
                add(
                    "R88_CHAT_CROSS_AGENT_REGISTRY_FAIL_CLOSED",
                    agents_chat,
                    line_number(text, offset),
                    "enumerate_other_agent_specs must return Result so registry projection failures cannot become empty peer ability lists",
                )
            for token, detail in (
                (
                    "Err(_) => return Vec::new()",
                    "enumerate_other_agent_specs must not hide Agent aggregate load failures as no cross-agent abilities",
                ),
                (
                    "Err(_) => Vec::new()",
                    "enumerate_other_agent_specs must not hide Agent aggregate load failures as no cross-agent abilities",
                ),
                (
                    "Err(_)",
                    "enumerate_other_agent_specs must not match-and-ignore Agent aggregate load failures",
                ),
            ):
                if token in fn_body:
                    add(
                        "R88_CHAT_CROSS_AGENT_REGISTRY_FAIL_CLOSED",
                        agents_chat,
                        line_number(text, body_start + fn_body.find(token)),
                        detail,
                    )
            if "load cross-agent ability registry projection" not in fn_body:
                add(
                    "R88_CHAT_CROSS_AGENT_REGISTRY_FAIL_CLOSED",
                    agents_chat,
                    line_number(text, offset),
                    "enumerate_other_agent_specs must classify Agent aggregate load failure as cross-agent ability registry projection failure",
                )

        required_call_tokens = {
            "invoke_direct_with_progress": "enumerate_other_agent_specs(agent_name)?",
            "stream_handler": "enumerate_other_agent_specs(agent_name)?",
        }
        for method_name, token in required_call_tokens.items():
            if method_name not in text:
                continue
            call_body = chat_fn_body(method_name)
            if call_body is None:
                add(
                    "R88_CHAT_CROSS_AGENT_REGISTRY_FAIL_CLOSED",
                    agents_chat,
                    1,
                    f"agents.chat {method_name} must preserve cross-agent discovery failures",
                )
                continue
            offset, fn_body = call_body
            if token not in fn_body:
                add(
                    "R88_CHAT_CROSS_AGENT_REGISTRY_FAIL_CLOSED",
                    agents_chat,
                    line_number(text, offset),
                    f"agents.chat {method_name} must propagate enumerate_other_agent_specs failures",
                )


# Rule 89: permission pending queue is admission/operator state, not a
# best-effort UI cache. A poisoned SubscriberBroker pending queue must fail
# consent list/subscribe and Kernel pending snapshots; it must not become an
# empty queue or null request row.
permission_mod = cli_root / "src/daemon/execution/permission/mod.rs"
consent_mod = cli_root / "src/daemon/ability/builtins/governance/consent.rs"
kernel_mod = cli_root / "src/daemon/boot/kernel/mod.rs"
if permission_mod.exists():
    text = source(permission_mod)
    body = rust_method_body(text, "pending_snapshot")
    if body is None:
        add(
            "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
            permission_mod,
            1,
            "SubscriberBroker must keep pending_snapshot as the pending queue snapshot authority",
        )
    else:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<PermissionRequest>>" not in signature:
            add(
                "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                permission_mod,
                line_number(text, offset),
                "SubscriberBroker::pending_snapshot must return Result so poisoned queue state cannot become an empty pending list",
            )
        for token, detail in (
            (
                ".read()\n            .ok()",
                "SubscriberBroker::pending_snapshot must not erase poisoned queue errors with .ok()",
            ),
            (
                ".read().ok()",
                "SubscriberBroker::pending_snapshot must not erase poisoned queue errors with .ok()",
            ),
            (
                "unwrap_or_default()",
                "SubscriberBroker::pending_snapshot must not project unavailable queue state as empty pending list",
            ),
        ):
            if token in fn_body:
                add(
                    "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                    permission_mod,
                    line_number(text, body_start + fn_body.find(token)),
                    detail,
                )
        if "SubscriberBroker pending queue lock poisoned" not in fn_body:
            add(
                "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                permission_mod,
                line_number(text, offset),
                "SubscriberBroker::pending_snapshot must preserve poisoned queue as explicit unavailable state",
            )

    body = rust_method_body(text, "pending")
    if body is not None:
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        signature = text[offset:body_start]
        if "Result<Vec<PermissionRequest>>" not in signature:
            add(
                "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                permission_mod,
                line_number(text, offset),
                "PermissionService::pending must return Result so SubscriberBroker failures propagate",
            )
        if "s.pending_snapshot()" not in fn_body or "unwrap_or_default()" in fn_body:
            add(
                "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                permission_mod,
                line_number(text, offset),
                "PermissionService::pending must propagate SubscriberBroker::pending_snapshot and only return empty when no SubscriberBroker exists",
            )

if consent_mod.exists():
    text = source(consent_mod)
    for method_name, detail in (
        (
            "subscribe_handler",
            "consent.subscribe must propagate PermissionService::pending failures before creating a stream snapshot",
        ),
        (
            "list_pending_handler",
            "consent.list_pending must propagate PermissionService::pending failures",
        ),
    ):
        body = rust_method_body(text, method_name)
        if body is None:
            continue
        offset, fn_body = body
        body_start = text.find("{", offset) + 1
        if ".pending()?" not in fn_body:
            add(
                "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                consent_mod,
                line_number(text, offset),
                detail,
            )
        if "unwrap_or(Value::Null)" in fn_body:
            add(
                "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                consent_mod,
                line_number(text, body_start + fn_body.find("unwrap_or(Value::Null)")),
                "consent pending snapshots must not serialize failed PermissionRequest rows as null",
            )

if kernel_mod.exists():
    text = source(kernel_mod)
    body = rust_method_body(text, "pending_permission_requests")
    if body is not None:
        offset, fn_body = body
        if "self.permission.pending()" not in fn_body or "Ok(self.permission.pending())" in fn_body:
            add(
                "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED",
                kernel_mod,
                line_number(text, offset),
                "Kernel::pending_permission_requests must return PermissionService::pending directly and propagate pending queue failures",
            )

federation_directory = cli_root / "src/daemon/federation/directory.rs"
federation_wrappers = cli_root / "src/daemon/invocation/dispatch/federation_wrappers.rs"
stream_dispatcher = cli_root / "src/daemon/invocation/streams/stream_dispatcher.rs"
if federation_directory.exists():
    text = source(federation_directory)
    raw_text = federation_directory.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "fn agent_ura_to_node_id",
            "federation directory must not retain raw-URA node_id fallback helper",
        ),
        (
            "unwrap_or_else(|| agent_ura.to_string())",
            "federation directory must not use raw URA as fallback node_id",
        ),
        (
            "node_id = agent_ura.clone()",
            "federation directory comments must not preserve raw-URA node_id fallback semantics",
        ),
        (
            "presence_ura_to_directory_entry_falls_back_when_ura_non_canonical",
            "federation directory tests must reject malformed URAs instead of protecting fallback projection",
        ),
        (
            "presence_ura_to_directory_entry_treats_legacy_agent_shape_as_non_canonical",
            "federation directory tests must reject agent-shaped URAs instead of protecting legacy projection",
        ),
    ):
        haystack = raw_text if "tests must" in detail else text
        if token in haystack:
            match = re.search(re.escape(token), haystack)
            add(
                "R90_FEDERATION_DIRECTORY_DEVICE_PROJECTION",
                federation_directory,
                line_number(haystack, match.start() if match else 0),
                detail,
            )
    for token, detail in (
        (
            "fn canonical_device_node_id",
            "federation directory must centralize Device URA validation before projection",
        ),
        (
            "parsed.kind != crate::core::ura::URAKind::Device",
            "federation directory must reject non-Device URAs",
        ),
        (
            "crate::core::ura::device_ura(&parsed.realm, node_id)",
            "federation directory must rebuild and compare canonical Device URA",
        ),
        (
            "apply_snapshot_rejects_invalid_agent_ura_without_mutating_view",
            "federation directory must test atomic rejection for invalid snapshots",
        ),
        (
            "presence_event_rejects_non_device_ura",
            "federation directory must test live event rejection for non-Device URAs",
        ),
    ):
        haystack = raw_text if "test" in detail else text
        if token not in haystack:
            add(
                "R90_FEDERATION_DIRECTORY_DEVICE_PROJECTION",
                federation_directory,
                1,
                detail,
            )
    apply_frame = rust_method_body(text, "apply_frame")
    if apply_frame is None:
        add(
            "R90_FEDERATION_DIRECTORY_DEVICE_PROJECTION",
            federation_directory,
            1,
            "DirectoryView::apply_frame must remain the fail-closed remote directory mutation boundary",
        )
    else:
        offset, body = apply_frame
        signature = text[offset : text.find("{", offset) + 1]
        if "Result<(), String>" not in signature:
            add(
                "R90_FEDERATION_DIRECTORY_DEVICE_PROJECTION",
                federation_directory,
                line_number(text, offset),
                "DirectoryView::apply_frame must return Result so invalid frames cannot mutate state",
            )
        for token, detail in (
            (
                "let mut next_entries = BTreeMap::new();",
                "DirectoryView::apply_frame must stage snapshots before committing",
            ),
            (
                "self.entries = next_entries;",
                "DirectoryView::apply_frame must commit snapshots atomically after validation",
            ),
            (
                "directory_agent_summary_to_entry(raw, &self.peer_realm)?",
                "DirectoryView::apply_frame must validate every snapshot entry",
            ),
            (
                "canonical_device_node_id(agent_ura, \"directory revoke event\")?",
                "DirectoryView::apply_frame must validate revoke URAs",
            ),
        ):
            if token not in body:
                add(
                    "R90_FEDERATION_DIRECTORY_DEVICE_PROJECTION",
                    federation_directory,
                    line_number(text, offset),
                    detail,
                )

if federation_wrappers.exists():
    raw_text = federation_wrappers.read_text(encoding="utf-8", errors="replace")
    if "build_subscribe_directory_v2_snapshot_rejects_non_device_presence_row" not in raw_text:
        add(
            "R90_FEDERATION_DIRECTORY_DEVICE_PROJECTION",
            federation_wrappers,
            1,
            "subscribe_directory_v2 snapshot builder must reject non-Device presence rows",
        )
if stream_dispatcher.exists():
    text = source(stream_dispatcher)
    for token, detail in (
        (
            "invalid_presence_event",
            "subscribe_directory_v2 stream must surface invalid live presence events",
        ),
        (
            "invalid_presence_snapshot",
            "subscribe_directory_v2 stream must surface invalid lag-recovery snapshots",
        ),
    ):
        if token not in text:
            add(
                "R90_FEDERATION_DIRECTORY_DEVICE_PROJECTION",
                stream_dispatcher,
                1,
                detail,
            )

invocation_wire = cli_root / "src/daemon/invocation/dispatch/invocation_wire.rs"
invocation_wire_callers = [
    cli_root / "src/daemon/invocation/dispatch/unary_dispatcher.rs",
    cli_root / "src/daemon/invocation/streams/stream_dispatcher.rs",
    cli_root / "src/daemon/invocation/bidi/bidi_dispatcher.rs",
    cli_root / "src/daemon/invocation/dispatch/local_session_dispatcher.rs",
]
if invocation_wire.exists():
    text = source(invocation_wire)
    raw_text = invocation_wire.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "fn target_ura_from_envelope",
            "invocation wire must not retain caller-fallback target helper",
        ),
        (
            "caller as fallback",
            "invocation wire must not document caller as a target fallback",
        ),
        (
            "callee or caller URA",
            "invocation wire must not accept caller as a substitute route target",
        ),
        (
            ".or(envelope.caller.as_ref())",
            "invocation wire target extraction must not fall back to caller",
        ),
        (
            "or(envelope.caller",
            "invocation wire target extraction must not fall back to caller",
        ),
    ):
        if token in text:
            match = re.search(re.escape(token), text)
            add(
                "R91_INVOCATION_WIRE_CALLEE_TARGET",
                invocation_wire,
                line_number(text, match.start() if match else 0),
                detail,
            )
    helper = rust_method_body(text, "callee_ura_from_envelope")
    if helper is None:
        add(
            "R91_INVOCATION_WIRE_CALLEE_TARGET",
            invocation_wire,
            1,
            "invocation wire must centralize callee-only target extraction",
        )
    else:
        offset, body = helper
        for present, detail in (
            (
                "callee" in body and ".as_ref()" in body,
                "callee-only target extraction must read the callee tuple field",
            ),
            (
                "must carry callee URA" in body,
                "callee-only target extraction must reject missing callee explicitly",
            ),
            (
                "crate::core::ura::parse_ura(callee_ura)" in body,
                "callee-only target extraction must validate callee URA grammar",
            ),
        ):
            if not present:
                add(
                    "R91_INVOCATION_WIRE_CALLEE_TARGET",
                    invocation_wire,
                    line_number(text, offset),
                    detail,
                )
        if "envelope.caller" in body:
            add(
                "R91_INVOCATION_WIRE_CALLEE_TARGET",
                invocation_wire,
                line_number(text, offset),
                "callee-only target extraction must not read caller",
            )
    for token, detail in (
        (
            "callee_ura_from_envelope_extracts_explicit_callee",
            "missing callee target positive regression test",
        ),
        (
            "callee_ura_from_envelope_rejects_caller_only_tuple",
            "missing caller-only tuple rejection regression test",
        ),
    ):
        if token not in raw_text:
            add(
                "R91_INVOCATION_WIRE_CALLEE_TARGET",
                invocation_wire,
                1,
                detail,
            )

for caller in invocation_wire_callers:
    if caller.exists():
        text = source(caller)
        if "target_ura_from_envelope" in text:
            match = re.search(r"target_ura_from_envelope", text)
            add(
                "R91_INVOCATION_WIRE_CALLEE_TARGET",
                caller,
                line_number(text, match.start() if match else 0),
                "dispatch callers must not use retired caller-fallback target helper",
            )
        if "callee_ura_from_envelope" not in text:
            add(
                "R91_INVOCATION_WIRE_CALLEE_TARGET",
                caller,
                1,
                "dispatch callers must route through callee-only target extraction",
            )


local_daemon_grpc = cli_root / "src/support/platform/local_daemon_grpc.rs"
if local_daemon_grpc.exists():
    text = source(local_daemon_grpc)
    for token, detail in (
        (
            "LocalDaemonSelf",
            "local daemon system subject must not be derived from callee",
        ),
        (
            "local_daemon_self",
            "local daemon system must not expose a self-subject constructor",
        ),
        (
            "local_daemon_default_callee_ura",
            "local daemon identity helper must not be named as a callee-only fallback",
        ),
        (
            "fn local_root(",
            "local daemon system must not expose a subjectless local_root constructor",
        ),
        (
            "LocalDaemonSystemTuplePlan::local_root(",
            "local daemon system callers must bind an explicit subject",
        ),
        (
            "fn invoke_local_daemon_ability(",
            "subjectless generic local daemon helper is retired",
        ),
        (
            "pub(crate) fn invoke_local_daemon_ability(",
            "subjectless generic local daemon helper is retired",
        ),
    ):
        if token in text:
            match = re.search(re.escape(token), text)
            add(
                "R92_LOCAL_DAEMON_SYSTEM_EXPLICIT_SUBJECT",
                local_daemon_grpc,
                line_number(text, match.start() if match else 0),
                detail,
            )

    subject_policy = re.search(
        r"impl LocalDaemonSystemSubjectPolicy\s*\{(?P<body>.*?)\n\}",
        text,
        re.S,
    )
    if subject_policy is None:
        add(
            "R92_LOCAL_DAEMON_SYSTEM_EXPLICIT_SUBJECT",
            local_daemon_grpc,
            1,
            "local daemon system subject policy implementation is missing",
        )
    else:
        body = subject_policy.group("body")
        offset = subject_policy.start("body")
        if "fn resolve(&self) -> anyhow::Result<String>" not in body:
            add(
                "R92_LOCAL_DAEMON_SYSTEM_EXPLICIT_SUBJECT",
                local_daemon_grpc,
                line_number(text, offset),
                "local daemon system subject resolution must not depend on callee",
            )
        if "callee_ura" in body:
            add(
                "R92_LOCAL_DAEMON_SYSTEM_EXPLICIT_SUBJECT",
                local_daemon_grpc,
                line_number(text, offset),
                "local daemon system subject policy must not read callee_ura",
            )

    raw_text = local_daemon_grpc.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "local_system_invoke_request_does_not_pre_resolve_descriptor_ref",
            "missing local daemon system descriptor projection regression test",
        ),
        (
            "local_system_tuple_plan_requires_explicit_targeted_subject",
            "missing local daemon system explicit-subject regression test",
        ),
    ):
        if token not in raw_text:
            add(
                "R92_LOCAL_DAEMON_SYSTEM_EXPLICIT_SUBJECT",
                local_daemon_grpc,
                1,
                detail,
            )


local_target_subject_sources = [
    cli_root / "src/daemon/invocation/routing/target.rs",
    cli_root / "src/support/platform/local_invoke.rs",
    cli_root / "src/daemon/ability/builtins/integrations/mcp/bridge.rs",
    cli_root / "src/daemon/ability/builtins/integrations/a2a/bridge.rs",
    cli_root / "src/daemon/ability/catalog/profiles/mcp.rs",
]
existing_local_target_subject_sources = [
    path for path in local_target_subject_sources if path.exists()
]
if len(existing_local_target_subject_sources) != len(local_target_subject_sources):
    for path in local_target_subject_sources:
        if not path.exists():
            add(
                "R93_LOCAL_ABILITY_TARGET_SUBJECT_POLICY",
                path,
                1,
                "local ability target subject policy source is missing",
            )
else:
    source_texts = [source(path) for path in local_target_subject_sources]
    raw_source_texts = [
        path.read_text(encoding="utf-8", errors="replace")
        for path in local_target_subject_sources
    ]
    production_text = "\n".join(
        text.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]
        for text in source_texts
    )
    if "default_subject_ura" in production_text:
        match = re.search(r"default_subject_ura", production_text)
        add(
            "R93_LOCAL_ABILITY_TARGET_SUBJECT_POLICY",
            local_target_subject_sources[0],
            line_number(production_text, match.start() if match else 0),
            "LocalAbilityTarget must not expose product-visible subject fallback policy",
        )
    target_text, local_invoke_text, mcp_bridge_text, a2a_bridge_text, mcp_profile_text = source_texts
    for token, detail in (
        (
            "fn daemon_system_subject_ura_for_descriptor(",
            "descriptor-derived daemon-system subject policy must remain centralized",
        ),
        (
            "pub(crate) fn daemon_system_subject_ura(&self) -> anyhow::Result<String>",
            "LocalAbilityTarget subject policy must remain crate-private",
        ),
        (
            "pub fn local_root_for_target(",
            "SystemInvocationTargetIssuer must issue target-bound daemon-system invocations",
        ),
        (
            "pub struct LocalTargetRootInvocation",
            "target-bound daemon-system invocation facts must have an issued value object",
        ),
        (
            "pub fn local_target_root(",
            "SystemInvocationTargetIssuer must issue local target root invocation facts",
        ),
    ):
        if token not in target_text:
            add(
                "R93_LOCAL_ABILITY_TARGET_SUBJECT_POLICY",
                local_target_subject_sources[0],
                1,
                detail,
            )
    for token, detail in (
        (
            "invoke_issued_target_root_timeout",
            "local daemon system invoker must consume issued target-root facts",
        ),
        (
            "root_context_for_target",
            "local system invocation context must expose a target-bound subject helper",
        ),
    ):
        if token not in local_invoke_text:
            add(
                "R93_LOCAL_ABILITY_TARGET_SUBJECT_POLICY",
                local_target_subject_sources[1],
                1,
                detail,
            )
    if "invoke_target_root_derived_subject_timeout" in local_invoke_text:
        add(
            "R93_LOCAL_ABILITY_TARGET_SUBJECT_POLICY",
            local_target_subject_sources[1],
            1,
            "local invoke must not derive target subjects outside SystemInvocationTargetIssuer",
        )
    raw_local_invoke_text = raw_source_texts[1]
    for token, detail in (
        (
            "local_system_context_for_agent_target_uses_agent_owner_subject",
            "missing agent-owner subject regression test",
        ),
        (
            "local_system_context_for_realm_authority_target_uses_ability_subject",
            "missing realm Authority subject regression test",
        ),
    ):
        if token not in raw_local_invoke_text:
            add(
                "R93_LOCAL_ABILITY_TARGET_SUBJECT_POLICY",
                local_target_subject_sources[1],
                1,
                detail,
            )
    for path, text, detail in (
        (
            local_target_subject_sources[2],
            mcp_bridge_text,
            "MCP bridge must use runtime-issued target subject policy",
        ),
        (
            local_target_subject_sources[3],
            a2a_bridge_text,
            "A2A bridge must use runtime-issued target subject policy",
        ),
        (
            local_target_subject_sources[4],
            mcp_profile_text,
            "MCP daemon profile must use runtime-issued target context policy",
        ),
    ):
        if "local_root_for_target" not in text and "root_context_for_target" not in text:
            add(
                "R93_LOCAL_ABILITY_TARGET_SUBJECT_POLICY",
                path,
                1,
                detail,
            )


bridge_lib_resolver = cli_root / "src/cli/daemon_client/bridge_lib.rs"
if bridge_lib_resolver.exists():
    bridge_text = source(bridge_lib_resolver)
    raw_bridge_text = bridge_lib_resolver.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "fn bridge_lib_from_claude_settings() -> anyhow::Result<Option<String>>",
            "Claude bridge config source must distinguish absence from corruption",
        ),
        (
            "fn bridge_lib_from_codex_config() -> anyhow::Result<Option<String>>",
            "Codex bridge config source must distinguish absence from corruption",
        ),
        (
            "fn read_optional_text(",
            "bridge config file loading must centralize optional-file state handling",
        ),
    ):
        if token not in bridge_text:
            add(
                "R94_BRIDGE_CONFIG_FAIL_CLOSED",
                bridge_lib_resolver,
                1,
                detail,
            )
    for token, detail in (
        (
            "fs::read_to_string(path).ok()?",
            "bridge config file read errors must not be erased as absence",
        ),
        (
            "serde_json::from_str(&data).ok()?",
            "Claude bridge config parse errors must not be erased as absence",
        ),
        (
            "data.parse::<DocumentMut>().ok()?",
            "Codex bridge config parse errors must not be erased as absence",
        ),
    ):
        offset = raw_bridge_text.find(token)
        if offset != -1:
            add(
                "R94_BRIDGE_CONFIG_FAIL_CLOSED",
                bridge_lib_resolver,
                line_number(raw_bridge_text, offset),
                detail,
            )


agent_mcp_authoring_sources = (
    cli_root / "src/cli/commands/agent/mcp.rs",
    cli_root / "src/daemon/ability/builtins/agents/authoring.rs",
)
if all(path.exists() for path in agent_mcp_authoring_sources):
    cli_mcp_text = source(agent_mcp_authoring_sources[0])
    daemon_authoring_text = source(agent_mcp_authoring_sources[1])
    raw_sources = [
        path.read_text(encoding="utf-8", errors="replace")
        for path in agent_mcp_authoring_sources
    ]
    for path, text, token, detail in (
        (
            agent_mcp_authoring_sources[0],
            cli_mcp_text,
            "pub(super) fn existing_mcp_binding(body: &str) -> anyhow::Result<Option<(String, String)>>",
            "CLI MCP binding projection must distinguish non-MCP manifests from corrupt manifests",
        ),
        (
            agent_mcp_authoring_sources[0],
            cli_mcp_text,
            "fn existing_mcp_binding_from_path(",
            "CLI MCP dry-run must read existing manifests through a path-qualified fail-closed adapter",
        ),
        (
            agent_mcp_authoring_sources[1],
            daemon_authoring_text,
            "fn existing_binding_matches(\n    prior: &[u8],\n    proposed: &AbilityManifest,\n    path: &std::path::Path,\n) -> anyhow::Result<bool>",
            "daemon authoring retain-same-binding must fail closed on corrupt existing manifests",
        ),
    ):
        if token not in text:
            add(
                "R95_AGENT_MCP_BINDING_FAIL_CLOSED",
                path,
                1,
                detail,
            )
    for path, raw_text in zip(agent_mcp_authoring_sources, raw_sources):
        for token, detail in (
            (
                "AbilityManifest::from_toml_str(body).ok()",
                "ability manifest parse errors must not be erased while comparing existing bindings",
            ),
            (
                "existing_mcp_binding(body: &str) -> Option",
                "CLI MCP binding projection must not collapse parse errors into None",
            ),
            (
                "std::fs::read_to_string(&path).ok()",
                "CLI MCP dry-run must not erase existing manifest read errors",
            ),
        ):
            offset = raw_text.find(token)
            if offset != -1:
                add(
                    "R95_AGENT_MCP_BINDING_FAIL_CLOSED",
                    path,
                    line_number(raw_text, offset),
                    detail,
                )


agent_session_inventory_sources = (
    cli_root / "src/daemon/persistence/chat_sessions.rs",
    cli_root / "src/cli/commands/agent/history.rs",
)
if all(path.exists() for path in agent_session_inventory_sources):
    chat_sessions_text = source(agent_session_inventory_sources[0])
    agent_history_text = source(agent_session_inventory_sources[1])
    raw_history_text = agent_session_inventory_sources[1].read_text(
        encoding="utf-8", errors="replace"
    )
    for path, text, token, detail in (
        (
            agent_session_inventory_sources[0],
            chat_sessions_text,
            "pub struct SessionInventory",
            "chat session list views must consume a validated inventory projection",
        ),
        (
            agent_session_inventory_sources[0],
            chat_sessions_text,
            "pub fn load_session_inventory(agent: &str) -> anyhow::Result<SessionInventory>",
            "chat session inventory loader must validate latest marker state",
        ),
        (
            agent_session_inventory_sources[1],
            agent_history_text,
            "chat_sessions::load_session_inventory(agent)?",
            "agent chat-history list must render from one validated index snapshot",
        ),
    ):
        if token not in text:
            add(
                "R96_AGENT_SESSION_INVENTORY_FAIL_CLOSED",
                path,
                1,
                detail,
            )
    for token, detail in (
        (
            "latest_session(agent)?.unwrap_or_default()",
            "agent chat-history list must not silently erase missing latest marker state",
        ),
        (
            "let latest = chat_sessions::latest_session(agent)?",
            "agent chat-history list must not load latest pointer separately from sessions",
        ),
    ):
        offset = raw_history_text.find(token)
        if offset != -1:
            add(
                "R96_AGENT_SESSION_INVENTORY_FAIL_CLOSED",
                agent_session_inventory_sources[1],
                line_number(raw_history_text, offset),
                detail,
            )


media_resource_bootstrap = (
    cli_root / "src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"
)
if media_resource_bootstrap.exists():
    media_bootstrap_text = source(media_resource_bootstrap)
    raw_media_bootstrap_text = media_resource_bootstrap.read_text(
        encoding="utf-8", errors="replace"
    )
    for token, detail in (
        (
            "fn display_hardware_id_from_monitor_id(",
            "media display bootstrap must route display identity through a stable monitor-id projector",
        ),
        (
            "display_without_stable_identity_skipped",
            "media display bootstrap must make missing stable monitor identity an explicit omitted state",
        ),
        (
            "display_hardware_identity_does_not_synthesize_index_size_name_fallback",
            "media display bootstrap must pin the no synthetic display hardware-id fallback contract",
        ),
    ):
        if token not in raw_media_bootstrap_text:
            add(
                "R97_MEDIA_DISPLAY_STABLE_IDENTITY",
                media_resource_bootstrap,
                1,
                detail,
            )
    for token, detail in (
        (
            "display:xcap:{idx}",
            "display hardware_id must not include enumeration index fallback",
        ),
        (
            "let fallback_id = format!",
            "display bootstrap must not construct a fallback hardware_id",
        ),
        (
            "unwrap_or(fallback_id)",
            "display hardware_id must not fall back from missing monitor id",
        ),
    ):
        offset = raw_media_bootstrap_text.find(token)
        if offset != -1:
            add(
                "R97_MEDIA_DISPLAY_STABLE_IDENTITY",
                media_resource_bootstrap,
                line_number(raw_media_bootstrap_text, offset),
                detail,
            )


access_control_governance = (
    cli_root / "src/daemon/ability/builtins/governance/access_control.rs"
)
if access_control_governance.exists():
    text = source(access_control_governance)
    raw_text = access_control_governance.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "owner_source: OwnerSource",
            "access-control check must make owner-source provenance a required request fact",
        ),
        (
            '"owner_ura", "owner_source", "caller_ura"',
            "authority_binding.check schema must require owner_source before caller_ura",
        ),
        (
            "authority_binding_check_requires_explicit_owner_source",
            "access-control tests must prove omitted owner_source is rejected",
        ),
    ):
        if token not in raw_text:
            add("R98_ACCESS_CONTROL_EXPLICIT_OWNER_SOURCE", access_control_governance, 1, detail)
    for token, detail in (
        (
            "request.owner_source.unwrap_or(OwnerSource::Subject)",
            "access-control check must not preserve the legacy Subject owner_source default",
        ),
        (
            "request.owner_source.unwrap_or_default()",
            "access-control check must not default missing owner_source",
        ),
    ):
        offset = raw_text.find(token)
        if offset != -1:
            add(
                "R98_ACCESS_CONTROL_EXPLICIT_OWNER_SOURCE",
                access_control_governance,
                line_number(raw_text, offset),
                detail,
            )

go_access_control = cli_root / "sdk/go/access_control.go"
if go_access_control.exists():
    text = go_access_control.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            'invalidAccessControl("owner_source is required"',
            "Go SDK access-control check must reject missing owner_source before provider I/O",
        ),
        (
            '"owner_source":                  ownerSource',
            "Go SDK access-control check must always project explicit owner_source",
        ),
    ):
        if token not in text:
            add("R98_ACCESS_CONTROL_EXPLICIT_OWNER_SOURCE", go_access_control, 1, detail)
    token = 'optionalStringArg(args, "owner_source", request.OwnerSource)'
    offset = text.find(token)
    if offset != -1:
        add(
            "R98_ACCESS_CONTROL_EXPLICIT_OWNER_SOURCE",
            go_access_control,
            line_number(text, offset),
            "Go SDK access-control check must not keep owner_source optional",
        )

python_access_control = cli_root / "sdk/python/easynet_sdk/access_control.py"
if python_access_control.exists():
    text = python_access_control.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            '_required_text(request.owner_source, "owner_source")',
            "Python SDK access-control check must reject missing owner_source before provider I/O",
        ),
        (
            '"owner_source": owner_source',
            "Python SDK access-control check must always project explicit owner_source",
        ),
    ):
        if token not in text:
            add("R98_ACCESS_CONTROL_EXPLICIT_OWNER_SOURCE", python_access_control, 1, detail)
    token = '_optional(args, "owner_source", request.owner_source)'
    offset = text.find(token)
    if offset != -1:
        add(
            "R98_ACCESS_CONTROL_EXPLICIT_OWNER_SOURCE",
            python_access_control,
            line_number(text, offset),
            "Python SDK access-control check must not keep owner_source optional",
        )


mcp_profile = cli_root / "src/daemon/ability/catalog/profiles/mcp.rs"
ability_manifest = cli_root / "src/daemon/ability/manifest.rs"
if mcp_profile.exists():
    text = mcp_profile.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "enum CostMetadataProjection",
            "MCP cost projection must stay centralized in CostMetadataProjection",
        ),
        (
            'Self::Undeclared => "unknown"',
            "undeclared MCP cost must project as unknown",
        ),
        (
            'Self::Undeclared => "cost not declared"',
            "undeclared MCP cost label must remain explicit",
        ),
        (
            "mcp_cost_projection_marks_agent_owned_undeclared_rows_unknown",
            "MCP tests must prove agent-owned undeclared cost is not inferred",
        ),
    ):
        if token not in text:
            add("R99_MCP_COST_NO_SOURCE_HEURISTIC", mcp_profile, 1, detail)
    for token, detail in (
        (
            "UndeclaredKnownLlm",
            "MCP cost projection must not infer LLM billing from agent source",
        ),
        (
            'source.starts_with("agent:")',
            "MCP cost projection must not branch on source metadata for billing",
        ),
        (
            "known to be an LLM dispatch path",
            "MCP cost comments must not preserve source-based billing inference",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R99_MCP_COST_NO_SOURCE_HEURISTIC",
                mcp_profile,
                line_number(text, offset),
                detail,
            )

if ability_manifest.exists():
    text = ability_manifest.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "falls back to an exec-kind heuristic",
            "ability manifest cost docs must not describe retired exec-kind cost inference",
        ),
        (
            "profiles::mcp::inferred_cost_kind",
            "ability manifest cost docs must not reference retired inferred cost helper",
        ),
        (
            "exec-kind inference",
            "ability manifest cost docs must not instruct consumers to infer cost from exec kind",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R99_MCP_COST_NO_SOURCE_HEURISTIC",
                ability_manifest,
                line_number(text, offset),
                detail,
            )


policy_gate = cli_root / "src/daemon/invocation/admission/policy_gate.rs"
policy_gate_tests = cli_root / "src/daemon/invocation/admission/policy_gate_tests.rs"
admission_facade = cli_root / "src/daemon/invocation/admission/admission_facade.rs"
bootstrap_authority = cli_root / "src/daemon/invocation/admission/bootstrap_authority.rs"
if policy_gate.exists():
    text = policy_gate.read_text(encoding="utf-8", errors="replace")
    test_text = text
    if policy_gate_tests.exists():
        test_text += "\n" + policy_gate_tests.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "local_device_owner_fact(",
            "ordinary policy gate must not project device ownership from local credentials",
        ),
        (
            "owner_fact_from_local_device(",
            "ordinary policy gate must not keep a local-device owner fallback helper",
        ),
        (
            "LOCAL_OWNER_CREDENTIALS_UNAVAILABLE",
            "ordinary policy gate must not expose local credential owner fallback errors",
        ),
        (
            "LOCAL_DEVICE_PRINCIPAL_OWNER_UNAVAILABLE",
            "ordinary policy principal projection must not read local credentials",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R100_POLICY_GATE_NO_LOCAL_CREDENTIAL_OWNER_FALLBACK",
                policy_gate,
                line_number(text, offset),
                detail,
            )
    for token, detail in (
        (
            "paired_device_subject_does_not_project_credentials_owner",
            "policy tests must prove paired credentials do not define ordinary subject ownership",
        ),
        (
            "paired_device_ability_does_not_project_credentials_owner",
            "policy tests must prove paired credentials do not define ordinary device-ability ownership",
        ),
        (
            "device_principal_projection_ignores_malformed_local_credentials",
            "policy tests must prove malformed local credentials are outside ordinary principal projection",
        ),
    ):
        if token not in test_text:
            add(
                "R100_POLICY_GATE_NO_LOCAL_CREDENTIAL_OWNER_FALLBACK",
                policy_gate,
                1,
                detail,
            )

    for token, detail in (
        (
            "pub(crate) struct AdmissionPolicyContext",
            "policy context must remain crate-private because it carries TrustedCallerPath directly",
        ),
        (
            "trusted_path: TrustedCallerPath",
            "policy context must receive TrustedCallerPath directly, not persisted TrustAnchorRole",
        ),
        (
            "context.trusted_path",
            "policy gate must consume trusted_path from AdmissionPolicyContext",
        ),
        (
            "principal_for(",
            "policy gate must derive logical principal facts from TrustedCallerPath inside policy, not from persisted TrustAnchorRole",
        ),
        (
            "TrustedCallerPath::DeviceCustody(path_purpose)",
            "Device policy rules must destructure the exact verified DeviceCustody purpose, not a broad Device label",
        ),
    ):
        if token not in text:
            add(
                "R114_TRUST_PATH_PRINCIPAL_SPLIT",
                policy_gate,
                1,
                detail,
            )
    for token, detail in (
        (
            "pub trusted_role: TrustAnchorRole",
            "policy context must not expose TrustAnchorRole as a generic trusted_role principal input",
        ),
        (
            "pub persisted_trust_anchor_role: TrustAnchorRole",
            "policy context must not receive persisted TrustAnchorRole when TrustedCallerPath is already known",
        ),
        (
            "VerifiedCallerProjection::from_trust_anchor_role",
            "policy gate must not retain a caller-projection constructor that round-trips through persisted TrustAnchorRole",
        ),
        (
            "from_persisted_trust_anchor_role",
            "policy gate caller projection must not keep a persisted-role compatibility constructor",
        ),
        (
            "fn persisted_trust_anchor_role",
            "TrustedCallerPath must not expose a generic role round-trip after runtime policy receives TrustedCallerPath directly",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R114_TRUST_PATH_PRINCIPAL_SPLIT",
                policy_gate,
                line_number(text, offset),
                detail,
            )
    if admission_facade.exists():
        facade_text = source(admission_facade)
        for token, detail in (
            (
            "TrustedCallerPath::from_role_and_caller",
            "AdmissionFacade must not lower TrustAnchorRole directly inside runtime policy classification",
        ),
            (
            "TrustedCallerPath::from_local_hosted_agent_custody",
            "AdmissionFacade must not classify local hosted-Agent key evidence through a compatibility helper",
        ),
        (
            "TrustedCallerPath::from_federated_caller",
            "AdmissionFacade must not classify federated evidence through an ability-agnostic helper",
        ),
        ):
            offset = facade_text.find(token)
            if offset != -1:
                add(
                    "R114_TRUST_PATH_PRINCIPAL_SPLIT",
                    admission_facade,
                    line_number(facade_text, offset),
                    detail,
                )
    for token, detail in (
        (
            "verified_caller_projection_separates_custody_path_from_principal_kind",
            "policy tests must prove device-mediated Agent custody remains PrincipalKind::Agent",
        ),
        (
            "agent_device_custody_never_matches_device_publication_custody_scope",
            "policy tests must prove AgentDeviceCustody cannot impersonate Device publication custody",
        ),
    ):
        if token not in test_text:
            add(
                "R114_TRUST_PATH_PRINCIPAL_SPLIT",
                policy_gate,
                1,
                detail,
            )

trust_anchor_rs = cli_root / "src/daemon/trust/anchor.rs"
for path in production_files(cli_root / "src", {".rs"}):
    text = source(path)
    offset = text.find("TrustedAgentRole")
    if offset != -1:
        add(
            "R129_TRUST_ANCHOR_ROLE_ONTOLOGY",
            path,
            line_number(text, offset),
            "production code must name persisted trust roles as TrustAnchorRole, not TrustedAgentRole",
        )
if trust_anchor_rs.exists():
    text = source(trust_anchor_rs)
    raw_text = trust_anchor_rs.read_text(encoding="utf-8", errors="replace")
    if "pub enum TrustAnchorRole" not in text:
        add(
            "R129_TRUST_ANCHOR_ROLE_ONTOLOGY",
            trust_anchor_rs,
            1,
            "trust anchor role enum must be named as persisted trust-anchor evidence",
        )
    for token, detail in (
        ("this enum is not an Agent ontology", "TrustAnchorRole documentation must state that the role is not an Agent ontology"),
        ("TrustedCallerPath", "TrustAnchorRole documentation must name the lowering boundary into TrustedCallerPath"),
        (
            "storage/wire compatibility name",
            "TrustedAgent.agent_ura documentation must confine agent_ura to storage/wire compatibility",
        ),
        (
            "runtime principal/caller URA",
            "trust-anchor runtime docs must name trusted values as runtime principal/caller URAs",
        ),
    ):
        if token not in raw_text:
            add(
                "R129_TRUST_ANCHOR_ROLE_ONTOLOGY",
                trust_anchor_rs,
                1,
                detail,
            )
    for token, detail in (
        (
            "fn canonical_ura_for_role(\n    agent_ura: &str",
            "trust-anchor canonical role validator must not name generic runtime principals as agent_ura",
        ),
        (
            "fn canonical_ura_for_role(\n    principal_ura: &str",
            "trust-anchor canonical role validator must name runtime_principal_ura explicitly, not a broad principal_ura alias",
        ),
        (
            "fn canonical_ura_for_runtime_principal(agent_ura: &str)",
            "runtime principal canonicalizer must name its input principal_ura, not agent_ura",
        ),
        (
            "pub fn lookup(&self, agent_ura: &str)",
            "trust-anchor lookup API must name singleton lookup input as principal/caller URA, not agent_ura",
        ),
        (
            "Role the agent plays in the realm",
            "TrustAnchorRole field docs must not describe every trusted principal as an Agent",
        ),
    ):
        offset = raw_text.find(token)
        if offset != -1:
            add(
                "R129_TRUST_ANCHOR_ROLE_ONTOLOGY",
                trust_anchor_rs,
                line_number(raw_text, offset),
                detail,
            )

    key_resolver_rs = cli_root / "src/daemon/trust/key_resolver.rs"
    if key_resolver_rs.exists():
        text = key_resolver_rs.read_text(encoding="utf-8", errors="replace")
        for token, detail in (
            (
                "fn resolve(&self, caller_ura: &str)",
                "TrustAnchorKeyResolver must resolve exact caller URAs at runtime",
            ),
            (
                "fn resolve_all(&self, caller_ura: &str)",
                "TrustAnchorKeyResolver multi-key path must resolve exact caller URAs at runtime",
            ),
        ):
            if token not in text:
                add("R129_TRUST_ANCHOR_ROLE_ONTOLOGY", key_resolver_rs, 1, detail)
        for token, detail in (
            (
                "fn resolve(&self, agent_ura: &str)",
                "TrustAnchorKeyResolver runtime signature must not preserve the historical agent_ura name",
            ),
        (
            "fn resolve_all(&self, agent_ura: &str)",
            "TrustAnchorKeyResolver multi-key signature must not preserve the historical agent_ura name",
        ),
        (
            "fn decode_pubkey(public_key_b64: &str, agent_ura: &str)",
            "TrustAnchorKeyResolver pubkey diagnostics must not name the caller slot agent_ura",
        ),
        (
            "caller_key_unavailable(agent_ura",
            "TrustAnchorKeyResolver caller-key diagnostics must not pass an overloaded agent_ura variable",
        ),
        ):
            offset = text.find(token)
            if offset != -1:
                add(
                    "R129_TRUST_ANCHOR_ROLE_ONTOLOGY",
                    key_resolver_rs,
                    line_number(text, offset),
                    detail,
                )
federated_key_resolver_rs = cli_root / "src/daemon/invocation/admission/federated_key_resolver.rs"
if federated_key_resolver_rs.exists():
    text = federated_key_resolver_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "Resolves a caller/principal URA",
            "federated key resolver docs must not describe every resolved URA as an Agent",
        ),
        (
            "fn resolve_federated(&self, caller_ura: &str)",
            "federated cross-realm resolver must name runtime input as caller_ura",
        ),
        (
            "fn resolve(&self, caller_ura: &str)",
            "federated KeyResolver implementation must resolve exact caller URAs at runtime",
        ),
        (
            "fn resolve_all(&self, caller_ura: &str)",
            "federated KeyResolver multi-key implementation must resolve exact caller URAs at runtime",
        ),
        (
            "caller_ura:{caller_ura}",
            "federated key resolver diagnostics must name caller_ura, not agent_ura",
        ),
    ):
        if token not in text:
            add("R129_TRUST_ANCHOR_ROLE_ONTOLOGY", federated_key_resolver_rs, 1, detail)
    for token, detail in (
        (
            "Resolves an `agent_ura`",
            "federated key resolver docs must not call every resolved caller/principal URA an agent_ura",
        ),
        (
            "fn resolve_federated(&self, agent_ura: &str)",
            "federated cross-realm resolver must not preserve the historical agent_ura name",
        ),
        (
            "fn resolve(&self, agent_ura: &str)",
            "federated KeyResolver runtime signature must not preserve the historical agent_ura name",
        ),
        (
            "fn resolve_all(&self, agent_ura: &str)",
            "federated KeyResolver multi-key signature must not preserve the historical agent_ura name",
        ),
        (
            "caller_key_not_found(agent_ura",
            "federated key resolver call sites must not pass runtime caller IDs as agent_ura",
        ),
        (
            "agent_ura:{agent_ura}",
            "federated key resolver diagnostics must not report runtime caller IDs as agent_ura",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R129_TRUST_ANCHOR_ROLE_ONTOLOGY",
                federated_key_resolver_rs,
                line_number(text, offset),
                detail,
            )

if admission_facade.exists():
    text = admission_facade.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "resolve_invocation_verifying_key(caller_ura)",
            "local hosted-Agent verification key fallback must verify the caller Agent before classifying AgentDeviceCustody",
        ),
        (
            "self.is_federated_caller(caller_ura)",
            "federated caller trust-path classification must remain explicit in AdmissionFacade",
        ),
        (
            "trusted_path,",
            "admission facade must pass TrustedCallerPath directly to AdmissionPolicyGate",
        ),
    ):
        if token not in text:
            add(
                "R130_HOSTED_AGENT_CUSTODY_DIRECT_PATH",
                admission_facade,
                1,
                detail,
            )
    if "fn federated_caller_path(" in text:
        add(
            "R130_HOSTED_AGENT_CUSTODY_DIRECT_PATH",
            admission_facade,
            line_number(text, text.find("fn federated_caller_path(")),
            "admission facade must not keep a second federated caller classifier",
        )
    for token, detail in (
        (
            "TrustedCallerPath::from_local_hosted_agent_custody(caller_ura)",
            "admission facade must not use the retired local hosted-Agent compatibility helper",
        ),
        (
            "TrustedCallerPath::from_federated_caller(caller_ura)",
            "admission facade must not use the retired ability-agnostic federated classifier",
        ),
        (
            "TrustedCallerPath::from_role_and_caller",
            "admission facade must not lower TrustAnchorRole directly inside runtime policy classification",
        ),
        (
            "TrustedCallerPath::from_verified_caller(",
            "admission facade must not use the retired ability-agnostic verified-caller classifier",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R130_HOSTED_AGENT_CUSTODY_DIRECT_PATH",
                admission_facade,
                line_number(text, offset),
                detail,
            )
    policy_gate_text = source(policy_gate) if policy_gate.exists() else ""
    for token, detail in (
        (
            "LOCAL_HOSTED_AGENT_CALLER_KIND_MISMATCH",
            "local hosted-Agent custody classification must reject non-Agent caller URAs",
        ),
        (
            "FEDERATED_CALLER_KIND_MISMATCH",
            "federated caller classification must reject non-actor URA kinds",
        ),
        (
            "DEVICE_CALLER_PURPOSE_UNVERIFIED",
            "federated Device caller classification must be guarded by public ability purpose",
        ),
    ):
        if token not in policy_gate_text:
            add(
                "R130_HOSTED_AGENT_CUSTODY_DIRECT_PATH",
                policy_gate,
                1,
                detail,
            )
    search_from = 0
    while True:
        offset = text.find("resolve_invocation_verifying_key", search_from)
        if offset == -1:
            break
        window = text[offset : offset + 900]
        if "TrustAnchorRole::Device" in window and "from_role_and_caller" in window:
            add(
                "R130_HOSTED_AGENT_CUSTODY_DIRECT_PATH",
                admission_facade,
                line_number(text, offset),
                "hosted-Agent local key fallback must not fabricate TrustAnchorRole::Device before AgentDeviceCustody",
            )
        search_from = offset + len("resolve_invocation_verifying_key")
    for token, detail in (
        (
            "local_hosted_agent_key_fallback_projects_direct_agent_custody_path",
            "admission facade tests must prove local hosted-Agent key fallback projects direct AgentDeviceCustody",
        ),
        (
            "local_hosted_agent_key_fallback_rejects_device_caller_ura",
            "admission facade tests must prove local hosted-Agent key fallback rejects Device caller URAs",
        ),
        (
            "trusted_device_row_requires_public_invocation_purpose",
            "admission facade tests must prove trusted Device rows require DeviceCallerPurpose classification",
        ),
    ):
        if token not in text:
            add(
                "R130_HOSTED_AGENT_CUSTODY_DIRECT_PATH",
                admission_facade,
                1,
                detail,
            )

if admission_facade.exists() and policy_gate.exists():
    facade_text = admission_facade.read_text(encoding="utf-8", errors="replace")
    gate_text = policy_gate.read_text(encoding="utf-8", errors="replace")
    policy_tests_text = (
        policy_gate_tests.read_text(encoding="utf-8", errors="replace")
        if policy_gate_tests.exists()
        else ""
    )
    for token, detail in (
        (
            "&input.ability,\n                device_purpose,",
            "runtime admission must pass the selected public ability into trust-path classification",
        ),
        (
            "public_ability: &str",
            "AdmissionFacade trusted_path_for_caller must require the public ability for Device-caller purpose classification",
        ),
    ):
        if token not in facade_text:
            add("R151_DEVICE_CALLER_PURPOSE_TRUST_PATH", admission_facade, 1, detail)
    for token, detail in (
        (
            "require_device_caller_purpose(",
            "TrustedCallerPath must validate DeviceCustody through DeviceCallerPurpose before policy",
        ),
        (
            "DEVICE_CALLER_PURPOSE_UNVERIFIED",
            "invocation-aware federated Device classification must reject ordinary public abilities",
        ),
        (
            "public_ability: &str",
            "federated Device caller classification must require selected public ability context",
        ),
    ):
        if token not in gate_text:
            add("R151_DEVICE_CALLER_PURPOSE_TRUST_PATH", policy_gate, 1, detail)
    for token, detail in (
        (
            "trusted_caller_path_requires_invocation_purpose_for_federated_device",
            "policy tests must prove federated Device callers require public ability purpose",
        ),
        (
            "DEVICE_CALLER_PURPOSE_UNVERIFIED",
            "policy tests must prove ordinary public abilities reject Device callers before policy",
        ),
    ):
        if token not in policy_tests_text:
            add("R151_DEVICE_CALLER_PURPOSE_TRUST_PATH", policy_gate_tests, 1, detail)
    for token, detail in (
        (
            "TrustedCallerPath::from_verified_caller(\n                caller_ura,\n                VerifiedCallerEvidence::TrustAnchorRole",
            "AdmissionFacade must not classify trust-anchor callers with the ability-agnostic verified-caller API",
        ),
        (
            "TrustedCallerPath::from_verified_caller(\n                caller_ura,\n                VerifiedCallerEvidence::Federated",
            "AdmissionFacade must not classify federated callers with the ability-agnostic verified-caller API",
        ),
        (
            "trusted_path_for_caller(caller_ura, trust_anchor.as_ref())",
            "runtime admission must not classify callers without selected public ability context",
        ),
    ):
        offset = facade_text.find(token)
        if offset != -1:
            add(
                "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                admission_facade,
                line_number(facade_text, offset),
                detail,
            )
    for token, detail in (
        (
            "pub(crate) fn from_verified_caller(",
            "TrustedCallerPath must not expose the retired ability-agnostic verified-caller API",
        ),
        (
            "pub(crate) fn from_federated_caller(",
            "TrustedCallerPath must not expose ability-agnostic federated Device classification",
        ),
    ):
        offset = gate_text.find(token)
        if offset != -1:
            add(
                "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                policy_gate,
                line_number(gate_text, offset),
                detail,
            )
    for token, detail in (
        (
            "device_owned_ability_matches",
            "Realm Authority public-read admission must not revive direct Device-owned Ability URAs as policy subjects",
        ),
    ):
        offset = gate_text.find(token)
        if offset != -1:
            add(
                "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                policy_gate,
                line_number(gate_text, offset),
                detail,
            )
    owner_fact_body = rust_method_body(gate_text, "owner_fact_from_ura")
    if owner_fact_body is None:
        add(
            "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
            policy_gate,
            1,
            "policy gate must keep explicit owner_fact_from_ura resolution so direct Device-owned Ability URAs cannot project through Device owners",
        )
    else:
        start, body = owner_fact_body
        projection = re.search(
            r"Some\(AbilityOwner::Device\s*\{[^}]*\}\)\s*=>\s*\{[^}]*owner_fact_from_trust_anchor",
            body,
            re.DOTALL,
        )
        if projection:
            add(
                "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                policy_gate,
                line_number(gate_text, start + projection.start()),
                "direct Device-owned Ability URAs are migration facts and must not project through Device trust-anchor owners",
            )
        if "device_owned_ability_subject_does_not_project_device_owner" not in policy_tests_text:
            add(
                "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                policy_gate_tests,
                1,
                "policy tests must prove direct Device-owned Ability subjects stay owner-unresolved even when the Device has a trusted owner",
            )
    realm_public_read_body = rust_method_body(gate_text, "realm_authority_public_read_scope")
    if realm_public_read_body is None:
        add(
            "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
            policy_gate,
            1,
            "policy gate must keep a named Realm Authority public-read scope for local DeviceProfileProjection metadata",
        )
    else:
        start, body = realm_public_read_body
        if "subject_ura == callee_ura ||" in body:
            add(
                "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                policy_gate,
                line_number(gate_text, start + body.find("subject_ura == callee_ura ||")),
                "Realm Authority public-read admission must be bounded to the local DeviceProfileProjection subject only",
            )
    if "realm_authority_public_read_does_not_admit_device_owned_ability_subject" not in policy_tests_text:
        add(
            "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
            policy_gate_tests,
            1,
            "policy tests must prove Hub public-read cannot admit direct Device-owned Ability subjects",
        )
    owner_fact_body = rust_method_body(gate_text, "owner_fact_from_ura")
    if owner_fact_body is None:
        add(
            "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
            policy_gate,
            1,
            "policy gate must keep owner_fact_from_ura as the single ordinary-policy owner projection boundary",
        )
    else:
        start, body = owner_fact_body
        device_owner_start = body.find("Some(AbilityOwner::Device")
        device_owner_end = body.find("Some(AbilityOwner::Service", device_owner_start)
        if device_owner_end == -1:
            device_owner_end = body.find("Some(AbilityOwner::SystemAgent", device_owner_start)
        if device_owner_start == -1:
            add(
                "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                policy_gate,
                line_number(gate_text, start),
                "owner_fact_from_ura must explicitly fail closed for direct Device-owned Ability URAs",
            )
        else:
            device_owner_branch = body[
                device_owner_start : device_owner_end if device_owner_end != -1 else len(body)
            ]
            if (
                "=> None" not in device_owner_branch
                or "owner_fact_from_trust_anchor" in device_owner_branch
                or "device_ura(" in device_owner_branch
                or "OwnerFact::" in device_owner_branch
            ):
                add(
                    "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
                    policy_gate,
                    line_number(gate_text, start + device_owner_start),
                    "direct Device-owned Ability URAs are migration facts and must not project Device owners into ordinary policy",
                )
    if "device_owned_ability_subject_does_not_project_device_owner" not in policy_tests_text:
        add(
            "R151_DEVICE_CALLER_PURPOSE_TRUST_PATH",
            policy_gate_tests,
            1,
            "policy tests must prove direct Device-owned Ability subjects do not inherit Device owner anchors",
        )

if bootstrap_authority.exists():
    text = bootstrap_authority.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "TrustedCallerPath",
            "bootstrap authority verifier must consume TrustedCallerPath directly",
        ),
        (
            "trusted_path: TrustedCallerPath",
            "bootstrap authority verifier signature must name caller custody path, not persisted TrustAnchorRole",
        ),
        (
            "trusted_path == TrustedCallerPath::Hub",
            "hub bootstrap authority must branch on TrustedCallerPath::Hub",
        ),
        (
            "trusted_path == TrustedCallerPath::User",
            "user bootstrap authority must branch on TrustedCallerPath::User",
        ),
        (
            "let TrustedCallerPath::DeviceCustody(device_purpose) = trusted_path",
            "device bootstrap authority must require and carry the exact verified DeviceCustody purpose",
        ),
        (
            "hosted_agent_custody_path_does_not_inherit_device_bootstrap_authority",
            "bootstrap authority tests must prove AgentDeviceCustody cannot inherit Device bootstrap authority",
        ),
    ):
        if token not in text:
            add(
                "R131_BOOTSTRAP_AUTHORITY_TRUSTED_PATH",
                bootstrap_authority,
                1,
                detail,
            )
    for token, detail in (
        (
            "trusted_role: TrustAnchorRole",
            "bootstrap authority verifier must not accept persisted TrustAnchorRole as runtime trust-path input",
        ),
        (
            "trusted_role == TrustAnchorRole",
            "bootstrap authority verifier must not branch on persisted TrustAnchorRole",
        ),
        (
            "trusted_role != TrustAnchorRole",
            "bootstrap authority verifier must not branch on persisted TrustAnchorRole",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R131_BOOTSTRAP_AUTHORITY_TRUSTED_PATH",
                bootstrap_authority,
                line_number(text, offset),
                detail,
            )
    if "local_device_owner_fact(caller_ura)" not in text:
        add(
            "R100_POLICY_GATE_NO_LOCAL_CREDENTIAL_OWNER_FALLBACK",
            bootstrap_authority,
            1,
            "bootstrap authority must remain the explicit bounded owner of local paired-device projection",
        )

if admission_facade.exists():
    text = admission_facade.read_text(encoding="utf-8", errors="replace")
    offset = text.find("persisted_trust_anchor_role()")
    if offset != -1:
        add(
            "R131_BOOTSTRAP_AUTHORITY_TRUSTED_PATH",
            admission_facade,
            line_number(text, offset),
            "admission facade must pass TrustedCallerPath directly to bootstrap authority without role round-tripping",
        )

ability_deploy_ops = (
    cli_root / "src/daemon/ability/builtins/device_control/ability_management/ops.rs"
)
ability_deploy_registrar = (
    cli_root / "src/daemon/ability/builtins/device_control/ability_management/registrar.rs"
)
ability_deploy_store = (
    cli_root / "src/daemon/ability/builtins/device_control/ability_management/store.rs"
)
ability_deploy_target = cli_root / "src/daemon/invocation/routing/target.rs"
ability_deploy_descriptor = (
    cli_root / "ability-descriptors/system/device_control/ability.deploy.ability.toml"
)
ability_deploy_cli = cli_root / "src/cli/commands/deploy.rs"
ability_group_cli = cli_root / "src/cli/commands/groups/ability.rs"
remote_system_facade = cli_root / "src/cli/daemon_client/remote_system_ability.rs"
remote_device_support = cli_root / "src/support/platform/remote_device.rs"
easyremote_e2e = cli_root / "tools/scripts/docker-two-node-easyremote-cli-e2e.sh"

for path in production_files(cli_root / "src/daemon", {".rs"}):
    text = source(path)
    for match in re.finditer(
        r"\.hot_register\w*\s*\([^;{}]*?OwnerKind::(?:DeviceProfileProjection|Device)\b",
        text,
        flags=re.S,
    ):
        add(
            "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, match.start()),
            "production dynamic hot-register paths must not create direct Device-owned dynamic abilities; use a SystemAgent or explicit hosted Agent owner",
        )

if ability_deploy_ops.exists():
    text = source(ability_deploy_ops)
    raw_text = ability_deploy_ops.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "let host_device_ura = crate::core::ura::device_ura",
            "ability.deploy must name target Device as execution host, not descriptor owner",
        ),
        (
            "let deployed_owner_ura = crate::core::ura::device_agent_ura",
            "ability.deploy must derive deployed descriptor owner as a device-sponsored SystemAgent",
        ),
        (
            "ABILITY_MANAGEMENT_SYSTEM_AGENT_ID",
            "ability.deploy dynamic owner must be the ability-management SystemAgent",
        ),
        (
            "owner_ability_ura(&deployed_owner_ura, &key)",
            "ability.deploy must derive Ability URA from deployed_owner_ura, not host_device_ura",
        ),
        (
            '"owner_ura": deployed_owner_ura',
            "ability.deploy response must expose descriptor owner separately from target Device host",
        ),
        (
            "AbilityManagementMutationActor::from_envelope",
            "ability.deploy handler must preserve the already-admitted logical mutation actor",
        ),
        (
            "ability_management_actor_rejects_device_custody_as_mutation_authority",
            "ability.deploy tests must prove Device custody cannot become mutation authority",
        ),
    ):
        haystack = raw_text if "tests must prove" in detail else text
        if token not in haystack:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_ops, 1, detail)
    for token, detail in (
        (
            "owner_ability_ura(&host_device_ura, &key)",
            "ability.deploy must not derive dynamic Ability URA from Device execution host",
        ),
        (
            "owner_ability_ura(&owner_ura, &key)",
            "ability.deploy must not use a generic owner_ura variable for the Device execution host",
        ),
        (
            "require_local_device_authority(",
            "ability.deploy handler must not equate local Device custody with mutation authority",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
                ability_deploy_ops,
                line_number(text, offset),
                detail,
            )
if ability_deploy_registrar.exists():
    text = source(ability_deploy_registrar)
    raw_text = ability_deploy_registrar.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "selector.owner_kind() != \"system-agent\"",
            "dynamic deploy install validation must require SystemAgent-owned Ability URAs",
        ),
        (
            "ABILITY_MANAGEMENT_SYSTEM_AGENT_ID",
            "dynamic deploy install validation must require the ability-management SystemAgent owner",
        ),
        (
            "struct DeployedAbilityControlPlaneKey",
            "dynamic deploy control-plane key must be named as deployed ability, not Device owner",
        ),
        (
            "owner_projection: format!(\"system-agent:{}\", selector.dispatch_target())",
            "dynamic deploy control-plane authority scope must use SystemAgent projection",
        ),
        (
            "install_rejects_direct_device_owned_dynamic_ability_ura",
            "registrar tests must prove ability.deploy rejects direct Device-owned dynamic URAs",
        ),
    ):
        haystack = raw_text if "tests must prove" in detail else text
        if token not in haystack:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_registrar, 1, detail)
    for token, detail in (
        (
            "selector.owner_kind() != \"device\"",
            "dynamic deploy registrar must not require Device-owned Ability URAs",
        ),
        (
            "AuthorityScope::new(\"device\"",
            "dynamic deploy control-plane authority scope must not be direct Device",
        ),
        (
            "DeviceAbilityControlPlaneKey",
            "dynamic deploy control-plane key must not preserve Device-owner naming",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
                ability_deploy_registrar,
                line_number(text, offset),
                detail,
            )
if ability_deploy_store.exists():
    text = source(ability_deploy_store)
    raw_text = ability_deploy_store.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "ability_management_system_agent_owner_is_hosted_by_device(",
            "deploy store replay quarantine must classify ability-management SystemAgent owners explicitly",
        ),
        (
            "device_agent_ids()",
            "deploy store replay quarantine must inspect SystemAgent sponsoring Device",
        ),
        (
            "ABILITY_MANAGEMENT_SYSTEM_AGENT_ID",
            "deploy store replay quarantine must restrict replay to ability-management SystemAgent owners",
        ),
        (
            "quarantine_unhosted_device_authority_hides_old_system_agent_rows_from_replay",
            "store tests must prove old device-sponsored SystemAgent rows are hidden after rejoin",
        ),
        (
            "quarantine_unhosted_device_authority_hides_direct_device_rows_from_replay",
            "store tests must prove legacy direct Device-owned deployment rows do not replay",
        ),
        (
            "quarantine_unhosted_device_authority_hides_same_device_non_ability_management_rows",
            "store tests must prove same-device non-ability-management SystemAgent rows do not replay",
        ),
    ):
        haystack = raw_text if "tests must prove" in detail else text
        if token not in haystack:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_store, 1, detail)
    for token, detail in (
        (
            "owner_ura == hosted_device_ura",
            "deploy store replay must not treat direct Device-owned Ability URAs as hosted deployments",
        ),
        (
            "fn ability_owner_is_hosted_by_device",
            "deploy store replay predicate name must not preserve direct Device owner semantics",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
                ability_deploy_store,
                line_number(text, offset),
                detail,
            )
if ability_deploy_descriptor.exists():
    raw_text = ability_deploy_descriptor.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "ability-management SystemAgent",
            "ability.deploy public descriptor must say deployed descriptors are owned by ability-management SystemAgent",
        ),
        (
            "execution host/custody only",
            "ability.deploy public descriptor must say target Device is host/custody only",
        ),
        (
            "not descriptor owner or public callee",
            "ability.deploy public descriptor must reject Device-as-owner/callee wording",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_descriptor, 1, detail)
    for token, detail in (
        (
            "Publish a host_stream device ability bundle ResourceRef to a canonical Device URA",
            "ability.deploy public descriptor must not describe deploy as publishing a device-owned ability to Device",
        ),
    ):
        offset = raw_text.find(token)
        if offset != -1:
            add(
                "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
                ability_deploy_descriptor,
                line_number(raw_text, offset),
                detail,
            )
if ability_deploy_cli.exists():
    raw_text = ability_deploy_cli.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "ability-management SystemAgent",
            "ability deploy CLI docs must name the SystemAgent descriptor owner",
        ),
        (
            "execution host/custody",
            "ability deploy CLI docs must keep target Device as host/custody, not descriptor owner",
        ),
        (
            "PairedInvocationIdentity::load(\"ability deploy\")",
            "ability deploy CLI must keep accountable caller and local execution host as distinct typed facts",
        ),
        (
            "invocation.caller_user_ura()",
            "ability deploy CLI must consume the paired User caller instead of Device custody",
        ),
        (
            "deploy_invocation_separates_user_caller_from_device_execution_host",
            "ability deploy tests must prove User caller and Device execution host remain distinct",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_cli, 1, detail)
    for token, detail in (
        (
            "invoke_remote_ability_deploy(",
            "ability deploy CLI must route peer Device targets through an explicit remote deploy path",
        ),
        (
            "resource_ref_for_target_tmp_relative_path",
            "remote ability deploy must stage a target-local ResourceRef instead of passing a caller-local ResourceRef",
        ),
        (
            "upload_target_resource_via_file_transfer",
            "ability deploy CLI must delegate remote ResourceRef upload to the remote system ability facade",
        ),
        (
            "invoke_target_ability_deploy_from_resource",
            "ability deploy CLI must use one accountable target ability.deploy invocation for local and remote hosts",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_cli, 1, detail)
    for token, detail in (
        (
            "Description: Publish an ability bundle to a canonical Device URA",
            "ability deploy CLI docs must not describe deploy as direct Device-owned publishing",
        ),
        (
            "require_caller_device_ura_from_credentials()",
            "ability deploy CLI must not use Device custody as the public deployment caller",
        ),
    ):
        offset = raw_text.find(token)
        if offset != -1:
            add(
                "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
                ability_deploy_cli,
                line_number(raw_text, offset),
                detail,
            )

if remote_device_support.exists():
    raw_text = remote_device_support.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "struct PairedInvocationIdentity",
            "CLI mutation identity must keep User accountability and Device locality in one typed value",
        ),
        (
            "RuntimeUserBinding::Bound { user_ura }",
            "CLI mutation identity must derive its caller from the paired User binding",
        ),
        (
            "caller_user_ura",
            "CLI mutation identity must expose the accountable User fact explicitly",
        ),
        (
            "local_device_ura",
            "CLI mutation identity must expose Device locality separately from caller identity",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", remote_device_support, 1, detail)

if ability_group_cli.exists():
    raw_text = ability_group_cli.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "PairedInvocationIdentity::load(",
            "ability uninstall must require a paired User accountability root",
        ),
        (
            "invoke_target_ability_uninstall(",
            "ability uninstall must use the same target SystemAgent invocation boundary as deploy",
        ),
        (
            "identity.caller_user_ura()",
            "ability uninstall must preserve the paired User as caller",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_group_cli, 1, detail)
    obsolete = "LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(\"ability.uninstall\""
    offset = raw_text.find(obsolete)
    if offset != -1:
        add(
            "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
            ability_group_cli,
            line_number(raw_text, offset),
            "public ability uninstall must not replace the paired User caller with _system.local",
        )

if remote_system_facade.exists():
    raw_text = remote_system_facade.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "upload_target_resource_via_file_transfer",
            "remote system facade must expose the remote deploy ResourceRef upload operation",
        ),
        (
            "invoke_target_ability_deploy_from_resource",
            "remote system facade must expose the target-owned ability.deploy operation",
        ),
        (
            "invoke_target_ability_uninstall",
            "remote system facade must expose the target-owned ability.uninstall operation",
        ),
        (
            "FS_TRANSFER",
            "remote deploy upload must materialize the bundle through target fs.transfer",
        ),
        (
            "ABILITY_DEPLOY",
            "remote deploy must invoke the target ability-management SystemAgent ability.deploy selector",
        ),
        (
            "RemoteAbilityInvocationTarget::for_target_owned_selector",
            "remote deploy facade must use descriptor-bound target-owned SystemAgent invocation",
        ),
        (
            "RemoteUserActionInvocationIssuer::caller_declared_root_plan",
            "remote deploy facade must delegate named subject/nonce/causal root policy to the User-action issuer",
        ),
        (
            "ABILITY_UNINSTALL",
            "remote uninstall must invoke the target ability-management SystemAgent selector",
        ),
        (
            "caller_ura,\n        ability_ura,",
            "remote uninstall must preserve User caller and removed Ability subject as distinct tuple facts",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", remote_system_facade, 1, detail)

# Public CLI work is accountable to the paired User Principal. A Device may
# select locality, host execution, publish custody, and carry bootstrap
# federation traffic, but it must never be substituted as the caller of an
# operator-originated ability invocation.
for public_cli_path in (
    cli_root / "src/cli/commands/invoke.rs",
    cli_root / "src/cli/commands/ability_stream.rs",
    cli_root / "src/cli/commands/ability_bidi.rs",
):
    if not public_cli_path.exists():
        continue
    raw_text = public_cli_path.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "PairedInvocationIdentity::load(",
            "remote public ability ingress must load paired User accountability separately from Device locality",
        ),
        (
            "identity.caller_user_ura()",
            "remote public ability ingress must submit the paired User as caller",
        ),
    ):
        if token not in raw_text:
            add("R161_PUBLIC_CLI_USER_CALLER_DEVICE_HOST_SPLIT", public_cli_path, 1, detail)
    for token, detail in (
        (
            "remote_device::caller_device_ura(",
            "remote public ability ingress must not use Device custody as caller",
        ),
        (
            "caller_ura = local_device_ura",
            "remote public ability ingress must not copy Device locality into caller identity",
        ),
    ):
        offset = raw_text.find(token)
        if offset != -1:
            add(
                "R161_PUBLIC_CLI_USER_CALLER_DEVICE_HOST_SPLIT",
                public_cli_path,
                line_number(raw_text, offset),
                detail,
            )

if remote_system_facade.exists():
    raw_text = remote_system_facade.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "PairedInvocationIdentity::load(action_label)",
            "remote system/catalogue facade must load paired User accountability",
        ),
        (
            "identity.caller_user_ura()",
            "remote system/catalogue facade must use User caller independently of target Device",
        ),
        (
            "identity.local_device_ura()",
            "remote catalogue routing must use Device only for locality decisions",
        ),
        (
            "RuntimeUserBinding::Bound { user_ura }",
            "current Realm Hub calls must derive caller from the bound User Principal",
        ),
    ):
        if token not in raw_text:
            add("R161_PUBLIC_CLI_USER_CALLER_DEVICE_HOST_SPLIT", remote_system_facade, 1, detail)

terminal_cli = cli_root / "src/cli/commands/device_terminal.rs"
if terminal_cli.exists():
    raw_text = terminal_cli.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "PairedInvocationIdentity::load(",
            "remote terminal must load paired User accountability separately from Device locality",
        ),
        (
            "identity.caller_user_ura().to_string()",
            "remote terminal invocation and session authority must be issued by the paired User",
        ),
        (
            "load_remote_invocation_caller_signer(\n                    &caller_ura,",
            "remote terminal authority must use the signer bound to its User caller",
        ),
    ):
        if token not in raw_text:
            add("R161_PUBLIC_CLI_USER_CALLER_DEVICE_HOST_SPLIT", terminal_cli, 1, detail)
    offset = raw_text.find("let caller_ura = local_device_ura")
    if offset != -1:
        add(
            "R161_PUBLIC_CLI_USER_CALLER_DEVICE_HOST_SPLIT",
            terminal_cli,
            line_number(raw_text, offset),
            "remote terminal must not use the local Device as caller or authority issuer",
        )

if remote_device_support.exists():
    raw_text = remote_device_support.read_text(encoding="utf-8", errors="replace")
    offset = raw_text.find("pub(crate) fn caller_device_ura(")
    if offset != -1:
        add(
            "R161_PUBLIC_CLI_USER_CALLER_DEVICE_HOST_SPLIT",
            remote_device_support,
            line_number(raw_text, offset),
            "Device locality derivation must stay private to PairedInvocationIdentity",
        )

if local_invoke.exists():
    raw_text = local_invoke.read_text(encoding="utf-8", errors="replace")
    if "pub struct LocalDaemonSystemAbilityIssuer" in raw_text:
        for token, detail in (
            (
                "LocalAbilityTarget::for_device_sponsored_system_ability(",
                "local daemon system issuer must project a Device execution host to the registry-declared SystemAgent owner",
            ),
            (
                "crate::core::ura::URAKind::Authority =>",
                "local daemon system issuer must preserve Authority-owned Hub abilities explicitly",
            ),
            (
                "Self::invoke_target_root_timeout(&target, args, subject_ura, timeout)",
                "local daemon system issuer must submit the resolved owner target instead of the execution host",
            ),
        ):
            if token not in raw_text:
                add("R159_LOCAL_SYSTEM_ABILITY_OWNER_HOST_SPLIT", local_invoke, 1, detail)
        obsolete = "invoke_local_daemon_system_ability_root_for_subject_timeout("
        offset = raw_text.find(obsolete)
        if offset != -1:
            add(
                "R159_LOCAL_SYSTEM_ABILITY_OWNER_HOST_SPLIT",
                local_invoke,
                line_number(raw_text, offset),
                "local daemon issuer must not submit an unowned Device-as-callee root",
            )

if easyremote_e2e.exists():
    raw_text = easyremote_e2e.read_text(encoding="utf-8", errors="replace")
    for stale, detail in (
        (
            'device_exec_records[0].get("caller_ura") == caller_ura',
            "EasyRemote E2E must prove remote process execution is accountable to the paired User",
        ),
        (
            'terminal_create_records[0].get("caller_ura") == caller_ura',
            "EasyRemote E2E must prove remote terminal creation is accountable to the paired User",
        ),
    ):
        offset = raw_text.find(stale)
        if offset != -1:
            add(
                "R161_PUBLIC_CLI_USER_CALLER_DEVICE_HOST_SPLIT",
                easyremote_e2e,
                line_number(raw_text, offset),
                detail,
            )

if ability_deploy_descriptor.exists():
    raw_text = ability_deploy_descriptor.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            'scope_subjects_kind = "only_ura_kinds"',
            "ability.deploy descriptor must kind-gate its staged Resource subject",
        ),
        (
            'scope_subjects_uras = ["resource"]',
            "ability.deploy descriptor must admit only Resource subjects",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_descriptor, 1, detail)

ability_uninstall_descriptor = (
    cli_root / "ability-descriptors/system/device_control/ability.uninstall.ability.toml"
)
if ability_uninstall_descriptor.exists():
    raw_text = ability_uninstall_descriptor.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            'scope_subjects_kind = "only_ura_kinds"',
            "ability.uninstall descriptor must kind-gate its removed Ability subject",
        ),
        (
            'scope_subjects_uras = ["ability"]',
            "ability.uninstall descriptor must admit only Ability subjects",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_uninstall_descriptor, 1, detail)

if easyremote_e2e.exists():
    raw_text = easyremote_e2e.read_text(encoding="utf-8", errors="replace")
    for stale, detail in (
        (
            'user_plugin_show.get("owner_ura") == provider_ura',
            "EasyRemote E2E must not expect a user plugin AbilityDescriptor to be Device-owned",
        ),
        (
            'native_discovery[0].get("owner_ura") == provider_ura',
            "EasyRemote E2E must not expect a deployed AbilityDescriptor to be Device-owned",
        ),
    ):
        offset = raw_text.find(stale)
        if offset != -1:
            add(
                "R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER",
                easyremote_e2e,
                line_number(raw_text, offset),
                detail,
            )

filesystem_resource_ref = cli_root / "src/daemon/resources/files/mod.rs"
if filesystem_resource_ref.exists():
    text = source(filesystem_resource_ref)
    for token, detail in (
        (
            "pub struct FilesystemResourceProvider",
            "filesystem ResourceRef operations must be owned by an explicit Device provider",
        ),
        (
            "FilesystemResourceProvider::for_device",
            "filesystem ResourceRef call sites must validate an explicit Device authority",
        ),
        (
            "reference.owner_ura != local_owner_ura",
            "filesystem ResourceRef resolution must reject foreign Device-owned refs",
        ),
        (
            "filesystem ResourceRefs are daemon-local",
            "foreign ResourceRef rejection must document daemon-local capability semantics",
        ),
        (
            "resource_ref_for_target_tmp_relative_path",
            "remote staging must use an explicit target-tmp ResourceRef constructor",
        ),
    ):
        if token not in text:
            add("R159_FILESYSTEM_RESOURCE_REF_LOCAL_OWNER", filesystem_resource_ref, 1, detail)
    forbidden_ambient_owner = "identity::local_invocation::local_device_ura"
    offset = text.find(forbidden_ambient_owner)
    if offset != -1:
        add(
            "R159_FILESYSTEM_RESOURCE_REF_LOCAL_OWNER",
            filesystem_resource_ref,
            line_number(text, offset),
            "filesystem ResourceRef domain must not recover Device authority from ambient process identity",
        )
if ability_deploy_target.exists():
    text = source(ability_deploy_target)
    raw_text = ability_deploy_target.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "DeployedAbilitySubject",
            "daemon-system target policy must name deployed ability subject separately from callee owner",
        ),
        (
            "is_ability_management_system_agent_callee",
            "daemon-system target policy must recognize ability-management SystemAgent callees",
        ),
        (
            "selector.owner_ura() == callee_ura",
            "daemon-system target policy must only accept canonical dynamic Ability URAs owned by the callee",
        ),
        (
            "owner_ability_ura(callee_ura, ability)",
            "daemon-system target policy must derive public-name dynamic Ability subjects from the ability-management callee",
        ),
    ):
        if token not in text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_target, 1, detail)
    for token, detail in (
        (
            "deployed_dynamic_ability_subject_resolves_to_ability_ura_for_public_name",
            "target tests must prove public-name dynamic abilities use the deployed Ability URA as subject",
        ),
        (
            "deployed_dynamic_ability_subject_accepts_canonical_ability_ura",
            "target tests must prove canonical dynamic Ability URAs remain the subject",
        ),
        (
            "deployed_dynamic_ability_subject_rejects_cross_owner_ability_ura",
            "target tests must prove dynamic Ability URAs from a different owner do not become the subject",
        ),
    ):
        if token not in raw_text:
            add("R132_ABILITY_DEPLOY_SYSTEM_AGENT_OWNER", ability_deploy_target, 1, detail)

hosted_authority = cli_root / "src/daemon/ability/authority/mod.rs"
hosted_delegation_issuer = (
    cli_root / "src/daemon/invocation/admission/hosted_agent_delegation.rs"
)
teach_grants_store = cli_root / "src/daemon/persistence/teach_grants.rs"
if hosted_authority.exists():
    text = source(hosted_authority)
    raw_text = hosted_authority.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "host_device_ura: String",
            "hosted-agent delegation binding/claims/context must carry host Device separately from wire callee",
        ),
        (
            "host_device_ura: impl Into<String>",
            "hosted-agent delegation envelope constructor must receive an explicit host Device URA",
        ),
        (
            "validate_host_device_ura(",
            "hosted-agent delegation must validate host_device_ura as a Device URA",
        ),
        (
            "validate_delegation_callee_ura(",
            "hosted-agent delegation must validate wire callee as Agent/SystemAgent or Authority, never Device",
        ),
        (
            "host_device_ura != envelope.host_device_ura()",
            "hosted-agent delegation metadata verification must compare signed host Device against the envelope binding",
        ),
        (
            'let expected_host_authority = format!("hosted_by:{}", self.host_device_ura);',
            "hosted-agent authorization must compare persisted authority against host_device_ura, not wire callee",
        ),
    ):
        if token not in text:
            add("R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT", hosted_authority, 1, detail)
    for token, detail in (
        (
            "hosted_agent_delegation_binds_system_agent_callee_to_separate_host_device",
            "authority tests must prove SystemAgent wire callee binds to a separate Device host",
        ),
        (
            "hosted_agent_delegation_rejects_device_callee",
            "authority tests must prove Device URAs are rejected as hosted-agent delegation wire callees",
        ),
    ):
        if token not in raw_text:
            add("R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT", hosted_authority, 1, detail)
    authorize_body = rust_method_body(text, "authorize")
    if authorize_body is not None:
        offset, body = authorize_body
        for token, detail in (
            (
                "wire_callee_ura",
                "hosted-agent authorization must not use wire_callee_ura as host evidence",
            ),
            (
                "parse_ura(&self.wire_callee_ura",
                "hosted-agent authorization must not parse public callee as the host Device",
            ),
            (
                'format!("hosted_by:{}", self.wire_callee_ura',
                "hosted-agent authorization must not build hosted_by authority from wire callee",
            ),
        ):
            local = body.find(token)
            if local != -1:
                add(
                    "R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT",
                    hosted_authority,
                    line_number(text, offset + local),
                    detail,
                )
if hosted_delegation_issuer.exists():
    text = source(hosted_delegation_issuer)
    raw_text = hosted_delegation_issuer.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "host_device_ura: &str",
            "hosted-agent delegation issuer must receive explicit host Device URA from dispatch",
        ),
        (
            "caller_ura,\n            callee_ura,\n            host_device_ura,",
            "hosted-agent delegation issuer must sign wire callee and host Device as separate fields",
        ),
        (
            "materialize_request_metadata_binds_system_agent_callee_to_host_device",
            "issuer tests must prove signed metadata separates SystemAgent callee from Device host",
        ),
    ):
        haystack = raw_text if "tests must prove" in detail else text
        if token not in haystack:
            add("R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT", hosted_delegation_issuer, 1, detail)
for dispatch_file in (
    cli_root / "src/daemon/invocation/dispatch/unary_dispatcher.rs",
    cli_root / "src/daemon/invocation/streams/stream_dispatcher.rs",
    cli_root / "src/daemon/invocation/bidi/bidi_dispatcher.rs",
):
    if not dispatch_file.exists():
        continue
    text = source(dispatch_file)
    if (
        "HostedAgentDelegationIssuer::materialize_request_metadata(" in text
        and "&selected_route.execution_host_ura," not in text
    ):
        add(
            "R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT",
            dispatch_file,
            1,
            "selected-route dispatch must pass execution_host_ura as hosted-agent delegation host Device",
        )

if teach_grants_store.exists():
    text = source(teach_grants_store)
    raw_text = teach_grants_store.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "validate_hosted_delegation_snapshot_callee(",
            "teach grant durable validation must reject Device public callees for hosted delegation snapshots",
        ),
        (
            "host Device belongs in hosted authority host_device_ura",
            "teach grant durable validation must keep callee separate from host Device authority",
        ),
    ):
        if token not in text:
            add("R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT", teach_grants_store, 1, detail)
    if "snapshot.callee_ura != *host_device_ura" in text:
        add(
            "R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT",
            teach_grants_store,
            line_number(text, text.find("snapshot.callee_ura != *host_device_ura")),
            "teach grant hosted delegation validation must not require public callee to equal host Device",
        )
    if "hosted_delegation_snapshot_rejects_device_callee" not in raw_text:
        add(
            "R133_HOSTED_DELEGATION_CALLEE_HOST_SPLIT",
            teach_grants_store,
            1,
            "teach grant tests must prove Device callees are rejected for hosted delegation snapshots",
        )

policy_engine = cli_root / "src/daemon/invocation/admission/policy_engine.rs"
if policy_engine.exists():
    text = source(policy_engine)
    policy_input_start = text.find("pub struct PolicyInput")
    policy_input_end = (
        text.find("\n}\n\npub struct PolicyEngine", policy_input_start)
        if policy_input_start != -1
        else -1
    )
    policy_input_body = (
        text[policy_input_start:policy_input_end]
        if policy_input_start != -1 and policy_input_end != -1
        else ""
    )
    if "pub enum SystemPolicyRuleMatch" not in text:
        add(
            "R102_POLICY_SYSTEM_RULE_MATCH_TYPED",
            policy_engine,
            1,
            "policy system allow facts must use typed SystemPolicyRuleMatch",
        )
    if "pub system_rule_matches: Vec<SystemPolicyRuleMatch>" not in policy_input_body:
        add(
            "R102_POLICY_SYSTEM_RULE_MATCH_TYPED",
            policy_engine,
            1,
            "PolicyInput must carry typed system_rule_matches",
        )
    for token in (
        "pub authority_self_read:",
        "pub authority_self_manage:",
        "pub authority_self_stream:",
        "pub authority_peer_directory_stream:",
        "pub realm_authority_public_read:",
        "pub device_publication_custody_manage:",
        "pub device_self_session_stream:",
        "pub remote_owner_forward_allowed:",
    ):
        offset = policy_input_body.find(token)
        if offset != -1 and policy_input_start != -1:
            add(
                "R102_POLICY_SYSTEM_RULE_MATCH_TYPED",
                policy_engine,
                line_number(text, policy_input_start + offset),
                "PolicyInput must not reintroduce procedural system allow booleans",
            )
    allow_reason_start = text.find("fn allow_reason(")
    allow_reason_end = (
        text.find("\n}\n\n#[derive(Debug, Clone)]\npub struct PolicyInput", allow_reason_start)
        if allow_reason_start != -1
        else -1
    )
    allow_reason_body = (
        text[allow_reason_start:allow_reason_end]
        if allow_reason_start != -1 and allow_reason_end != -1
        else ""
    )
    if "PolicyDecisionReason::ExplicitGrantAllow" in allow_reason_body:
        add(
            "R105_POLICY_RULE_AUDIT_REASON_BOUNDARY",
            policy_engine,
            line_number(
                text,
                allow_reason_start
                + allow_reason_body.find("PolicyDecisionReason::ExplicitGrantAllow"),
            ),
            "typed system policy rules must not be audited as durable ExplicitGrantAllow",
        )
    verified_authority_start = text.find("if input.verified_authority_id.is_some()")
    verified_authority_end = (
        text.find("let Some(owner_user_ura)", verified_authority_start)
        if verified_authority_start != -1
        else -1
    )
    verified_authority_body = (
        text[verified_authority_start:verified_authority_end]
        if verified_authority_start != -1 and verified_authority_end != -1
        else ""
    )
    if "PolicyDecisionReason::AuthorityProofAllow" not in verified_authority_body:
        add(
            "R105_POLICY_RULE_AUDIT_REASON_BOUNDARY",
            policy_engine,
            line_number(text, verified_authority_start if verified_authority_start != -1 else 0),
            "verified authority proof admission must use AuthorityProofAllow, not ExplicitGrantAllow",
        )

owner_resolution = cli_root / "src/daemon/invocation/admission/owner_resolution.rs"
if owner_resolution.exists():
    text = source(owner_resolution)
    owner_fact_start = text.find("pub struct OwnerFact")
    owner_fact_end = (
        text.find("\n}\n\nimpl OwnerFact", owner_fact_start)
        if owner_fact_start != -1
        else -1
    )
    owner_fact_body = (
        text[owner_fact_start:owner_fact_end]
        if owner_fact_start != -1 and owner_fact_end != -1
        else ""
    )
    if "pub owner_user_ura: Option<String>" not in owner_fact_body:
        add(
            "R104_OWNER_FACT_USER_URA_BOUNDARY",
            owner_resolution,
            1,
            "OwnerFact must carry canonical owner_user_ura, not bare owner_user_id",
        )
    if "pub owner_user_id:" in owner_fact_body:
        add(
            "R104_OWNER_FACT_USER_URA_BOUNDARY",
            owner_resolution,
            1,
            "OwnerFact must not expose owner_user_id because that name admits bare id confusion",
        )
    if "pub fn user_ura(" not in text:
        add(
            "R104_OWNER_FACT_USER_URA_BOUNDARY",
            owner_resolution,
            1,
            "OwnerFact constructor must require a canonical User URA boundary",
        )
    if "OwnerFact::user(" in text:
        add(
            "R104_OWNER_FACT_USER_URA_BOUNDARY",
            owner_resolution,
            1,
            "OwnerFact::user must not be reintroduced; use OwnerFact::user_ura",
        )

terminal_owner_files = (
    cli_root / "src/daemon/ability/builtins/device_control/terminal/lifecycle.rs",
    cli_root / "src/daemon/ability/builtins/device_control/terminal/io.rs",
    cli_root / "src/daemon/ability/builtins/device_control/terminal/attach.rs",
)
for path in terminal_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R106_TERMINAL_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "terminal.* abilities must be owned by the device-sponsored terminal SystemAgent, not direct Device",
        )

profiles_mod = cli_root / "src/daemon/ability/catalog/profiles/mod.rs"
device_control_names = cli_root / "src/daemon/ability/names/device_control.rs"
dispatch_rs = cli_root / "src/daemon/ability/dispatch.rs"
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "struct SystemAgentDescriptorProjection",
            "SystemAgent host descriptor projection must be declared through one registry entry type",
        ),
        (
            "const SYSTEM_AGENT_DESCRIPTOR_PROJECTIONS",
            "SystemAgent host descriptor projection must be table-driven",
        ),
        (
            "fn system_agent_descriptor_projections_for_device(",
            "SystemAgent host descriptor aggregation must use one registry iterator",
        ),
        (
            "system_agent_descriptor_projections_for_device(device_ura)",
            "host profile aggregation must consume the SystemAgent projection registry",
        ),
    ):
        if token not in text:
            add("R136_SYSTEM_AGENT_DESCRIPTOR_PROJECTION_REGISTRY", profiles_mod, 1, detail)
    wrapper_pattern = re.compile(
        r"\bfn\s+(?!system_agent_descriptors_for_device\b)"
        r"(?!system_agent_descriptor_projections_for_device\b)"
        r"\w+_system_descriptors_for_device\s*\("
    )
    wrapper_match = wrapper_pattern.search(text)
    if wrapper_match:
        add(
            "R136_SYSTEM_AGENT_DESCRIPTOR_PROJECTION_REGISTRY",
            profiles_mod,
            line_number(text, wrapper_match.start()),
            "SystemAgent descriptor projection must not keep one wrapper function per family; add a registry entry instead",
        )
    for rule, (system_agent_id, owner_constructor) in PROFILE_PROJECTION_RULES.items():
        if system_agent_id not in text or owner_constructor not in text:
            add(
                "R136_SYSTEM_AGENT_DESCRIPTOR_PROJECTION_REGISTRY",
                profiles_mod,
                1,
                f"SystemAgent projection registry missing {system_agent_id} / {owner_constructor} for {rule}",
            )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "terminal_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include terminal SystemAgent descriptors",
        ),
        (
            "fn terminal_system_descriptors_for_device(",
            "terminal SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R106_TERMINAL_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
if device_control_names.exists():
    text = source(device_control_names)
    if "pub const TERMINAL_SYSTEM_AGENT_ID" not in text:
        add(
            "R106_TERMINAL_SYSTEM_AGENT_OWNER",
            device_control_names,
            1,
            "terminal SystemAgent id must be named in the device-control ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn terminal_system() -> Self" not in text:
        add(
            "R106_TERMINAL_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical terminal SystemAgent constructor",
        )
route_resolver = cli_root / "src/daemon/invocation/routing/route_resolver.rs"
if route_resolver.exists():
    text = source(route_resolver)
    for token, detail in (
        (
            '"system-agent" => Ok(Self::SystemAgent)',
            "route selector must route SystemAgent-owned Ability URAs explicitly",
        ),
        (
            "RouteOwnerKind::Agent | RouteOwnerKind::SystemAgent",
            "local runtime routing must treat SystemAgent as a routable local Agent owner",
        ),
        (
            "descriptor_public_ability_name(owner_ura, ability_name)",
            "owner-local route queries must preserve SystemAgent public ability prefixes",
        ),
    ):
        if token not in text:
            add("R106_TERMINAL_SYSTEM_AGENT_OWNER", route_resolver, 1, detail)
    for token, detail in (
        (
            '"device_hosted_ability"',
            "final-route Device-hosted execution arm must be named as host/callee split, not direct Device ability",
        ),
        (
            '"host_device_ura"',
            "final-route Device-hosted execution arm must expose Device only as host_device_ura",
        ),
        (
            '"callee_ura"',
            "final-route Device-hosted execution arm must carry an explicit routable callee",
        ),
    ):
        if token not in text:
            add("R154_FINAL_ROUTE_DEVICE_HOSTED_ARM", route_resolver, 1, detail)
    offset = text.find('"local_device_ability"')
    if offset != -1:
        add(
            "R154_FINAL_ROUTE_DEVICE_HOSTED_ARM",
            route_resolver,
            line_number(text, offset),
            "final-route next_hop must not use the obsolete local_device_ability arm",
        )
    for token, detail in (
        (
            "direct Device-owned catalog projections are not dispatchable public routes",
            "route projection must reject direct Device-owned catalog rows as public routes",
        ),
        (
            "migrate the descriptor owner to a device-sponsored SystemAgent",
            "route projection rejection must direct dynamic/device-hosted abilities to SystemAgent ownership",
        ),
    ):
        if token not in text:
            add("R155_NO_SAME_REALM_DEVICE_ROUTE_KIND", route_resolver, 1, detail)
    offset = text.find("SelectedRouteKind::SameRealmDevice")
    if offset != -1:
        add(
            "R155_NO_SAME_REALM_DEVICE_ROUTE_KIND",
            route_resolver,
            line_number(text, offset),
            "route resolver must not preserve SameRealmDevice as a direct Device owner/callee dispatch kind",
        )
    if "RoutableAgentOwned" not in text:
        add(
            "R157_ROUTABLE_AGENT_ROUTE_KIND",
            route_resolver,
            1,
            "daemon-internal selected route kind must name Agent/SystemAgent callees as routable Agent-family owners",
        )
    offset = text.find("SelectedRouteKind::HostedAgent")
    if offset != -1:
        add(
            "R157_ROUTABLE_AGENT_ROUTE_KIND",
            route_resolver,
            line_number(text, offset),
            "daemon-internal selected route kind must not call Agent/SystemAgent owner selection HostedAgent; keep HostedAgent only as wire/product projection",
        )

key_resolver_rs = cli_root / "src/daemon/trust/key_resolver.rs"
if key_resolver_rs.exists():
    text = source(key_resolver_rs)
    raw_text = key_resolver_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "sponsor_device_ura_for_system_agent",
            "trust key resolver must document the SystemAgent -> sponsor Device key-custody projection",
        ),
        (
            "parsed.device_agent_ids()",
            "SystemAgent key-custody projection must use the canonical URA parser",
        ),
    ):
        if token not in text:
            add("R158_SYSTEM_AGENT_RECEIPT_KEY_CUSTODY", key_resolver_rs, 1, detail)
    for token, detail in (
        (
            "resolve_system_agent_uses_sponsor_device_key",
            "trust key resolver must test SystemAgent signer resolution through its sponsor Device",
        ),
        (
            "resolve_user_hosted_agent_does_not_use_device_sponsor_fallback",
            "ordinary hosted Agents must not inherit Device trust keys",
        ),
    ):
        if token not in raw_text:
            add("R158_SYSTEM_AGENT_RECEIPT_KEY_CUSTODY", key_resolver_rs, 1, detail)

receipt_signing_rs = cli_root / "src/daemon/identity/receipt_signing.rs"
if receipt_signing_rs.exists():
    text = source(receipt_signing_rs)
    raw_text = receipt_signing_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "system_agent_sponsor_device_identity",
            "production receipt signing must classify SystemAgent callees through their sponsor Device",
        ),
        (
            "system_agent_sponsor_device",
            "production receipt signing must fail closed unless the sponsor Device signer is owner-bound",
        ),
        (
            "hosted_agent_device.as_ref()",
            "production SystemAgent custody must be limited to the configured hosted Device sponsor",
        ),
        (
            "is_declared_daemon_native_system_agent_id",
            "production SystemAgent custody must be limited to declared daemon-native SystemAgent ids",
        ),
        (
            "system_agent_authority",
            "production receipt signing must project SystemAgent callee receipts to sponsor Device signer authority",
        ),
        (
            "system_agent_sponsor_device(caller_ura)",
            "production child Invocation signing must project SystemAgent callers to sponsor Device key custody",
        ),
        (
            "resolve_invocation_verifying_key",
            "production child Invocation verification must project SystemAgent callers to sponsor Device verification keys",
        ),
    ):
        if token not in text:
            add("R158_SYSTEM_AGENT_RECEIPT_KEY_CUSTODY", receipt_signing_rs, 1, detail)
    for token, detail in (
        (
            "system_agent_receipt_signer_uses_sponsor_device_key_custody",
            "receipt-signing tests must prove SystemAgent receipts are signed by sponsor Device",
        ),
        (
            "system_agent_invocation_signing_and_verification_use_sponsor_device",
            "receipt-signing tests must prove SystemAgent child Invocation signing and verification use sponsor Device keys",
        ),
        (
            "unknown_system_agent_id_has_no_sponsor_device_custody",
            "receipt-signing tests must prove undeclared SystemAgent ids do not receive Device key custody",
        ),
        (
            "foreign_system_agent_sponsor_does_not_inherit_local_device_key",
            "receipt-signing tests must prove foreign Device-sponsored SystemAgents do not inherit local Device keys",
        ),
    ):
        if token not in raw_text:
            add("R158_SYSTEM_AGENT_RECEIPT_KEY_CUSTODY", receipt_signing_rs, 1, detail)

runtime_factory_rs = cli_root / "src/daemon/axon_bridge/runtime_factory.rs"
if runtime_factory_rs.exists():
    text = source(runtime_factory_rs)
    raw_text = runtime_factory_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "EphemeralTestReceiptSigningAuthority::for_callee",
            "test receipt provider must select hosted signing authority by callee ontology",
        ),
        (
            "sponsor_device_identity_for_system_agent",
            "test receipt provider must project device-sponsored SystemAgent callee to sponsor Device signer",
        ),
        (
            "canonical_host_attestation_bytes",
            "hosted SystemAgent test receipts must carry canonical host attestation",
        ),
        (
            "ephemeral_receipt_provider_uses_sponsor_device_for_system_agent",
            "test receipt provider must prove SystemAgent receipts are signed by sponsor Device",
        ),
    ):
        if token not in raw_text:
            add("R158_SYSTEM_AGENT_RECEIPT_KEY_CUSTODY", runtime_factory_rs, 1, detail)

dispatch_selected_route_fixtures = (
    cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs",
    cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests/local_rpc.rs",
    cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs",
    cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests/stream.rs",
)
for path in dispatch_selected_route_fixtures:
    if not path.exists():
        continue
    text = source(path)
    required_tokens = [
        (
            "catalog_test_descriptor_ref(",
            "selected-route descriptor refs must be catalog-bound and owner-scoped",
        )
    ]
    if path.name != "stream.rs":
        required_tokens.append(
            (
                "test_dispatch_system_agent_ura",
                "selected-route positive fixtures must use a device-sponsored SystemAgent owner/callee helper",
            )
        )
    for token, detail in required_tokens:
        if token not in text:
            add("R158_SYSTEM_AGENT_RECEIPT_KEY_CUSTODY", path, 1, detail)
    for token, detail in (
        (
            "owner_ability_ura(TEST_DAEMON_URA, \"demo.",
            "positive selected-route demo fixtures must not use direct Device ability owners",
        ),
        (
            "publish_test_route(&svc, TEST_DAEMON_URA, \"demo.",
            "positive selected-route demo fixtures must not publish direct Device-owned demo routes",
        ),
        (
            "catalog_with_json_echo_on_runtime(\n        TARGET_DEVICE_URA",
            "remote selected-route stream fixture must use a SystemAgent callee owner, not target Device",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add("R158_SYSTEM_AGENT_RECEIPT_KEY_CUSTODY", path, line_number(text, offset), detail)

locomotion_owner_files = (
    cli_root / "src/daemon/ability/builtins/device_control/files.rs",
    cli_root / "src/daemon/ability/builtins/device_control/file_edit.rs",
    cli_root / "src/daemon/ability/builtins/device_control/process.rs",
    cli_root / "src/daemon/ability/builtins/device_control/shell.rs",
    cli_root / "src/daemon/ability/builtins/device_control/http.rs",
    cli_root / "src/daemon/ability/builtins/device_control/file_transfer.rs",
)
for path in locomotion_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R107_LOCOMOTION_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "locomotion abilities must be owned by the device-sponsored locomotion SystemAgent, not direct Device",
        )
    if "OwnerKind::locomotion_system()" not in text:
        add(
            "R107_LOCOMOTION_SYSTEM_AGENT_OWNER",
            path,
            1,
            "locomotion ability registration must use the canonical OwnerKind::locomotion_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "locomotion_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include locomotion SystemAgent descriptors",
        ),
        (
            "fn locomotion_system_descriptors_for_device(",
            "locomotion SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R107_LOCOMOTION_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
if device_control_names.exists():
    text = source(device_control_names)
    if "pub const LOCOMOTION_SYSTEM_AGENT_ID" not in text:
        add(
            "R107_LOCOMOTION_SYSTEM_AGENT_OWNER",
            device_control_names,
            1,
            "locomotion SystemAgent id must be named in the device-control ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn locomotion_system() -> Self" not in text:
        add(
            "R107_LOCOMOTION_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical locomotion SystemAgent constructor",
        )

skill_owner_files = (
    cli_root / "src/daemon/ability/builtins/resources/skills/install.rs",
    cli_root / "src/daemon/ability/builtins/resources/skills/publish.rs",
)
for path in skill_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R108_SKILL_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "skill.* management abilities must be owned by the device-sponsored skill-management SystemAgent, not direct Device",
        )
    if "OwnerKind::skill_management_system()" not in text:
        add(
            "R108_SKILL_SYSTEM_AGENT_OWNER",
            path,
            1,
            "skill.* management ability registration must use the canonical OwnerKind::skill_management_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "skill_management_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include skill-management SystemAgent descriptors",
        ),
        (
            "fn skill_management_system_descriptors_for_device(",
            "skill-management SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R108_SKILL_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
resources_names = cli_root / "src/daemon/ability/names/resources.rs"
if resources_names.exists():
    text = source(resources_names)
    if "pub const SKILL_MANAGEMENT_SYSTEM_AGENT_ID" not in text:
        add(
            "R108_SKILL_SYSTEM_AGENT_OWNER",
            resources_names,
            1,
            "skill-management SystemAgent id must be named in the resource ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn skill_management_system() -> Self" not in text:
        add(
            "R108_SKILL_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical skill-management SystemAgent constructor",
        )

context_owner_files = (
    cli_root / "src/daemon/ability/builtins/resources/context/ability.rs",
)
for path in context_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R109_CONTEXT_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "context.* abilities must be owned by the device-sponsored context SystemAgent, not direct Device",
        )
    if "OwnerKind::context_system()" not in text:
        add(
            "R109_CONTEXT_SYSTEM_AGENT_OWNER",
            path,
            1,
            "context.* ability registration must use the canonical OwnerKind::context_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "context_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include context SystemAgent descriptors",
        ),
        (
            "fn context_system_descriptors_for_device(",
            "context SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R109_CONTEXT_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
if resources_names.exists():
    text = source(resources_names)
    if "pub const CONTEXT_SYSTEM_AGENT_ID" not in text:
        add(
            "R109_CONTEXT_SYSTEM_AGENT_OWNER",
            resources_names,
            1,
            "context SystemAgent id must be named in the resource ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn context_system() -> Self" not in text:
        add(
            "R109_CONTEXT_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical context SystemAgent constructor",
        )

media_owner_files = (
    cli_root / "src/daemon/ability/builtins/resources/media/mic_subscribe.rs",
    cli_root / "src/daemon/ability/builtins/resources/media/camera_snapshot.rs",
    cli_root / "src/daemon/ability/builtins/resources/media/screen_snapshot.rs",
)
for path in media_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R110_MEDIA_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "local sensor media abilities must be owned by the device-sponsored media SystemAgent, not direct Device",
        )
    if "OwnerKind::media_system()" not in text:
        add(
            "R110_MEDIA_SYSTEM_AGENT_OWNER",
            path,
            1,
            "local sensor media registration must use the canonical OwnerKind::media_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "media_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include media SystemAgent descriptors",
        ),
        (
            "fn media_system_descriptors_for_device(",
            "media SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R110_MEDIA_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
if resources_names.exists():
    text = source(resources_names)
    if "pub const MEDIA_SYSTEM_AGENT_ID" not in text:
        add(
            "R110_MEDIA_SYSTEM_AGENT_OWNER",
            resources_names,
            1,
            "media SystemAgent id must be named in the resource ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn media_system() -> Self" not in text:
        add(
            "R110_MEDIA_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical media SystemAgent constructor",
        )

plugin_owner_files = (
    cli_root / "src/daemon/ability/builtins/integrations/plugins.rs",
)
for path in plugin_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R111_PLUGIN_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "plugin lifecycle abilities must be owned by the device-sponsored plugin-management SystemAgent, not direct Device",
        )
    if "OwnerKind::plugin_management_system()" not in text:
        add(
            "R111_PLUGIN_SYSTEM_AGENT_OWNER",
            path,
            1,
            "plugin lifecycle ability registration must use the canonical OwnerKind::plugin_management_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "plugin_management_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include plugin-management SystemAgent descriptors",
        ),
        (
            "fn plugin_management_system_descriptors_for_device(",
            "plugin-management SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R111_PLUGIN_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
integrations_names = cli_root / "src/daemon/ability/names/integrations.rs"
if integrations_names.exists():
    text = source(integrations_names)
    if "pub const PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID" not in text:
        add(
            "R111_PLUGIN_SYSTEM_AGENT_OWNER",
            integrations_names,
            1,
            "plugin-management SystemAgent id must be named in the integration ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn plugin_management_system() -> Self" not in text:
        add(
            "R111_PLUGIN_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical plugin-management SystemAgent constructor",
        )

automation_owner_files = (
    cli_root / "src/daemon/ability/builtins/automation/loop_ability.rs",
    cli_root / "src/daemon/ability/builtins/automation/schedule.rs",
    cli_root / "src/daemon/ability/builtins/automation/discuss.rs",
    cli_root / "src/daemon/ability/builtins/automation/mission.rs",
    cli_root / "src/daemon/ability/builtins/automation/think.rs",
    cli_root / "src/daemon/ability/builtins/automation/orchestration.rs",
)
for path in automation_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R115_AUTOMATION_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "automation abilities must be owned by the device-sponsored automation SystemAgent, not direct Device",
        )
    registers_automation_ability = any(
        token in text
        for token in (
            "register_rpc_with_owner",
            "register_stream_with_owner",
            "register_rpc_with_envelope_and_owner",
            "register_rpc_with_owner_and_action",
        )
    )
    if registers_automation_ability and "OwnerKind::automation_system()" not in text:
        add(
            "R115_AUTOMATION_SYSTEM_AGENT_OWNER",
            path,
            1,
            "automation ability registration must use the canonical OwnerKind::automation_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "automation_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include automation SystemAgent descriptors",
        ),
        (
            "fn automation_system_descriptors_for_device(",
            "automation SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R115_AUTOMATION_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
automation_names = cli_root / "src/daemon/ability/names/automation.rs"
if automation_names.exists():
    text = source(automation_names)
    if "pub const AUTOMATION_SYSTEM_AGENT_ID" not in text:
        add(
            "R115_AUTOMATION_SYSTEM_AGENT_OWNER",
            automation_names,
            1,
            "automation SystemAgent id must be named in the automation ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn automation_system() -> Self" not in text:
        add(
            "R115_AUTOMATION_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical automation SystemAgent constructor",
        )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if "automation_system_agent_ability_resolves_from_local_device_authority" not in raw_text:
        add(
            "R115_AUTOMATION_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove automation SystemAgent callee with Device execution host",
        )

session_owner_files = (
    cli_root / "src/daemon/ability/builtins/device_control/session.rs",
)
for path in session_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R116_SESSION_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "session.list/session.attach must be owned by the device-sponsored session SystemAgent, not direct Device",
        )
    registers_session_ability = any(
        token in text
        for token in (
            "register_rpc_with_owner",
            "register_stream_with_owner",
            "register_rpc_with_envelope_and_owner",
            "register_rpc_with_owner_and_action",
        )
    )
    if registers_session_ability and "OwnerKind::session_system()" not in text:
        add(
            "R116_SESSION_SYSTEM_AGENT_OWNER",
            path,
            1,
            "session ability registration must use the canonical OwnerKind::session_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "session_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include session SystemAgent descriptors",
        ),
        (
            "fn session_system_descriptors_for_device(",
            "session SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R116_SESSION_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
device_control_names = cli_root / "src/daemon/ability/names/device_control.rs"
if device_control_names.exists():
    text = source(device_control_names)
    if "pub const SESSION_SYSTEM_AGENT_ID" not in text:
        add(
            "R116_SESSION_SYSTEM_AGENT_OWNER",
            device_control_names,
            1,
            "session SystemAgent id must be named in the device-control ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn session_system() -> Self" not in text:
        add(
            "R116_SESSION_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical session SystemAgent constructor",
        )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if "session_system_agent_ability_resolves_from_local_device_authority" not in raw_text:
        add(
            "R116_SESSION_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove session SystemAgent callee with Device execution host",
        )

runtime_governance_owner_files = (
    cli_root / "src/daemon/ability/builtins/governance/access_control.rs",
)
for path in runtime_governance_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R117_RUNTIME_GOVERNANCE_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "authority.binding.*, policy.request.*, and admission.explain must be owned by the device-sponsored runtime-governance SystemAgent, not direct Device",
        )
    registers_runtime_governance_ability = any(
        token in text
        for token in (
            "AUTHORITY_BINDING_GRANT",
            "POLICY_REQUEST_CREATE",
            "ADMISSION_EXPLAIN",
        )
    )
    if (
        registers_runtime_governance_ability
        and "OwnerKind::runtime_governance_system()" not in text
    ):
        add(
            "R117_RUNTIME_GOVERNANCE_SYSTEM_AGENT_OWNER",
            path,
            1,
            "runtime governance ability registration must use the canonical OwnerKind::runtime_governance_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "runtime_governance_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include runtime-governance SystemAgent descriptors",
        ),
        (
            "fn runtime_governance_system_descriptors_for_device(",
            "runtime-governance SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R117_RUNTIME_GOVERNANCE_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
governance_names = cli_root / "src/daemon/ability/names/governance.rs"
if governance_names.exists():
    text = source(governance_names)
    if "pub const RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID" not in text:
        add(
            "R117_RUNTIME_GOVERNANCE_SYSTEM_AGENT_OWNER",
            governance_names,
            1,
            "runtime-governance SystemAgent id must be named in the governance ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn runtime_governance_system() -> Self" not in text:
        add(
            "R117_RUNTIME_GOVERNANCE_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical runtime-governance SystemAgent constructor",
        )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if (
        "runtime_governance_system_agent_ability_resolves_from_local_device_authority"
        not in raw_text
    ):
        add(
            "R117_RUNTIME_GOVERNANCE_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove runtime-governance SystemAgent callee with Device execution host",
        )

invocation_governance_owner_files = (
    cli_root / "src/daemon/ability/builtins/governance/invocation_history.rs",
    cli_root / "src/daemon/ability/builtins/governance/invocation_cancel.rs",
)
for path in invocation_governance_owner_files:
    if not path.exists():
        continue
    text = source(path)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R122_INVOCATION_GOVERNANCE_SYSTEM_AGENT_OWNER",
            path,
            line_number(text, offset),
            "Device-scoped invocation history/cancel abilities must be owned by runtime-governance SystemAgent, not direct Device",
        )
if (cli_root / "src/daemon/ability/builtins/governance/invocation_cancel.rs").exists():
    invocation_cancel_text = source(
        cli_root / "src/daemon/ability/builtins/governance/invocation_cancel.rs"
    )
    if "OwnerKind::runtime_governance_system()" not in invocation_cancel_text:
        add(
            "R122_INVOCATION_GOVERNANCE_SYSTEM_AGENT_OWNER",
            cli_root / "src/daemon/ability/builtins/governance/invocation_cancel.rs",
            1,
            "invocation.cancel Device-side registration must use runtime-governance SystemAgent",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    ledger_governance_offset = text.find("pub(crate) fn ledger_governance_owner")
    if ledger_governance_offset == -1:
        add(
            "R122_INVOCATION_GOVERNANCE_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "AbilityAuthorityContext must centralize ledger governance owner projection",
        )
    else:
        ledger_governance_body = text[ledger_governance_offset : ledger_governance_offset + 500]
        if "OwnerKind::runtime_governance_system()" not in ledger_governance_body:
            add(
                "R122_INVOCATION_GOVERNANCE_SYSTEM_AGENT_OWNER",
                dispatch_rs,
                line_number(text, ledger_governance_offset),
                "Device ledger governance owner must project to runtime-governance SystemAgent",
            )
invocation_governance_device_profile = (
    cli_root / "src/daemon/ability/catalog/profiles/device.rs"
)
if invocation_governance_device_profile.exists():
    text = source(invocation_governance_device_profile)
    direct_inventory_offset = text.find("fn direct_device_owner_inventory_is_explicit")
    direct_inventory_body = (
        text[direct_inventory_offset : direct_inventory_offset + 1200]
        if direct_inventory_offset != -1
        else text
    )
    for token in (
        '"invocation.cancel"',
        '"invocation.history.get"',
        '"invocation.history.list"',
        '"invocation.history.path"',
        '"invocation.record.get"',
        '"invocation.trace.get"',
    ):
        offset = direct_inventory_body.find(token)
        if offset != -1:
            add(
                "R122_INVOCATION_GOVERNANCE_SYSTEM_AGENT_OWNER",
                invocation_governance_device_profile,
                line_number(text, direct_inventory_offset + offset),
                "invocation governance abilities must not remain in direct Device profile inventory",
            )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if (
        "invocation_governance_system_agent_ability_resolves_from_local_device_authority"
        not in raw_text
    ):
        add(
            "R122_INVOCATION_GOVERNANCE_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove invocation governance SystemAgent callee with Device execution host",
        )

runtime_health_owner_files = (
    cli_root / "src/daemon/ability/builtins/governance/health.rs",
    cli_root / "src/daemon/ability/builtins/governance/network_health.rs",
    cli_root / "src/daemon/ability/builtins/governance/admin_status.rs",
)
runtime_health_tokens = (
    "OBSERVE_HEALTH",
    "OBSERVE_NETWORK_HEALTH",
    "ADMIN_STATUS",
    '"observe.health"',
    '"observe.network_health"',
    '"admin.status"',
)
for path in runtime_health_owner_files:
    if not path.exists():
        continue
    text = source(path)
    for token in runtime_health_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            window = text[offset : offset + 700]
            if "OwnerKind::Device" in window and "register_rpc_with_owner" in window:
                add(
                    "R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER",
                    path,
                    line_number(text, offset),
                    "observe.health, observe.network_health, and admin.status must be owned by the device-sponsored runtime-health SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_runtime_health = any(token in text for token in runtime_health_tokens) and (
        "register_rpc_with_owner" in text
    )
    if registers_runtime_health and "OwnerKind::runtime_health_system()" not in text:
        add(
            "R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER",
            path,
            1,
            "runtime health/admin registration must use the canonical OwnerKind::runtime_health_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "runtime_health_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include runtime-health SystemAgent descriptors",
        ),
        (
            "fn runtime_health_system_descriptors_for_device(",
            "runtime-health SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
governance_names = cli_root / "src/daemon/ability/names/governance.rs"
if governance_names.exists():
    text = source(governance_names)
    if "pub const RUNTIME_HEALTH_SYSTEM_AGENT_ID" not in text:
        add(
            "R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER",
            governance_names,
            1,
            "runtime-health SystemAgent id must be named in the governance ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn runtime_health_system() -> Self" not in text:
        add(
            "R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical runtime-health SystemAgent constructor",
        )
if invocation_governance_device_profile.exists():
    text = source(invocation_governance_device_profile)
    direct_inventory_offset = text.find("fn direct_device_owner_inventory_is_explicit")
    direct_inventory_body = (
        text[direct_inventory_offset : direct_inventory_offset + 1200]
        if direct_inventory_offset != -1
        else text
    )
    for token in (
        '"observe.health"',
        '"observe.network_health"',
        '"admin.status"',
    ):
        offset = direct_inventory_body.find(token)
        if offset != -1:
            add(
                "R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER",
                invocation_governance_device_profile,
                line_number(text, direct_inventory_offset + offset),
                "runtime health/admin abilities must not remain in direct Device profile inventory",
            )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if (
        "runtime_health_system_agent_ability_resolves_from_local_device_authority"
        not in raw_text
    ):
        add(
            "R123_RUNTIME_HEALTH_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove runtime-health SystemAgent callee with Device execution host",
        )

node_management_owner = cli_root / "src/daemon/ability/builtins/device_control/ability_management/ops.rs"
node_management_tokens = (
    "ABILITY_DESCRIBE_NODE",
    "ABILITY_REMOVE_NODE",
    "NODE_DESCRIBE",
    "NODE_REMOVE",
    '"node.describe"',
    '"node.remove"',
)
if node_management_owner.exists():
    text = source(node_management_owner)
    for token in node_management_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            window = text[offset : offset + 700]
            if "OwnerKind::Device" in window and (
                "register_rpc_with_owner" in window
                or "register_rpc_with_spec" in window
            ):
                add(
                    "R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER",
                    node_management_owner,
                    line_number(text, offset),
                    "node.describe and node.remove must be owned by the device-sponsored node-management SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_node_management = any(token in text for token in node_management_tokens) and (
        "register_rpc_with_owner" in text or "register_rpc_with_spec" in text
    )
    if registers_node_management and "OwnerKind::node_management_system()" not in text:
        add(
            "R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER",
            node_management_owner,
            1,
            "node lifecycle registration must use the canonical OwnerKind::node_management_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "node_management_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include node-management SystemAgent descriptors",
        ),
        (
            "fn node_management_system_descriptors_for_device(",
            "node-management SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
device_control_names = cli_root / "src/daemon/ability/names/device_control.rs"
if device_control_names.exists():
    text = source(device_control_names)
    if "pub const NODE_MANAGEMENT_SYSTEM_AGENT_ID" not in text:
        add(
            "R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER",
            device_control_names,
            1,
            "node-management SystemAgent id must be named in the device-control ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn node_management_system() -> Self" not in text:
        add(
            "R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical node-management SystemAgent constructor",
        )
if invocation_governance_device_profile.exists():
    text = source(invocation_governance_device_profile)
    direct_inventory_offset = text.find("fn direct_device_owner_inventory_is_explicit")
    direct_inventory_body = (
        text[direct_inventory_offset : direct_inventory_offset + 1200]
        if direct_inventory_offset != -1
        else text
    )
    for token in ('"node.describe"', '"node.remove"'):
        offset = direct_inventory_body.find(token)
        if offset != -1:
            add(
                "R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER",
                invocation_governance_device_profile,
                line_number(text, direct_inventory_offset + offset),
                "node lifecycle abilities must not remain in direct Device profile inventory",
            )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if (
        "node_management_system_agent_ability_resolves_from_local_device_authority"
        not in raw_text
    ):
        add(
            "R124_NODE_MANAGEMENT_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove node-management SystemAgent callee with Device execution host",
        )

runtime_introspection_owner_files = (
    cli_root / "src/daemon/ability/builtins/governance/meta.rs",
    cli_root / "src/daemon/ability/builtins/resources/list.rs",
)
runtime_introspection_tokens = (
    "ABILITY_DESCRIBE",
    "ABILITY_LIST_ABILITIES",
    "ABILITY_META_LIST_RESOURCES",
    "META_DESCRIBE",
    "META_LIST_ABILITIES",
    "META_LIST_RESOURCES",
    '"meta.describe"',
    '"meta.list_abilities"',
    '"meta.list_resources"',
)
for path in runtime_introspection_owner_files:
    if not path.exists():
        continue
    text = source(path)
    for token in runtime_introspection_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            statement_start = text.rfind(";", 0, offset) + 1
            statement_end = text.find(";", offset)
            statement = text[
                statement_start : statement_end + 1 if statement_end != -1 else len(text)
            ]
            if "OwnerKind::Device" in statement and (
                "register_rpc_with_owner" in statement
                or "register_rpc_with_envelope_and_owner" in statement
                or "register_rpc_with_spec" in statement
            ):
                add(
                    "R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER",
                    path,
                    line_number(text, offset),
                    "meta.describe/list_abilities/list_resources must be owned by runtime-introspection SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_runtime_introspection = any(
        token in text for token in runtime_introspection_tokens
    ) and (
        "register_rpc_with_owner" in text
        or "register_rpc_with_envelope_and_owner" in text
        or "register_rpc_with_spec" in text
    )
    if (
        registers_runtime_introspection
        and "OwnerKind::runtime_introspection_system()" not in text
        and "runtime_introspection_system()" not in text
    ):
        add(
            "R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER",
            path,
            1,
            "runtime introspection read registration must use the canonical OwnerKind::runtime_introspection_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "out.extend(runtime_introspection_system_descriptors_for_device",
            "host profile aggregation must include runtime-introspection SystemAgent descriptors",
        ),
        (
            "fn runtime_introspection_system_descriptors_for_device(",
            "runtime-introspection SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
governance_names = cli_root / "src/daemon/ability/names/governance.rs"
if governance_names.exists():
    text = source(governance_names)
    if "pub const RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID" not in text:
        add(
            "R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER",
            governance_names,
            1,
            "runtime-introspection SystemAgent id must be named in the governance ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn runtime_introspection_system() -> Self" not in text:
        add(
            "R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical runtime-introspection SystemAgent constructor",
        )
if invocation_governance_device_profile.exists():
    text = source(invocation_governance_device_profile)
    direct_inventory_offset = text.find("fn direct_device_owner_inventory_is_explicit")
    direct_inventory_body = (
        text[direct_inventory_offset : direct_inventory_offset + 1200]
        if direct_inventory_offset != -1
        else text
    )
    for token in ('"meta.describe"', '"meta.list_abilities"', '"meta.list_resources"'):
        offset = direct_inventory_body.find(token)
        if offset != -1:
            add(
                "R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER",
                invocation_governance_device_profile,
                line_number(text, direct_inventory_offset + offset),
                "runtime introspection read abilities must not remain in direct Device profile inventory",
            )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if (
        "runtime_introspection_system_agent_ability_resolves_from_local_device_authority"
        not in raw_text
    ):
        add(
            "R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove runtime-introspection SystemAgent callee with Device execution host",
        )
target_rs = cli_root / "src/daemon/invocation/routing/target.rs"
if target_rs.exists():
    raw_text = target_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "runtime_introspection_subject_ura",
            "target subject policy must derive runtime-introspection read subject from the host Device",
        ),
        (
            "runtime_introspection_subject_projects_system_agent_callee_to_host_device",
            "target tests must prove runtime-introspection callee and Device subject remain distinct",
        ),
    ):
        if token not in raw_text:
            add("R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER", target_rs, 1, detail)
remote_invoke_rs = cli_root / "src/daemon/invocation/routing/remote_invoke.rs"
if remote_invoke_rs.exists():
    raw_text = remote_invoke_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "runtime_introspection_owner_for_execution_target",
            "remote catalogue reads must bind Device execution targets to runtime-introspection SystemAgent owners",
        ),
        (
            "remote_catalogue_read_issuer_uses_read_projection_subject",
            "remote catalogue read tests must prove subject remains the execution target, not the SystemAgent callee",
        ),
    ):
        if token not in raw_text:
            add("R125_RUNTIME_INTROSPECTION_SYSTEM_AGENT_OWNER", remote_invoke_rs, 1, detail)
    for token, detail in (
        (
            "remote_target_owned_selector_owner_ura(execution_target_ura, selector)",
            "target-owned remote selectors must project descriptor owner from execution target and selector before deriving Ability URA",
        ),
        (
            "fn remote_target_owned_selector_owner_ura(",
            "remote selector owner projection must be centralized in one helper",
        ),
        (
            "execution_target_owner_ura_for_public_ability(",
            "Device target selector projection must consume the centralized execution-target owner projection",
        ),
        (
            "RemoteRootAbilityAdmission::evaluate(&public_ability).require(&public_ability)?",
            "target-owned selector admission must run on the public ability before owner projection",
        ),
        (
            "target_owned_selector_projects_device_target_to_system_agent_callee",
            "remote invoke tests must prove Device target selectors project to SystemAgent callees",
        ),
        (
            "remote_execution_validation_rejects_direct_device_owned_ability_on_device_target",
            "remote invoke tests must prove direct Device-owned Ability URAs are migration-only",
        ),
        (
            '(URAKind::Device, "device") => bail!',
            "remote execution validation must reject direct Device-owned Ability URAs on Device targets",
        ),
        (
            "migration-only",
            "direct Device-owned remote Ability URA rejection must name migration-only semantics",
        ),
        (
            "callee.device_agent_ids().is_some()",
            "target-owned remote subject selection must recognize device-sponsored SystemAgent callees",
        ),
        (
            "target.execution_target_ura().to_string()",
            "SystemAgent target-owned remote subjects must project to the host Device, not the SystemAgent callee",
        ),
    ):
        if token not in raw_text:
            add("R135_REMOTE_DESCRIPTOR_OWNER_CALLEE_SPLIT", remote_invoke_rs, 1, detail)
    forbidden_tokens = (
        (
            "owner_ability_ura(execution_target_ura",
            "target-owned selector factory must not derive Ability URA from Device execution target",
        ),
        (
            '(URAKind::Device, "device") => Ok',
            "remote execution validation must not admit direct Device-owned Ability URAs",
        ),
        (
            "URAKind::Device => target.callee_ura().to_string()",
            "target-owned remote subject selection must not treat Device as a routable callee",
        ),
        (
            "fn device_system_agent_id_for_public_ability(",
            "remote routing must not maintain a second public-ability-to-SystemAgent mapping table",
        ),
        (
            "match public_ability.trim()",
            "remote routing must not infer SystemAgent ownership from an ability-name match table",
        ),
    )
    for token, detail in forbidden_tokens:
        offset = raw_text.find(token)
        if offset != -1:
            add(
                "R135_REMOTE_DESCRIPTOR_OWNER_CALLEE_SPLIT",
                remote_invoke_rs,
                line_number(raw_text, offset),
                detail,
            )

descriptor_ref_rs = cli_root / "src/daemon/axon_bridge/descriptor_ref.rs"
if descriptor_ref_rs.exists():
    raw_text = descriptor_ref_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "is_declared_daemon_native_system_agent_id(",
            "descriptor-ref owner projection must reject undeclared device-scoped Agents",
        ),
        (
            "catalog_owner_rejects_undeclared_device_scoped_agent",
            "descriptor-ref tests must prove unknown device-scoped Agents cannot become SystemAgent owners",
        ),
    ):
        if token not in raw_text:
            add("R135_REMOTE_DESCRIPTOR_OWNER_CALLEE_SPLIT", descriptor_ref_rs, 1, detail)
if ffi_invocation.exists():
    raw_text = ffi_invocation.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "parse_invocation_json_rejects_device_callee_before_daemon_io",
            "FFI tests must prove public invocation JSON rejects Device callees before daemon I/O",
        ),
        (
            "URAKind::Device => Err(InvocationJsonError::InvalidInvocationRole",
            "FFI invocation ingress must reject Device callees as invalid public invocation roles",
        ),
    ):
        if token not in raw_text:
            add("R135_REMOTE_DESCRIPTOR_OWNER_CALLEE_SPLIT", ffi_invocation, 1, detail)

route_resolver_rs = cli_root / "src/daemon/invocation/routing/route_resolver.rs"
if route_resolver_rs.exists():
    raw_text = route_resolver_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "RouteOwnerKind::LegacyDeviceProfileProjection",
            "route selectors must quarantine Device-owned abilities as legacy projections",
        ),
        (
            "Device-owned Ability URAs and descriptor refs are migration read-models only",
            "route data sources must reject explicit Device-owned selectors while allowing Device placement plus a public ability to resolve through the registry",
        ),
        (
            "system_agent_ability_online_resolves_final_local_execution_route",
            "positive local routing tests must separate SystemAgent callee from Device execution host",
        ),
    ):
        if token not in raw_text:
            add("R135_REMOTE_DESCRIPTOR_OWNER_CALLEE_SPLIT", route_resolver_rs, 1, detail)

dispatch_tests_rs = cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs"
if dispatch_tests_rs.exists():
    raw_text = dispatch_tests_rs.read_text(encoding="utf-8", errors="replace")
    if "positive test routes must not make Device an ability owner/callee" not in raw_text:
        add(
            "R135_REMOTE_DESCRIPTOR_OWNER_CALLEE_SPLIT",
            dispatch_tests_rs,
            1,
            "generic positive dispatch fixtures must reject Device owner/callee inputs",
        )

descriptor_transfer_owner = cli_root / "src/daemon/ability/builtins/governance/teach.rs"
descriptor_transfer_tokens = (
    "TEACH",
    "ACQUIRE",
    "FORGET",
    "META_TEACH",
    "META_ACQUIRE",
    "META_FORGET",
    '"meta.teach"',
    '"meta.acquire"',
    '"meta.forget"',
)
if descriptor_transfer_owner.exists():
    text = source(descriptor_transfer_owner)
    for token in descriptor_transfer_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            statement_start = text.rfind(";", 0, offset) + 1
            statement_end = text.find(";", offset)
            statement = text[
                statement_start : statement_end + 1 if statement_end != -1 else len(text)
            ]
            if "OwnerKind::Device" in statement and (
                "register_rpc_with_owner" in statement
                or "register_rpc_with_envelope_and_owner" in statement
                or "register_rpc_with_spec" in statement
                or "register_rpc_with_envelope_and_spec" in statement
            ):
                add(
                    "R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER",
                    descriptor_transfer_owner,
                    line_number(text, offset),
                    "meta.teach/acquire/forget must be owned by descriptor-transfer SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_descriptor_transfer = any(
        token in text for token in descriptor_transfer_tokens
    ) and (
        "register_rpc_with_owner" in text
        or "register_rpc_with_envelope_and_owner" in text
        or "register_rpc_with_spec" in text
        or "register_rpc_with_envelope_and_spec" in text
    )
    if registers_descriptor_transfer and "OwnerKind::descriptor_transfer_system()" not in text:
        add(
            "R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER",
            descriptor_transfer_owner,
            1,
            "descriptor-transfer registration must use the canonical OwnerKind::descriptor_transfer_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "out.extend(descriptor_transfer_system_descriptors_for_device",
            "host profile aggregation must include descriptor-transfer SystemAgent descriptors",
        ),
        (
            "fn descriptor_transfer_system_descriptors_for_device(",
            "descriptor-transfer SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
if governance_names.exists():
    text = source(governance_names)
    if "pub const DESCRIPTOR_TRANSFER_SYSTEM_AGENT_ID" not in text:
        add(
            "R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER",
            governance_names,
            1,
            "descriptor-transfer SystemAgent id must be named in the governance ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn descriptor_transfer_system() -> Self" not in text:
        add(
            "R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical descriptor-transfer SystemAgent constructor",
        )
if invocation_governance_device_profile.exists():
    text = source(invocation_governance_device_profile)
    direct_inventory_offset = text.find("fn direct_device_owner_inventory_is_explicit")
    direct_inventory_body = (
        text[direct_inventory_offset : direct_inventory_offset + 1200]
        if direct_inventory_offset != -1
        else text
    )
    for token in ('"meta.teach"', '"meta.acquire"', '"meta.forget"'):
        offset = direct_inventory_body.find(token)
        if offset != -1:
            add(
                "R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER",
                invocation_governance_device_profile,
                line_number(text, direct_inventory_offset + offset),
                "descriptor-transfer abilities must not remain in direct Device profile inventory",
            )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if (
        "descriptor_transfer_system_agent_ability_resolves_from_local_device_authority"
        not in raw_text
    ):
        add(
            "R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove descriptor-transfer SystemAgent callee with Device execution host",
        )
conformance_rs = cli_root / "src/daemon/ability/conformance.rs"
if conformance_rs.exists():
    raw_text = conformance_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "DescriptorTransferSystemAgent",
            "conformance baseline must name the descriptor-transfer SystemAgent domain",
        ),
        (
            'local_rpc!("meta.acquire", DescriptorTransferSystemAgent)',
            "conformance baseline must include descriptor-transfer lifecycle abilities",
        ),
    ):
        if token not in raw_text:
            add("R126_DESCRIPTOR_TRANSFER_SYSTEM_AGENT_OWNER", conformance_rs, 1, detail)

ability_management_owner_files = (
    cli_root / "src/daemon/ability/builtins/device_control/ability_management/publish.rs",
    cli_root / "src/daemon/ability/builtins/device_control/ability_management/ops.rs",
)
ability_management_tokens = (
    "ABILITY_PUBLISH",
    "ABILITY_UNPUBLISH",
    "ABILITY_DEPLOY_ABILITY",
    "ABILITY_UNINSTALL_ABILITY",
    '"ability.publish"',
    '"ability.unpublish"',
    '"ability.deploy"',
    '"ability.uninstall"',
)
for path in ability_management_owner_files:
    if not path.exists():
        continue
    text = source(path)
    for token in ability_management_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            window = text[offset : offset + 700]
            if "OwnerKind::Device" in window and (
                "register_rpc_with_owner" in window
                or "register_rpc_with_spec" in window
                or "register_rpc_with_envelope_and_spec" in window
            ):
                add(
                    "R118_ABILITY_MANAGEMENT_SYSTEM_AGENT_OWNER",
                    path,
                    line_number(text, offset),
                    "ability.publish/unpublish/deploy/uninstall must be owned by the device-sponsored ability-management SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_ability_management = any(token in text for token in ability_management_tokens)
    if registers_ability_management and "OwnerKind::ability_management_system()" not in text:
        add(
            "R118_ABILITY_MANAGEMENT_SYSTEM_AGENT_OWNER",
            path,
            1,
            "ability-management registration must use the canonical OwnerKind::ability_management_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "ability_management_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include ability-management SystemAgent descriptors",
        ),
        (
            "fn ability_management_system_descriptors_for_device(",
            "ability-management SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R118_ABILITY_MANAGEMENT_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
federation_names = cli_root / "src/daemon/ability/names/federation.rs"
if federation_names.exists():
    text = source(federation_names)
    if "pub const ABILITY_MANAGEMENT_SYSTEM_AGENT_ID" not in text:
        add(
            "R118_ABILITY_MANAGEMENT_SYSTEM_AGENT_OWNER",
            federation_names,
            1,
            "ability-management SystemAgent id must be named with the ability-management ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn ability_management_system() -> Self" not in text:
        add(
            "R118_ABILITY_MANAGEMENT_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical ability-management SystemAgent constructor",
        )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if (
        "ability_management_system_agent_ability_resolves_from_local_device_authority"
        not in raw_text
    ):
        add(
            "R118_ABILITY_MANAGEMENT_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove ability-management SystemAgent callee with Device execution host",
        )

openai_compat_owner = cli_root / "src/daemon/ability/builtins/integrations/openai_compat.rs"
openai_compat_tokens = (
    "OPENAI_CHAT_COMPLETIONS",
    "OPENAI_LIST_MODELS",
    "OPENAI_FILES_UPLOAD",
    "OPENAI_FILES_RETRIEVE",
    "OPENAI_FILES_DELETE",
    '"openai.chat_completions"',
    '"openai.list_models"',
    '"openai.files.upload"',
    '"openai.files.retrieve"',
    '"openai.files.delete"',
)
if openai_compat_owner.exists():
    text = source(openai_compat_owner)
    for token in openai_compat_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            window = text[offset : offset + 700]
            if "OwnerKind::Device" in window and "register_rpc_with_owner" in window:
                add(
                    "R119_OPENAI_COMPAT_SYSTEM_AGENT_OWNER",
                    openai_compat_owner,
                    line_number(text, offset),
                    "openai.* compatibility abilities must be owned by the device-sponsored openai-compat SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_openai_compat = any(token in text for token in openai_compat_tokens) and (
        "register_rpc_with_owner" in text
    )
    if registers_openai_compat and "OwnerKind::openai_compat_system()" not in text:
        add(
            "R119_OPENAI_COMPAT_SYSTEM_AGENT_OWNER",
            openai_compat_owner,
            1,
            "OpenAI compatibility registration must use the canonical OwnerKind::openai_compat_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "openai_compat_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include openai-compat SystemAgent descriptors",
        ),
        (
            "fn openai_compat_system_descriptors_for_device(",
            "openai-compat SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R119_OPENAI_COMPAT_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
integrations_names = cli_root / "src/daemon/ability/names/integrations.rs"
if integrations_names.exists():
    text = source(integrations_names)
    if "pub const OPENAI_COMPAT_SYSTEM_AGENT_ID" not in text:
        add(
            "R119_OPENAI_COMPAT_SYSTEM_AGENT_OWNER",
            integrations_names,
            1,
            "openai-compat SystemAgent id must be named in the integrations ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn openai_compat_system() -> Self" not in text:
        add(
            "R119_OPENAI_COMPAT_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical openai-compat SystemAgent constructor",
        )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if "openai_compat_system_agent_ability_resolves_from_local_device_authority" not in raw_text:
        add(
            "R119_OPENAI_COMPAT_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove openai-compat SystemAgent callee with Device execution host",
        )

api_key_owner = cli_root / "src/daemon/ability/builtins/governance/api_key.rs"
api_key_tokens = (
    ".api_key.create",
    ".api_key.list",
    ".api_key.revoke",
)
if api_key_owner.exists():
    text = source(api_key_owner)
    for token in api_key_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            window = text[max(0, offset - 450) : offset + 700]
            if "OwnerKind::Device" in window and (
                "register_rpc_with_owner" in window
                or "register_rpc_with_owner_and_action" in window
            ):
                add(
                    "R120_API_KEY_SYSTEM_AGENT_OWNER",
                    api_key_owner,
                    line_number(text, offset),
                    "<user>.api_key.* lifecycle abilities must be owned by the device-sponsored api-key-management SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_api_key = any(token in text for token in api_key_tokens) and (
        "register_rpc_with_owner" in text
        or "register_rpc_with_owner_and_action" in text
    )
    if registers_api_key and "OwnerKind::api_key_management_system()" not in text:
        add(
            "R120_API_KEY_SYSTEM_AGENT_OWNER",
            api_key_owner,
            1,
            "API-key lifecycle registration must use the canonical OwnerKind::api_key_management_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "api_key_management_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include api-key-management SystemAgent descriptors",
        ),
        (
            "fn api_key_management_system_descriptors_for_device(",
            "api-key-management SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R120_API_KEY_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
governance_names = cli_root / "src/daemon/ability/names/governance.rs"
if governance_names.exists():
    text = source(governance_names)
    if "pub const API_KEY_MANAGEMENT_SYSTEM_AGENT_ID" not in text:
        add(
            "R120_API_KEY_SYSTEM_AGENT_OWNER",
            governance_names,
            1,
            "api-key-management SystemAgent id must be named in the governance ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn api_key_management_system() -> Self" not in text:
        add(
            "R120_API_KEY_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical api-key-management SystemAgent constructor",
        )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if "api_key_management_system_agent_ability_resolves_from_local_device_authority" not in raw_text:
        add(
            "R120_API_KEY_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove api-key-management SystemAgent callee with Device execution host",
        )

a2a_owner_files = (
    cli_root / "src/daemon/ability/builtins/integrations/a2a/bridge.rs",
    cli_root / "src/daemon/ability/builtins/integrations/a2a/client.rs",
)
a2a_tokens = (
    "A2A_BRIDGE_LIST_SKILLS",
    "A2A_BRIDGE_SEND_TASK",
    "A2A_CLIENT_SEND_TASK",
    '"a2a.bridge.list_skills"',
    '"a2a.bridge.send_task"',
    '"a2a.client.send_task"',
)
for path in a2a_owner_files:
    if not path.exists():
        continue
    text = source(path)
    for token in a2a_tokens:
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            window = text[offset : offset + 800]
            if "OwnerKind::Device" in window and (
                "register_rpc_with_owner" in window
                or "register_rpc_with_envelope_and_owner" in window
            ):
                add(
                    "R121_A2A_INTEGRATION_SYSTEM_AGENT_OWNER",
                    path,
                    line_number(text, offset),
                    "A2A bridge/client abilities must be owned by the device-sponsored a2a-integration SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    registers_a2a = any(token in text for token in a2a_tokens) and (
        "register_rpc_with_owner" in text
        or "register_rpc_with_envelope_and_owner" in text
    )
    if registers_a2a and "OwnerKind::a2a_integration_system()" not in text:
        add(
            "R121_A2A_INTEGRATION_SYSTEM_AGENT_OWNER",
            path,
            1,
            "A2A bridge/client registration must use the canonical OwnerKind::a2a_integration_system constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "a2a_integration_system_descriptors_for_device(device_ura)",
            "host profile aggregation must include a2a-integration SystemAgent descriptors",
        ),
        (
            "fn a2a_integration_system_descriptors_for_device(",
            "a2a-integration SystemAgent descriptor projection must be explicit",
        ),
    ):
        if token not in text:
            add("R121_A2A_INTEGRATION_SYSTEM_AGENT_OWNER", profiles_mod, 1, detail)
integrations_names = cli_root / "src/daemon/ability/names/integrations.rs"
if integrations_names.exists():
    text = source(integrations_names)
    if "pub const A2A_INTEGRATION_SYSTEM_AGENT_ID" not in text:
        add(
            "R121_A2A_INTEGRATION_SYSTEM_AGENT_OWNER",
            integrations_names,
            1,
            "a2a-integration SystemAgent id must be named in the integrations ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn a2a_integration_system() -> Self" not in text:
        add(
            "R121_A2A_INTEGRATION_SYSTEM_AGENT_OWNER",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical a2a-integration SystemAgent constructor",
        )
if route_resolver.exists():
    raw_text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if "a2a_integration_system_agent_ability_resolves_from_local_device_authority" not in raw_text:
        add(
            "R121_A2A_INTEGRATION_SYSTEM_AGENT_OWNER",
            route_resolver,
            1,
            "route resolver must prove a2a-integration SystemAgent callee with Device execution host",
        )

mcp_reflective_registry = (
    cli_root / "src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs"
)
if mcp_reflective_registry.exists():
    text = source(mcp_reflective_registry)
    forbidden = (
        "crate::core::ura::URAKind::Device => (\n"
        "            OwnerKind::Device"
    )
    offset = text.find(forbidden)
    if offset != -1:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            mcp_reflective_registry,
            line_number(text, offset),
            "MCP reflective descriptors must reject direct Device owners; MCP owner must be MCP SystemAgent, explicit hosted Agent, or Authority",
        )
    if "Device cannot own MCP reflective descriptors" not in text:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            mcp_reflective_registry,
            1,
            "MCP reflective owner classifier must fail closed for direct Device owner URAs",
        )
    if "User Principal cannot own MCP reflective descriptors" not in text:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            mcp_reflective_registry,
            1,
            "MCP reflective owner classifier must fail closed for User Principal owner URAs",
        )
    if "not the MCP integration SystemAgent" not in text:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            mcp_reflective_registry,
            1,
            "MCP reflective owner classifier must admit only the exact MCP integration SystemAgent",
        )
    if '"authority".to_string()' not in text:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            mcp_reflective_registry,
            1,
            "MCP reflective Authority owner must use the canonical authority scope marker, not hub/device aliases",
        )
    if '"hub".to_string()' in text:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            mcp_reflective_registry,
            line_number(text, text.find('"hub".to_string()')),
            "MCP reflective Authority owner must not use the obsolete hub authority-scope marker",
        )
mcp_bridge = cli_root / "src/daemon/ability/builtins/integrations/mcp/bridge.rs"
mcp_client = cli_root / "src/daemon/ability/builtins/integrations/mcp/client.rs"
for path in (mcp_bridge, mcp_client):
    if not path.exists():
        continue
    text = source(path)
    for token in (
        '"mcp.bridge.list_tools"',
        '"mcp.bridge.call_tool"',
        "MCP_CLIENT_LIST",
        "MCP_CLIENT_CALL",
        "ABILITY_LIST",
        "ABILITY_CALL",
    ):
        search_from = 0
        while True:
            offset = text.find(token, search_from)
            if offset == -1:
                break
            window = text[offset : offset + 700]
            if "OwnerKind::Device" in window and "register_rpc_with_owner" in window:
                add(
                    "R127_MCP_HOSTED_AGENT_ONTOLOGY",
                    path,
                    line_number(text, offset),
                    "MCP bridge/client abilities must be owned by the MCP integration SystemAgent, not direct Device",
                )
            search_from = offset + len(token)
    if "register_rpc_with_owner" in text and "OwnerKind::mcp_integration_system()" not in text:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            path,
            1,
            "MCP bridge/client registration must use OwnerKind::mcp_integration_system()",
        )
mcp_profile = cli_root / "src/daemon/ability/catalog/profiles/mcp.rs"
if mcp_profile.exists():
    text = source(mcp_profile)
    if "OwnerKind::mcp_integration_system()" not in text:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            mcp_profile,
            1,
            "MCP profile descriptors must project the MCP integration SystemAgent owner",
        )
conformance_rs = cli_root / "src/daemon/ability/conformance.rs"
if conformance_rs.exists():
    text = source(conformance_rs)
    offset = text.find("DeviceBridge")
    if offset != -1:
        add(
            "R127_MCP_HOSTED_AGENT_ONTOLOGY",
            conformance_rs,
            line_number(text, offset),
            "MCP conformance domain must be hosted MCP Agent, not DeviceBridge",
        )
    for token in (
        "McpIntegrationSystemAgent",
        'local_rpc!("mcp.bridge.list_tools", McpIntegrationSystemAgent)',
        'local_rpc!("mcp.client.call", McpIntegrationSystemAgent)',
    ):
        if token not in text:
            add(
                "R127_MCP_HOSTED_AGENT_ONTOLOGY",
                conformance_rs,
                1,
                "MCP conformance baseline must classify bridge/client rows as McpIntegrationSystemAgent",
            )

account_agent_patterns = (
    (
        re.compile(r"\bagent_ura\s*\([^)]*\"account\"[^)]*\)", re.S),
        "production code must not synthesize a User account as an Agent URA",
    ),
    (
        re.compile(r"\.account\b"),
        "production code must not use .account Agent fallback vocabulary",
    ),
    (
        re.compile(r"\baccount-as-agent\b"),
        "production code must not preserve account-as-agent fallback vocabulary",
    ),
    (
        re.compile(r"\buser\.<account>\b"),
        "production code must not describe user.<account> as an Agent fallback",
    ),
)
for path in production_files(cli_root / "src", {".rs"}):
    text = source(path)
    for pattern, detail in account_agent_patterns:
        match = pattern.search(text)
        if match:
            add(
                "R128_ACCOUNT_NOT_AGENT_ONTOLOGY",
                path,
                line_number(text, match.start()),
                detail,
            )

login_profile_doc = cli_root / "docs/login-profile-join.md"
if login_profile_doc.exists():
    text = login_profile_doc.read_text(encoding="utf-8", errors="replace")
    if "An Account is not an Agent" not in text:
        add(
            "R128_ACCOUNT_NOT_AGENT_ONTOLOGY",
            login_profile_doc,
            1,
            "login/profile documentation must state that Account is not Agent",
        )

ontology_doc = cli_root / "docs/easynet_ontology.tex"
if ontology_doc.exists():
    text = ontology_doc.read_text(encoding="utf-8", errors="replace")
    if "A User account is not an Agent" not in text:
        add(
            "R128_ACCOUNT_NOT_AGENT_ONTOLOGY",
            ontology_doc,
            1,
            "ontology must state that User account is Principal/accountability, not Agent",
        )
    for token, detail in (
        (
            r"\item \textbf{Agent} --- routable behavior subject",
            "ontology must define ordinary Agent as a routable behavior subject",
        ),
        (
            r"\item \textbf{Authority} --- routable realm governance actor",
            "ontology must define Authority separately from execution substrate",
        ),
        (
            "Service, or Authority} identities advertising governed AbilityDescriptors.",
            "ontology must name ordinary public callees as Agent/SystemAgent/Service/Authority, not Device",
        ),
        (
            "Device selector to choose the execution host",
            "ability.deploy ontology must describe --to <node> as execution-host selection",
        ),
        (
            "ability-management\nSystem Agent",
            "ability.deploy ontology must state deployed descriptors are owned by the ability-management SystemAgent",
        ),
    ):
        if token not in text:
            add("R138_ONTOLOGY_ACTOR_MODEL_CLOSED", ontology_doc, 1, detail)
    for token, detail in (
        (
            "Agent (the only addressable actor)",
            "ontology must not preserve the pre-SystemAgent claim that ordinary Agent is the only addressable actor",
        ),
        (
            "This is \\textbf{not fixed in the current PR}",
            "ability.deploy ontology must not claim the Device-owner leak remains unfixed after SystemAgent owner migration",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add("R138_ONTOLOGY_ACTOR_MODEL_CLOSED", ontology_doc, line_number(text, offset), detail)

    for token, detail in (
        (
            "\\textbf{Canonical actor contract.}",
            "active ontology must expose one canonical actor contract block",
        ),
        (
            "User/Account = Principal/accountability",
            "canonical actor contract must pin User/Account to principal accountability, not Agent",
        ),
        (
            "Agent = routable AbilityDescriptor owner/callee",
            "canonical actor contract must define Agent as the routable descriptor owner/callee",
        ),
        (
            "System Agent = Device-sponsored restricted Agent",
            "canonical actor contract must define SystemAgent as Device-sponsored Agent, not Device",
        ),
        (
            "Authority = realm governance actor",
            "canonical actor contract must keep Authority distinct from execution substrate",
        ),
        (
            "Device = execution host/custody/sponsor, not an ordinary public actor",
            "canonical actor contract must define Device as host/custody/sponsor, not ordinary actor",
        ),
        (
            "\\texttt{DeviceCallerPurpose} classifier",
            "ontology must bind bounded Device caller exceptions to the shared DeviceCallerPurpose classifier",
        ),
        (
            "AbilityImpl/Plugin/Skill = implementation/resource plane, not public ownership identity",
            "canonical actor contract must keep implementation/resource plane out of public ownership identity",
        ),
    ):
        if token not in text:
            add("R148_CANONICAL_ACTOR_CONTRACT", ontology_doc, 1, detail)

authority_proof_rs = cli_root / "src/daemon/invocation/admission/authority_proof.rs"
if authority_proof_rs.exists():
    text = source(authority_proof_rs)
    raw_text = authority_proof_rs.read_text(encoding="utf-8", errors="replace")
    proof_body = rust_struct_body(text, "AuthorityProof")
    if proof_body is None:
        add(
            "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
            authority_proof_rs,
            1,
            "AuthorityProof must remain an explicit signed authority proof contract",
        )
    else:
        start, body = proof_body
        if "pub owner_user_ura: String" not in body:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, start),
                "AuthorityProof must expose canonical owner_user_ura internally",
            )
        if re.search(r"\bpub\s+owner_user_id\s*:", body):
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, start),
                "AuthorityProof must not expose owner_user_id as a runtime field",
            )
        if 'serde(rename = "owner_user_id"' not in body:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, start),
                "AuthorityProof must preserve serialized owner_user_id compatibility through serde rename",
            )
        if "pub session_owner_user_ura: Option<String>" not in body:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, start),
                "AuthorityProof session owner must be canonical session_owner_user_ura internally",
            )
        if re.search(r"\bpub\s+session_owner_user_id\s*:", body):
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, start),
                "AuthorityProof must not expose session_owner_user_id as a runtime field",
            )
        if 'rename = "session_owner_user_id"' not in body:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, start),
                "AuthorityProof must preserve serialized session_owner_user_id compatibility through serde rename",
            )
    context_match = re.search(
        r"pub\s+struct\s+AuthorityProofVerificationContext\s*<[^>]+>\s*\{(?P<body>.*?)\n\}",
        text,
        re.S,
    )
    if context_match is None:
        add(
            "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
            authority_proof_rs,
            1,
            "AuthorityProofVerificationContext must remain an explicit runtime authority context",
        )
    else:
        body = context_match.group("body")
        if "pub owner_user_ura: &'a str" not in body:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, context_match.start()),
                "runtime AuthorityProofVerificationContext must expose owner_user_ura, not owner_user_id",
            )
        owner_user_id_field = re.search(r"\bpub\s+owner_user_id\s*:", body)
        if owner_user_id_field is not None:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, context_match.start() + owner_user_id_field.start()),
                "runtime authority verification context must not use owner_user_id compatibility naming",
            )
        if "pub session_owner_user_ura: Option<&'a str>" not in body:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, context_match.start()),
                "runtime AuthorityProofVerificationContext must expose session_owner_user_ura, not session_owner_user_id",
            )
        session_owner_user_id_field = re.search(r"\bpub\s+session_owner_user_id\s*:", body)
        if session_owner_user_id_field is not None:
            add(
                "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
                authority_proof_rs,
                line_number(text, context_match.start() + session_owner_user_id_field.start()),
                "runtime authority verification context must not use session_owner_user_id compatibility naming",
            )
    for token, detail in (
        (
            "RFC-014 durable/wire compatibility only",
            "AuthorityProof owner_user_id compatibility field must document its durable/wire-only lifecycle",
        ),
        (
            "not reinterpret it as a bare account id or Agent identity",
            "AuthorityProof owner_user_id compatibility field must explicitly reject Account/User-as-Agent confusion",
        ),
        (
            "compatibility wire name",
            "AuthorityProof session_owner_user_id must be documented as a compatibility wire key",
        ),
        (
            "must not be used as a runtime scalar helper name",
            "AuthorityProof session owner compatibility key must not leak back into runtime naming",
        ),
        (
            "fn issuer_authorized_for_owner_ura(",
            "issuer authorization resolver must name the canonical User URA contract",
        ),
        (
            "validate_user_principal_ura(context.owner_user_ura, \"owner_user_ura\")",
            "runtime authority context owner must be validated as canonical User URA",
        ),
        (
            "proof.owner_user_ura != context.owner_user_ura",
            "AuthorityProof runtime owner must be compared through canonical owner_user_ura",
        ),
        (
            "validate_user_principal_ura(session_owner_user_ura, \"session_owner_user_id\")",
            "AuthorityProof session owner must be validated as canonical User URA despite the compatibility wire key",
        ),
        (
            "proof.session_owner_user_ura.as_deref() != context.session_owner_user_ura",
            "AuthorityProof session owner must be compared through canonical session_owner_user_ura",
        ),
        (
            "verifier_rejects_bare_context_owner_user_id",
            "authority proof tests must reject bare user ids at the runtime context boundary",
        ),
        (
            "verifier_rejects_bare_context_session_owner_user_id",
            "authority proof tests must reject bare session owner user ids at the runtime context boundary",
        ),
    ):
        haystack = raw_text if token.startswith("verifier_") or " " in token else text
        if token not in haystack:
            add("R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA", authority_proof_rs, 1, detail)
    if "fn issuer_authorized_for_owner(" in text:
        add(
            "R137_AUTHORITY_PROOF_OWNER_PRINCIPAL_URA",
            authority_proof_rs,
            line_number(text, text.find("fn issuer_authorized_for_owner(")),
            "issuer authorization resolver must not keep ambiguous owner_user_id method naming",
        )

authority_metadata_rs = cli_root / "src/daemon/invocation/admission/authority_metadata.rs"
if authority_metadata_rs.exists():
    text = source(authority_metadata_rs)
    raw_text = authority_metadata_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "Public session-authority wire scalar",
            "SessionAuthorityPayload must document session_owner_user_id as a public wire scalar",
        ),
        (
            "It is not a User URA",
            "SessionAuthorityPayload must state that session_owner_user_id is not a runtime User URA",
        ),
        (
            "do not use this field as a runtime User URA",
            "SessionAuthorityRequest must keep session_owner_user_id confined to request/wire scalar semantics",
        ),
        (
            "parsed.user_id() != Some(payload.session_owner_user_id.as_str())",
            "session authority validation must compare the wire scalar to the User segment derived from subject_ura",
        ),
    ):
        if token not in raw_text:
            add("R149_SESSION_AUTHORITY_OWNER_SCALAR_BOUNDARY", authority_metadata_rs, 1, detail)
if admission_facade.exists():
    text = source(admission_facade)
    for token, detail in (
        (
            "fn session_owner_user_ura(payload: &SessionAuthorityPayload) -> Result<String, Status>",
            "admission must name conversion from session_owner_user_id scalar to canonical User URA as session_owner_user_ura",
        ),
        (
            "let owner_user_ura = session_owner_user_ura(payload)?",
            "session issuer authorization must consume a canonical owner_user_ura variable",
        ),
    ):
        if token not in text:
            add("R149_SESSION_AUTHORITY_OWNER_SCALAR_BOUNDARY", admission_facade, 1, detail)
    legacy = "fn session_owner_user_id(payload: &SessionAuthorityPayload) -> Result<String, Status>"
    offset = text.find(legacy)
    if offset != -1:
        add(
            "R149_SESSION_AUTHORITY_OWNER_SCALAR_BOUNDARY",
            admission_facade,
            line_number(text, offset),
            "function returning canonical User URA must not be named session_owner_user_id",
        )

pages_mod_rs = cli_root / "src/daemon/ability/builtins/resources/pages/mod.rs"
pages_health_rs = cli_root / "src/daemon/ability/builtins/resources/pages/list_get_unpublish.rs"
if pages_mod_rs.exists():
    text = source(pages_mod_rs)
    raw_text = pages_mod_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "pub(crate) fn pages_service_owner(owner_user_id: &str) -> OwnerKind",
            "Pages abilities must centralize the principal-scoped Pages Service owner projection",
        ),
        (
            "handle_health(&owner_ura, &user, &realm, args)",
            "pages.health must report the committed Pages Service owner",
        ),
        (
            "runtime_binding_facts_for_mode",
            "Pages health owner must come from the committed descriptor binding",
        ),
    ):
        if token not in raw_text:
            add("R150_PAGES_OWNER_SEGMENT_BOUNDARY", pages_mod_rs, 1, detail)
    for token, detail in (
        (
            "OwnerKind::pages_system()",
            "Pages abilities must use the principal-scoped Pages Service, not the retired Pages SystemAgent",
        ),
        ("pages_authority_scope", "Pages must not declare a shared hosted-Agent authority scope"),
        ("management_agent_ura", "Pages must not reconstruct a shared user-hosted Agent URA"),
        ('OwnerKind::Agent("pages"', "Pages must not use a hosted User Agent owner"),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R150_PAGES_OWNER_SEGMENT_BOUNDARY",
                pages_mod_rs,
                line_number(text, offset),
                detail,
            )
if pages_health_rs.exists():
    text = source(pages_health_rs)
    raw_text = pages_health_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "pub fn handle_health(\n    owner_ura: &str,\n    user: &str,",
            "pages.health must receive the committed execution owner separately from display user",
        ),
        (
            "owner_ura.to_string()",
            "pages.health must return the supplied committed SystemAgent owner",
        ),
    ):
        if token not in raw_text:
            add("R150_PAGES_OWNER_SEGMENT_BOUNDARY", pages_health_rs, 1, detail)
    for token, detail in (
        (
            'let owner_ura = crate::core::ura::agent_ura(realm, user, "pages")',
            "pages.health must not build owner_ura from display user slug",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R150_PAGES_OWNER_SEGMENT_BOUNDARY",
                pages_health_rs,
                line_number(text, offset),
                detail,
            )

hosted_agent_publication_rs = (
    cli_root / "src/daemon/invocation/admission/hosted_agent_publication.rs"
)
unary_dispatcher_rs = cli_root / "src/daemon/invocation/dispatch/unary_dispatcher.rs"
federation_client_contract_rs = cli_root / "src/daemon/federation/client/ability_contract.rs"
federation_wire_contract_rs = cli_root / "src/daemon/federation/wire_contract.rs"
if hosted_agent_publication_rs.exists():
    text = source(hosted_agent_publication_rs)
    body = rust_struct_body(text, "HostedAgentPublication")
    if body is not None:
        start, struct_body = body
        if "owner_user_segment: String" not in struct_body:
            add(
                "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
                hosted_agent_publication_rs,
                line_number(text, start),
                "HostedAgentPublication must name its private bare user-id segment owner_user_segment",
            )
        if re.search(r"\bowner_user_id\s*:\s*String\b", struct_body):
            add(
                "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
                hosted_agent_publication_rs,
                line_number(text, start),
                "HostedAgentPublication must not name a private runtime field owner_user_id",
            )
    if "pub(crate) fn owner_user_id(" in text:
        add(
            "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
            hosted_agent_publication_rs,
            line_number(text, text.find("pub(crate) fn owner_user_id(")),
            "HostedAgentPublication must expose owner_user_segment, not owner_user_id",
        )
if unary_dispatcher_rs.exists():
    text = source(unary_dispatcher_rs)
    body = rust_struct_body(text, "PrincipalOwnerFilter")
    if body is not None:
        start, struct_body = body
        if "owner_user_segment: String" not in struct_body:
            add(
                "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
                unary_dispatcher_rs,
                line_number(text, start),
                "PrincipalOwnerFilter must name its private bare user-id segment owner_user_segment",
            )
        if re.search(r"\bowner_user_id\s*:\s*String\b", struct_body):
            add(
                "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
                unary_dispatcher_rs,
                line_number(text, start),
                "PrincipalOwnerFilter must not name a private runtime field owner_user_id",
            )
    if "let Some(owner_user_id) = request.local_user_id.as_deref()" in text:
        add(
            "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
            unary_dispatcher_rs,
            line_number(text, text.find("let Some(owner_user_id) = request.local_user_id.as_deref()")),
            "federation.discover local_user_id scalar must be named owner_user_segment internally",
        )
if federation_client_contract_rs.exists():
    raw_text = federation_client_contract_rs.read_text(encoding="utf-8", errors="replace")
    if "principal_owner_user_id" in raw_text and "This is not a runtime User URA" not in raw_text:
        add(
            "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
            federation_client_contract_rs,
            1,
            "federation client DTO must document principal_owner_user_id as a wire scalar, not a runtime User URA",
        )
if federation_wire_contract_rs.exists():
    raw_text = federation_wire_contract_rs.read_text(encoding="utf-8", errors="replace")
    if "principal_owner_user_id" in raw_text and "This is not a runtime User URA" not in raw_text:
        add(
            "R152_PRODUCT_OWNER_USER_ID_ADAPTER_BOUNDARY",
            federation_wire_contract_rs,
            1,
            "federation wire DTO must document principal_owner_user_id as a wire scalar, not a runtime User URA",
        )

daemon_invocation_service_tests_rs = (
    cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs"
)
daemon_invocation_service_unary_tests_rs = (
    cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs"
)
if unary_dispatcher_rs.exists():
    text = source(unary_dispatcher_rs)
    raw_text = unary_dispatcher_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "enum FederationDiscoverReadScope",
            "federation.discover must resolve an explicit read scope before merging directory rows",
        ),
        (
            "OperatorAudit",
            "federation.discover must name the unfiltered operator/audit read scope explicitly",
        ),
        (
            "federation.discover unfiltered operator/audit scope requires the local daemon principal",
            "unfiltered federation.discover must require the local Authority/daemon principal",
        ),
        (
            "caller.kind != crate::core::ura::URAKind::User",
            "user-scoped federation.discover must require a canonical User caller",
        ),
        (
            "caller.user_id() != Some(owner_user_segment)",
            "user-scoped federation.discover must bind caller User segment to local_user_id",
        ),
        (
            "owner_filter(&self) -> Option<&PrincipalOwnerFilter>",
            "federation.discover must keep user-scoped owner filtering explicit",
        ),
    ):
        if token not in raw_text:
            add("R153_FEDERATION_DISCOVER_AUTHORITY_SCOPE", unary_dispatcher_rs, 1, detail)
    for token, detail in (
        (
            "return Ok(Self::OperatorAudit);",
            "unfiltered federation.discover must not return OperatorAudit before checking the local daemon principal",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            window = text[max(0, offset - 260): offset]
            if "Some(caller_ura) != local_daemon_ura" not in window:
                add(
                    "R153_FEDERATION_DISCOVER_AUTHORITY_SCOPE",
                    unary_dispatcher_rs,
                    line_number(text, offset),
                    detail,
                )
if daemon_invocation_service_tests_rs.exists():
    raw_text = daemon_invocation_service_tests_rs.read_text(
        encoding="utf-8", errors="replace"
    )
    if "authority_invoke_request" in raw_text:
        for token, detail in (
            (
                "fn authority_invoke_request(function_name: &str, args_json: &str) -> Request<InvokeRequest>",
                "federation.discover tests must provide an Authority self-call request helper",
            ),
            (
                "signed_invoke_request(\n        TEST_HUB_URA,\n        TEST_HUB_URA,\n        TEST_HUB_URA,",
                "Authority request helper must sign caller, callee, and subject as the local Authority",
            ),
            (
                "TrustAnchorRole::Hub",
                "test trust anchor must register the local Authority as a Hub/Authority actor",
            ),
        ):
            if token not in raw_text:
                add("R153_FEDERATION_DISCOVER_AUTHORITY_SCOPE", daemon_invocation_service_tests_rs, 1, detail)
if daemon_invocation_service_unary_tests_rs.exists():
    text = daemon_invocation_service_unary_tests_rs.read_text(
        encoding="utf-8", errors="replace"
    )
    if "ABILITY_FEDERATION_DISCOVER" in text or "federation_discover" in text:
        for token, detail in (
            (
                ".invoke(authority_invoke_request(ABILITY_FEDERATION_DISCOVER, \"{}\"))",
                "unfiltered federation.discover positive test must call as Authority, not Device",
            ),
            (
                "make_unregistered_service_for_route_owner(TEST_HUB_URA).with_session_realm(\"local-realm\")",
                "local-presence federation.discover test must register the route owner as Authority",
            ),
            (
                "register_test_daemon_routes(svc, TEST_HUB_URA)",
                "federation.discover local-presence test must route through the Authority owner",
            ),
            (
                "user_scoped_discover_request",
                "federation.discover tests must retain a separate canonical User caller path for local_user_id",
            ),
        ):
            if token not in text:
                add("R153_FEDERATION_DISCOVER_AUTHORITY_SCOPE", daemon_invocation_service_unary_tests_rs, 1, detail)
        for token, detail in (
            (
                ".invoke(invoke_request(ABILITY_FEDERATION_DISCOVER",
                "federation.discover tests must not exercise unfiltered operator/audit scope through generic Device-style invoke_request",
            ),
            (
                "make_unregistered_service_for_route_owner(TEST_DAEMON_URA).with_session_realm(\"local-realm\")",
                "federation.discover tests must not use the Device route owner for operator/audit discovery",
            ),
        ):
            offset = text.find(token)
            if offset != -1:
                add(
                    "R153_FEDERATION_DISCOVER_AUTHORITY_SCOPE",
                    daemon_invocation_service_unary_tests_rs,
                    line_number(text, offset),
                    detail,
                )

decision_rs = cli_root / "src/daemon/invocation/admission/decision.rs"
grant_matcher_rs = cli_root / "src/daemon/invocation/admission/grant_matcher.rs"
persistence_access_control_rs = cli_root / "src/daemon/persistence/access_control.rs"
if decision_rs.exists():
    text = source(decision_rs)
    raw_text = decision_rs.read_text(encoding="utf-8", errors="replace")
    principal_kind = rust_enum_body(text, "PrincipalKind")
    if principal_kind is None:
        add("R141_DEVICE_CUSTODY_NOT_PRINCIPAL", decision_rs, 1, "PrincipalKind must remain explicit")
    else:
        start, body = principal_kind
        if "DeviceCustody" not in body:
            add(
                "R141_DEVICE_CUSTODY_NOT_PRINCIPAL",
                decision_rs,
                line_number(text, start),
                "device custody projection must be named DeviceCustody, not generic Device",
            )
        if re.search(r"(^|[\s,])Device\s*(?:,|$)", body):
            add(
                "R141_DEVICE_CUSTODY_NOT_PRINCIPAL",
                decision_rs,
                line_number(text, start),
                "PrincipalKind must not expose a generic Device actor variant",
            )
        if 'serde(rename = "device")' not in body:
            add(
                "R141_DEVICE_CUSTODY_NOT_PRINCIPAL",
                decision_rs,
                line_number(text, start),
                "DeviceCustody may preserve wire `device` compatibility only through serde rename",
            )
    for struct_name, required_field in (
        ("OwnerResolution", "pub owner_user_ura: Option<String>"),
        ("PolicyDecision", "pub owner_user_ura: Option<String>"),
        ("PermissionRequest", "pub owner_user_ura: String"),
    ):
        body = rust_struct_body(text, struct_name)
        if body is None:
            add("R139_RUNTIME_OWNER_USER_URA_NAMING", decision_rs, 1, f"{struct_name} must remain present")
            continue
        start, struct_body = body
        if required_field not in struct_body:
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                decision_rs,
                line_number(text, start),
                f"{struct_name} must expose canonical owner_user_ura internally",
            )
        if re.search(r"\bpub\s+owner_user_id\s*:", struct_body):
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                decision_rs,
                line_number(text, start),
                f"{struct_name} must not expose owner_user_id as a runtime field",
            )
        if 'serde(rename = "owner_user_id"' not in struct_body:
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                decision_rs,
                line_number(text, start),
                f"{struct_name} must preserve serialized owner_user_id compatibility through serde rename",
            )
    for token, detail in (
        (
            "key is retained only for durable/wire compatibility",
            "OwnerResolution owner_user_id compatibility field must document its durable/wire-only lifecycle",
        ),
        (
            "policy-decision wire compatibility",
            "PolicyDecision owner_user_id compatibility field must document its policy-decision wire lifecycle",
        ),
        (
            "permission-request durable/wire compatibility",
            "PermissionRequest owner_user_id compatibility field must document its permission-request compatibility lifecycle",
        ),
    ):
        if token not in raw_text:
            add("R139_RUNTIME_OWNER_USER_URA_NAMING", decision_rs, 1, detail)
if grant_matcher_rs.exists():
    text = source(grant_matcher_rs)
    raw_text = grant_matcher_rs.read_text(encoding="utf-8", errors="replace")
    grant_body = rust_struct_body(text, "PermissionGrant")
    if grant_body is None:
        add("R139_RUNTIME_OWNER_USER_URA_NAMING", grant_matcher_rs, 1, "PermissionGrant must remain present")
    else:
        start, body = grant_body
        if "pub owner_user_ura: String" not in body:
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                grant_matcher_rs,
                line_number(text, start),
                "PermissionGrant must expose canonical owner_user_ura internally",
            )
        if re.search(r"\bpub\s+owner_user_id\s*:", body):
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                grant_matcher_rs,
                line_number(text, start),
                "PermissionGrant must not expose owner_user_id as a runtime field",
            )
        if 'serde(rename = "owner_user_id"' not in body:
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                grant_matcher_rs,
                line_number(text, start),
                "PermissionGrant must preserve serialized owner_user_id compatibility through serde rename",
            )
    if "permission-grant durable/wire compatibility" not in raw_text:
        add(
            "R139_RUNTIME_OWNER_USER_URA_NAMING",
            grant_matcher_rs,
            1,
            "PermissionGrant owner_user_id compatibility field must document its permission-grant lifecycle",
        )
    input_body = rust_struct_body(text, "GrantMatchInput")
    if input_body is None:
        add("R139_RUNTIME_OWNER_USER_URA_NAMING", grant_matcher_rs, 1, "GrantMatchInput must remain present")
    else:
        start, body = input_body
        if "pub owner_user_ura: &'a str" not in body:
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                grant_matcher_rs,
                line_number(text, start),
                "GrantMatchInput must expose canonical owner_user_ura internally",
            )
        if re.search(r"\bpub\s+owner_user_id\s*:", body):
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                grant_matcher_rs,
                line_number(text, start),
                "GrantMatchInput must not expose owner_user_id as a runtime field",
            )
if access_control.exists():
    text = source(access_control)
    for token, detail in (
        (
            "owner_user_ura_from_boundary(",
            "access-control ability boundary must name canonical owner User URA extraction",
        ),
        (
            "owner_user_ura_from_mutation_boundary(",
            "access-control mutation boundary must name canonical owner User URA extraction",
        ),
        (
            "owner_user_ura =",
            "access-control runtime locals must use owner_user_ura naming",
        ),
    ):
        if token not in text:
            add("R139_RUNTIME_OWNER_USER_URA_NAMING", access_control, 1, detail)
    for token, detail in (
        (
            "owner_user_id_from_boundary(",
            "access-control runtime helper must not preserve owner_user_id naming for canonical User URAs",
        ),
        (
            "owner_user_id_from_mutation_boundary(",
            "access-control mutation helper must not preserve owner_user_id naming for canonical User URAs",
        ),
        (
            "let owner_user_id",
            "access-control runtime local must not preserve owner_user_id naming for canonical User URAs",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R139_RUNTIME_OWNER_USER_URA_NAMING",
                access_control,
                line_number(text, offset),
                detail,
            )
if persistence_access_control_rs.exists():
    raw_text = persistence_access_control_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "v0 policy-store compatibility name",
            "policy store owner_user_id key must be documented as compatibility storage, not a runtime identity",
        ),
        (
            "audit-log compatibility only",
            "grant audit owner_user_id key must be documented as audit-log compatibility storage",
        ),
    ):
        if token not in raw_text:
            add("R139_RUNTIME_OWNER_USER_URA_NAMING", persistence_access_control_rs, 1, detail)

ffi_invocation = cli_root / "src/ffi/invocation/mod.rs"
if ffi_invocation.exists():
    text = source(ffi_invocation)
    raw_text = ffi_invocation.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            'let caller_ura = required_exact_string(obj, "caller_ura")?',
            "public Invocation caller_ura must be extracted without trimming",
        ),
        (
            'let callee_ura = required_exact_string(obj, "callee_ura")?',
            "public Invocation callee_ura must be extracted without trimming",
        ),
        (
            'let subject_ura = required_exact_string(obj, "subject_ura")?',
            "public Invocation subject_ura must be extracted without trimming",
        ),
    ):
        if token not in text:
            add("R140_PUBLIC_INVOCATION_TUPLE_URA_EXACT", ffi_invocation, 1, detail)
    tuple_body = rust_method_body(text, "validate_public_tuple_ura")
    if tuple_body is None:
        add(
            "R140_PUBLIC_INVOCATION_TUPLE_URA_EXACT",
            ffi_invocation,
            1,
            "public Invocation FFI must keep a single tuple URA validator",
        )
    else:
        start, body = tuple_body
        if "value.trim() != value" not in body:
            add(
                "R140_PUBLIC_INVOCATION_TUPLE_URA_EXACT",
                ffi_invocation,
                line_number(text, start),
                "public Invocation tuple URAs must reject surrounding whitespace before parse",
            )
        if "parse_ura(value.trim())" in body:
            add(
                "R140_PUBLIC_INVOCATION_TUPLE_URA_EXACT",
                ffi_invocation,
                line_number(text, start + body.find("parse_ura(value.trim())")),
                "public Invocation tuple URAs must not be trimmed into canonical form",
            )
    if "parse_invocation_json_rejects_whitespace_padded_tuple_uras_before_daemon_io" not in raw_text:
        add(
            "R140_PUBLIC_INVOCATION_TUPLE_URA_EXACT",
            ffi_invocation,
            1,
            "FFI tests must reject whitespace-padded caller/callee/subject URAs before daemon I/O",
        )

ability_descriptor_surface = cli_root / "src/daemon/ability/descriptors/surface.rs"
if ability_descriptor_surface.exists():
    raw_text = ability_descriptor_surface.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "callee ∈ {authority, device, agent}",
            "AbilityDescriptor docs must not describe Device URAs as ordinary public callees",
        ),
        (
            "device-built-ins",
            "AbilityDescriptor docs must not preserve device-native builtin public callee vocabulary",
        ),
        (
            "DeviceAgent",
            "AbilityDescriptor docs must use SystemAgent or DeviceProfileProjection, not DeviceAgent",
        ),
        (
            "ability/device/",
            "AbilityDescriptor examples must route device-hosted public abilities through SystemAgent URAs",
        ),
    ):
        offset = raw_text.find(token)
        if offset != -1:
            add(
                "R142_DEVICE_NOT_PUBLIC_CALLEE",
                ability_descriptor_surface,
                line_number(raw_text, offset),
                detail,
            )
    for required, detail in (
        (
            "SystemAgent",
            "AbilityDescriptor docs must name the device-sponsored SystemAgent public callee model",
        ),
        (
            "never an AbilityDescriptor owner/callee",
            "AbilityDescriptor docs must state that Device is never a descriptor owner/callee",
        ),
    ):
        if required not in raw_text:
            add("R142_DEVICE_NOT_PUBLIC_CALLEE", ability_descriptor_surface, 1, detail)

for state_doc in (
    cli_root / "docs/状态机器/easynet-state-machines.md",
    cli_root / "docs/状态机器/easynet-state-machines.zh-CN.tex",
    cli_root / "docs/状态机器/current-code-audit-and-corrected-state-machines.md",
):
    if not state_doc.exists():
        continue
    raw_text = state_doc.read_text(encoding="utf-8", errors="replace")
    for pattern, detail in (
        (
            r'"callee_ura"\s*:\s*"easynet:///r/[^"]*/device/',
            "state-machine examples must not use Device URAs as public invocation callees",
        ),
        (
            r'"ability_ura"\s*:\s*"easynet:///r/[^"]*/ability/device[./]',
            "state-machine examples must not publish device-hosted public abilities as ability/device rows",
        ),
        (
            r"\\texttt\{callee\\_ura\}\s*:\s*\\texttt\{easynet:///r/[^}]*/device/",
            "state-machine TeX examples must not use Device URAs as public invocation callees",
        ),
        (
            r"\\texttt\{ability\\_ura\}\s*:\s*\\texttt\{easynet:///r/[^}]*/ability/device[./]",
            "state-machine TeX examples must not publish device-hosted public abilities as ability/device rows",
        ),
    ):
        match = re.search(pattern, raw_text)
        if match:
            add("R142_DEVICE_NOT_PUBLIC_CALLEE", state_doc, line_number(raw_text, match.start()), detail)

for registrar_file in (
    cli_root / "src/daemon/ability/builtins/device_control/ability_management/ops.rs",
    cli_root / "src/daemon/ability/builtins/device_control/ability_management/registrar.rs",
    cli_root / "src/daemon/ability/catalog/build.rs",
    cli_root / "src/bin/easynet-daemon.rs",
    cli_root / "tests/seven_axes_fixture/mod.rs",
):
    if not registrar_file.exists():
        continue
    text = source(registrar_file)
    for token in (
        "DeviceAbilityRecord",
        "DeviceAbilityStore",
        "DeviceAbilityInstall",
        "DeviceAbilityUninstall",
        "DeviceAbilityCallModeResolution",
        "DeviceAbilityRegistrar",
        "SharedDeviceRegistrarCell",
        "device_registrar_cell",
        "require_device_registrar",
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R143_ABILITY_DEPLOYMENT_REGISTRAR_NAMING",
                registrar_file,
                line_number(text, offset),
                f"ability deployment lifecycle must not keep obsolete registrar name `{token}`",
            )
if (cli_root / "src/daemon/ability/builtins/device_control/ability_management/registrar.rs").exists():
    registrar_text = source(cli_root / "src/daemon/ability/builtins/device_control/ability_management/registrar.rs")
    for required in ("AbilityDeploymentRegistrar", "SharedAbilityDeploymentRegistrarCell"):
        if required not in registrar_text:
            add(
                "R143_ABILITY_DEPLOYMENT_REGISTRAR_NAMING",
                cli_root / "src/daemon/ability/builtins/device_control/ability_management/registrar.rs",
                1,
                f"ability deployment registrar module must expose `{required}`",
            )

daemon_service = cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service.rs"
daemon_service_tests = cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests"
boot_invocation = cli_root / "src/daemon/boot/invocation/mod.rs"
catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
if daemon_service.exists():
    text = source(daemon_service)
    for token, detail in (
        (
            "fn validate_daemon_route_authority_owner(",
            "daemon exact route registration must have one shared Authority-owner validator",
        ),
        (
            "parsed.kind != crate::core::ura::URAKind::Authority",
            "daemon exact route validator must reject every non-Authority owner",
        ),
        (
            'validate_daemon_route_authority_owner(owner_ura, "unary")',
            "unary exact route owner normalization must reuse the Authority-owner validator",
        ),
        (
            'validate_daemon_route_authority_owner(owner_ura.trim(), "stream")',
            "stream exact route registration must reuse the Authority-owner validator",
        ),
        (
            'validate_daemon_route_authority_owner(owner_ura.trim(), "bidi")',
            "bidi exact route registration must reuse the Authority-owner validator",
        ),
    ):
        if token not in text:
            add("R144_DAEMON_INVOCATION_AUTHORITY_OWNER", daemon_service, 1, detail)
    for token, detail in (
        (
            "URAKind::Device | crate::core::ura::URAKind::Authority",
            "daemon exact route owner validation must not accept Device as an alternative to Authority",
        ),
        (
            "canonical Device or Authority URA",
            "daemon exact route owner error text must not present Device as valid public route owner",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add(
                "R144_DAEMON_INVOCATION_AUTHORITY_OWNER",
                daemon_service,
                line_number(text, offset),
                detail,
            )
if daemon_service_tests.exists():
    test_files = [
        daemon_service_tests / "unary.rs",
        daemon_service_tests / "stream.rs",
        daemon_service_tests / "bidi.rs",
        cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs",
        daemon_service,
    ]
    test_text = "\n".join(source(path) for path in test_files if path.exists())
    if "daemon_exact_route_owner_rejects_device_ura" not in test_text:
        add(
            "R144_DAEMON_INVOCATION_AUTHORITY_OWNER",
            daemon_service_tests,
            1,
            "service tests must prove Device cannot own public daemon exact routes",
        )
    for specific_token, generic_token, detail in (
        (
            "daemon_exact_unary_route_registration_rejects_device_owner",
            "daemon_exact_route_owner_rejects_device_ura",
            "service tests must prove Device cannot own public daemon exact unary routes",
        ),
        (
            "daemon_exact_stream_route_registration_rejects_device_owner",
            "daemon_exact_route_owner_rejects_device_ura",
            "service tests must prove Device cannot own public daemon exact stream routes",
        ),
        (
            "exact_bidi_route_registration_rejects_device_owner",
            "daemon_exact_route_owner_rejects_device_ura",
            "service tests must prove Device cannot own public daemon exact bidi routes",
        ),
    ):
        if specific_token not in test_text and generic_token not in test_text:
            add("R144_DAEMON_INVOCATION_AUTHORITY_OWNER", daemon_service_tests, 1, detail)
if boot_invocation.exists():
    text = source(boot_invocation)
    required_hub_route_tokens = (
        (
            "Invocation transport cannot register Hub daemon routes without its canonical Authority URA",
            "Hub route boot must fail closed without canonical Authority URA",
        ),
        (
            "service.register_daemon_unary_routes(daemon_route_owner)",
            "Hub boot must register unary exact routes under the canonical Authority owner",
        ),
        (
            "service.register_daemon_stream_routes(daemon_route_owner)",
            "Hub boot must register stream exact routes under the canonical Authority owner",
        ),
        (
            "service.register_daemon_bidi_routes(daemon_route_owner)",
            "Hub boot must register bidi exact routes under the canonical Authority owner",
        ),
    )
    hub_block_starts = [
        match.start()
        for match in re.finditer(r"\bif\s+capabilities\.hub_runtime\s*\{", text)
    ]
    if not hub_block_starts:
        add(
            "R144_DAEMON_INVOCATION_AUTHORITY_OWNER",
            boot_invocation,
            1,
            "boot must guard exact daemon route registration behind hub_runtime",
        )
    else:
        route_blocks = [
            brace_block_from(text, offset)
            for offset in hub_block_starts
            if any(token in brace_block_from(text, offset) for token, _ in required_hub_route_tokens)
        ]
        hub_block = route_blocks[0] if route_blocks else ""
        for token, detail in required_hub_route_tokens:
            if token not in hub_block:
                add("R144_DAEMON_INVOCATION_AUTHORITY_OWNER", boot_invocation, 1, detail)
if catalog_build.exists():
    text = source(catalog_build)
    device_contract = re.search(
        r"daemon_invocation_contracts::register_for_owner\([^)]*OwnerKind::Device",
        text,
        re.S,
    )
    if device_contract:
        add(
            "R144_DAEMON_INVOCATION_AUTHORITY_OWNER",
            catalog_build,
            line_number(text, device_contract.start()),
            "daemon Invocation descriptor contracts must not be registered for direct Device owners",
        )
    if "register Authority-owned daemon Invocation descriptor contracts" not in text:
        add(
            "R144_DAEMON_INVOCATION_AUTHORITY_OWNER",
            catalog_build,
            1,
            "catalog build must document daemon Invocation contracts as Authority-owned",
        )

keyring_abilities = cli_root / "src/daemon/keyring/abilities.rs"
plugin_contribution = cli_root / "src/daemon/plugins/contribution.rs"
catalog_publication = cli_root / "src/daemon/ability/catalog/publication.rs"
catalog_metadata = cli_root / "src/daemon/ability/catalog/catalog_metadata.rs"
assembly_tests = cli_root / "src/daemon/ability/catalog/assembly_tests.rs"
remote_desktop_registration = cli_root / "plugins/remote-desktop/src/registration.rs"
remote_desktop_ability_dir = cli_root / "plugins/remote-desktop/abilities"
ability_toml_parser = cli_root / "src/daemon/ability/catalog/ability_toml.rs"
ability_descriptor_dir = cli_root / "ability-descriptors"
ability_manifest = cli_root / "src/daemon/ability/manifest.rs"
governance_names = cli_root / "src/daemon/ability/names/governance.rs"
if keyring_abilities.exists():
    text = source(keyring_abilities)
    offset = text.find("OwnerKind::Device")
    if offset != -1:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            keyring_abilities,
            line_number(text, offset),
            "device.keyring.* public descriptors must be owned by keyring-management SystemAgent, not direct Device",
        )
    if "OwnerKind::keyring_management_system()" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            keyring_abilities,
            1,
            "keyring registration must use the canonical OwnerKind::keyring_management_system constructor",
        )
if governance_names.exists():
    text = source(governance_names)
    if "pub const KEYRING_MANAGEMENT_SYSTEM_AGENT_ID" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            governance_names,
            1,
            "keyring-management SystemAgent id must be named in the governance ability namespace",
        )
if dispatch_rs.exists():
    text = source(dispatch_rs)
    if "pub(crate) fn keyring_management_system() -> Self" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            dispatch_rs,
            1,
            "OwnerKind must expose a canonical keyring-management SystemAgent constructor",
        )
if profiles_mod.exists():
    text = source(profiles_mod)
    for token, detail in (
        (
            "KEYRING_MANAGEMENT_SYSTEM_AGENT_ID",
            "host profile aggregation must include keyring-management SystemAgent descriptors",
        ),
        (
            "OwnerKind::keyring_management_system",
            "keyring-management SystemAgent descriptor projection must use the canonical owner constructor",
        ),
    ):
        if token not in text:
            add("R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY", profiles_mod, 1, detail)
if catalog_build.exists():
    text = catalog_build.read_text(encoding="utf-8", errors="replace")
    if "keyring-management" not in text or "Device callee/owner assertion" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            catalog_build,
            1,
            "catalog build must document device.keyring.* as legacy local names, not Device-owned descriptors",
        )
if plugin_contribution.exists():
    text = source(plugin_contribution)
    owner_policy_device = re.search(r"owner_policy\s*:\s*OwnerKind::Device", text)
    if owner_policy_device:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            plugin_contribution,
            line_number(text, owner_policy_device.start()),
            "plugin-provided AbilityImpls must publish through plugin-management SystemAgent, not direct Device",
        )
    if "public_owner: OwnerKind::plugin_management_system()" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            plugin_contribution,
            1,
            "PluginContributionBuilder must default generic plugin contribution owners to plugin-management SystemAgent",
        )
    if "set_public_owner(&mut self, owner: OwnerKind)" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            plugin_contribution,
            1,
            "daemon-native plugin packages must have an explicit public owner policy override",
        )
if catalog_publication.exists():
    text = catalog_publication.read_text(encoding="utf-8", errors="replace")
    if '"plugin.dynamic"' in text and "OwnerKind::Device" in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            catalog_publication,
            line_number(text, text.find('"plugin.dynamic"')),
            "publication tests must not model plugin dynamic abilities as direct Device-owned rows",
        )
if catalog_metadata.exists():
    text = source(catalog_metadata)
    for forbidden, detail in (
        (
            '.filter(|row| !row.name.starts_with("device.keyring."))',
            "keyring abilities must not be excluded from published layer classification",
        ),
        (
            "filter(|n| !n.starts_with(\"device.keyring.\"))",
            "keyring abilities must not be excluded from published layer classification",
        ),
    ):
        offset = text.find(forbidden)
        if offset != -1:
            add(
                "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
                catalog_metadata,
                line_number(text, offset),
                detail,
            )
    for token, detail in (
        ('"device.keyring.list"', "keyring list must be classified as an introspection ability"),
        ('"device.keyring.get_public"', "keyring get_public must be classified as an introspection ability"),
        ('"device.keyring.peer_list"', "keyring peer_list must be classified as an introspection ability"),
        ('"device.keyring.peer_add"', "keyring peer_add must be classified as a control ability"),
        ('"device.keyring.create"', "keyring create must be classified as an operational ability"),
        ('"device.keyring.rotate"', "keyring rotate must be classified as an operational ability"),
        ('"device.keyring.revoke"', "keyring revoke must be classified as an operational ability"),
        ('"device.keyring.expire_set"', "keyring expire_set must be classified as an operational ability"),
        ('"device.keyring.bind_subject"', "keyring bind_subject must be classified as an operational ability"),
        (
            '"device.keyring.federate_user_identity_token"',
            "keyring federate_user_identity_token must be classified as an operational ability",
        ),
    ):
        if token not in text:
            add("R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY", catalog_metadata, 1, detail)
if assembly_tests.exists():
    text = source(assembly_tests)
    raw_text = assembly_tests.read_text(encoding="utf-8", errors="replace")
    offset = text.find('filter(|n| !n.starts_with("device.keyring."))')
    if offset != -1:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            assembly_tests,
            line_number(text, offset),
            "classification completeness tests must not carve out keyring abilities",
        )
    if "OwnerKind::keyring_management_system()" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            assembly_tests,
            1,
            "registry assembly tests must pin device.keyring.* rows to keyring-management SystemAgent owner",
        )
    if (
        '"remote_desktop."' in raw_text
        and "OwnerKind::remote_desktop_system()" not in raw_text
    ):
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            assembly_tests,
            1,
            "registry assembly tests must pin remote_desktop.* rows to remote-desktop SystemAgent owner",
        )
if remote_desktop_registration.exists():
    text = source(remote_desktop_registration)
    raw_text = remote_desktop_registration.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            'const RESOURCE_SUBJECT_KINDS: &[&str] = &["resource"];',
            "remote-desktop resource-bound specs must name the dynamic Resource subject kind explicitly",
        ),
        (
            'const PERMISSION_PROBE_SUBJECT_KINDS: &[&str] = &["agent", "resource", "user"];',
            "remote-desktop permission probes must name Agent/User and descriptor-bound Resource probe subject kinds explicitly",
        ),
        (
            "subject_ura_kinds: RESOURCE_SUBJECT_KINDS",
            "remote-desktop resource-bound descriptors must not publish broad `any` subject scope",
        ),
        (
            "subject_ura_kinds: PERMISSION_PROBE_SUBJECT_KINDS",
            "remote-desktop permission probe descriptors must not publish broad `any` subject scope",
        ),
    ):
        if token not in text:
            add("R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY", remote_desktop_registration, 1, detail)
    if "ScopeRule::OnlyUraKinds" not in raw_text:
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            remote_desktop_registration,
            1,
            "remote-desktop catalog snapshot tests must pin kind-gated subject scopes",
        )
if remote_desktop_ability_dir.exists():
    for ability_toml in sorted(remote_desktop_ability_dir.glob("remote_desktop.*.ability.toml")):
        raw_text = ability_toml.read_text(encoding="utf-8", errors="replace")
        offset = raw_text.find('scope_subjects_kind = "any"')
        if offset != -1:
            add(
                "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
                ability_toml,
                line_number(raw_text, offset),
                "remote-desktop public ability TOML must use kind-gated subject scope, not `any`",
            )
        if 'scope_subjects_kind = "only_ura_kinds"' not in raw_text:
            add(
                "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
                ability_toml,
                1,
                "remote-desktop public ability TOML must declare `only_ura_kinds` subject scope",
            )
        if ability_toml.name in (
            "remote_desktop.permission_status.ability.toml",
            "remote_desktop.request_permission.ability.toml",
        ):
            if 'scope_subjects_uras = ["agent", "resource", "user"]' not in raw_text:
                add(
                    "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
                    ability_toml,
                    1,
                    "remote-desktop permission probes must admit Agent/User and descriptor-bound Resource subjects",
                )
        elif 'scope_subjects_uras = ["resource"]' not in raw_text:
            add(
                "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
                ability_toml,
                1,
                "remote-desktop session/control abilities must only admit Resource subjects",
            )
core_ura = cli_root / "src/core/ura/mod.rs"
if core_ura.exists():
    text = source(core_ura)
    raw_text = core_ura.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "pub const URA_KIND_SCOPE_LABELS",
            "core URA facade must own canonical URA-kind scope label inventory",
        ),
        (
            "pub fn ura_kind_scope_label(",
            "core URA facade must own URA-kind to scope-label projection",
        ),
        (
            "pub fn canonical_ura_kind_scope_labels(",
            "core URA facade must own sorted/deduplicated URA-kind scope validation",
        ),
    ):
        if token not in text:
            add("R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY", core_ura, 1, detail)
    if "canonical_ura_kind_scope_labels_rejects_drift" not in raw_text:
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            core_ura,
            1,
            "core URA tests must reject unknown, unsorted, duplicate, and whitespace-mutated scope labels",
        )
if ability_descriptor_surface.exists():
    text = source(ability_descriptor_surface)
    for token, detail in (
        (
            "OnlyUraKinds",
            "descriptor subject policy must support dynamic URA-kind scopes",
        ),
        (
            "crate::core::ura::ura_kind_scope_label",
            "descriptor subject policy must use the shared core URA-kind scope label projection",
        ),
        (
            "scope_subjects_from_manifest",
            "compiled plugin manifest subject scope must project into governed descriptors",
        ),
    ):
        if token not in text:
            add("R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY", ability_descriptor_surface, 1, detail)
    local_label = re.search(r"fn\s+ura_kind_scope_label\s*\(", text)
    if local_label:
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            ability_descriptor_surface,
            line_number(text, local_label.start()),
            "descriptor surface must not keep a second URA-kind scope label mapper",
        )
    admits_agent_body = rust_method_body(text, "admits_agent")
    if admits_agent_body is None:
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            ability_descriptor_surface,
            1,
            "descriptor scope rule must keep a distinct caller-axis admission method",
        )
    else:
        start, body = admits_agent_body
        if "ScopeRule::OnlyUraKinds(_) => false" not in body:
            add(
                "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
                ability_descriptor_surface,
                line_number(text, start),
                "caller/agent scope must fail closed for `OnlyUraKinds`; kind-wide scopes are subject-axis only",
            )
    raw_text = ability_descriptor_surface.read_text(encoding="utf-8", errors="replace")
    if "scope_rule_rejects_ura_kind_scope_on_caller_axis" not in raw_text:
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            ability_descriptor_surface,
            1,
            "descriptor tests must prove `OnlyUraKinds` fails closed on caller/agent scope",
        )
if ability_toml_parser.exists():
    text = source(ability_toml_parser)
    if (
        "fn parse_subject_scope" not in text
        or '("only_ura_kinds", false)' not in text
        or "ScopeRule::OnlyUraKinds" not in text
        or "crate::core::ura::canonical_ura_kind_scope_labels(&uras)" not in text
    ):
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            ability_toml_parser,
            1,
            "ability TOML parser must round-trip `only_ura_kinds` subject scopes through the shared core label validator",
        )
    if (
        "fn parse_agent_scope" not in text
        or 'scope_agents_kind cannot be `only_ura_kinds`' not in text
        or "scope_agents cannot use only_ura_kinds" not in text
    ):
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            ability_toml_parser,
            1,
            "ability TOML parser must reject `only_ura_kinds` on caller/agent scopes",
        )
if ability_descriptor_dir.exists():
    for ability_toml in sorted(ability_descriptor_dir.glob("**/*.ability.toml")):
        raw_text = ability_toml.read_text(encoding="utf-8", errors="replace")
        offset = raw_text.find('scope_agents_kind = "only_ura_kinds"')
        if offset != -1:
            add(
                "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
                ability_toml,
                line_number(raw_text, offset),
                "caller/agent scope must not use `only_ura_kinds`; use explicit URAs or authority bindings",
            )
if ability_manifest.exists():
    text = ability_manifest.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "ManifestSubjectScope",
            "compiled plugin manifests must carry subject-scope policy",
        ),
        (
            "OnlyUraKinds",
            "compiled plugin manifests must support dynamic URA-kind subject scopes",
        ),
        (
            "crate::core::ura::canonical_ura_kind_scope_labels(kinds)",
            "compiled plugin manifests must validate URA-kind subject scopes through the shared core label validator",
        ),
    ):
        if token not in text:
            add("R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY", ability_manifest, 1, detail)
    local_canonical = re.search(r"fn\s+canonical_subject_scope_values\s*\(", text)
    if local_canonical:
        add(
            "R146_REMOTE_DESKTOP_SUBJECT_SCOPE_BOUNDARY",
            ability_manifest,
            line_number(text, local_canonical.start()),
            "ability manifest must not keep a second URA-kind subject-scope validator",
        )
if route_resolver.exists():
    text = route_resolver.read_text(encoding="utf-8", errors="replace")
    if "keyring_management_system_agent_ability_resolves_from_local_device_authority" not in text:
        add(
            "R145_KEYRING_AND_PLUGIN_PUBLIC_OWNER_BOUNDARY",
            route_resolver,
            1,
            "route resolver tests must prove device.keyring.* resolves to keyring-management SystemAgent callee",
        )

owner_projection_rs = cli_root / "src/daemon/ability/owner_projection.rs"
ability_authority_mod = cli_root / "src/daemon/ability/authority/mod.rs"
if owner_projection_rs.exists():
    text = source(owner_projection_rs)
    raw_text = owner_projection_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "pub(crate) enum OwnerProjection",
            "ability owner-projection marker grammar must live in the shared OwnerProjection value object",
        ),
        (
            "pub(crate) fn parse(",
            "shared OwnerProjection must own marker parsing and validation",
        ),
        (
            "pub(crate) fn canonical(",
            "shared OwnerProjection must own canonical marker rendering",
        ),
        (
            "Plugin(String)",
            "shared OwnerProjection must keep plugin-plane marker parsing even though dispatch OwnerKind does not map it",
        ),
        (
            "fn is_valid_owner_projection_id(",
            "shared OwnerProjection must own owner-plane id validation",
        ),
    ):
        if token not in text:
            add("R156_OWNER_PROJECTION_VALUE_OBJECT", owner_projection_rs, 1, detail)
    for test_name in (
        "owner_projection_round_trips_all_canonical_shapes",
        "owner_projection_rejects_ambiguous_or_retired_markers",
    ):
        if test_name not in raw_text:
            add(
                "R156_OWNER_PROJECTION_VALUE_OBJECT",
                owner_projection_rs,
                1,
                f"OwnerProjection tests must keep {test_name}",
            )
else:
    add(
        "R156_OWNER_PROJECTION_VALUE_OBJECT",
        cli_root / "src/daemon/ability",
        1,
        "ability owner-projection marker grammar must be centralized in src/daemon/ability/owner_projection.rs",
    )
if ability_authority_mod.exists():
    text = source(ability_authority_mod)
    if "super::owner_projection::OwnerProjection" not in text:
        add(
            "R156_OWNER_PROJECTION_VALUE_OBJECT",
            ability_authority_mod,
            1,
            "AuthorityScope must validate owner_projection through the shared OwnerProjection value object",
        )
    for token, detail in (
        (
            "enum OwnerProjection",
            "authority module must not keep a second OwnerProjection enum",
        ),
        (
            "fn is_valid_owner_projection_id(",
            "authority module must not keep a second owner-projection id validator",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add("R156_OWNER_PROJECTION_VALUE_OBJECT", ability_authority_mod, line_number(text, offset), detail)
if dispatch_rs.exists():
    text = source(dispatch_rs)
    for token, detail in (
        (
            "owner_projection_for_kind(self).canonical()",
            "OwnerKind -> owner_projection rendering must go through the shared OwnerProjection value object",
        ),
        (
            "OwnerProjection::parse(owner_projection).ok()?",
            "owner_projection -> OwnerKind parsing must go through the shared OwnerProjection value object",
        ),
        (
            "OwnerProjection::Plugin(_) => None",
            "dispatch must explicitly fail closed for plugin-plane projections because OwnerKind has no Plugin actor",
        ),
    ):
        if token not in text:
            add("R156_OWNER_PROJECTION_VALUE_OBJECT", dispatch_rs, 1, detail)
    for token, detail in (
        (
            'strip_prefix("system-agent:")',
            "dispatch must not manually parse system-agent owner projections",
        ),
        (
            'strip_prefix("agent:")',
            "dispatch must not manually parse hosted-Agent owner projections",
        ),
        (
            'format!("system-agent:{agent_id}")',
            "dispatch must not manually render system-agent owner projections",
        ),
        (
            'format!("agent:{agent_id}")',
            "dispatch must not manually render hosted-Agent owner projections",
        ),
    ):
        offset = text.find(token)
        if offset != -1:
            add("R156_OWNER_PROJECTION_VALUE_OBJECT", dispatch_rs, line_number(text, offset), detail)

if dispatch_rs.exists():
    text = source(dispatch_rs)
    agent_root_body = rust_method_body(text, "agent_authority_root")
    if agent_root_body is None:
        add(
            "R112_AGENT_OWNER_EXPLICIT_HOSTED_ROOT",
            dispatch_rs,
            1,
            "OwnerKind::Agent must resolve through an explicit hosted-Agent authority root",
        )
    else:
        start, body = agent_root_body
        signature = text[start : text.find("{", start)]
        if "Result<String, AbilityControlPlaneError>" not in signature:
            add(
                "R112_AGENT_OWNER_EXPLICIT_HOSTED_ROOT",
                dispatch_rs,
                line_number(text, start),
                "agent_authority_root must fail closed with AbilityControlPlaneError instead of returning a synthesized String",
            )
        device_agent_offset = body.find("device_agent_ura")
        if device_agent_offset != -1:
            add(
                "R112_AGENT_OWNER_EXPLICIT_HOSTED_ROOT",
                dispatch_rs,
                line_number(text, start + device_agent_offset),
                "OwnerKind::Agent must not synthesize a device-sponsored SystemAgent URA when a hosted root is missing",
            )
        if "UnsupportedOwnerForAuthoritySet" not in body:
            add(
                "R112_AGENT_OWNER_EXPLICIT_HOSTED_ROOT",
                dispatch_rs,
                line_number(text, start),
                "missing hosted Agent roots must be reported as an unsupported owner plane",
            )
    supports_owner_body = rust_method_body(text, "supports_owner")
    if supports_owner_body is None:
        add(
            "R112_AGENT_OWNER_EXPLICIT_HOSTED_ROOT",
            dispatch_rs,
            1,
            "AbilityAuthorityContext must gate OwnerKind::Agent support explicitly",
        )
    else:
        start, body = supports_owner_body
        if "OwnerKind::Agent(agent_id) => self.hosted_agent_authority_root(agent_id).is_some()" not in body:
            add(
                "R112_AGENT_OWNER_EXPLICIT_HOSTED_ROOT",
                dispatch_rs,
                line_number(text, start),
                "supports_owner(OwnerKind::Agent) must depend on an explicit hosted-Agent root, not Device authority presence",
            )
    hosted_roots_body = rust_method_body(text, "hosted_agent_roots_for_device")
    if hosted_roots_body is not None:
        start, body = hosted_roots_body
        device_agent_offset = body.find("device_agent_ids")
        if device_agent_offset != -1:
            add(
                "R112_AGENT_OWNER_EXPLICIT_HOSTED_ROOT",
                dispatch_rs,
                line_number(text, start + device_agent_offset),
                "hosted-Agent authority inventory must reject device-sponsored SystemAgent URAs",
            )

device_control_plane_model = cli_root / "docs/design/ability-control-plane-model.md"
if device_control_plane_model.exists():
    text = source(device_control_plane_model)
    offset = text.find("DeviceAgent")
    if offset != -1:
        add(
            "R113_DEVICE_OWNER_INVENTORY_BOUNDARY",
            device_control_plane_model,
            line_number(text, offset),
            "target architecture notes must use SystemAgent or DeviceProfileProjection, not DeviceAgent",
        )
    if "DeviceProfileProjection" not in text:
        add(
            "R113_DEVICE_OWNER_INVENTORY_BOUNDARY",
            device_control_plane_model,
            1,
            "control-plane model must name the remaining direct Device-owner projection as a migration read-model",
        )

owner_truth_table = cli_root / "docs/spec/owner-truth-table/ability-owner-truth-table.tex"
if owner_truth_table.exists():
    text = owner_truth_table.read_text(encoding="utf-8", errors="replace")
    raw_text = text
    for token, detail in (
        (
            "SystemAgent-owned, Device-hosted",
            "active owner truth table must describe device.* descriptors as SystemAgent-owned and Device-hosted",
        ),
        (
            "\\codew{device.*} is a legacy/local public-name namespace, not a",
            "active owner truth table must separate legacy public names from descriptor ownership",
        ),
        (
            "A User account is a principal and",
            "active owner truth table must state that User/Account is principal/accountability, not Agent",
        ),
        (
            "agent/device.<id>.ability-management",
            "active owner truth table must pin dynamic deployment owner to the device-sponsored ability-management SystemAgent",
        ),
        (
            "agent/device.<id>.keyring-management",
            "active owner truth table must pin keyring owner to the keyring-management SystemAgent",
        ),
    ):
        if token not in raw_text:
            add("R146_OWNER_TRUTH_TABLE_CURRENT_ACTOR_MODEL", owner_truth_table, 1, detail)
    for forbidden, detail in (
        (
            "\\subsection{Device-owned ---",
            "active owner truth table must not use obsolete Device-owned section headings",
        ),
        (
            "with \\code{owner\\_ura = device URA}",
            "active owner truth table must not describe daemon-bundle descriptors as direct Device-owned rows",
        ),
        (
            "\\code{user/<user>} &",
            "active owner truth table must not model User account as a callable ability owner row",
        ),
        (
            "device/<id>}            & \\code{<id>} & \\code{-}    & \\code{-}    & rows~1",
            "CLI render projection must not map current rows 1/2/3/5 to direct Device ownership",
        ),
    ):
        offset = raw_text.find(forbidden)
        if offset != -1:
            add(
                "R146_OWNER_TRUTH_TABLE_CURRENT_ACTOR_MODEL",
                owner_truth_table,
                line_number(raw_text, offset),
                detail,
            )

device_profile_rs = cli_root / "src/daemon/ability/catalog/profiles/device.rs"
if device_profile_rs.exists():
    text = source(device_profile_rs)
    raw_text = device_profile_rs.read_text(encoding="utf-8", errors="replace")
    if "OwnerKind::DeviceProfileProjection" not in raw_text:
        add(
            "R113_DEVICE_OWNER_INVENTORY_BOUNDARY",
            device_profile_rs,
            1,
            "direct Device migration inventory must use OwnerKind::DeviceProfileProjection, not an ordinary Device owner name",
        )
    old_owner = re.search(r"\bOwnerKind::Device\b", raw_text)
    if old_owner is not None:
        add(
            "R113_DEVICE_OWNER_INVENTORY_BOUNDARY",
            device_profile_rs,
            line_number(raw_text, old_owner.start()),
            "direct Device migration inventory must not use obsolete OwnerKind::Device spelling",
        )
    for forbidden, detail in (
        (
            "device-profile Agent",
            "device profile comments must not describe a target architecture Agent identity",
        ),
        (
            "daemon-hosted Agent anchored at the local device URA",
            "device profile must be described as a migration projection, not a daemon-hosted Agent",
        ),
    ):
        offset = text.find(forbidden)
        if offset != -1:
            add(
                "R113_DEVICE_OWNER_INVENTORY_BOUNDARY",
                device_profile_rs,
                line_number(text, offset),
                detail,
            )
    if "fn direct_device_owner_inventory_is_explicit(" not in raw_text:
        add(
            "R113_DEVICE_OWNER_INVENTORY_BOUNDARY",
            device_profile_rs,
            1,
            "remaining direct OwnerKind::Device abilities must be pinned by an explicit live-catalog inventory test",
        )
    for token, detail in (
        (
            "let expected: Vec<String> = Vec::new();",
            "direct Device owner inventory must remain empty unless a bootstrap/self-maintenance exception is explicitly designed",
        ),
        (
            "direct Device-owner ability inventory changed; migrate the family to a SystemAgent",
            "direct Device owner inventory assertion must direct new families toward SystemAgent ownership",
        ),
    ):
        if token not in raw_text:
            add("R113_DEVICE_OWNER_INVENTORY_BOUNDARY", device_profile_rs, 1, detail)

seven_axes_fixture = cli_root / "tests/seven_axes_fixture/mod.rs"
if seven_axes_fixture.exists():
    text = seven_axes_fixture.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "invoke_device_hosted_system_ability",
            "positive seven-axes fixture helper must name Device as host, not descriptor owner",
        ),
        (
            "Invoke one Device-hosted system ability",
            "positive seven-axes fixture docs must say Device-hosted, not Device-owned",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", seven_axes_fixture, 1, detail)
    for forbidden, detail in (
        (
            "invoke_device_system_ability",
            "positive seven-axes fixture helper must not preserve obsolete Device-owner naming",
        ),
        (
            "Device-owned system ability",
            "positive seven-axes fixture docs must not describe SystemAgent-owned calls as Device-owned",
        ),
    ):
        offset = text.find(forbidden)
        if offset != -1:
            add(
                "R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL",
                seven_axes_fixture,
                line_number(text, offset),
                detail,
            )

conformance_rs = cli_root / "src/daemon/ability/conformance.rs"
if conformance_rs.exists():
    text = conformance_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "RemoteDesktopSystemAgent",
            "positive remote_desktop conformance domain must name remote-desktop SystemAgent ownership, not Device or plugin-management ownership",
        ),
        (
            "REMOTE_DESKTOP_SYSTEM_AGENT_BASELINE",
            "remote_desktop conformance baseline constant must name remote-desktop SystemAgent ownership",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", conformance_rs, 1, detail)
    for forbidden, detail in (
        (
            "DeviceRemoteDesktop",
            "positive remote_desktop conformance domain must not imply direct Device ownership",
        ),
        (
            "DEVICE_REMOTE_DESKTOP_BASELINE",
            "positive remote_desktop baseline constant must not imply direct Device ownership",
        ),
    ):
        offset = text.find(forbidden)
        if offset != -1:
            add(
                "R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL",
                conformance_rs,
                line_number(text, offset),
                detail,
            )

resolve_before_invoke_e2e = cli_root / "tests/resolve_before_invoke_e2e.rs"
if resolve_before_invoke_e2e.exists():
    text = resolve_before_invoke_e2e.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "REMOTE_RUNTIME_HEALTH_SYSTEM_AGENT_URA",
            "resolve-before-invoke positive fixture must use the remote runtime-health SystemAgent as callee",
        ),
        (
            '"owner_ura": REMOTE_RUNTIME_HEALTH_SYSTEM_AGENT_URA',
            "hub ability projection fixture must publish the remote SystemAgent owner, not the Device host",
        ),
        (
            '"host_device_ura": REMOTE_DEVICE_URA',
            "hub ability projection fixture must keep Device as execution host",
        ),
        (
            "OwnerKind::SystemAgent(",
            "fixture descriptor lookup must select a SystemAgent owner kind",
        ),
        (
            "RUNTIME_HEALTH_SYSTEM_AGENT_ID",
            "fixture descriptor lookup must select the runtime-health SystemAgent id",
        ),
        (
            "invoke_resolves_published_system_agent_ability_to_session_dispatch",
            "positive resolve-before-invoke test name must say SystemAgent ability, not Device-owned ability",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", resolve_before_invoke_e2e, 1, detail)
    for forbidden, detail in (
        (
            "URAKind::Device => OwnerKind::Device",
            "positive resolve-before-invoke fixture must not map a Device callee to direct Device owner",
        ),
        (
            "invoke_resolves_published_device_ability_to_session_dispatch",
            "positive resolve-before-invoke test name must not preserve obsolete Device ability wording",
        ),
    ):
        offset = text.find(forbidden)
        if offset != -1:
            add(
                "R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL",
                resolve_before_invoke_e2e,
                line_number(text, offset),
                detail,
            )

ability_catalog_read_model = cli_root / "src/daemon/federation/read_model/ability_catalog.rs"
if ability_catalog_read_model.exists():
    text = ability_catalog_read_model.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "projection_rows_for_live_hosts_at",
            "ability projection read model must expose live-host joins for Device-hosted SystemAgent owners",
        ),
        (
            "SystemAgent must not be inserted into `PresenceRegistry`",
            "ability projection read model docs must forbid synthetic SystemAgent presence rows",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", ability_catalog_read_model, 1, detail)

federation_wrappers_rs = cli_root / "src/daemon/invocation/dispatch/federation_wrappers.rs"
if federation_wrappers_rs.exists():
    text = federation_wrappers_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "device_sponsored_system_agent_host_node_id",
            "federation.resolve must join Device-hosted SystemAgent projections through the host Device",
        ),
        (
            "is_declared_daemon_native_system_agent_id",
            "federation.resolve must not surface arbitrary device-scoped Agents as SystemAgents",
        ),
        (
            "handle_resolve_includes_device_sponsored_system_agent_when_host_device_is_online",
            "resolve tests must prove host Device liveness surfaces its SystemAgent projection",
        ),
        (
            "SystemAgent visibility must be driven by host Device presence",
            "resolve tests must prove SystemAgent visibility is not an independent presence row",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", federation_wrappers_rs, 1, detail)

route_resolver_rs = cli_root / "src/daemon/invocation/routing/route_resolver.rs"
if route_resolver_rs.exists():
    text = route_resolver_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "device-scoped Agent owner is not a declared daemon-native SystemAgent",
            "route resolver must refuse undeclared device-scoped Agent owners on the SystemAgent execution path",
        ),
        (
            "device-sponsored SystemAgent host_node_id does not match owner device",
            "route resolver must verify SystemAgent owner Device matches the selected execution host",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", route_resolver_rs, 1, detail)

dispatch_rs = cli_root / "src/daemon/ability/dispatch.rs"
if dispatch_rs.exists():
    text = dispatch_rs.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "is_declared_daemon_native_system_agent_id(\n                        agent_id,",
            "ability authority context must only support declared daemon-native SystemAgent owners",
        ),
        (
            "owner_projection_refuses_undeclared_system_agent_owner_kind",
            "dispatch tests must reject undeclared system-agent owner projections",
        ),
        (
            "undeclared device-scoped Agent must not become a SystemAgent owner",
            "dispatch tests must reject arbitrary device-scoped Agent ids as SystemAgent owners",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", dispatch_rs, 1, detail)
    forbidden = "OwnerKind::Device | OwnerKind::SystemAgent(_) => self.authorities.device().is_some()"
    offset = text.find(forbidden)
    if offset != -1:
        add(
            "R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL",
            dispatch_rs,
            line_number(text, offset),
            "authority support must not admit arbitrary SystemAgent ids through Device authority",
        )

llm_profile = cli_root / "src/daemon/ability/catalog/profiles/llm.rs"
if llm_profile.exists():
    text = llm_profile.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "SYSTEM_SKILL_ABILITIES",
            "LLM profile tests must not preserve obsolete Device-skill naming for SystemAgent-owned skill abilities",
        ),
        (
            "Runtime-introspection metadata",
            "LLM profile fixture metadata must describe meta.* as runtime-introspection, not Device-owned metadata",
        ),
    ):
        if token not in text:
            add("R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL", llm_profile, 1, detail)
    for forbidden, detail in (
        (
            "DEVICE_SKILL_ABILITIES",
            "LLM profile must not preserve obsolete Device-skill naming for SystemAgent-owned skill abilities",
        ),
        (
            "Device-owned metadata",
            "LLM profile positive fixture metadata must not describe runtime-introspection as Device-owned",
        ),
    ):
        offset = text.find(forbidden)
        if offset != -1:
            add(
                "R147_ACTIVE_FIXTURE_CURRENT_ACTOR_MODEL",
                llm_profile,
                line_number(text, offset),
                detail,
            )

daemon_route_runtime = cli_root / "src/daemon/invocation/dispatch/daemon_route_runtime.rs"
if daemon_route_runtime.exists():
    text = daemon_route_runtime.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "delegation_exact_route_rejects_direct_device_callee",
            "daemon exact-route tests must prove hosted-agent delegation rejects direct Device callees",
        ),
        (
            "delegation_exact_route_derives_host_from_system_agent_callee",
            "daemon exact-route tests must prove delegation derives host only from SystemAgent callee",
        ),
        (
            "requires a device-sponsored SystemAgent callee",
            "hosted-agent delegation helper must name SystemAgent callee as the only valid callee",
        ),
    ):
        if token not in text:
            add("R148_DISPATCH_FIXTURE_SYSTEM_AGENT_CALLEE", daemon_route_runtime, 1, detail)
    for forbidden, detail in (
        (
            "Device or device-sponsored SystemAgent",
            "hosted-agent delegation helper must not accept direct Device callee semantics",
        ),
        (
            "if parsed.kind == crate::core::ura::URAKind::Device",
            "hosted-agent delegation helper must not derive host Device from a direct Device callee",
        ),
    ):
        offset = text.find(forbidden)
        if offset != -1:
            add(
                "R148_DISPATCH_FIXTURE_SYSTEM_AGENT_CALLEE",
                daemon_route_runtime,
                line_number(text, offset),
                detail,
            )

for fixture_path, forbidden_tokens in (
    (
        cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests/stream.rs",
        (
            (
                "publish_test_stream_route(&svc, TARGET_DEVICE_URA, CATALOGUE_READ)",
                "stream governance fixture must publish SystemAgent callee hosted by target Device",
            ),
            (
                "test_descriptor_ref(TARGET_DEVICE_URA, CATALOGUE_READ)",
                "stream governance fixture descriptor_ref must bind SystemAgent callee, not direct Device",
            ),
        ),
    ),
    (
        cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service_tests/bidi.rs",
        (
            (
                "resolve_route(TARGET_DEVICE_URA, CATALOGUE_READ)",
                "bidi governance fixture must resolve SystemAgent callee, not direct Device target/callee",
            ),
            (
                "test_descriptor_ref(TARGET_DEVICE_URA, CATALOGUE_READ)",
                "bidi governance fixture descriptor_ref must bind SystemAgent callee, not direct Device",
            ),
        ),
    ),
):
    if fixture_path.exists():
        text = fixture_path.read_text(encoding="utf-8", errors="replace")
        for forbidden, detail in forbidden_tokens:
            offset = text.find(forbidden)
            if offset != -1:
                add(
                    "R148_DISPATCH_FIXTURE_SYSTEM_AGENT_CALLEE",
                    fixture_path,
                    line_number(text, offset),
                    detail,
                )

companion_manager = cli_root / "src/daemon/plugins/companion/mod.rs"
if companion_manager.exists():
    text = companion_manager.read_text(encoding="utf-8", errors="replace")
    post_ready_start = text.find("pub fn ensure_running_after_daemon_ready")
    post_ready_end = text.find("pub fn enable", post_ready_start) if post_ready_start != -1 else -1
    post_ready_body = (
        text[post_ready_start:post_ready_end]
        if post_ready_start != -1 and post_ready_end != -1
        else ""
    )
    runtime_stop_start = text.find("pub fn stop_for_runtime_stop")
    runtime_stop_end = text.find("\n    }\n}", runtime_stop_start) if runtime_stop_start != -1 else -1
    runtime_stop_body = (
        text[runtime_stop_start:runtime_stop_end]
        if runtime_stop_start != -1 and runtime_stop_end != -1
        else ""
    )
    for token, detail in (
        (
            "DesktopCompanionReconcileFailure::plan_failed",
            "desktop companion daemon lifecycle must report plan failures",
        ),
        (
            "DesktopCompanionReconcileFailure::state_store_read_failed",
            "desktop companion post-ready reconcile must report desired-state read failures",
        ),
        (
            "post_ready_reconcile_reports_corrupt_desired_state_instead_of_skipping",
            "desktop companion tests must prove corrupt desired-state is not silently skipped",
        ),
        (
            "post_ready_reconcile_reports_plan_failure_instead_of_skipping",
            "desktop companion tests must prove plan failures are not silently skipped",
        ),
        (
            "runtime_stop_reports_stop_policy_plan_failure_instead_of_skipping",
            "desktop companion tests must prove runtime-stop plan failures are not silently skipped",
        ),
    ):
        if token not in text:
            add("R101_COMPANION_RECONCILE_NO_SILENT_SKIP", companion_manager, 1, detail)
    for token, detail in (
        (
            "let Ok(plan) = self.plan_package(package) else",
            "desktop companion post-ready reconcile must not silently continue on plan failure",
        ),
        (
            "let Ok(desired) = self\n                .state_store\n                .desired_state",
            "desktop companion post-ready reconcile must not silently continue on desired-state read failure",
        ),
    ):
        offset = post_ready_body.find(token)
        if offset != -1 and post_ready_start != -1:
            add(
                "R101_COMPANION_RECONCILE_NO_SILENT_SKIP",
                companion_manager,
                line_number(text, post_ready_start + offset),
                detail,
            )
    for token, detail in (
        (
            "let Ok(plan) = self.plan_package(package) else",
            "desktop companion runtime-stop cleanup must not silently continue on plan failure",
        ),
    ):
        offset = runtime_stop_body.find(token)
        if offset != -1 and runtime_stop_start != -1:
            add(
                "R101_COMPANION_RECONCILE_NO_SILENT_SKIP",
                companion_manager,
                line_number(text, runtime_stop_start + offset),
                detail,
            )

for adapter_path in (
    cli_root / "src/cli/commands/teach.rs",
    cli_root / "src/cli/commands/discover.rs",
):
    if adapter_path.exists():
        text = adapter_path.read_text(encoding="utf-8", errors="replace")
        if "LocalAbilityTarget::for_device_sponsored_system_ability(" not in text:
            add(
                "R159_LOCAL_SYSTEM_ABILITY_OWNER_HOST_SPLIT",
                adapter_path,
                1,
                "local system-ability adapters must resolve the catalog-owned SystemAgent callee separately from the Device execution host",
            )

if ability_deploy_target.exists():
    text = ability_deploy_target.read_text(encoding="utf-8", errors="replace")
    if "pub fn for_device_sponsored_system_ability" in text:
        for token, detail in (
            (
                "descriptor_public_ability_name(&callee_ura, &public_ability)",
                "local SystemAgent target construction must derive the Ability URA from the public descriptor name",
            ),
            (
                "for_device_sponsored_system_ability_with_dispatch",
                "local SystemAgent target construction must preserve the split between public descriptor name and daemon-local dispatch key",
            ),
            (
                "local_terminal_target_preserves_namespace_when_owner_has_same_id",
                "local target tests must prove terminal owner/ability URA boundaries remain invertible",
            ),
            (
                "local_service_target_can_separate_public_descriptor_from_dispatch_key",
                "local target tests must prove Service descriptor identity can differ from daemon-local dispatch key",
            ),
        ):
            if token not in text:
                add("R159_LOCAL_SYSTEM_ABILITY_OWNER_HOST_SPLIT", ability_deploy_target, 1, detail)

remote_invoke_path = cli_root / "src/daemon/invocation/routing/remote_invoke.rs"
if remote_invoke_path.exists():
    text = remote_invoke_path.read_text(encoding="utf-8", errors="replace")
    if "pub(crate) fn for_target_owned_selector" in text:
        for token, detail in (
            (
                "descriptor_public_ability_name(&owner_ura, selector)",
                "remote SystemAgent target construction must preserve a public namespace that equals its owner id",
            ),
            (
                "target_owned_terminal_selector_preserves_owner_ability_boundary",
                "remote target tests must prove terminal owner/ability URA boundaries remain invertible",
            ),
        ):
            if token not in text:
                add("R159_LOCAL_SYSTEM_ABILITY_OWNER_HOST_SPLIT", remote_invoke_path, 1, detail)

dispatch_catalog = cli_root / "src/daemon/ability/dispatch.rs"
if dispatch_catalog.exists():
    text = dispatch_catalog.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "dynamic_catalog_index",
            "dynamic descriptor lifecycle must not be inferred from the execution index",
        ),
        (
            "hot_register_descriptor_only_with_authority_scope",
            "the canonical catalog must accept discoverable descriptors without installing execution handlers",
        ),
        (
            "ControlPlaneImplementation::descriptor_only()",
            "descriptor-only catalog rows must declare the absence of an execution implementation explicitly",
        ),
    ):
        if token not in text:
            add("R160_DESCRIPTOR_CATALOG_EXECUTION_SPLIT", dispatch_catalog, 1, detail)

hot_agent_registrar = cli_root / "src/daemon/axon_bridge/hot_agent_registrar.rs"
if hot_agent_registrar.exists():
    text = hot_agent_registrar.read_text(encoding="utf-8", errors="replace")
    for token, detail in (
        (
            "register_descriptor_only(",
            "hosted-Agent synchronization must publish declaration-only manifests into the canonical catalog",
        ),
        (
            "dynamic_catalog_abilities_for_authority(",
            "hosted-Agent reconciliation must include non-executable catalog rows instead of enumerating only runtime handlers",
        ),
    ):
        if token not in text:
            add("R160_DESCRIPTOR_CATALOG_EXECUTION_SPLIT", hot_agent_registrar, 1, detail)

# Rules 161-164: durable automation and dynamic mutation authority must remain
# explicit across restart. These are lifecycle facts, not optional read-model
# decoration or ambient local-system defaults.
domain_mod = cli_root / "src/core/domain/mod.rs"
if domain_mod.exists():
    text = source(domain_mod)
    for token, detail in (
        ("pub fire_ledger: BTreeMap<i64, ScheduleFireRecord>", "schedule must persist a per-fire causality ledger"),
        ("pub invocation_ledger: Vec<LoopInvocationRecord>", "loop must persist body/verify invocation causality"),
        ("pub causal_parent_receipt_ura: Option<String>", "loop invocation records must preserve the causal parent receipt"),
    ):
        if token not in text:
            add("R161_DEFERRED_AUTOMATION_CAUSAL_LEDGER", domain_mod, 1, detail)

if schedule_mod.exists():
    text = source(schedule_mod)
    for token, detail in (
        ("fn update_fire_record(", "schedule terminal transitions must address an exact fire record"),
        ("fire_at_unix_ms: i64", "schedule complete/fail operations must carry the exact fire key"),
        ("validate_schedule_fire_ledger(&entry)?", "schedule boot bind must validate its causality ledger"),
    ):
        if token not in text:
            add("R161_DEFERRED_AUTOMATION_CAUSAL_LEDGER", schedule_mod, 1, detail)
    if "fn current_fire_at(" in text or "pub last_fire:" in text:
        add("R161_DEFERRED_AUTOMATION_CAUSAL_LEDGER", schedule_mod, 1, "schedule must not collapse concurrent fires into one ambient cursor")

if loop_mod.exists():
    text = source(loop_mod)
    for token, detail in (
        ("fn reserve_loop_invocation(", "loop must reserve a body/verify step before dispatch"),
        ("fn complete_loop_invocation(", "loop must persist exact invocation and receipt completion facts"),
        ("CausalContext::Scalar", "loop verify/next-body dispatch must cite its prior receipt"),
        ("validate_loop_invocation_ledger(&inst)?", "loop boot bind must validate its causality ledger"),
    ):
        if token not in text:
            add("R162_LOOP_INVOCATION_CAUSALITY", loop_mod, 1, detail)

deployment_store = cli_root / "src/daemon/ability/builtins/device_control/ability_management/store.rs"
if deployment_store.exists():
    text = source(deployment_store)
    for token, detail in (
        ("mutated_by: String", "dynamic ability records must retain the admitted logical actor"),
        ("creator_invocation_id: String", "dynamic ability records must retain their creating Invocation id"),
        ("validate_mutation_authority", "dynamic ability state must validate authority on read and write"),
    ):
        if token not in text:
            add("R163_DYNAMIC_ABILITY_MUTATION_AUTHORITY", deployment_store, 1, detail)

ability_ops = cli_root / "src/daemon/ability/builtins/device_control/ability_management/ops.rs"
if ability_ops.exists():
    text = source(ability_ops).split("#[cfg(test)]", 1)[0]
    if "Self::LocalSystem" in text or "return Ok(Self::LocalSystem)" in text:
        add("R163_DYNAMIC_ABILITY_MUTATION_AUTHORITY", ability_ops, 1, "public deploy/uninstall must not accept ambient _system.local mutation authority")

desktop_identity = cli_root / "plugins/remote-desktop/src/session_identity.rs"
desktop_consent = cli_root / "plugins/remote-desktop/src/session_consent.rs"
if desktop_identity.exists() and "creator_caller_ura: Option<String>" in source(desktop_identity):
    add("R164_REMOTE_DESKTOP_REQUIRED_AUTHORITY", desktop_identity, 1, "remote desktop creator caller must be structurally required")
if desktop_consent.exists() and "approval_receipt: Option<RemoteDesktopConsentReceipt>" in source(desktop_consent):
    add("R164_REMOTE_DESKTOP_REQUIRED_AUTHORITY", desktop_consent, 1, "remote desktop consent receipt must be structurally required")

# Rule 165: the retired CLI-private execution sidecar must stay absent. Local
# and remote calls share the daemon invocation adapter -> Axon LocalRuntime
# path; reintroducing a second socket would restore split execution ownership.
retired_runtime_dispatch = cli_root / "src/services/control/runtime_dispatch.rs"
if retired_runtime_dispatch.exists():
    add(
        "R165_SINGLE_RUNTIME_EXECUTION_PATH",
        retired_runtime_dispatch,
        1,
        "runtime-dispatch sidecar is retired; execution must enter Axon LocalRuntime through the canonical daemon invocation path",
    )
for path in production_files(cli_root / "src"):
    text = source(path)
    offset = text.find("runtime-dispatch.sock")
    if offset != -1:
        add(
            "R165_SINGLE_RUNTIME_EXECUTION_PATH",
            path,
            line_number(text, offset),
            "production code must not bind or depend on the retired runtime-dispatch socket",
        )


if violations:
    for violation in sorted(violations):
        print(
            f"{violation.rule}: {violation.path}:{violation.line}: {violation.detail}",
            file=sys.stderr,
        )
    print(
        f"architecture-convergence: FAILED ({len(violations)} violation(s))",
        file=sys.stderr,
    )
    raise SystemExit(1)

print("architecture-convergence: OK")
PY
