#!/usr/bin/env python3
"""Verify the CLI's single pinned Axon compatibility coordinate."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


SCHEMA_VERSION = "easynet.cli.axon-lock.v1"
LOCK_PATH = Path("compatibility/axon.lock.json")
AXON_CONTRACT_PATH = Path("compatibility/contract.json")
HEX_40 = re.compile(r"^[0-9a-f]{40}$")
HEX_64 = re.compile(r"^[0-9a-f]{64}$")


class LockError(RuntimeError):
    """A pinned compatibility fact is missing, malformed, or inconsistent."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json_object(path: Path, label: str) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise LockError(f"cannot read {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise LockError(f"{label} must be a JSON object: {path}")
    return value


def require_exact_keys(
    value: dict[str, object], expected: set[str], label: str
) -> None:
    actual = set(value)
    if actual != expected:
        raise LockError(
            f"{label} keys differ: missing={sorted(expected - actual)} "
            f"unexpected={sorted(actual - expected)}"
        )


def require_string(value: object, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise LockError(f"{label} must be a non-empty string")
    return value


def validate_lock(lock: dict[str, object]) -> dict[str, object]:
    require_exact_keys(lock, {"schema_version", "axon", "cli"}, "lock")
    if lock["schema_version"] != SCHEMA_VERSION:
        raise LockError(f"unsupported schema_version: {lock['schema_version']!r}")

    axon = lock["axon"]
    cli = lock["cli"]
    if not isinstance(axon, dict) or not isinstance(cli, dict):
        raise LockError("axon and cli must be JSON objects")
    require_exact_keys(
        axon,
        {
            "repository",
            "git_revision",
            "contract_sha256",
            "release_version",
            "protocol",
            "ffi",
            "sdks",
        },
        "axon",
    )
    require_exact_keys(cli, {"runtime_version", "sdks"}, "cli")
    if (
        require_string(axon["repository"], "axon.repository")
        != "EasyRemote/EasyNet-Axon"
    ):
        raise LockError("axon.repository must be EasyRemote/EasyNet-Axon")
    revision = require_string(axon["git_revision"], "axon.git_revision")
    contract_hash = require_string(axon["contract_sha256"], "axon.contract_sha256")
    if not HEX_40.fullmatch(revision):
        raise LockError(
            "axon.git_revision must be a lowercase 40-character Git object id"
        )
    if not HEX_64.fullmatch(contract_hash):
        raise LockError("axon.contract_sha256 must be a lowercase SHA-256 digest")

    protocol = axon["protocol"]
    ffi = axon["ffi"]
    axon_sdks = axon["sdks"]
    cli_sdks = cli["sdks"]
    if not all(
        isinstance(value, dict) for value in (protocol, ffi, axon_sdks, cli_sdks)
    ):
        raise LockError("protocol, ffi, and sdk projections must be JSON objects")
    require_exact_keys(protocol, {"descriptor_set_sha256"}, "axon.protocol")
    require_exact_keys(
        ffi, {"dendrite_abi_version", "public_header_sha256"}, "axon.ffi"
    )
    require_exact_keys(
        axon_sdks,
        {"rust", "python", "go", "node", "react", "java", "swift"},
        "axon.sdks",
    )
    require_exact_keys(cli_sdks, {"python", "node"}, "cli.sdks")
    for label, digest in (
        ("axon.protocol.descriptor_set_sha256", protocol["descriptor_set_sha256"]),
        ("axon.ffi.public_header_sha256", ffi["public_header_sha256"]),
    ):
        if not isinstance(digest, str) or not HEX_64.fullmatch(digest):
            raise LockError(f"{label} must be a lowercase SHA-256 digest")
    if (
        not isinstance(ffi["dendrite_abi_version"], int)
        or ffi["dendrite_abi_version"] < 1
    ):
        raise LockError("axon.ffi.dendrite_abi_version must be a positive integer")
    for label, value in (
        ("axon.release_version", axon["release_version"]),
        ("cli.runtime_version", cli["runtime_version"]),
        *((f"axon.sdks.{name}", version) for name, version in axon_sdks.items()),
        *((f"cli.sdks.{name}", version) for name, version in cli_sdks.items()),
    ):
        require_string(value, label)
    return lock


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
        raise LockError(
            f"cannot resolve Axon checkout HEAD: {completed.stderr.strip()}"
        )
    return completed.stdout.strip()


def require_clean_checkout(root: Path) -> None:
    completed = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if completed.returncode != 0:
        raise LockError(f"cannot inspect Axon checkout: {completed.stderr.strip()}")
    if completed.stdout.strip():
        raise LockError("Axon checkout must be clean before compatibility verification")


def read_toml(path: Path) -> dict[str, object]:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise LockError(f"cannot parse {path}: {error}") from error


def next_minor(version: str) -> str:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[.+-].*)?", version)
    if match is None:
        raise LockError(f"SDK version is not semver-like: {version!r}")
    return f"{match.group(1)}.{int(match.group(2)) + 1}"


def verify_axon_checkout(axon_root: Path, axon: dict[str, object]) -> None:
    axon_root = axon_root.resolve()
    require_clean_checkout(axon_root)
    expected_revision = str(axon["git_revision"])
    actual_revision = git_head(axon_root)
    if actual_revision != expected_revision:
        raise LockError(
            f"Axon revision mismatch: expected={expected_revision} actual={actual_revision}"
        )
    contract_path = axon_root / AXON_CONTRACT_PATH
    actual_contract_hash = sha256_file(contract_path)
    if actual_contract_hash != axon["contract_sha256"]:
        raise LockError(
            "Axon contract digest mismatch: "
            f"expected={axon['contract_sha256']} actual={actual_contract_hash}"
        )
    contract = load_json_object(contract_path, "Axon compatibility contract")
    expected_contract = {
        "release_version": axon["release_version"],
        "protocol_digest": axon["protocol"]["descriptor_set_sha256"],
        "ffi_abi": axon["ffi"]["dendrite_abi_version"],
        "ffi_header": axon["ffi"]["public_header_sha256"],
    }
    actual_contract = {
        "release_version": contract.get("axon_release_version"),
        "protocol_digest": contract.get("protocol", {}).get("descriptor_set_sha256"),
        "ffi_abi": contract.get("ffi", {}).get("dendrite_abi_version"),
        "ffi_header": contract.get("ffi", {}).get("public_header_sha256"),
    }
    if actual_contract != expected_contract:
        raise LockError(
            f"Axon contract projection mismatch: expected={expected_contract} actual={actual_contract}"
        )
    contract_sdks = contract.get("sdks")
    if not isinstance(contract_sdks, dict):
        raise LockError("Axon contract sdks projection is missing")
    for language, version in axon["sdks"].items():
        if contract_sdks.get(language) != version:
            raise LockError(
                f"Axon {language} SDK mismatch: expected={version} actual={contract_sdks.get(language)}"
            )


def verify_cli_sources(root: Path, lock: dict[str, object]) -> None:
    root = root.resolve()
    axon = lock["axon"]
    cli = lock["cli"]
    runtime_version = (root / "VERSION").read_text(encoding="utf-8").strip()
    cargo = read_toml(root / "Cargo.toml")
    cargo_version = cargo.get("package", {}).get("version")
    if runtime_version != cli["runtime_version"] or cargo_version != runtime_version:
        raise LockError(
            "CLI runtime version mismatch: "
            f"lock={cli['runtime_version']} VERSION={runtime_version} Cargo.toml={cargo_version}"
        )

    python_project = read_toml(root / "sdk/python/pyproject.toml")
    python_metadata = python_project.get("project", {})
    if python_metadata.get("version") != cli["sdks"]["python"]:
        raise LockError("CLI Python SDK version differs from axon.lock.json")
    dependencies = python_metadata.get("dependencies", [])
    matching = [
        item
        for item in dependencies
        if isinstance(item, str) and item.startswith("axon-runtime-sdk")
    ]
    if len(matching) != 1:
        raise LockError(
            "CLI Python SDK must declare exactly one axon-runtime-sdk dependency"
        )
    expected_prefix = (
        f"axon-runtime-sdk>={axon['sdks']['python']},"
        f"<{next_minor(str(axon['sdks']['python']))}"
    )
    if matching[0].replace(" ", "") != expected_prefix:
        raise LockError(
            f"CLI Python Axon constraint mismatch: expected={expected_prefix} actual={matching[0]}"
        )
    node = load_json_object(root / "sdk/node/package.json", "CLI Node SDK manifest")
    if node.get("version") != cli["sdks"]["node"]:
        raise LockError("CLI Node SDK version differs from axon.lock.json")

    cargo_lock = (root / "Cargo.lock").read_text(encoding="utf-8")
    for package in ("axon-sdk", "axon-ura"):
        pattern = re.compile(
            rf'\[\[package\]\]\nname = "{re.escape(package)}"\nversion = "([^"]+)"'
        )
        match = pattern.search(cargo_lock)
        if match is None or match.group(1) != axon["sdks"]["rust"]:
            actual = None if match is None else match.group(1)
            raise LockError(
                f"Cargo.lock {package} mismatch: expected={axon['sdks']['rust']} actual={actual}"
            )

    uv_lock = read_toml(root / "sdk/python/uv.lock")
    packages = uv_lock.get("package", [])
    versions = [
        package.get("version")
        for package in packages
        if isinstance(package, dict) and package.get("name") == "axon-runtime-sdk"
    ]
    if versions != [axon["sdks"]["python"]]:
        raise LockError(
            f"uv.lock Axon package mismatch: expected={[axon['sdks']['python']]} actual={versions}"
        )

    go_mod = (root / "sdk/go/go.mod").read_text(encoding="utf-8")
    go_match = re.search(r"(?m)^\s*axon\.run/sdk/go\s+(v\S+)", go_mod)
    expected_go = f"v{axon['sdks']['go']}"
    actual_go = None if go_match is None else go_match.group(1)
    if actual_go != expected_go:
        raise LockError(
            f"CLI Go Axon dependency mismatch: expected={expected_go} actual={actual_go}"
        )
    if re.search(r"(?m)^replace\s+axon\.run/sdk/go\s+=>", go_mod):
        raise LockError(
            "CLI Go module must keep local Axon replacement in root go.work"
        )


def repository_root() -> Path:
    return Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=repository_root(), help=argparse.SUPPRESS
    )
    parser.add_argument(
        "--axon-root", type=Path, help="Axon checkout (default: sibling EasyNet-Axon)"
    )
    parser.add_argument(
        "--lock-only",
        action="store_true",
        help="validate and emit the coordinate before checkout",
    )
    parser.add_argument(
        "--github-output",
        action="store_true",
        help="write coordinate outputs to GITHUB_OUTPUT",
    )
    arguments = parser.parse_args(argv)
    root = arguments.root.resolve()
    try:
        lock = validate_lock(load_json_object(root / LOCK_PATH, "Axon lock"))
        axon = lock["axon"]
        if arguments.github_output:
            output_path = os.environ.get("GITHUB_OUTPUT")
            if not output_path:
                raise LockError("--github-output requires GITHUB_OUTPUT")
            with Path(output_path).open("a", encoding="utf-8") as output:
                output.write(f"axon_revision={axon['git_revision']}\n")
                output.write(f"axon_contract_sha256={axon['contract_sha256']}\n")
        if not arguments.lock_only:
            verify_cli_sources(root, lock)
            axon_root = arguments.axon_root or root.parent / "EasyNet-Axon"
            verify_axon_checkout(axon_root, axon)
            print("CLI pinned Axon compatibility checks passed.")
        else:
            print("CLI Axon lock schema checks passed.")
        return 0
    except (LockError, OSError, subprocess.TimeoutExpired) as error:
        print(f"check-axon-lock: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
