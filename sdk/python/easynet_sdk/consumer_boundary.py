"""Static consumer boundary audit helpers."""

from __future__ import annotations

import ast
import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable


_RAW_SYMBOL_PREFIX = "easynet" + "_"
_RAW_AXON_MODULE = "easynet" + "_" + "axon"
_SDK_MODULE = "easynet" + "_" + "sdk"
_RAW_ABI_SYMBOL = re.compile(r"\b" + _RAW_SYMBOL_PREFIX + r"(?!sdk\b)[A-Za-z0-9_]+")
_RAW_FFI_MARKERS = (
    "ctypes.CDLL",
    "ctypes.PyDLL",
    "ctypes.cdll",
    "ctypes.pydll",
    "dl" + "open",
)
_PYTHON_MANIFEST_NAMES = {"pyproject.toml", "setup.cfg", "setup.py"}
_REQUIREMENTS_FILE = re.compile(r"requirements(?:[-_A-Za-z0-9]*)?\.txt$")
_DEPENDENCY_NAME = re.compile(r"^\s*([A-Za-z0-9_.-]+)")
_FORBIDDEN_DEPENDENCY_NAMES = {
    "axon",
    "axon-pb2",
    "easynet-axon",
    "easynet-run-axon",
    "libeasynet-cli",
}
_LEGACY_TRANSPORT_PACKAGE = "_transport"


@dataclass(frozen=True)
class BoundaryViolation:
    """One consumer source boundary violation."""

    path: str
    rule: str
    detail: str
    line: int = 0


@dataclass(frozen=True)
class BoundaryAuditResult:
    """Boundary audit result for one consumer source tree."""

    root: str
    violations: tuple[BoundaryViolation, ...] = field(default_factory=tuple)

    @property
    def ok(self) -> bool:
        return not self.violations

    def require_ok(self) -> None:
        if self.ok:
            return
        details = "; ".join(
            f"{item.path}:{item.line or 1} {item.rule}: {item.detail}"
            for item in self.violations
        )
        raise AssertionError(f"consumer boundary audit failed: {details}")


@dataclass(frozen=True)
class ConsumerBoundaryAuditor:
    """Audit an SDK consumer for SDK boundary regressions."""

    ignored_dirs: tuple[str, ...] = (
        ".git",
        ".mypy_cache",
        ".pytest_cache",
        "__pycache__",
        "build",
        "dist",
        "tests",
        ".venv",
        "venv",
    )

    def audit_path(self, root: str | Path) -> BoundaryAuditResult:
        root_path = Path(root)
        violations: list[BoundaryViolation] = []
        for source in self._python_sources(root_path):
            violations.extend(self._audit_source(root_path, source))
        for manifest in self._python_manifests(root_path):
            violations.extend(self._audit_manifest(root_path, manifest))
        return BoundaryAuditResult(str(root_path), tuple(violations))

    def _python_sources(self, root: Path) -> Iterable[Path]:
        if root.is_file():
            if root.suffix == ".py":
                yield root
            return
        for source in root.rglob("*.py"):
            if any(part in self.ignored_dirs for part in source.parts):
                continue
            yield source

    def _python_manifests(self, root: Path) -> Iterable[Path]:
        if root.is_file():
            if _is_python_manifest(root):
                yield root
            return
        for manifest in root.rglob("*"):
            if any(part in self.ignored_dirs for part in manifest.parts):
                continue
            if manifest.is_file() and _is_python_manifest(manifest):
                yield manifest

    def _audit_source(self, root: Path, source: Path) -> tuple[BoundaryViolation, ...]:
        text = source.read_text(encoding="utf-8")
        relative = str(source.relative_to(root) if source != root else source.name)
        violations: list[BoundaryViolation] = []
        violations.extend(_audit_legacy_transport_path(relative))
        violations.extend(_audit_imports(relative, text))
        violations.extend(_audit_raw_ffi_markers(relative, text))
        violations.extend(_audit_raw_abi_symbols(relative, text))
        violations.extend(_audit_invocation_codec(relative, text))
        violations.extend(_audit_host_stream_codec(relative, text))
        violations.extend(_audit_receipt_chain_semantics(relative, text))
        violations.extend(_audit_context_causal_semantics(relative, text))
        violations.extend(_audit_publication_carrier_semantics(relative, text))
        violations.extend(_audit_admin_carrier_semantics(relative, text))
        violations.extend(_audit_mission_carrier_semantics(relative, text))
        violations.extend(_audit_addressing_semantics(relative, text))
        return tuple(violations)

    def _audit_manifest(
        self, root: Path, manifest: Path
    ) -> tuple[BoundaryViolation, ...]:
        text = manifest.read_text(encoding="utf-8")
        relative = str(manifest.relative_to(root) if manifest != root else manifest.name)
        return _audit_manifest_dependencies(relative, manifest.name, text)


def audit_consumer_boundary(root: str | Path) -> BoundaryAuditResult:
    """Audit whether an SDK consumer source tree stays above the SDK boundary."""

    return ConsumerBoundaryAuditor().audit_path(root)


def _audit_imports(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    try:
        tree = ast.parse(text)
    except SyntaxError as exc:
        return (
            BoundaryViolation(
                path=path,
                rule="python_parse",
                detail=str(exc),
                line=exc.lineno or 1,
            ),
        )
    violations: list[BoundaryViolation] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                root = alias.name.split(".", 1)[0]
                if root in {"ctypes", _RAW_AXON_MODULE} or _is_raw_axon_module(alias.name):
                    violations.append(
                        BoundaryViolation(
                            path=path,
                            rule="raw_lower_layer_import",
                            detail=alias.name,
                            line=node.lineno,
                        )
                    )
                if _is_legacy_transport_module(alias.name):
                    violations.append(
                        BoundaryViolation(
                            path=path,
                            rule="raw_transport_module",
                            detail=alias.name,
                            line=node.lineno,
                        )
                    )
        elif isinstance(node, ast.ImportFrom):
            module = node.module or ""
            root = module.split(".", 1)[0]
            if root in {"ctypes", _RAW_AXON_MODULE} or _is_raw_axon_module(module):
                violations.append(
                    BoundaryViolation(
                        path=path,
                        rule="raw_lower_layer_import",
                        detail=module,
                        line=node.lineno,
                    )
                )
            if _is_legacy_transport_module(module, level=node.level):
                violations.append(
                    BoundaryViolation(
                        path=path,
                        rule="raw_transport_module",
                        detail=module or "." * node.level,
                        line=node.lineno,
                    )
                )
            for alias in node.names:
                if _is_legacy_transport_module(alias.name, level=node.level):
                    violations.append(
                        BoundaryViolation(
                            path=path,
                            rule="raw_transport_module",
                            detail=alias.name,
                            line=node.lineno,
                        )
                    )
    return tuple(violations)


def _audit_legacy_transport_path(path: str) -> tuple[BoundaryViolation, ...]:
    parts = Path(path).parts
    if _LEGACY_TRANSPORT_PACKAGE not in parts:
        return tuple()
    return (
        BoundaryViolation(
            path=path,
            rule="raw_transport_module",
            detail="/".join(parts),
            line=1,
        ),
    )


def _audit_raw_abi_symbols(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return tuple(violations)
    docstrings = _docstring_node_ids(tree)
    for node in ast.walk(tree):
        if id(node) in docstrings:
            continue
        for symbol in sorted(_raw_abi_symbols_in_node(node)):
            violations.append(
                BoundaryViolation(
                    path=path,
                    rule="raw_c_abi_symbol",
                    detail=symbol,
                    line=getattr(node, "lineno", 1),
                )
            )
    return tuple(violations)


def _audit_raw_ffi_markers(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return tuple(violations)
    docstrings = _docstring_node_ids(tree)
    for node in ast.walk(tree):
        if id(node) in docstrings:
            continue
        for marker in sorted(_raw_ffi_markers_in_node(node)):
            violations.append(
                BoundaryViolation(
                    path=path,
                    rule="raw_ffi_loader",
                    detail=marker,
                    line=getattr(node, "lineno", 1),
                )
            )
    return tuple(violations)


def _raw_abi_symbols_in_node(node: ast.AST) -> set[str]:
    if isinstance(node, ast.Name):
        return {node.id} if _is_raw_abi_symbol(node.id) else set()
    if isinstance(node, ast.Attribute):
        return {node.attr} if _is_raw_abi_symbol(node.attr) else set()
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return {
            symbol
            for symbol in _RAW_ABI_SYMBOL.findall(node.value)
            if _is_raw_abi_symbol(symbol)
        }
    return set()


def _raw_ffi_markers_in_node(node: ast.AST) -> set[str]:
    if isinstance(node, ast.Call):
        dotted = _dotted_name(node.func)
        if dotted in _RAW_FFI_MARKERS:
            return {dotted}
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return {marker for marker in _RAW_FFI_MARKERS if marker in node.value}
    return set()


def _dotted_name(node: ast.AST) -> str:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        prefix = _dotted_name(node.value)
        return f"{prefix}.{node.attr}" if prefix else node.attr
    return ""


def _docstring_node_ids(tree: ast.AST) -> set[int]:
    ids: set[int] = set()
    for node in ast.walk(tree):
        if not isinstance(
            node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)
        ):
            continue
        body = getattr(node, "body", [])
        if not body:
            continue
        first = body[0]
        if (
            isinstance(first, ast.Expr)
            and isinstance(first.value, ast.Constant)
            and isinstance(first.value.value, str)
        ):
            ids.add(id(first.value))
    return ids


def _is_raw_abi_symbol(value: str) -> bool:
    return (
        _RAW_ABI_SYMBOL.fullmatch(value) is not None
        and value != _RAW_AXON_MODULE
        and not value.startswith("easynet" + "_" + "sdk")
    )


def _audit_invocation_codec(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        tree = None
    if tree is not None:
        for node in ast.walk(tree):
            if isinstance(node, ast.Call) and _is_json_codec_call(node):
                markers = sorted(_invocation_field_markers(node))
            elif isinstance(node, ast.Dict):
                markers = sorted(_invocation_dict_fields(node))
            else:
                continue
            if markers:
                violations.append(
                    BoundaryViolation(
                        path=path,
                        rule="raw_invocation_json_codec",
                        detail=", ".join(markers),
                        line=node.lineno,
                    )
                )
    return tuple(violations)


def _audit_host_stream_codec(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return tuple(violations)
    docstrings = _docstring_node_ids(tree)
    for node in ast.walk(tree):
        if id(node) in docstrings:
            continue
        markers = sorted(_host_stream_codec_markers(node))
        if not markers:
            continue
        violations.append(
            BoundaryViolation(
                path=path,
                rule="raw_host_stream_codec",
                detail=", ".join(markers),
                line=getattr(node, "lineno", 1),
            )
        )
    return tuple(violations)


def _host_stream_codec_markers(node: ast.AST) -> set[str]:
    markers: set[str] = set()
    if isinstance(node, ast.ClassDef) and node.name == "_RollingHash":
        markers.add("_RollingHash")
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in {
        "_canonical_json",
        "_stream_error",
    }:
        markers.add(node.name)
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        if node.value == "host_stream hash frame":
            markers.add("host_stream hash frame")
    if isinstance(node, ast.Dict):
        fields = {
            key.value
            for key in node.keys
            if isinstance(key, ast.Constant) and isinstance(key.value, str)
        }
        if {"stream_item", "seq"}.issubset(fields):
            markers.add("stream_item_frame")
        if "terminal" in fields and _dict_contains_string(node, "output_hash"):
            markers.add("terminal_output_hash_frame")
    return markers


def _dict_contains_string(node: ast.Dict, value: str) -> bool:
    for child in ast.walk(node):
        if isinstance(child, ast.Constant) and child.value == value:
            return True
    return False


def _audit_receipt_chain_semantics(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return tuple(violations)
    for node in ast.walk(tree):
        if not isinstance(node, ast.Compare):
            continue
        names = _attribute_names(node)
        if {"prev_receipt_hash", "self_hash"}.issubset(names):
            violations.append(
                BoundaryViolation(
                    path=path,
                    rule="raw_receipt_chain_semantics",
                    detail="prev_receipt_hash/self_hash continuity check",
                    line=getattr(node, "lineno", 1),
                )
            )
    return tuple(violations)


def _audit_context_causal_semantics(
    path: str, text: str
) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return tuple(violations)
    if _uses_sdk_receipt_projection(tree):
        return tuple()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        if not _is_causal_ref_constructor(node):
            continue
        if not _subtree_mentions_name(
            node, {"receipt", "parent_receipt", "receipt_ura", "receipt_hash_hex"}
        ):
            continue
        violations.append(
            BoundaryViolation(
                path=path,
                rule="raw_context_causal_ref",
                detail="Context child causal refs must come from SDK Receipt projection",
                line=getattr(node, "lineno", 1),
            )
        )
    return tuple(violations)


def _uses_sdk_receipt_projection(tree: ast.AST) -> bool:
    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute) and node.attr in {
            "ReceiptClient",
            "ReceiptRef",
        }:
            return True
        if isinstance(node, ast.Name) and node.id in {
            "ReceiptClient",
            "ReceiptRef",
        }:
            return True
    return False


def _is_causal_ref_constructor(node: ast.Call) -> bool:
    return _dotted_name(node.func).split(".")[-1] == "CausalRef"


def _audit_publication_carrier_semantics(
    path: str, text: str
) -> tuple[BoundaryViolation, ...]:
    return _audit_string_literals(
        path,
        text,
        rule="raw_publication_carrier",
        values={"ability.deploy", "meta.list_abilities"},
    )


def _audit_admin_carrier_semantics(
    path: str, text: str
) -> tuple[BoundaryViolation, ...]:
    return _audit_string_literals(
        path,
        text,
        rule="raw_admin_carrier",
        values={
            "agent.start",
            "agent.list",
            "agent.stop",
            "agent.refresh",
            "gateway.status",
            "session.list",
            "session.create",
            "session.delete",
            "hub.join",
            "hub.leave",
            "pairing.preflight",
            "pairing.validate",
            "credential.verify",
            "pairing.create",
            "federation.revoke",
        },
    )


def _audit_mission_carrier_semantics(
    path: str, text: str
) -> tuple[BoundaryViolation, ...]:
    return _audit_string_literals(
        path,
        text,
        rule="raw_mission_carrier",
        values={"mission.run", "mission.track", "mission.cancel", "mission.events"},
    )


_RAW_ADDRESSING_HELPER_NAMES = {
    "parse_ura",
    "owner_ability_ura",
    "owner_ura_for_ability",
    "canonical_ability_descriptor_ref",
    "ability_ura_from_descriptor_ref",
}


def _audit_addressing_semantics(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return tuple(violations)
    sdk_identity_facade = _is_sdk_identity_facade_module(path, tree)
    docstrings = _docstring_node_ids(tree)
    for node in ast.walk(tree):
        if id(node) in docstrings:
            continue
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            if node.name in _RAW_ADDRESSING_HELPER_NAMES and not sdk_identity_facade:
                violations.append(
                    BoundaryViolation(
                        path=path,
                        rule="raw_addressing_helper",
                        detail=node.name,
                        line=node.lineno,
                    )
                )
        if _is_descriptor_ref_assembly(node):
            violations.append(
                BoundaryViolation(
                    path=path,
                    rule="raw_descriptor_ref_assembly",
                    detail="DescriptorRef must come from SDK/Axon helper",
                    line=getattr(node, "lineno", 1),
                )
            )
        if _is_descriptor_ref_split(node):
            violations.append(
                BoundaryViolation(
                    path=path,
                    rule="raw_descriptor_ref_assembly",
                    detail="DescriptorRef parsing must come from SDK/Axon helper",
                    line=getattr(node, "lineno", 1),
                )
            )
    return tuple(violations)


def _is_sdk_identity_facade_module(path: str, tree: ast.AST) -> bool:
    if Path(path).name != "_sdk_identity.py":
        return False
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name == _SDK_MODULE:
                    return True
        if isinstance(node, ast.ImportFrom) and node.module == _SDK_MODULE:
            return True
    return False


def _is_descriptor_ref_assembly(node: ast.AST) -> bool:
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        return _subtree_has_string(node, "@") and _subtree_mentions_descriptor_parts(node)
    if isinstance(node, ast.JoinedStr):
        return _joined_str_has_at(node) and _subtree_mentions_descriptor_parts(node)
    return False


def _is_descriptor_ref_split(node: ast.AST) -> bool:
    if not isinstance(node, ast.Call):
        return False
    if not isinstance(node.func, ast.Attribute) or node.func.attr not in {"split", "rsplit"}:
        return False
    if not node.args or not isinstance(node.args[0], ast.Constant) or node.args[0].value != "@":
        return False
    return _subtree_mentions_name(node.func.value, {"descriptor_ref", "ref"})


def _joined_str_has_at(node: ast.JoinedStr) -> bool:
    return any(
        isinstance(value, ast.Constant)
        and isinstance(value.value, str)
        and "@" in value.value
        for value in node.values
    )


def _subtree_has_string(node: ast.AST, value: str) -> bool:
    return any(
        isinstance(child, ast.Constant) and child.value == value
        for child in ast.walk(node)
    )


def _subtree_mentions_descriptor_parts(node: ast.AST) -> bool:
    return _subtree_mentions_name(
        node,
        {
            "ability_ura",
            "ability",
            "descriptor_version",
            "version",
            "descriptor_ref",
            "ref",
        },
    )


def _subtree_mentions_name(node: ast.AST, names: set[str]) -> bool:
    for child in ast.walk(node):
        if isinstance(child, ast.Name) and child.id in names:
            return True
        if isinstance(child, ast.Attribute) and child.attr in names:
            return True
    return False


def _audit_string_literals(
    path: str, text: str, *, rule: str, values: set[str]
) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return tuple(violations)
    docstrings = _docstring_node_ids(tree)
    for node in ast.walk(tree):
        if id(node) in docstrings:
            continue
        if isinstance(node, ast.Constant) and node.value in values:
            violations.append(
                BoundaryViolation(
                    path=path,
                    rule=rule,
                    detail=str(node.value),
                    line=getattr(node, "lineno", 1),
                )
            )
    return tuple(violations)


def _attribute_names(node: ast.AST) -> set[str]:
    names: set[str] = set()
    for child in ast.walk(node):
        if isinstance(child, ast.Attribute):
            names.add(child.attr)
    return names


def _is_json_codec_call(node: ast.Call) -> bool:
    func = node.func
    return (
        isinstance(func, ast.Attribute)
        and func.attr in {"dumps", "loads"}
        and isinstance(func.value, ast.Name)
        and func.value.id == "json"
    )


def _invocation_field_markers(node: ast.AST) -> set[str]:
    fields = {
        "caller_ura",
        "callee_ura",
        "descriptor_ref",
        "subject_ura",
        "nonce_base64",
        "causal_context",
    }
    markers: set[str] = set()
    for child in ast.walk(node):
        if isinstance(child, ast.Constant) and isinstance(child.value, str):
            if child.value in fields:
                markers.add(child.value)
    return markers


def _invocation_dict_fields(node: ast.Dict) -> set[str]:
    fields = {
        "caller_ura",
        "callee_ura",
        "descriptor_ref",
        "subject_ura",
        "nonce_base64",
        "causal_context",
    }
    markers: set[str] = set()
    for key in node.keys:
        if isinstance(key, ast.Constant) and isinstance(key.value, str):
            if key.value in fields:
                markers.add(key.value)
    return markers if len(markers) >= 3 else set()


def _is_raw_axon_module(module: str) -> bool:
    return module == "axon" or module.startswith("axon.") or "axon_pb2" in module


def _is_legacy_transport_module(module: str, *, level: int = 0) -> bool:
    if module == _LEGACY_TRANSPORT_PACKAGE or module.startswith(
        _LEGACY_TRANSPORT_PACKAGE + "."
    ):
        return True
    if level and (
        module == _LEGACY_TRANSPORT_PACKAGE
        or module.startswith(_LEGACY_TRANSPORT_PACKAGE + ".")
    ):
        return True
    parts = module.split(".")
    return _LEGACY_TRANSPORT_PACKAGE in parts


def _is_python_manifest(path: Path) -> bool:
    name = path.name
    return name in _PYTHON_MANIFEST_NAMES or _REQUIREMENTS_FILE.fullmatch(name) is not None


def _audit_manifest_dependencies(
    path: str, filename: str, text: str
) -> tuple[BoundaryViolation, ...]:
    if filename == "pyproject.toml":
        return _audit_pyproject_dependencies(path, text)
    if filename == "setup.py":
        return _audit_setup_py_dependencies(path, text)
    return _audit_dependency_lines(path, text)


def _audit_pyproject_dependencies(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    try:
        document = tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        return (
            BoundaryViolation(
                path=path,
                rule="python_manifest_parse",
                detail=str(exc),
                line=getattr(exc, "lineno", 1) or 1,
            ),
        )
    dependencies: list[str] = []
    project = document.get("project")
    if isinstance(project, dict):
        dependencies.extend(_string_list(project.get("dependencies")))
        optional = project.get("optional-dependencies")
        if isinstance(optional, dict):
            for values in optional.values():
                dependencies.extend(_string_list(values))
    groups = document.get("dependency-groups")
    if isinstance(groups, dict):
        for values in groups.values():
            dependencies.extend(_string_list(values))
    build_system = document.get("build-system")
    if isinstance(build_system, dict):
        dependencies.extend(_string_list(build_system.get("requires")))
    return _violations_for_dependency_entries(path, dependencies)


def _audit_setup_py_dependencies(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    try:
        tree = ast.parse(text)
    except SyntaxError as exc:
        return (
            BoundaryViolation(
                path=path,
                rule="python_manifest_parse",
                detail=str(exc),
                line=exc.lineno or 1,
            ),
        )
    docstrings = _docstring_node_ids(tree)
    dependencies: list[tuple[str, int]] = []
    for node in ast.walk(tree):
        if id(node) in docstrings:
            continue
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            dependencies.append((node.value, getattr(node, "lineno", 1)))
    return _violations_for_dependency_entries(path, dependencies)


def _audit_dependency_lines(path: str, text: str) -> tuple[BoundaryViolation, ...]:
    dependencies: list[tuple[str, int]] = []
    for index, line in enumerate(text.splitlines(), start=1):
        stripped = line.split("#", 1)[0].strip()
        if not stripped or stripped.startswith(("[", "-r ", "--")):
            continue
        dependencies.append((stripped, index))
    return _violations_for_dependency_entries(path, dependencies)


def _violations_for_dependency_entries(
    path: str, entries: Iterable[str | tuple[str, int]]
) -> tuple[BoundaryViolation, ...]:
    violations: list[BoundaryViolation] = []
    for entry in entries:
        if isinstance(entry, tuple):
            raw, line = entry
        else:
            raw, line = entry, 1
        name = _dependency_name(raw)
        if name in _FORBIDDEN_DEPENDENCY_NAMES:
            violations.append(
                BoundaryViolation(
                    path=path,
                    rule="raw_lower_layer_dependency",
                    detail=name,
                    line=line,
                )
            )
    return tuple(violations)


def _dependency_name(value: str) -> str:
    match = _DEPENDENCY_NAME.match(value)
    if match is None:
        return ""
    return match.group(1).replace("_", "-").lower()


def _string_list(value: object) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item for item in value if isinstance(item, str)]
