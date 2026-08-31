#!/usr/bin/env python3
"""Derive and synchronize CLI dependency metadata from one Axon checkout."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib


LOCK_PATH = Path("compatibility/axon.lock.json")
CONTRACT_PATH = Path("compatibility/contract.json")
PYPROJECT_PATH = Path("sdk/python/pyproject.toml")
GO_MOD_PATH = Path("sdk/go/go.mod")
SCHEMA_VERSION = "easynet.cli.axon-lock.v1"


class DependencyError(RuntimeError):
    """Axon dependency metadata cannot be derived or synchronized."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise DependencyError(f"cannot read JSON {path}: {error}") from error
    if not isinstance(value, dict):
        raise DependencyError(f"expected JSON object: {path}")
    return value


def git_head(root: Path) -> str:
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if completed.returncode != 0:
        raise DependencyError(f"cannot resolve Axon HEAD: {completed.stderr.strip()}")
    return completed.stdout.strip()


def next_minor(version: str) -> str:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[.+-].*)?", version)
    if match is None:
        raise DependencyError(f"Axon release version is not semver-like: {version!r}")
    return f"{match.group(1)}.{int(match.group(2)) + 1}"


def toml_version(path: Path) -> str:
    try:
        value = tomllib.loads(path.read_text(encoding="utf-8"))["project"]["version"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as error:
        raise DependencyError(f"cannot read Python SDK version: {error}") from error
    if not isinstance(value, str) or not value:
        raise DependencyError("Python SDK version must be a non-empty string")
    return value


def derive(root: Path, axon_root: Path) -> tuple[dict[str, object], str, str]:
    contract_path = axon_root / CONTRACT_PATH
    contract = load_json(contract_path)
    sdks = contract.get("sdks")
    if not isinstance(sdks, dict):
        raise DependencyError("Axon compatibility contract has no SDK projection")
    required_sdks = {"rust", "python", "go", "node", "react", "java", "swift"}
    if not required_sdks.issubset(sdks):
        raise DependencyError(
            "Axon compatibility contract has an incomplete SDK projection"
        )
    release_version = contract.get("axon_release_version")
    protocol = contract.get("protocol")
    ffi = contract.get("ffi")
    if (
        not isinstance(release_version, str)
        or not isinstance(protocol, dict)
        or not isinstance(ffi, dict)
    ):
        raise DependencyError("Axon compatibility contract is incomplete")
    runtime_version = (root / "VERSION").read_text(encoding="utf-8").strip()
    node = load_json(root / "sdk/node/package.json")
    lock = {
        "axon": {
            "contract_sha256": sha256_file(contract_path),
            "ffi": {
                "dendrite_abi_version": ffi.get("dendrite_abi_version"),
                "public_header_sha256": ffi.get("public_header_sha256"),
            },
            "git_revision": git_head(axon_root),
            "protocol": {
                "descriptor_set_sha256": protocol.get("descriptor_set_sha256")
            },
            "release_version": release_version,
            "repository": "EasyRemote/EasyNet-Axon",
            "sdks": {name: sdks[name] for name in sorted(required_sdks)},
        },
        "cli": {
            "runtime_version": runtime_version,
            "sdks": {
                "node": node.get("version"),
                "python": toml_version(root / PYPROJECT_PATH),
            },
        },
        "schema_version": SCHEMA_VERSION,
    }
    constraint = (
        f"axon-runtime-sdk>={sdks['python']},<{next_minor(str(sdks['python']))}"
    )
    go_version = f"v{sdks['go']}"
    return lock, constraint, go_version


def canonical_json(value: object) -> str:
    return json.dumps(value, indent=2, sort_keys=True) + "\n"


def replace_atomic(path: Path, text: str) -> None:
    descriptor, name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent, text=True
    )
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            output.write(text)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def project_python_constraint(text: str, expected: str) -> str:
    pattern = re.compile(r'(?m)^[ \t]*"axon-runtime-sdk[^"\n]*",[ \t]*$')
    matches = pattern.findall(text)
    if len(matches) != 1:
        raise DependencyError(
            "Python SDK must declare exactly one axon-runtime-sdk dependency"
        )
    return pattern.sub(f'    "{expected}",', text)


def project_go_requirement(text: str, expected: str) -> str:
    pattern = re.compile(
        r"(?m)^([ \t]*(?:require[ \t]+)?axon\.run/sdk/go[ \t]+)"
        r"v\S+([ \t]*(?://.*)?)$"
    )
    if len(pattern.findall(text)) != 1:
        raise DependencyError(
            "Go SDK must declare exactly one axon.run/sdk/go requirement"
        )
    projected = pattern.sub(rf"\g<1>{expected}\g<2>", text)
    if re.search(r"(?m)^replace\s+axon\.run/sdk/go\s+=>", projected):
        raise DependencyError(
            "local Axon replacement must live in root go.work, not sdk/go/go.mod"
        )
    return projected


def check_or_write(root: Path, axon_root: Path, write: bool) -> None:
    lock, constraint, go_version = derive(root, axon_root)
    projections = {
        root / LOCK_PATH: canonical_json(lock),
        root / PYPROJECT_PATH: project_python_constraint(
            (root / PYPROJECT_PATH).read_text(encoding="utf-8"), constraint
        ),
        root / GO_MOD_PATH: project_go_requirement(
            (root / GO_MOD_PATH).read_text(encoding="utf-8"), go_version
        ),
    }
    drift = [
        path.relative_to(root).as_posix()
        for path, expected in projections.items()
        if path.read_text(encoding="utf-8") != expected
    ]
    if write:
        originals = {path: path.read_text(encoding="utf-8") for path in projections}
        try:
            for path, expected in projections.items():
                if originals[path] != expected:
                    replace_atomic(path, expected)
        except OSError:
            for path, original in originals.items():
                replace_atomic(path, original)
            raise
        print(
            f"Synchronized Axon dependency {lock['axon']['release_version']} at {lock['axon']['git_revision']}."
        )
    elif drift:
        raise DependencyError(f"Axon dependency metadata drift: {drift}")
    else:
        print("CLI Axon dependency metadata checks passed.")


def repository_root() -> Path:
    return Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check", action="store_true", help="verify without writing (default)"
    )
    mode.add_argument(
        "--write", action="store_true", help="atomically update dependency metadata"
    )
    parser.add_argument(
        "--root", type=Path, default=repository_root(), help=argparse.SUPPRESS
    )
    parser.add_argument(
        "--axon-root", type=Path, help="Axon checkout (default: sibling EasyNet-Axon)"
    )
    arguments = parser.parse_args(argv)
    root = arguments.root.resolve()
    axon_root = (arguments.axon_root or root.parent / "EasyNet-Axon").resolve()
    try:
        check_or_write(root, axon_root, arguments.write)
        return 0
    except (DependencyError, OSError, subprocess.TimeoutExpired) as error:
        print(f"update-axon-dependency: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
