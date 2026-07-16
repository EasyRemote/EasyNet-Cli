#!/usr/bin/env bash
set -euo pipefail

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
    axon_root / "core/runtime-rs/src/services/invocation",
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


def display(path: Path) -> str:
    for label, root in (("EasyNet-Cli", cli_root), ("EasyNet-Axon", axon_root)):
        try:
            return f"{label}/{path.relative_to(root)}"
        except ValueError:
            pass
    return str(path)


def add(rule: str, path: Path, line: int, detail: str) -> None:
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
        r"invoke_(?:local|remote)[A-Za-z0-9_]*\s*\(|"
        r"(?:DaemonInvocation|DescriptorBoundInvocation|InvocationClient)"
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


# Rule 2: Axon runtime owns terminal finalization. CLI and per-geometry
# runtime surfaces may project a finalized receipt, but may not mint one.
owner = axon_root / "core/runtime-rs/src/services/invocation/terminal_finalization.rs"
owner_text = source(owner) if owner.exists() else ""
owner_contract = (
    re.search(r"struct\s+TerminalFinalizationService\b", owner_text),
    re.search(r"fn\s+finalize\s*\(", owner_text),
    re.search(r"emit_terminal_receipt_from_admission\s*\(", owner_text),
    re.search(r"commit_side_effects\s*\(", owner_text),
)
if not all(owner_contract):
    add(
        "R2_TERMINAL_OWNER_MISSING",
        owner,
        1,
        "TerminalFinalizationService must own proof emission and terminal side effects",
    )

axon_invocation = axon_root / "core/runtime-rs/src/services/invocation"
receipt_factory = axon_invocation / "receipt_emitter.rs"
terminal_scan_files = production_files(axon_invocation, {".rs"}) + production_files(
    cli_root / "src/daemon/invocation", {".rs"}
)
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
        cli_root / "sdk/python/easynet_sdk/direct_runtime.py",
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
        cli_root / "sdk/python/easynet_sdk/direct_runtime.py",
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
# abilities are Device-owned `fs.*`; user blob resources are owner-local
# `files.*` abilities executed by the daemon-native `<user>.files` Agent. The
# OpenAI facade may project `openai.files.*`, but must invoke the user-owned
# file surface through that explicit authority root instead of reviving
# `<user>.files.get`-style dispatch names.
files_store = cli_root / "src/daemon/ability/builtins/resources/files_store/mod.rs"
if files_store.exists():
    text = source(files_store)
    for token, detail in (
        (
            'agent_ura(realm, user, "files")',
            "Files resource surface must declare an explicit daemon-native files executor root",
        ),
        (
            "OwnerKind::User(config.user.clone())",
            "files.put/get/list must be user-owned resource abilities, not Device system abilities",
        ),
        (
            "ControlPlaneImplementation::native_daemon()",
            "files.put/get/list must be bound to the daemon-native implementation root",
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

device_files = cli_root / "src/daemon/ability/builtins/device_control/files.rs"
if device_files.exists():
    text = source(device_files)
    for token, detail in (
        ('"fs.read"', "Device filesystem surface must keep fs.read under device control"),
        ('"fs.write"', "Device filesystem surface must keep fs.write under device control"),
        ('"fs.stat"', "Device filesystem surface must keep fs.stat under device control"),
        ('"fs.list"', "Device filesystem surface must keep fs.list under device control"),
        ("OwnerKind::Device", "Device filesystem surface must remain Device-owned"),
    ):
        if token not in text:
            add("R31_FILE_RESOURCE_OWNERSHIP_FORK", device_files, 1, detail)
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
            ("files_store::management_agent_ura", '"files.put"', "invoke_user_owned_rpc"),
        ),
        (
            "handle_file_retrieve_with_context",
            ("files_store::management_agent_ura", '"files.get"', "invoke_user_owned_rpc"),
        ),
        (
            "deref_to_data_url",
            ("files_store::management_agent_ura", '"files.get"', "invoke_user_owned_rpc"),
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

catalog_build = cli_root / "src/daemon/ability/catalog/build.rs"
if catalog_build.exists():
    text = source(catalog_build)
    body = rust_method_body(text, "declare_daemon_native_agent_authorities")
    if body is None:
        add(
            "R31_FILE_RESOURCE_OWNERSHIP_FORK",
            catalog_build,
            1,
            "catalog assembly must declare daemon-native resource executor authorities",
        )
    else:
        offset, method_body = body
        if "files::management_agent_ura(realm, user)" not in method_body:
            add(
                "R31_FILE_RESOURCE_OWNERSHIP_FORK",
                catalog_build,
                line_number(text, offset),
                "catalog assembly must declare the Files executor authority before registration",
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


# Rule 9: realm-wide voice signaling has one Hub authority. Publishing the
# same state machine under Device and Hub creates duplicate descriptor truth.
voice_catalog = cli_root / "src/daemon/ability/builtins/resources/voice.rs"
if voice_catalog.exists():
    text = source(voice_catalog)
    device_owner = re.search(r"\bOwnerKind::Device\b", text)
    hub_owner = re.search(r"\bOwnerKind::Hub\b", text)
    if device_owner:
        add(
            "R9_VOICE_OWNER_FORK",
            voice_catalog,
            line_number(text, device_owner.start()),
            "voice signaling must not mirror its Hub state machine under Device authority",
        )
    if not hub_owner:
        add(
            "R9_VOICE_HUB_OWNER_MISSING",
            voice_catalog,
            1,
            "voice signaling must publish its realm-wide state under Hub authority",
        )

# A Hub-owned realm aggregate cannot be backed by a daemon-local filesystem
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
            "Hub voice aggregate must use an injected realm-shared repository",
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
# `workspaces/` name is permitted only inside the explicit, one-time migration
# owner and registry-prefix rewrite; no runtime reader may select it as a
# fallback authority.
legacy_agent_root_allowed = {
    cli_root / "src/daemon/persistence/config.rs",
    cli_root / "src/daemon/persistence/agent_registry.rs",
}
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
        if path in legacy_agent_root_allowed:
            continue
        add(
            "R12_AGENT_ROOT_FORK",
            path,
            line_number(text, match.start()),
            "runtime agent state must use agents_root; workspaces is migration-only",
        )


# Rule 15: post-load agent registry rows own their canonical root path.
# Fresh creation and registry migration may derive `agents_root()/name`; steady
# state readers must call AgentEntry::required_root_path and fail closed when a
# row lacks `root_path`.
agent_root_fallback_allowed = {
    cli_root / "src/daemon/persistence/agent_registry.rs",
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

# Rule 16: exact daemon unary and server-stream routes must enter Axon
# LocalRuntime through one adapter owner. The tonic service may classify
# transport ingress, and dispatchers may own product behavior behind provider
# objects, but neither may reintroduce a direct exact-route execution table
# outside DaemonRouteRuntimeAdapter.
daemon_service = cli_root / "src/daemon/invocation/dispatch/daemon_invocation_service.rs"
unary_dispatcher = cli_root / "src/daemon/invocation/dispatch/unary_dispatcher.rs"
stream_dispatcher = cli_root / "src/daemon/invocation/streams/stream_dispatcher.rs"
daemon_route_runtime = cli_root / "src/daemon/invocation/dispatch/daemon_route_runtime.rs"
boot_invocation_routes = cli_root / "src/daemon/boot/invocation/mod.rs"
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
            ".register(owner_ura",
            "exact route registration must install routes into LocalRuntime",
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
            ".register_streams(owner_ura",
            "exact stream route registration must install routes into LocalRuntime",
        ),
        (
            "streams.dispatch_daemon_route_runtime(route, &inner).await",
            "exact stream route dispatch must delegate to the LocalRuntime route path",
        ),
    )
    for token, detail in service_requirements:
        if token not in service_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", daemon_service, 1, detail)
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
            ".open_stream(route, request, local_self_admitted)",
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
    )
    for token, detail in boot_requirements:
        if token not in boot_text:
            add("R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK", boot_invocation_routes, 1, detail)
    listener_offsets = [
        offset
        for token in ("spawn_uds_listener(", "spawn_tcp_tls_listener(")
        if (offset := boot_text.find(token)) >= 0
    ]
    stream_registration_offset = boot_text.find(
        "register_daemon_stream_routes(daemon_route_owner)"
    )
    if (
        listener_offsets
        and stream_registration_offset >= 0
        and stream_registration_offset > min(listener_offsets)
    ):
        add(
            "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK",
            boot_invocation_routes,
            line_number(boot_text, stream_registration_offset),
            "exact stream routes must be registered before invocation listeners are spawned",
        )

# Rule 23: CLI command modules may not own target-owned remote system ability
# routing. They map user input into payloads; the CLI daemon-client facade owns
# remote device/hub selector projection and caller identity selection. The
# descriptor-bound `ability invoke --node` path remains separate because it
# carries explicit origin-proof and subject semantics.
remote_system_ability_facade = (
    cli_root / "src/cli/daemon_client/remote_system_ability.rs"
)
cli_remote_system_fork_patterns = (
    re.compile(r"RemoteAbilityInvocationTarget::for_target_owned_selector"),
    re.compile(r"remote_invoke::invoke_remote_target\s*\("),
    re.compile(r"daemon::invocation::routing::remote_invoke::invoke_remote_target\s*\("),
)
cli_root_dir = cli_root / "src/cli"
if cli_root_dir.exists():
    for path in production_files(cli_root_dir, {".rs"}):
        if path == remote_system_ability_facade:
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
            "fn require_actor_ura(actor_ura: Option<&str>) -> anyhow::Result<&str>",
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
            "let actor_ura = require_actor_ura(request.actor_ura.as_deref())?",
            "revoke must validate actor_ura through the shared boundary validator",
        ),
        (
            ".revoke_grant(&request.grant_id, &owner_user_id, actor_ura",
            "revoke audit must persist the validated actor_ura, not a scalar fallback",
        ),
    )
    for token, detail in access_requirements:
        if token not in text:
            add("R18_ACCESS_CONTROL_ACTOR_URA_FORK", access_control, 1, detail)

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

    scalar_actor_fallback = re.compile(
        r"actor_ura[\s\S]{0,160}(?:unwrap_or|unwrap_or_else|or_else)\s*\("
        r"[\s\S]{0,160}owner_user_id|"
        r"owner_user_id[\s\S]{0,160}(?:unwrap_or|unwrap_or_else|or_else)\s*\("
        r"[\s\S]{0,160}actor_ura",
        re.S,
    )
    match = scalar_actor_fallback.search(text)
    if match:
        add(
            "R18_ACCESS_CONTROL_ACTOR_URA_FORK",
            access_control,
            line_number(text, match.start()),
            "audited actor_ura must not fall back to scalar owner_user_id",
        )


# Rule 32: Agent destructive lifecycle has one public boundary. `agent.stop`
# remains a non-destructive row/authority removal; `agent.purge` is the only
# destructive root-removal entry point and must be projected as such by catalog
# metadata. Regressing this boundary revives the old stop/purge semantic fork.
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
            "reg.register_rpc_with_owner(\n        ABILITY_PURGE_AGENT,\n        OwnerKind::Device",
            "Agent purge must register as a Device-owned lifecycle ability",
        ),
        (
            "fn purge_agent_handler(",
            "Agent purge must have a dedicated handler separate from stop",
        ),
        (
            "ensure_identity_bound_purge_supported()?",
            "Agent purge must check identity-bound deletion support before mutation",
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
if catalog_metadata.exists():
    text = source(catalog_metadata)
    metadata_requirements = (
        (
            "destructive: public_name == agent_names::AGENT_PURGE",
            "catalog hints must mark only agent.purge as destructive",
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


# Rule 19: voice call signaling is Hub-owned realm state. Static descriptor
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
            "Hub voice repository must be production-qualified only after shared-root validation",
        ),
        (
            "ExclusiveFileLock::acquire_for_data_path",
            "Hub voice mutations require a cross-process write guard",
        ),
        (
            "SharedFileLock::acquire_for_data_path",
            "Hub voice reads require a shared repository guard",
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
            "if hosts_hub_authority",
            "voice handlers may register only for Hub-capable authority",
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


# Rule 20: stream/bidi cancellation at the C ABI provider boundary is a
# transport/resource cancel request until a canonical terminal receipt is
# observed. Language SDK adapters may expose the request state, but must not
# synthesize lifecycle terminality for local stream or bidi cancellation.
ffi_v5_spec = cli_root / "docs/spec/ffi-abi-v5.md"
ffi_invocation = cli_root / "src/ffi/invocation/mod.rs"
go_cabi_runtime = cli_root / "sdk/go/cabi_runtime.go"
python_cabi_runtime = cli_root / "sdk/python/easynet_sdk/_cabi.py"
go_direct_runtime = cli_root / "sdk/go/direct_runtime.go"
python_direct_runtime = cli_root / "sdk/python/easynet_sdk/direct_runtime.py"
go_stream_facade = cli_root / "sdk/go/stream.go"
go_bidi_facade = cli_root / "sdk/go/bidi.go"
python_stream_facade = cli_root / "sdk/python/easynet_sdk/stream.py"
python_bidi_facade = cli_root / "sdk/python/easynet_sdk/bidi.py"
go_stream_tests = cli_root / "sdk/go/stream_test.go"
go_bidi_tests = cli_root / "sdk/go/bidi_test.go"
go_direct_runtime_tests = cli_root / "sdk/go/direct_runtime_test.go"
python_stream_tests = cli_root / "sdk/python/tests/test_stream.py"
python_bidi_tests = cli_root / "sdk/python/tests/test_bidi.py"
python_direct_runtime_tests = cli_root / "sdk/python/tests/test_direct_runtime.py"

if ffi_v5_spec.exists():
    text = ffi_v5_spec.read_text(encoding="utf-8", errors="replace")
    if "stream cancel and bidi cancel are cancel-request operations" not in text:
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_v5_spec,
            1,
            "ABI v5 contract must name stream/bidi cancel as request state, not terminal proof",
        )
    if not re.search(r"must\s+not\s+claim\s+lifecycle\s+terminality", text):
        add(
            "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
            ffi_v5_spec,
            1,
            "ABI v5 contract must forbid local cancel from claiming runtime terminality",
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
            "ABI v5 must not define local stream/bidi cancel or close as lifecycle terminal",
        )
else:
    add(
        "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK",
        ffi_v5_spec,
        1,
        "ABI v5 contract is required for stream/bidi cancellation terminal authority",
    )

if ffi_invocation.exists():
    text = source(ffi_invocation)
    ffi_requirements = (
        (
            "release_stream_with_reader_cancel(",
            "stream cancel must remain a local reader/resource release until lifecycle control exists",
        ),
        (
            "session.cancel.cancel()",
            "bidi cancel must remain an explicit local session cancellation request",
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
        r"fn\s+easynet_invocation_stream_cancel\b.*?\n}\n",
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
    bidi_cancel = re.search(
        r"fn\s+easynet_invocation_bidi_cancel\b.*?\n}\n",
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

if go_direct_runtime.exists():
    text = source(go_direct_runtime)
    go_direct_cancel_contracts = (
        (
            r"func\s+\(t\s+\*directRuntimeStreamTransport\)\s+Cancel\b.*?CancelRequested.*?terminal\"\s*:\s*false",
            "Go direct runtime stream cancel must project CancelRequested with terminal=false",
        ),
        (
            r"func\s+\(t\s+\*directRuntimeBidiTransport\)\s+Cancel\b.*?CancelRequested.*?terminal\"\s*:\s*false",
            "Go direct runtime bidi cancel must project CancelRequested with terminal=false",
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

if python_direct_runtime.exists():
    text = source(python_direct_runtime)
    python_direct_cancel_contracts = (
        (
            r"class\s+DirectRuntimeStreamTransport\b.*?def\s+cancel\b.*?\"state\"\s*:\s*\"CancelRequested\".*?\"terminal\"\s*:\s*False",
            "Python direct runtime stream cancel must project CancelRequested with terminal=False",
        ),
        (
            r"class\s+DirectRuntimeBidiTransport\b.*?def\s+cancel\b.*?\"state\"\s*:\s*\"CancelRequested\".*?\"terminal\"\s*:\s*False",
            "Python direct runtime bidi cancel must project CancelRequested with terminal=False",
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
        "TestDirectRuntimeStreamCancelProjectsNonTerminalRequest",
        "TestDirectRuntimeBidiCancelProjectsNonTerminalRequest",
        "Go direct runtime tests must prove stream/bidi cancel request projection",
    ),
    (
        python_direct_runtime_tests,
        "test_direct_runtime_stream_cancel_projects_non_terminal_request",
        "test_direct_runtime_bidi_cancel_projects_non_terminal_request",
        "Python direct runtime tests must prove stream/bidi cancel request projection",
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
admission_facade = cli_root / "src/daemon/invocation/admission/admission_facade.rs"

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
                "sign_with_canonical_signer(&signer).await?",
                "request_cancel_signed must independently sign the cancel command",
            ),
            (
                "signed_cancel.into_daemon_invocation()",
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

if admission_facade.exists():
    raw_text = admission_facade.read_text(encoding="utf-8", errors="replace")
    if "signed_invocation_cancel_command_replay_is_rejected" not in raw_text:
        add(
            "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK",
            admission_facade,
            1,
            "admission tests must prove signed cancel command replay rejection",
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
            "self.admission.with_transport_boundary(boundary)",
            "DaemonInvocationService must delegate the boundary into AdmissionFacade",
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
    for token in (
        "selected_route_descriptor_ref_comes_from_live_catalog_for_all_modes",
        "selected_route_rejects_missing_catalog_descriptor_proof",
        "selected_route_rejects_runtime_proof_that_drifted_from_catalog",
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
