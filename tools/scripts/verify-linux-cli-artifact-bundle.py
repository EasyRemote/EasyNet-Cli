#!/usr/bin/env python3
"""Fail closed when a Linux runtime artifact bundle is incomplete or mixed."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 3
FILE_ARTIFACTS = (
    "easynet",
    "easynet-daemon",
    "easynet-keyring",
    "easynet-remoteapp-native-host",
    "easynet-remoteapp-media-host",
    "libeasynet_cli.so",
    "libaxon_dendrite_bridge.so",
)
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
REVISION_RE = re.compile(r"^[0-9a-f]{40}$")


class VerificationError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationError(message)


def require_object(value: Any, path: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{path} must be an object")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def descriptor_tree_identity(root: Path) -> tuple[str, int]:
    require(root.is_dir(), "ability-descriptors must be a directory")
    entries = sorted(root.rglob("*"), key=lambda path: path.relative_to(root).as_posix())
    require(not any(path.is_symlink() for path in entries), "ability-descriptors must not contain symlinks")
    files = [path for path in entries if path.is_file()]
    digest = hashlib.sha256()
    for path in files:
        relative = path.relative_to(root).as_posix().encode()
        digest.update(relative)
        digest.update(b"\0")
        digest.update(sha256_file(path).encode())
        digest.update(b"\0")
    return digest.hexdigest(), len(files)


def git_output(repository: Path, *args: str) -> bytes:
    try:
        return subprocess.check_output(("git", "-C", str(repository), *args))
    except (OSError, subprocess.CalledProcessError) as error:
        raise VerificationError(f"cannot inspect Git source {repository}: {error}") from error


def git_source_identity(repository: Path) -> tuple[str, bool, str]:
    require(repository.is_dir(), f"source repository not found: {repository}")
    revision = git_output(repository, "rev-parse", "HEAD").strip().decode()
    dirty = bool(git_output(repository, "status", "--porcelain", "--untracked-files=normal"))
    digest = hashlib.sha256()
    digest.update(b"revision\0")
    digest.update(revision.encode())
    digest.update(b"\0tracked-diff\0")
    digest.update(git_output(repository, "diff", "--binary", "HEAD", "--"))
    digest.update(b"\0untracked\0")
    untracked = git_output(repository, "ls-files", "--others", "--exclude-standard", "-z")
    for relative_bytes in filter(None, untracked.split(b"\0")):
        relative = relative_bytes.decode()
        content_hash = git_output(repository, "hash-object", "--", relative).strip()
        digest.update(relative_bytes)
        digest.update(b"\0")
        digest.update(content_hash)
        digest.update(b"\0")
    return revision, dirty, digest.hexdigest()


def verify_source(source: dict[str, Any], name: str) -> None:
    value = require_object(source.get(name), f"source.{name}")
    require(
        isinstance(value.get("revision"), str) and REVISION_RE.fullmatch(value["revision"]) is not None,
        f"source.{name}.revision must be a lowercase 40-character Git object id",
    )
    require(type(value.get("dirty")) is bool, f"source.{name}.dirty must be boolean")
    require(
        isinstance(value.get("worktree_sha256"), str)
        and SHA256_RE.fullmatch(value["worktree_sha256"]) is not None,
        f"source.{name}.worktree_sha256 must be a lowercase SHA-256",
    )


def verify_bundle(
    root: Path,
    expected_target: str | None,
    expected_media_profile: str | None,
    expected_build_profile: str | None,
    require_clean_source: bool,
    expected_cli_source: Path | None,
    expected_axon_source: Path | None,
) -> None:
    require(root.is_dir(), f"bundle directory not found: {root}")
    manifest_path = root / "runtime-build-profile.json"
    require(manifest_path.is_file(), f"missing manifest: {manifest_path}")
    try:
        manifest = require_object(json.loads(manifest_path.read_text()), "manifest")
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise VerificationError(f"invalid manifest JSON: {error}") from error

    require(manifest.get("schema_version") == SCHEMA_VERSION, f"unsupported schema_version: {manifest.get('schema_version')!r}")
    target = manifest.get("target")
    media_profile = manifest.get("media_profile")
    builder = manifest.get("builder")
    build_profile = manifest.get("build_profile")
    require(target in {"aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"}, f"unsupported target: {target!r}")
    require(media_profile in {"headless", "native"}, f"unsupported media_profile: {media_profile!r}")
    require(builder in {"zig", "docker"}, f"unsupported builder: {builder!r}")
    require(build_profile in {"dev", "release"}, f"unsupported build_profile: {build_profile!r}")
    require(media_profile != "native" or builder == "docker", "native media bundle must use the Docker builder")
    require(
        manifest.get("cargo_features") == ["axon-pb", f"{media_profile}-media", "remote-desktop"],
        "cargo_features do not match media_profile",
    )
    if expected_target is not None:
        require(target == expected_target, f"target mismatch: expected {expected_target}, got {target}")
    if expected_media_profile is not None:
        require(
            media_profile == expected_media_profile,
            f"media_profile mismatch: expected {expected_media_profile}, got {media_profile}",
        )
    if expected_build_profile is not None:
        require(
            build_profile == expected_build_profile,
            f"build_profile mismatch: expected {expected_build_profile}, got {build_profile}",
        )

    source = require_object(manifest.get("source"), "source")
    for name in ("easynet_cli", "easynet_axon"):
        verify_source(source, name)
        if require_clean_source:
            require(source[name]["dirty"] is False, f"source.{name} is dirty")
    for name, repository in (
        ("easynet_cli", expected_cli_source),
        ("easynet_axon", expected_axon_source),
    ):
        if repository is None:
            continue
        revision, dirty, worktree_sha256 = git_source_identity(repository.resolve())
        require(source[name]["revision"] == revision, f"source.{name}.revision is stale")
        require(source[name]["dirty"] is dirty, f"source.{name}.dirty is stale")
        require(
            source[name]["worktree_sha256"] == worktree_sha256,
            f"source.{name}.worktree_sha256 does not match the current source tree",
        )

    builder_identity = require_object(manifest.get("builder_identity"), "builder_identity")
    if builder == "docker":
        require(isinstance(builder_identity.get("image"), str) and builder_identity["image"], "Docker builder image is missing")
        require(
            isinstance(builder_identity.get("image_id"), str)
            and re.fullmatch(r"sha256:[0-9a-f]{64}", builder_identity["image_id"]) is not None,
            "Docker builder image_id must be a sha256 digest",
        )

    artifacts = require_object(manifest.get("artifacts"), "artifacts")
    expected_artifacts = {*FILE_ARTIFACTS, "ability-descriptors"}
    require(set(artifacts) == expected_artifacts, "manifest artifact set is incomplete or contains unknown entries")
    for name in FILE_ARTIFACTS:
        path = root / name
        require(path.is_file() and not path.is_symlink(), f"missing regular artifact: {name}")
        identity = require_object(artifacts[name], f"artifacts.{name}")
        expected_hash = identity.get("sha256")
        expected_bytes = identity.get("bytes")
        require(isinstance(expected_hash, str) and SHA256_RE.fullmatch(expected_hash) is not None, f"invalid SHA-256 for {name}")
        require(type(expected_bytes) is int and expected_bytes >= 0, f"invalid byte size for {name}")
        require(path.stat().st_size == expected_bytes, f"byte size mismatch for {name}")
        require(sha256_file(path) == expected_hash, f"SHA-256 mismatch for {name}")

    descriptor_identity = require_object(artifacts["ability-descriptors"], "artifacts.ability-descriptors")
    tree_hash, file_count = descriptor_tree_identity(root / "ability-descriptors")
    require(tree_hash == descriptor_identity.get("tree_sha256"), "ability-descriptors tree SHA-256 mismatch")
    require(file_count == descriptor_identity.get("files"), "ability-descriptors file count mismatch")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--expect-target")
    parser.add_argument("--expect-media-profile", choices=("headless", "native"))
    parser.add_argument("--expect-build-profile", choices=("dev", "release"))
    parser.add_argument("--require-clean-source", action="store_true")
    parser.add_argument("--expect-easynet-cli-source", type=Path)
    parser.add_argument("--expect-easynet-axon-source", type=Path)
    args = parser.parse_args()
    try:
        verify_bundle(
            args.bundle.resolve(),
            args.expect_target,
            args.expect_media_profile,
            args.expect_build_profile,
            args.require_clean_source,
            args.expect_easynet_cli_source,
            args.expect_easynet_axon_source,
        )
    except (OSError, VerificationError) as error:
        print(f"[FAIL] Linux CLI artifact bundle: {error}", file=sys.stderr)
        return 1
    print(f"[OK] verified Linux CLI artifact bundle: {args.bundle.resolve()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
