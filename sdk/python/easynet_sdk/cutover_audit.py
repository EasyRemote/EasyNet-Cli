"""Static consumer cutover audit helpers."""

from __future__ import annotations

import ast
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable


_RAW_SYMBOL_PREFIX = "easynet" + "_"
_RAW_AXON_MODULE = "easynet" + "_" + "axon"
_RAW_ABI_SYMBOL = re.compile(r"\b" + _RAW_SYMBOL_PREFIX + r"(?!sdk\b)[A-Za-z0-9_]+")
_RAW_FFI_MARKERS = (
    "ctypes.CDLL",
    "ctypes.PyDLL",
    "ctypes.cdll",
    "ctypes.pydll",
    "dl" + "open",
)
@dataclass(frozen=True)
class CutoverViolation:
    """One consumer source boundary violation."""

    path: str
    rule: str
    detail: str
    line: int = 0


@dataclass(frozen=True)
class CutoverAuditResult:
    """Cutover audit result for one consumer source tree."""

    root: str
    violations: tuple[CutoverViolation, ...] = field(default_factory=tuple)

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
        raise AssertionError(f"cutover audit failed: {details}")


@dataclass(frozen=True)
class EasyRemoteCutoverAuditor:
    """Audit an EasyRemote-like consumer for SDK boundary regressions."""

    ignored_dirs: tuple[str, ...] = (
        ".git",
        ".mypy_cache",
        ".pytest_cache",
        "__pycache__",
        "build",
        "dist",
        ".venv",
        "venv",
    )

    def audit_path(self, root: str | Path) -> CutoverAuditResult:
        root_path = Path(root)
        violations: list[CutoverViolation] = []
        for source in self._python_sources(root_path):
            violations.extend(self._audit_source(root_path, source))
        return CutoverAuditResult(str(root_path), tuple(violations))

    def _python_sources(self, root: Path) -> Iterable[Path]:
        if root.is_file():
            if root.suffix == ".py":
                yield root
            return
        for source in root.rglob("*.py"):
            if any(part in self.ignored_dirs for part in source.parts):
                continue
            yield source

    def _audit_source(self, root: Path, source: Path) -> tuple[CutoverViolation, ...]:
        text = source.read_text(encoding="utf-8")
        relative = str(source.relative_to(root) if source != root else source.name)
        violations: list[CutoverViolation] = []
        violations.extend(_audit_imports(relative, text))
        violations.extend(_audit_raw_ffi_markers(relative, text))
        violations.extend(_audit_raw_abi_symbols(relative, text))
        violations.extend(_audit_invocation_codec(relative, text))
        return tuple(violations)


def audit_easyremote_cutover(root: str | Path) -> CutoverAuditResult:
    """Audit whether an EasyRemote-like source tree stays above the SDK boundary."""

    return EasyRemoteCutoverAuditor().audit_path(root)


def _audit_imports(path: str, text: str) -> tuple[CutoverViolation, ...]:
    try:
        tree = ast.parse(text)
    except SyntaxError as exc:
        return (
            CutoverViolation(
                path=path,
                rule="python_parse",
                detail=str(exc),
                line=exc.lineno or 1,
            ),
        )
    violations: list[CutoverViolation] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                root = alias.name.split(".", 1)[0]
                if root in {"ctypes", _RAW_AXON_MODULE} or _is_raw_axon_module(alias.name):
                    violations.append(
                        CutoverViolation(
                            path=path,
                            rule="raw_lower_layer_import",
                            detail=alias.name,
                            line=node.lineno,
                        )
                    )
        elif isinstance(node, ast.ImportFrom):
            module = node.module or ""
            root = module.split(".", 1)[0]
            if root in {"ctypes", _RAW_AXON_MODULE} or _is_raw_axon_module(module):
                violations.append(
                    CutoverViolation(
                        path=path,
                        rule="raw_lower_layer_import",
                        detail=module,
                        line=node.lineno,
                    )
                )
    return tuple(violations)


def _audit_raw_abi_symbols(path: str, text: str) -> tuple[CutoverViolation, ...]:
    violations: list[CutoverViolation] = []
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
                CutoverViolation(
                    path=path,
                    rule="raw_c_abi_symbol",
                    detail=symbol,
                    line=getattr(node, "lineno", 1),
                )
            )
    return tuple(violations)


def _audit_raw_ffi_markers(path: str, text: str) -> tuple[CutoverViolation, ...]:
    violations: list[CutoverViolation] = []
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
                CutoverViolation(
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


def _audit_invocation_codec(path: str, text: str) -> tuple[CutoverViolation, ...]:
    violations: list[CutoverViolation] = []
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
                    CutoverViolation(
                        path=path,
                        rule="raw_invocation_json_codec",
                        detail=", ".join(markers),
                        line=node.lineno,
                    )
                )
    return tuple(violations)


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
