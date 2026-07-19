#!/usr/bin/env python3
from __future__ import annotations

import argparse
import ast
import copy
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_POLICY = ROOT / "sdk/conformance/edge-adapter-policy.v1.json"
DEFAULT_MANIFEST = ROOT / "sdk/conformance/canonical-public-api.json"
LANGUAGES = ("rust", "c_abi", "go", "python", "node", "java", "swift")
SECTIONS = ("languages", "members")
STABLE_VERSION = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")


class PolicyError(ValueError):
    pass


def fail(message: str) -> None:
    raise PolicyError(message)


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail(f"object_required:{path}")
    return value


def canonical_digest(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def stable_version(value: Any, field: str) -> tuple[int, int, int]:
    matched = STABLE_VERSION.fullmatch(str(value))
    if matched is None:
        fail(f"stable_semver_required:{field}:{value}")
    return tuple(int(part) for part in matched.groups())


def read_version(root: Path, source: dict[str, Any]) -> str:
    required = {"package_id", "path", "format", "declared_version"}
    if not isinstance(source, dict) or set(source) != required:
        fail("invalid_version_source")
    path = root / str(source["path"])
    if source["format"] == "plain":
        actual = path.read_text(encoding="utf-8").strip()
    elif source["format"] == "toml-project-version":
        document = tomllib.loads(path.read_text(encoding="utf-8"))
        actual = str(document.get("project", {}).get("version", ""))
    else:
        fail(f"unsupported_version_source_format:{source['format']}")
    if actual != source["declared_version"]:
        fail(
            f"declared_package_version_drift:{source['package_id']}:"
            f"expected={source['declared_version']}:actual={actual}"
        )
    stable_version(actual, f"package:{source['package_id']}")
    return actual


def validate_release_policy(policy: dict[str, Any], root: Path) -> None:
    release = policy.get("release_policy")
    required = {"removal_trigger", "removal_version", "version_sources"}
    if not isinstance(release, dict) or set(release) != required:
        fail("invalid_release_policy")
    if release["removal_trigger"] != "explicit-major-version-cutover":
        fail("invalid_removal_trigger")
    removal = stable_version(release["removal_version"], "removal_version")
    if removal[1:] != (0, 0):
        fail("removal_version_must_start_major_release")
    sources = release["version_sources"]
    if not isinstance(sources, list) or not sources:
        fail("version_sources_required")
    package_ids = [source.get("package_id") for source in sources]
    if package_ids != sorted(set(package_ids)):
        fail("version_sources_not_unique")
    current_versions = [
        stable_version(read_version(root, source), "current") for source in sources
    ]
    if any(current >= removal for current in current_versions):
        fail("removal_version_not_after_current_release")
    if any(current[0] >= removal[0] for current in current_versions):
        fail("removal_version_is_not_a_future_major_cutover")

    adapters = policy.get("adapters")
    if not isinstance(adapters, list):
        fail("edge_adapters_required")
    adapter_ids = [
        adapter.get("id") for adapter in adapters if isinstance(adapter, dict)
    ]
    if adapter_ids != sorted(set(adapter_ids)) or len(adapter_ids) != len(adapters):
        fail("edge_adapter_ids_not_unique")
    for adapter in adapters:
        if adapter.get("release_removal_version") != release["removal_version"]:
            fail(f"adapter_removal_version_mismatch:{adapter.get('id')}")


def surface_counts(surface: dict[str, Any]) -> dict[str, dict[str, int]]:
    counts: dict[str, dict[str, int]] = {}
    for section in SECTIONS:
        graph = surface.get(section)
        if not isinstance(graph, dict) or set(graph) != set(LANGUAGES):
            fail(f"invalid_non_canonical_surface:{section}")
        counts[section] = {}
        for language in LANGUAGES:
            values = graph[language]
            if not isinstance(values, list) or values != sorted(set(values)):
                fail(f"non_canonical_surface_not_sorted:{section}:{language}")
            counts[section][language] = len(values)
    return counts


def validate_frozen_surface(policy: dict[str, Any], manifest: dict[str, Any]) -> None:
    frozen = policy.get("frozen_non_canonical_public_surface")
    required = {
        "manifest_path",
        "json_pointer",
        "sha256",
        "shape_sha256",
        "counts",
    }
    if not isinstance(frozen, dict) or set(frozen) != required:
        fail("invalid_frozen_non_canonical_surface")
    if (
        frozen["manifest_path"] != "sdk/conformance/canonical-public-api.json"
        or frozen["json_pointer"] != "/non_canonical"
    ):
        fail("invalid_frozen_non_canonical_surface_reference")
    surface = manifest.get("non_canonical")
    if not isinstance(surface, dict):
        fail("manifest_non_canonical_surface_required")
    actual_counts = surface_counts(surface)
    if actual_counts != frozen["counts"]:
        fail("non_canonical_surface_count_drift")
    actual_digest = canonical_digest(surface)
    if actual_digest != frozen["sha256"]:
        fail(
            "non_canonical_surface_drift:"
            f"expected={frozen['sha256']}:actual={actual_digest}"
        )
    shapes = manifest.get("shape_sha256")
    if not isinstance(shapes, dict) or set(shapes) != set(LANGUAGES):
        fail("manifest_shape_inventory_required")
    legacy_shapes: dict[str, dict[str, str]] = {}
    for language in LANGUAGES:
        items = sorted(
            set(surface["languages"][language]) | set(surface["members"][language])
        )
        language_shapes = shapes[language]
        if not isinstance(language_shapes, dict):
            fail(f"manifest_shape_inventory_required:{language}")
        missing = sorted(set(items) - set(language_shapes))
        if missing:
            fail(f"non_canonical_shape_missing:{language}:{missing}")
        legacy_shapes[language] = {item: language_shapes[item] for item in items}
    actual_shape_digest = canonical_digest(legacy_shapes)
    if actual_shape_digest != frozen["shape_sha256"]:
        fail(
            "non_canonical_shape_drift:"
            f"expected={frozen['shape_sha256']}:actual={actual_shape_digest}"
        )


GO_AST_SCANNER = r"""
package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

type Site struct {
	Path        string `json:"path"`
	Kind        string `json:"kind"`
	Symbol      string `json:"symbol"`
	Owner       string `json:"owner"`
	Occurrences int    `json:"occurrences"`
}

type Output struct {
	Symbols []string `json:"symbols"`
	Members []string `json:"members"`
	Sites   []Site   `json:"sites"`
}

func receiverName(expr ast.Expr) string {
	switch value := expr.(type) {
	case *ast.Ident:
		return value.Name
	case *ast.StarExpr:
		return receiverName(value.X)
	case *ast.IndexExpr:
		return receiverName(value.X)
	case *ast.IndexListExpr:
		return receiverName(value.X)
	default:
		return ""
	}
}

func functionOwner(decl *ast.FuncDecl) string {
	if decl == nil {
		return "package"
	}
	if decl.Recv == nil || len(decl.Recv.List) == 0 {
		return decl.Name.Name
	}
	receiver := receiverName(decl.Recv.List[0].Type)
	if receiver == "" {
		return decl.Name.Name
	}
	return receiver + "." + decl.Name.Name
}

func exported(name string) bool {
	return ast.IsExported(name)
}

func within(node ast.Node, position token.Pos) bool {
	return node != nil && node.Pos() <= position && position <= node.End()
}

func identifierKind(stack []ast.Node, identifier *ast.Ident) string {
	for index := len(stack) - 1; index >= 0; index-- {
		switch node := stack[index].(type) {
		case *ast.TypeSpec:
			if node.Name == identifier {
				return "adapter_type_definition"
			}
		case *ast.FuncDecl:
			if node.Recv != nil && within(node.Recv, identifier.Pos()) {
				return "adapter_receiver_definition"
			}
		case *ast.CompositeLit:
			if within(node.Type, identifier.Pos()) {
				return "adapter_construction"
			}
		}
	}
	return "adapter_reference"
}

func enclosingFunction(stack []ast.Node) *ast.FuncDecl {
	for index := len(stack) - 1; index >= 0; index-- {
		if declaration, ok := stack[index].(*ast.FuncDecl); ok {
			return declaration
		}
	}
	return nil
}

func main() {
	if len(os.Args) != 5 {
		fmt.Fprintln(os.Stderr, "usage: scanner <root> <scan-dir> <adapter-source> <config-json>")
		os.Exit(2)
	}
	root, scanDir, adapterSource := os.Args[1], os.Args[2], os.Args[3]
	var config struct {
		AdapterType string   `json:"adapter_type"`
		OldCalls    []string `json:"old_calls"`
	}
	if err := json.Unmarshal([]byte(os.Args[4]), &config); err != nil {
		panic(err)
	}
	oldCalls := map[string]bool{}
	for _, name := range config.OldCalls {
		oldCalls[name] = true
	}
	fset := token.NewFileSet()
	siteCounts := map[string]int{}
	siteValues := map[string]Site{}
	symbols := map[string]bool{}
	members := map[string]bool{}

	err := filepath.WalkDir(filepath.Join(root, scanDir), func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if entry.IsDir() {
			if entry.Name() == "vendor" || entry.Name() == ".git" {
				return filepath.SkipDir
			}
			return nil
		}
		if !strings.HasSuffix(path, ".go") {
			return nil
		}
		file, parseErr := parser.ParseFile(fset, path, nil, parser.SkipObjectResolution)
		if parseErr != nil {
			return parseErr
		}
		legacyAliases := map[string]bool{}
		for _, imported := range file.Imports {
			pathValue, unquoteErr := strconv.Unquote(imported.Path.Value)
			if unquoteErr != nil {
				return unquoteErr
			}
			if pathValue != "easynet.run/cli/sdk/go" {
				continue
			}
			alias := "easynet"
			if imported.Name != nil {
				alias = imported.Name.Name
			}
			legacyAliases[alias] = true
		}
		relative, relativeErr := filepath.Rel(root, path)
		if relativeErr != nil {
			return relativeErr
		}
		relative = filepath.ToSlash(relative)
		samePackage := filepath.Dir(relative) == filepath.Dir(adapterSource)
		if relative == adapterSource {
			for _, declaration := range file.Decls {
				switch value := declaration.(type) {
				case *ast.GenDecl:
					for _, spec := range value.Specs {
						switch nested := spec.(type) {
						case *ast.TypeSpec:
							if exported(nested.Name.Name) {
								symbols[nested.Name.Name] = true
							}
							if interfaceType, ok := nested.Type.(*ast.InterfaceType); ok {
								for _, field := range interfaceType.Methods.List {
									for _, name := range field.Names {
										if exported(name.Name) {
											members[nested.Name.Name+"."+name.Name] = true
										}
									}
								}
							}
						case *ast.ValueSpec:
							for _, name := range nested.Names {
								if exported(name.Name) {
									symbols[name.Name] = true
								}
							}
						}
					}
				case *ast.FuncDecl:
					if exported(value.Name.Name) {
						if value.Recv == nil || len(value.Recv.List) == 0 {
							symbols[value.Name.Name] = true
						} else {
							receiver := receiverName(value.Recv.List[0].Type)
							if exported(receiver) {
								members[receiver+"."+value.Name.Name] = true
							}
						}
					}
				}
			}
		}

		stack := []ast.Node{}
		ast.Inspect(file, func(node ast.Node) bool {
			if node == nil {
				stack = stack[:len(stack)-1]
				return true
			}
			stack = append(stack, node)
			owner := functionOwner(enclosingFunction(stack))
			record := func(kind, symbol string) {
				key := strings.Join([]string{relative, kind, symbol, owner}, "\x00")
				siteCounts[key]++
				siteValues[key] = Site{Path: relative, Kind: kind, Symbol: symbol, Owner: owner}
			}
			if identifier, ok := node.(*ast.Ident); ok && identifier.Name == config.AdapterType {
				record(identifierKind(stack, identifier), identifier.Name)
			}
			if call, ok := node.(*ast.CallExpr); ok {
				if identifier, direct := call.Fun.(*ast.Ident); direct && oldCalls[identifier.Name] && (samePackage || legacyAliases["."]) {
					record("old_facade_call", identifier.Name)
				}
				if selector, selected := call.Fun.(*ast.SelectorExpr); selected && oldCalls[selector.Sel.Name] {
					if qualifier, qualified := selector.X.(*ast.Ident); qualified && legacyAliases[qualifier.Name] {
						record("old_facade_call", selector.Sel.Name)
					}
				}
			}
			return true
		})
		return nil
	})
	if err != nil {
		panic(err)
	}

	output := Output{}
	for name := range symbols {
		output.Symbols = append(output.Symbols, name)
	}
	for name := range members {
		output.Members = append(output.Members, name)
	}
	for key, site := range siteValues {
		site.Occurrences = siteCounts[key]
		output.Sites = append(output.Sites, site)
	}
	sort.Strings(output.Symbols)
	sort.Strings(output.Members)
	sort.Slice(output.Sites, func(left, right int) bool {
		a, b := output.Sites[left], output.Sites[right]
		if a.Path != b.Path { return a.Path < b.Path }
		if a.Kind != b.Kind { return a.Kind < b.Kind }
		if a.Symbol != b.Symbol { return a.Symbol < b.Symbol }
		return a.Owner < b.Owner
	})
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(output); err != nil {
		panic(err)
	}
}
"""


def go_inventory(root: Path, adapter: dict[str, Any]) -> dict[str, Any]:
    gate = adapter.get("zero_new_caller_gate")
    if not isinstance(gate, dict):
        fail(f"invalid_zero_new_caller_gate:{adapter.get('id')}")
    config = {
        "adapter_type": gate.get("tracked_adapter_type"),
        "old_calls": gate.get("tracked_old_facade_calls"),
    }
    with tempfile.TemporaryDirectory(prefix="edge-adapter-go-ast-") as directory:
        scanner = Path(directory) / "main.go"
        scanner.write_text(GO_AST_SCANNER, encoding="utf-8")
        completed = subprocess.run(
            [
                "go",
                "run",
                str(scanner),
                str(root),
                "sdk/go",
                str(adapter["source_path"]),
                json.dumps(config, separators=(",", ":")),
            ],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        )
    value = json.loads(completed.stdout)
    if not isinstance(value, dict):
        fail("go_ast_inventory_object_required")
    return value


def python_module_name(root: Path, path: Path) -> tuple[str, list[str]]:
    package_root = root / "sdk/python"
    relative = path.relative_to(package_root)
    parts = list(relative.with_suffix("").parts)
    if parts[-1] == "__init__":
        parts.pop()
        package = parts
    else:
        package = parts[:-1]
    return ".".join(parts), package


def resolve_python_import(
    current_package: list[str], level: int, module: str | None
) -> str:
    if level == 0:
        return module or ""
    retained = len(current_package) - level + 1
    if retained < 0:
        return ""
    parts = current_package[:retained]
    if module:
        parts.extend(module.split("."))
    return ".".join(parts)


class PythonCallerCollector(ast.NodeVisitor):
    def __init__(
        self,
        path: str,
        package: list[str],
        tracked_module: str,
        tracked_exports: set[str],
    ) -> None:
        self.path = path
        self.package = package
        self.tracked_module = tracked_module
        self.tracked_exports = tracked_exports
        self.owners = ["module"]
        self.sites: list[dict[str, Any]] = []
        self.direct_aliases: dict[str, str] = {}
        self.module_aliases: set[str] = set()
        self.package_aliases: set[str] = set()

    def _record(self, kind: str, names: list[str]) -> None:
        self.sites.append(
            {
                "path": self.path,
                "kind": kind,
                "module": self.tracked_module,
                "owner": ".".join(self.owners),
                "names": sorted(names),
                "occurrences": 1,
            }
        )

    def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
        self.owners.append(node.name)
        self.generic_visit(node)
        self.owners.pop()

    visit_AsyncFunctionDef = visit_FunctionDef

    def visit_ClassDef(self, node: ast.ClassDef) -> None:
        self.owners.append(node.name)
        self.generic_visit(node)
        self.owners.pop()

    def visit_Import(self, node: ast.Import) -> None:
        for alias in node.names:
            if alias.name == self.tracked_module:
                if alias.asname:
                    self.module_aliases.add(alias.asname)
                else:
                    self.package_aliases.add(alias.name.split(".", maxsplit=1)[0])
                self._record("import", [alias.asname or alias.name])
            elif alias.name == "easynet_sdk":
                self.package_aliases.add(alias.asname or alias.name)

    def visit_ImportFrom(self, node: ast.ImportFrom) -> None:
        module = resolve_python_import(self.package, node.level, node.module)
        if module == self.tracked_module:
            imported = []
            for alias in node.names:
                if alias.name == "*":
                    self.direct_aliases.update(
                        {name: name for name in self.tracked_exports}
                    )
                    imported.append("*")
                elif alias.name in self.tracked_exports:
                    local_name = alias.asname or alias.name
                    self.direct_aliases[local_name] = alias.name
                    imported.append(local_name)
            if imported:
                self._record("import_from", imported)
        elif module == "easynet_sdk":
            module_members = []
            imported_exports = []
            for alias in node.names:
                local_name = alias.asname or alias.name
                if alias.name == "daemon":
                    self.module_aliases.add(local_name)
                    module_members.append(local_name)
                elif alias.name == "*":
                    self.direct_aliases.update(
                        {name: name for name in self.tracked_exports}
                    )
                    imported_exports.append("*")
                elif alias.name in self.tracked_exports:
                    self.direct_aliases[local_name] = alias.name
                    imported_exports.append(local_name)
            if module_members:
                self._record("import_module_member", module_members)
            if imported_exports:
                self._record("import_legacy_export", imported_exports)

    def visit_Call(self, node: ast.Call) -> None:
        function = node.func
        is_dynamic_import = (
            isinstance(function, ast.Name)
            and function.id in {"__import__", "import_module"}
        ) or (isinstance(function, ast.Attribute) and function.attr == "import_module")
        if (
            is_dynamic_import
            and node.args
            and isinstance(node.args[0], ast.Constant)
            and node.args[0].value == self.tracked_module
        ):
            self._record("dynamic_import", [self.tracked_module])
        if isinstance(function, ast.Name) and function.id in self.direct_aliases:
            self._record(
                "legacy_export_call",
                [self.direct_aliases[function.id]],
            )
        elif (
            isinstance(function, ast.Attribute)
            and function.attr in self.tracked_exports
        ):
            owner = function.value
            module_call = isinstance(owner, ast.Name) and (
                owner.id in self.module_aliases or owner.id in self.package_aliases
            )
            nested_module_call = (
                isinstance(owner, ast.Attribute)
                and owner.attr == "daemon"
                and isinstance(owner.value, ast.Name)
                and owner.value.id in self.package_aliases
            )
            if module_call or nested_module_call:
                self._record("legacy_export_call", [function.attr])
        self.generic_visit(node)


def python_inventory(root: Path, adapter: dict[str, Any]) -> dict[str, Any]:
    gate = adapter.get("zero_new_caller_gate")
    if not isinstance(gate, dict):
        fail(f"invalid_zero_new_caller_gate:{adapter.get('id')}")
    tracked_module = str(gate.get("tracked_module", ""))
    source_path = root / str(adapter["source_path"])
    tree = ast.parse(source_path.read_text(encoding="utf-8"), filename=str(source_path))
    exported: list[str] | None = None
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == "__all__"
            for target in node.targets
        ):
            value = ast.literal_eval(node.value)
            if not isinstance(value, list) or not all(
                isinstance(item, str) for item in value
            ):
                fail("python_adapter_all_must_be_literal_string_list")
            exported = value
    if exported is None:
        fail("python_adapter_all_required")
    if exported != sorted(set(exported)):
        fail("python_adapter_all_not_sorted")

    sites: list[dict[str, Any]] = []
    python_root = root / "sdk/python"
    for path in sorted(python_root.rglob("*.py")):
        relative = path.relative_to(root).as_posix()
        _, package = python_module_name(root, path)
        collector = PythonCallerCollector(
            relative,
            package,
            tracked_module,
            set(exported),
        )
        collector.visit(ast.parse(path.read_text(encoding="utf-8"), filename=str(path)))
        sites.extend(collector.sites)
    site_counts: dict[tuple[Any, ...], int] = {}
    site_values: dict[tuple[Any, ...], dict[str, Any]] = {}
    for site in sites:
        key = (
            site["path"],
            site["kind"],
            site["module"],
            site["owner"],
            tuple(site["names"]),
        )
        site_counts[key] = site_counts.get(key, 0) + 1
        site_values[key] = site
    sites = []
    for key, site in site_values.items():
        site["occurrences"] = site_counts[key]
        sites.append(site)
    sites.sort(
        key=lambda site: (
            site["path"],
            site["kind"],
            site["module"],
            site["owner"],
            site["names"],
        )
    )
    return {"symbols": exported, "members": [], "sites": sites}


def validate_adapter_inventory(
    policy: dict[str, Any], manifest: dict[str, Any], root: Path
) -> None:
    canonical = {section: manifest.get(section, {}) for section in SECTIONS}
    non_canonical = manifest.get("non_canonical", {})
    for adapter in policy["adapters"]:
        required = {
            "id",
            "language",
            "source_path",
            "release_removal_version",
            "frozen_exports",
            "zero_new_caller_gate",
        }
        if not isinstance(adapter, dict) or set(adapter) != required:
            fail(
                f"invalid_edge_adapter:{adapter.get('id') if isinstance(adapter, dict) else ''}"
            )
        language = adapter["language"]
        if language not in {"go", "python"}:
            fail(f"unsupported_edge_adapter_language:{language}")
        if not (root / adapter["source_path"]).is_file():
            fail(f"edge_adapter_source_missing:{adapter['source_path']}")
        frozen_exports = adapter["frozen_exports"]
        if not isinstance(frozen_exports, dict) or set(frozen_exports) != {
            "symbols",
            "members",
            "shape_sha256",
        }:
            fail(f"invalid_frozen_adapter_exports:{adapter['id']}")
        for section in ("symbols", "members"):
            values = frozen_exports[section]
            if not isinstance(values, list) or values != sorted(set(values)):
                fail(f"frozen_adapter_exports_not_sorted:{adapter['id']}:{section}")

        inventory = (
            go_inventory(root, adapter)
            if language == "go"
            else python_inventory(root, adapter)
        )
        if {
            "symbols": inventory["symbols"],
            "members": inventory["members"],
        } != {
            "symbols": frozen_exports["symbols"],
            "members": frozen_exports["members"],
        }:
            fail(f"edge_adapter_export_drift:{adapter['id']}")
        gate = adapter["zero_new_caller_gate"]
        gate_required = (
            {"tracked_adapter_type", "tracked_old_facade_calls", "allowed_sites"}
            if language == "go"
            else {"tracked_module", "allowed_sites"}
        )
        if not isinstance(gate, dict) or set(gate) != gate_required:
            fail(f"invalid_zero_new_caller_gate:{adapter['id']}")
        if language == "go":
            tracked_type = gate["tracked_adapter_type"]
            tracked_calls = gate["tracked_old_facade_calls"]
            if not isinstance(tracked_type, str) or not tracked_type:
                fail(f"tracked_adapter_type_required:{adapter['id']}")
            if (
                not isinstance(tracked_calls, list)
                or tracked_calls != sorted(set(tracked_calls))
                or not tracked_calls
            ):
                fail(f"tracked_old_facade_calls_not_closed:{adapter['id']}")
        elif gate["tracked_module"] != "easynet_sdk.daemon":
            fail(f"tracked_python_module_mismatch:{adapter['id']}")
        allowed_sites = gate["allowed_sites"]
        if not isinstance(allowed_sites, list):
            fail(f"allowed_sites_required:{adapter['id']}")
        if inventory["sites"] != allowed_sites:
            allowed = {
                json.dumps(site, sort_keys=True, separators=(",", ":"))
                for site in allowed_sites
            }
            actual = {
                json.dumps(site, sort_keys=True, separators=(",", ":"))
                for site in inventory["sites"]
            }
            additions = sorted(actual - allowed)
            removals = sorted(allowed - actual)
            fail(
                f"zero_new_caller_drift:{adapter['id']}:"
                f"additions={additions}:removals={removals}"
            )

        manifest_section = "languages"
        public_values = set(canonical[manifest_section].get(language, []))
        public_values.update(non_canonical.get(manifest_section, {}).get(language, []))
        missing = sorted(set(frozen_exports["symbols"]) - public_values)
        if missing:
            fail(
                f"adapter_exports_missing_from_public_inventory:{adapter['id']}:{missing}"
            )
        public_members = set(canonical["members"].get(language, []))
        public_members.update(non_canonical.get("members", {}).get(language, []))
        missing_members = sorted(set(frozen_exports["members"]) - public_members)
        if missing_members:
            fail(
                f"adapter_members_missing_from_public_inventory:"
                f"{adapter['id']}:{missing_members}"
            )
        public_shapes = manifest["shape_sha256"][language]
        exported_items = sorted(
            set(frozen_exports["symbols"]) | set(frozen_exports["members"])
        )
        missing_shapes = sorted(set(exported_items) - set(public_shapes))
        if missing_shapes:
            fail(f"adapter_public_shape_missing:{adapter['id']}:{missing_shapes}")
        adapter_shapes = {item: public_shapes[item] for item in exported_items}
        actual_adapter_shape_digest = canonical_digest(adapter_shapes)
        if actual_adapter_shape_digest != frozen_exports["shape_sha256"]:
            fail(
                f"edge_adapter_public_shape_drift:{adapter['id']}:"
                f"expected={frozen_exports['shape_sha256']}:"
                f"actual={actual_adapter_shape_digest}"
            )


def validate_policy(
    policy: dict[str, Any],
    manifest: dict[str, Any],
    *,
    root: Path = ROOT,
    check_sources: bool = True,
) -> None:
    if policy.get("schema_version") != 1:
        fail("edge_adapter_policy_schema_version")
    if policy.get("policy_id") != "canonical-runtime.released-edge-adapters":
        fail("edge_adapter_policy_id")
    expected_keys = {
        "schema_version",
        "policy_id",
        "release_policy",
        "frozen_non_canonical_public_surface",
        "adapters",
    }
    if set(policy) != expected_keys:
        fail("edge_adapter_policy_shape")
    validate_release_policy(policy, root)
    validate_frozen_surface(policy, manifest)
    if check_sources:
        validate_adapter_inventory(policy, manifest, root)


def copy_source_tree(source: Path, destination: Path, suffix: str) -> None:
    for path in source.rglob(f"*{suffix}"):
        relative = path.relative_to(source)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)


def expect_policy_error(label: str, callback: Any) -> None:
    try:
        callback()
    except PolicyError:
        return
    fail(f"self_test_expected_failure:{label}")


def run_self_test(policy: dict[str, Any], manifest: dict[str, Any]) -> None:
    validate_policy(policy, manifest)

    expanded_surface = copy.deepcopy(manifest)
    expanded_surface["non_canonical"]["languages"]["go"].append(
        "UnreleasedLegacyDaemonFacade"
    )
    expanded_surface["non_canonical"]["languages"]["go"].sort()
    expect_policy_error(
        "non_canonical_surface_expansion",
        lambda: validate_policy(
            policy, expanded_surface, root=ROOT, check_sources=False
        ),
    )

    changed_legacy_shape = copy.deepcopy(manifest)
    legacy_item = changed_legacy_shape["non_canonical"]["languages"]["go"][0]
    changed_legacy_shape["shape_sha256"]["go"][legacy_item] = "0" * 64
    expect_policy_error(
        "non_canonical_shape_change",
        lambda: validate_policy(
            policy, changed_legacy_shape, root=ROOT, check_sources=False
        ),
    )

    if policy["adapters"]:
        changed_adapter_shape = copy.deepcopy(manifest)
        changed_adapter_shape["shape_sha256"]["go"]["Attach"] = "0" * 64
        expect_policy_error(
            "edge_adapter_public_shape_change",
            lambda: validate_policy(policy, changed_adapter_shape, root=ROOT),
        )

    invalid_removal = copy.deepcopy(policy)
    invalid_removal["release_policy"]["removal_version"] = "0.96.8"
    for adapter in invalid_removal["adapters"]:
        adapter["release_removal_version"] = "0.96.8"
    expect_policy_error(
        "non_major_removal_version",
        lambda: validate_policy(
            invalid_removal, manifest, root=ROOT, check_sources=False
        ),
    )

    if policy["adapters"]:
        with tempfile.TemporaryDirectory(prefix="edge-adapter-policy-self-test-") as raw:
            fixture = Path(raw)
            copy_source_tree(ROOT / "sdk/go", fixture / "sdk/go", ".go")
            copy_source_tree(ROOT / "sdk/python", fixture / "sdk/python", ".py")
            (fixture / "VERSION").write_text(
                (ROOT / "VERSION").read_text(encoding="utf-8"), encoding="utf-8"
            )
            python_project = fixture / "sdk/python/pyproject.toml"
            python_project.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / "sdk/python/pyproject.toml", python_project)

            go_negative = fixture / "sdk/go/edge_adapter_policy_negative.go"
            go_negative.write_text(
                "package easynet\n"
                "func edgeAdapterPolicyNegativeConstruction() {\n"
                "\t_ = runtimeLifecycleCompatibilityAdapter{}\n"
                "}\n",
                encoding="utf-8",
            )
            expect_policy_error(
                "go_adapter_construction",
                lambda: validate_policy(policy, manifest, root=fixture),
            )
            go_negative.write_text(
                "package easynet\n"
                "func edgeAdapterPolicyNegativeCall(transport DaemonTransport) {\n"
                "\t_, _ = NewDaemonControl(transport)\n"
                "}\n",
                encoding="utf-8",
            )
            expect_policy_error(
                "go_old_facade_call",
                lambda: validate_policy(policy, manifest, root=fixture),
            )
            go_negative.unlink()

            go_consumer = fixture / "sdk/go/consumer/edge_adapter_policy_negative.go"
            go_consumer.parent.mkdir(parents=True)
            go_consumer.write_text(
                "package consumer\n"
                'import runtimesdk "easynet.run/cli/sdk/go"\n'
                "func edgeAdapterPolicyNegativeQualifiedCall() {\n"
                "\t_, _ = runtimesdk.NewDaemonControl(nil)\n"
                "}\n",
                encoding="utf-8",
            )
            expect_policy_error(
                "go_qualified_old_facade_call",
                lambda: validate_policy(policy, manifest, root=fixture),
            )
            go_consumer.unlink()
            go_consumer.parent.rmdir()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    policy = load_json(args.policy)
    manifest = load_json(args.manifest)
    if args.self_test:
        run_self_test(policy, manifest)
        print("edge-adapter-policy self-test: OK")
    else:
        validate_policy(policy, manifest, root=args.root.resolve())
        print("edge-adapter-policy: OK")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (
        OSError,
        PolicyError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
        tomllib.TOMLDecodeError,
    ) as error:
        print(f"edge-adapter-policy: {error}", file=sys.stderr)
        raise SystemExit(1)
