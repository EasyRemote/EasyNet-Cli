#!/usr/bin/env python3
from __future__ import annotations

import argparse
import ast
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Iterable

from source_revision import AXON_REVISION_ROOTS, axon_root, git_source_revision

ROOT = Path(__file__).resolve().parents[2]
LANGUAGES = ("rust", "c_abi", "go", "python", "node", "java", "swift")
PYTHON_AST_SHAPE_SCHEMA = "python-ast-normalized-v1"


def run(
    command: list[str], *, cwd: Path = ROOT, env: dict[str, str] | None = None
) -> str:
    completed = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return completed.stdout


def result(
    parser: str,
    symbols: Iterable[str],
    members: Iterable[str],
    shapes: dict[str, str],
    **extra: Any,
) -> dict[str, Any]:
    symbol_list = sorted(set(symbols))
    member_list = sorted(set(members))
    if set(shapes) != set(symbol_list) | set(member_list):
        missing = sorted((set(symbol_list) | set(member_list)) - set(shapes))
        stale = sorted(set(shapes) - set(symbol_list) - set(member_list))
        raise ValueError(f"shape closure mismatch: missing={missing}:stale={stale}")
    return {
        "parser": parser,
        "symbols": symbol_list,
        "members": member_list,
        "shapes": dict(sorted(shapes.items())),
        **extra,
    }


def normalized_python_ast(value: Any) -> Any:
    if isinstance(value, ast.AST):
        fields = {}
        for name, nested in ast.iter_fields(value):
            if name == "type_params" and nested == []:
                continue
            fields[name] = normalized_python_ast(nested)
        return {"node": type(value).__name__, "fields": fields}
    if isinstance(value, list):
        return [normalized_python_ast(item) for item in value]
    if value is Ellipsis:
        return {"literal": "ellipsis"}
    if isinstance(value, bytes):
        return {"literal": "bytes", "hex": value.hex()}
    if isinstance(value, complex):
        return {"literal": "complex", "real": value.real, "imag": value.imag}
    return value


def python_shape(node: ast.AST) -> str:
    return json.dumps(
        normalized_python_ast(node),
        sort_keys=True,
        separators=(",", ":"),
    )


def python_inventory() -> dict[str, Any]:
    package = ROOT / "sdk/python/easynet_sdk"
    init_path = package / "__init__.py"
    init = ast.parse(init_path.read_text(encoding="utf-8"))
    exported: list[str] | None = None
    imports: dict[str, tuple[str, str]] = {}

    def record_import(public_name: str, module_name: str, source_name: str) -> None:
        normalized_module = module_name.lstrip(".")
        if not normalized_module or not source_name:
            raise ValueError(f"Python export map is incomplete: {public_name}")
        existing = imports.get(public_name)
        value = (normalized_module, source_name)
        if existing is not None and existing != value:
            raise ValueError(
                "Python export has conflicting explicit import targets: "
                f"{public_name}:{existing}:{value}"
            )
        imports[public_name] = value

    def literal_lazy_export_map(node: ast.AST) -> dict[str, tuple[str, str]]:
        try:
            value = ast.literal_eval(node)
        except Exception as exc:  # noqa: BLE001 - report fail-closed inventory cause.
            raise ValueError("Python _EXPORT_MODULES must be a literal dict") from exc
        if not isinstance(value, dict):
            raise ValueError("Python _EXPORT_MODULES must be a literal dict")
        mapped: dict[str, tuple[str, str]] = {}
        for key, item in value.items():
            if (
                not isinstance(key, str)
                or not isinstance(item, (list, tuple))
                or len(item) != 2
                or not all(isinstance(part, str) for part in item)
            ):
                raise ValueError(
                    "Python _EXPORT_MODULES entries must map string exports "
                    "to string module/name pairs"
                )
            if key in mapped:
                raise ValueError(f"Python _EXPORT_MODULES duplicate export: {key}")
            module_name, source_name = item
            mapped[key] = (module_name.lstrip("."), source_name)
        return mapped

    for node in init.body:
        if isinstance(node, ast.ImportFrom) and node.module:
            for alias in node.names:
                record_import(alias.asname or alias.name, node.module, alias.name)
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == "__all__"
            for target in node.targets
        ):
            value = ast.literal_eval(node.value)
            if not isinstance(value, (list, tuple)) or not all(
                isinstance(item, str) for item in value
            ):
                raise ValueError("Python __all__ must be a literal string list")
            exported = list(value)
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            if node.target.id == "_EXPORT_MODULES":
                for public_name, (module_name, source_name) in literal_lazy_export_map(
                    node.value
                ).items():
                    record_import(public_name, module_name, source_name)
    if exported is None:
        raise ValueError("Python public API requires explicit __all__")
    symbols: set[str] = set(exported)
    members: set[str] = set()
    shapes: dict[str, str] = {}
    modules: dict[str, ast.Module] = {}

    def module_path(module_name: str) -> Path:
        roots = {
            "easynet_sdk": ROOT / "sdk/python/easynet_sdk",
            "axon_sdk": axon_root() / "sdk/python/axon_sdk",
        }
        package_name, _, suffix = module_name.partition(".")
        base = roots.get(package_name)
        if base is None:
            raise ValueError(
                f"Python public re-export leaves owned SDK roots: {module_name}"
            )
        candidate = (
            base / (suffix.replace(".", "/") + ".py")
            if suffix
            else base / "__init__.py"
        )
        if not candidate.is_file():
            candidate = base / suffix.replace(".", "/") / "__init__.py"
        if not candidate.is_file():
            raise ValueError(f"Python public module is missing: {module_name}")
        return candidate

    def resolve(
        module_name: str, source_name: str, seen: set[tuple[str, str]]
    ) -> ast.AST:
        key = (module_name, source_name)
        if key in seen:
            raise ValueError(f"Python re-export cycle: {module_name}.{source_name}")
        seen.add(key)
        path = module_path(module_name)
        tree = modules.setdefault(
            str(path), ast.parse(path.read_text(encoding="utf-8"))
        )
        declarations = [
            node
            for node in tree.body
            if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name == source_name
        ]
        assignments = [
            node
            for node in tree.body
            if isinstance(node, (ast.Assign, ast.AnnAssign))
            and any(name == source_name for name in _assigned_names(node))
        ]
        if len(declarations) + len(assignments) == 1:
            return (declarations + assignments)[0]
        imported = []
        for node in tree.body:
            if not isinstance(node, ast.ImportFrom):
                continue
            for alias in node.names:
                if (alias.asname or alias.name) == source_name:
                    imported.append((node, alias))
        if len(imported) != 1:
            raise ValueError(
                f"Python export cannot be resolved uniquely: {module_name}.{source_name}"
            )
        node, alias = imported[0]
        if node.level:
            parts = module_name.split(".")
            package_depth = 0 if path.name == "__init__.py" else 1
            prefix = parts[: len(parts) - node.level + 1 - package_depth]
            target_module = ".".join([*prefix, *(node.module or "").split(".")])
        else:
            target_module = node.module or ""
        return resolve(target_module, alias.name, seen)

    for public_name in exported:
        if public_name not in imports:
            raise ValueError(f"Python export is not an explicit import: {public_name}")
        module_name, source_name = imports[public_name]
        declaration = resolve(f"easynet_sdk.{module_name}", source_name, set())
        if isinstance(declaration, ast.ClassDef):
            reject_duplicate_class_members(declaration, public_name)
        shapes[public_name] = python_shape(declaration)
        if isinstance(declaration, ast.ClassDef):
            for node in declaration.body:
                names: list[str] = []
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    names = [node.name]
                elif isinstance(node, (ast.Assign, ast.AnnAssign)):
                    names = list(_assigned_names(node))
                for name in names:
                    if name.startswith("_"):
                        continue
                    member = f"{public_name}.{name}"
                    members.add(member)
                    shapes[member] = python_shape(node)
    revision = git_source_revision(axon_root(), AXON_REVISION_ROOTS)
    return result(
        PYTHON_AST_SHAPE_SCHEMA,
        symbols,
        members,
        shapes,
        source_revision=revision,
    )


def reject_duplicate_class_members(node: ast.ClassDef, public_name: str) -> None:
    seen: dict[str, int] = {}
    for item in node.body:
        names: list[str] = []
        if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
            names = [item.name]
        elif isinstance(item, (ast.Assign, ast.AnnAssign)):
            names = list(_assigned_names(item))
        for name in names:
            if name in seen:
                raise ValueError(
                    "Python public class declares duplicate member: "
                    f"{public_name}.{name}:first_line={seen[name]}:"
                    f"duplicate_line={getattr(item, 'lineno', 0)}"
                )
            seen[name] = getattr(item, "lineno", 0)


def _assigned_names(node: ast.Assign | ast.AnnAssign) -> Iterable[str]:
    targets = node.targets if isinstance(node, ast.Assign) else [node.target]
    for target in targets:
        if isinstance(target, ast.Name):
            yield target.id


def go_inventory() -> dict[str, Any]:
    decoded = json.loads(
        run(["go", "run", "./tools/sdk-api-inventory/main.go", "-dir", "sdk/go"])
    )
    shapes = decoded.get("shapes")
    if not isinstance(shapes, dict):
        raise ValueError("Go inventory did not emit shapes")
    listed = run(
        ["go", "list", "-f", "{{.ImportPath}}|{{.Dir}}", "./..."], cwd=ROOT / "sdk/go"
    ).splitlines()
    package_roots = [
        str(Path(directory).resolve().relative_to(ROOT))
        for _, directory in (line.split("|", 1) for line in listed)
    ]
    return result(
        "go/ast",
        decoded["symbols"],
        decoded["members"],
        shapes,
        package_roots=sorted(package_roots),
    )


def node_inventory() -> dict[str, Any]:
    return json.loads(
        run(
            ["node", "sdk/conformance/typescript_public_api.mjs", "sdk/node/index.d.ts"]
        )
    )


def c_inventory() -> dict[str, Any]:
    with tempfile.NamedTemporaryFile(suffix=".json") as output:
        subprocess.run(
            [
                "clang",
                "-Xclang",
                "-ast-dump=json",
                "-fsyntax-only",
                "-x",
                "c",
                "include/easynet_cli.h",
            ],
            cwd=ROOT,
            check=True,
            stdout=output,
            stderr=subprocess.PIPE,
        )
        output.seek(0)
        tree = json.load(output)
    symbols: set[str] = set()
    members: set[str] = set()
    shapes: dict[str, str] = {}

    def walk(node: dict[str, Any], parent: str | None = None) -> None:
        kind = node.get("kind")
        name = node.get("name")
        public = isinstance(name, str) and (
            name.startswith("easynet_")
            or name.startswith("EASYNET_")
            or name.startswith("Easynet")
        )
        if public and kind in {"FunctionDecl", "TypedefDecl", "EnumDecl", "VarDecl"}:
            symbols.add(name)
            shapes[name] = json.dumps(
                _clang_shape(node), sort_keys=True, separators=(",", ":")
            )
            parent = name
        if (
            parent
            and kind in {"FieldDecl", "EnumConstantDecl"}
            and isinstance(name, str)
        ):
            item = f"{parent}.{name}"
            members.add(item)
            shapes[item] = json.dumps(
                _clang_shape(node), sort_keys=True, separators=(",", ":")
            )
        for child in node.get("inner", []):
            if isinstance(child, dict):
                walk(child, parent)

    walk(tree)
    return result("clang-ast", symbols, members, shapes)


def _clang_shape(node: dict[str, Any]) -> dict[str, Any]:
    return {
        key: _stable_clang_value(node[key])
        for key in ("kind", "name", "type", "storageClass", "variadic")
        if key in node
    }


def _stable_clang_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            key: _stable_clang_value(nested)
            for key, nested in sorted(value.items())
            if key not in {"id", "typeAliasDeclId"}
        }
    if isinstance(value, list):
        return [_stable_clang_value(item) for item in value]
    return value


def java_inventory() -> dict[str, Any]:
    sources = sorted(
        (ROOT / "sdk/java/src/main/java/run/runtime/sdk").glob("*.java")
    )
    with tempfile.TemporaryDirectory() as directory:
        classes = Path(directory)
        run(["javac", "--release", "17", "-d", str(classes), *map(str, sources)])
        class_names = [
            ".".join(path.relative_to(classes).with_suffix("").parts)
            for path in sorted(classes.rglob("*.class"))
            if "$" not in path.name
        ]
        symbols: set[str] = set()
        members: set[str] = set()
        shapes: dict[str, str] = {}
        for class_name in class_names:
            raw = run(
                ["javap", "-classpath", str(classes), "-public", "-s", class_name]
            )
            lines = [line.strip() for line in raw.splitlines() if line.strip()]
            header = next(
                (
                    line
                    for line in lines
                    if re.search(r"\b(class|interface|record|enum)\b", line)
                    and line.endswith("{")
                ),
                None,
            )
            if header is None:
                continue
            simple = class_name.rsplit(".", 1)[-1]
            symbols.add(simple)
            shapes[simple] = header
            pending: str | None = None
            for line in lines[lines.index(header) + 1 :]:
                if line == "}":
                    break
                if line.startswith("descriptor:"):
                    if pending is not None:
                        shapes[pending] += " " + line
                    continue
                if not line.startswith("public "):
                    continue
                match = re.search(r"([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:\(|;)", line)
                if match is None:
                    raise ValueError(f"unparsed javap member: {class_name}: {line}")
                name = match.group(1)
                if name == simple:
                    name = "new"
                item = f"{simple}.{name}"
                if item in members:
                    suffix = 2
                    while f"{item}#{suffix}" in members:
                        suffix += 1
                    item = f"{item}#{suffix}"
                members.add(item)
                shapes[item] = line
                pending = item
    return result("javap-classfile", symbols, members, shapes)


def swift_inventory() -> dict[str, Any]:
    package = ROOT / "sdk/swift"
    bin_path = run(["swift", "build", "--show-bin-path"], cwd=package).strip()
    target_info = json.loads(run(["swift", "-print-target-info"], cwd=package))
    target = target_info["target"]["triple"]
    sdk_path = run(["xcrun", "--sdk", "macosx", "--show-sdk-path"], cwd=package).strip()
    with tempfile.TemporaryDirectory() as directory:
        run(
            [
                "xcrun",
                "swift-symbolgraph-extract",
                "-module-name",
                "RuntimeSDK",
                "-target",
                target,
                "-sdk",
                sdk_path,
                "-I",
                str(Path(bin_path) / "Modules"),
                "-output-dir",
                directory,
                "-pretty-print",
                "-minimum-access-level",
                "public",
            ],
            cwd=package,
        )
        graphs = list(Path(directory).glob("*.symbols.json"))
        if len(graphs) != 1:
            raise ValueError(f"expected one Swift symbol graph, found {len(graphs)}")
        graph = json.loads(graphs[0].read_text(encoding="utf-8"))
    symbols: set[str] = set()
    members: set[str] = set()
    shapes: dict[str, str] = {}
    for entry in graph["symbols"]:
        identifier = entry["identifier"]["precise"]
        kind = entry["kind"]["identifier"]
        components = entry["pathComponents"]
        name = components[0]
        shape = "".join(
            fragment["spelling"] for fragment in entry["declarationFragments"]
        )
        if len(components) == 1 and kind not in {"swift.module"}:
            symbols.add(name)
            shapes[name] = shape
        elif len(components) > 1:
            item = f"{name}.{components[-1]}"
            if item in members:
                item += "#" + hashlib.sha256(identifier.encode()).hexdigest()[:8]
            members.add(item)
            shapes[item] = shape
    return result("swift-symbolgraph", symbols, members, shapes)


def rust_inventory() -> dict[str, Any]:
    source_root = axon_root()
    manifest = source_root / "sdk/rust/Cargo.toml"
    target = ROOT / "target/sdk-rustdoc-inventory"
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    run(
        [
            "cargo",
            "+nightly",
            "rustdoc",
            "--manifest-path",
            str(manifest),
            "--lib",
            "--",
            "-Z",
            "unstable-options",
            "--output-format",
            "json",
        ],
        env=env,
    )
    graph = json.loads((target / "doc/axon_sdk.json").read_text(encoding="utf-8"))
    index = graph["index"]
    crate_id = int(index[str(graph["root"])]["crate_id"])
    symbols: set[str] = set()
    members: set[str] = set()
    shapes: dict[str, str] = {}
    symbol_ids = {
        item_id
        for item_id, summary in graph["paths"].items()
        if summary.get("crate_id") == crate_id
    }
    parent_by_child: dict[str, str] = {}
    for parent_id, parent in index.items():
        for value in parent.get("inner", {}).values():
            if not isinstance(value, dict):
                continue
            for child_id in value.get("items", []):
                parent_by_child[str(child_id)] = parent_id
    public_symbols: list[tuple[str, str, str, dict[str, Any]]] = []
    for item_id in symbol_ids:
        item = index.get(item_id)
        if not item or item.get("visibility") != "public" or not item.get("name"):
            continue
        path = graph["paths"][item_id]["path"]
        name = path[-1]
        kind = graph["paths"][item_id]["kind"]
        public_symbols.append((name, item_id, kind, item))
    symbol_public_names: dict[str, str] = {}
    symbol_counts: dict[str, int] = {}
    for name, _, _, _ in public_symbols:
        symbol_counts[name] = symbol_counts.get(name, 0) + 1
    for name, item_id, kind, item in sorted(
        public_symbols,
        key=lambda entry: (entry[0], graph["paths"][entry[1]]["path"], entry[2]),
    ):
        public_name = name
        if symbol_counts[name] > 1:
            public_name += "#" + _stable_rustdoc_suffix(
                graph["paths"][item_id]["path"], kind
            )
        symbols.add(public_name)
        symbol_public_names[item_id] = public_name
        shapes[public_name] = json.dumps(
            {"kind": kind, "inner": _stable_rustdoc_value(item["inner"])},
            sort_keys=True,
            separators=(",", ":"),
        )
    for item_id, item in index.items():
        parent_id = parent_by_child.get(item_id)
        if (
            parent_id not in symbol_ids
            or item.get("visibility") != "public"
            or not item.get("name")
        ):
            continue
        owner = symbol_public_names.get(parent_id)
        if not owner:
            continue
        kind = next(iter(item.get("inner", {})), "unknown")
        public_name = f"{owner}.{item['name']}"
        if public_name in members:
            path = [*graph["paths"].get(parent_id, {}).get("path", []), item["name"]]
            public_name += "#" + _stable_rustdoc_suffix(path, kind)
        members.add(public_name)
        shapes[public_name] = json.dumps(
            {"kind": kind, "inner": _stable_rustdoc_value(item["inner"])},
            sort_keys=True,
            separators=(",", ":"),
        )
    revision = git_source_revision(source_root, AXON_REVISION_ROOTS)
    return result("rustdoc-json", symbols, members, shapes, source_revision=revision)


def _stable_rustdoc_suffix(path: list[str], kind: str) -> str:
    material = "::".join(path) + ":" + kind
    return hashlib.sha256(material.encode()).hexdigest()[:8]


def _stable_rustdoc_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            key: _stable_rustdoc_value(nested)
            for key, nested in sorted(value.items())
            if key not in {"crate_id", "id", "impls", "items"}
        }
    if isinstance(value, list):
        return [
            _stable_rustdoc_value(item) for item in value if not isinstance(item, int)
        ]
    return value


def self_test() -> None:
    duplicated = ast.parse(
        """
class DuplicateTrace:
    authority_proof_id: str = ""
    authority_proof_id: str = ""
"""
    ).body[0]
    if not isinstance(duplicated, ast.ClassDef):
        raise AssertionError("self-test fixture did not parse as a class")
    try:
        reject_duplicate_class_members(duplicated, "DuplicateTrace")
    except ValueError as error:
        if "DuplicateTrace.authority_proof_id" not in str(error):
            raise
    else:
        raise AssertionError("duplicate public class member was accepted")

    unique = ast.parse(
        """
class UniqueTrace:
    authority_proof_id: str = ""
    redacted: bool = False
    def explain(self) -> None:
        pass
"""
    ).body[0]
    if not isinstance(unique, ast.ClassDef):
        raise AssertionError("self-test unique fixture did not parse as a class")
    reject_duplicate_class_members(unique, "UniqueTrace")
    normalized = json.loads(python_shape(unique))
    if normalized["node"] != "ClassDef":
        raise AssertionError("normalized Python shape lost its declaration kind")
    if "type_params" in normalized["fields"]:
        raise AssertionError("normalized Python shape retained empty type parameters")
    ellipsis = ast.parse("value: str = ...").body[0]
    if '"literal":"ellipsis"' not in python_shape(ellipsis):
        raise AssertionError("normalized Python shape lost an ellipsis literal")

    with tempfile.TemporaryDirectory(prefix="sdk-source-revision-") as raw:
        repository = Path(raw)
        subprocess.run(["git", "init", "-q"], cwd=repository, check=True)
        subprocess.run(
            ["git", "config", "user.name", "Inventory Test"],
            cwd=repository,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.email", "inventory@example.test"],
            cwd=repository,
            check=True,
        )
        source = repository / "sdk/example.txt"
        source.parent.mkdir(parents=True)
        source.write_text("committed\n", encoding="utf-8")
        subprocess.run(["git", "add", "sdk/example.txt"], cwd=repository, check=True)
        subprocess.run(
            ["git", "commit", "-qm", "fixture"],
            cwd=repository,
            check=True,
        )
        clean = git_source_revision(repository, ("sdk",))
        if clean.startswith("working_tree:"):
            raise AssertionError("clean source revision was marked as a working tree")
        documentation = repository / "docs/evidence.md"
        documentation.parent.mkdir(parents=True)
        documentation.write_text("evidence\n", encoding="utf-8")
        subprocess.run(["git", "add", "docs/evidence.md"], cwd=repository, check=True)
        subprocess.run(
            ["git", "commit", "-qm", "documentation"],
            cwd=repository,
            check=True,
        )
        after_documentation = git_source_revision(repository, ("sdk",))
        if after_documentation != clean:
            raise AssertionError(
                "documentation-only commit changed the bounded source revision"
            )
        source.write_text("modified\n", encoding="utf-8")
        modified = git_source_revision(repository, ("sdk",))
        if not modified.startswith("working_tree:") or modified == clean:
            raise AssertionError("modified source revision was not content-attested")
        subprocess.run(["git", "add", "sdk/example.txt"], cwd=repository, check=True)
        subprocess.run(
            ["git", "commit", "-qm", "source update"],
            cwd=repository,
            check=True,
        )
        committed_update = git_source_revision(repository, ("sdk",))
        if committed_update.startswith("working_tree:") or committed_update == clean:
            raise AssertionError("committed source update did not advance the revision")
        source.unlink()
        deleted = git_source_revision(repository, ("sdk",))
        if not deleted.startswith("working_tree:") or deleted == committed_update:
            raise AssertionError("deleted source revision was not content-attested")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("language", nargs="?", choices=LANGUAGES)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("public_api_inventory self-test ok")
        return 0
    if args.language is None:
        parser.error("language is required unless --self-test is used")
    functions = {
        "rust": rust_inventory,
        "c_abi": c_inventory,
        "go": go_inventory,
        "python": python_inventory,
        "node": node_inventory,
        "java": java_inventory,
        "swift": swift_inventory,
    }
    try:
        inventory = functions[args.language]()
    except (
        OSError,
        ValueError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
    ) as error:
        print(f"public_api_inventory:{args.language}: {error}", file=sys.stderr)
        return 1
    encoded = json.dumps(inventory, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded, encoding="utf-8")
    else:
        print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
