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
