#!/usr/bin/env python3
"""Prepare, commit, and optionally push one Runtime/Axon release coordinate."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from enum import Enum, auto
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


AUTHOR_NAME = "Silan.Hu"
AUTHOR_EMAIL = "silan.hu@u.nus.edu"
PROTECTED_BRANCHES = frozenset({"main", "master"})
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[.+-][0-9A-Za-z-]+)*$")


class ReleaseError(RuntimeError):
    """The Runtime release coordinate cannot advance safely."""


class Action(Enum):
    CHECK = auto()
    COMMIT = auto()
    PUSH = auto()
    PUSH_ONLY = auto()


class Phase(Enum):
    CREATED = auto()
    PREFLIGHTED = auto()
    UPSTREAM_PINNED = auto()
    ISOLATED = auto()
    SYNCHRONIZED = auto()
    VERIFIED = auto()
    COMMITTED = auto()
    PUSHED = auto()


@dataclass(frozen=True)
class Options:
    root: Path
    axon_root: Path
    action: Action
    version: str | None
    remote: str


def run(
    command: list[str], *, cwd: Path, timeout: int = 1800, capture: bool = False
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        capture_output=capture,
        timeout=timeout,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() if capture else ""
        suffix = f": {detail}" if detail else ""
        raise ReleaseError(
            f"command failed ({completed.returncode}): {' '.join(command)}{suffix}"
        )
    return completed


def git(root: Path, *arguments: str) -> str:
    return run(["git", *arguments], cwd=root, capture=True).stdout.strip()


def require_clean(root: Path, label: str) -> None:
    if git(root, "status", "--porcelain=v1", "--untracked-files=all"):
        raise ReleaseError(f"{label} worktree must be completely clean")


def require_push_target(root: Path, remote: str, branch: str) -> None:
    if branch in PROTECTED_BRANCHES:
        raise ReleaseError(f"direct push to protected branch {branch!r} is forbidden")
    if remote not in set(git(root, "remote").splitlines()):
        raise ReleaseError(f"Git remote {remote!r} does not exist")


def resolve_version(root: Path, explicit: str | None) -> str:
    version = explicit
    if version is None:
        version = run(
            ["tide", "mark", "--local-only"], cwd=root, capture=True, timeout=60
        ).stdout.strip()
    if not SEMVER.fullmatch(version):
        raise ReleaseError(f"invalid Runtime release version: {version!r}")
    return version


def changed_paths(worktree: Path) -> list[str]:
    tracked = git(worktree, "diff", "--name-only", "--diff-filter=ACMRTUXB", "HEAD")
    untracked = git(worktree, "ls-files", "--others", "--exclude-standard")
    if untracked:
        raise ReleaseError(
            f"release generators created untracked paths: {untracked.splitlines()}"
        )
    return sorted(path for path in tracked.splitlines() if path)


def is_allowed_release_path(path: str) -> bool:
    if path in {
        "VERSION",
        "compatibility/axon.lock.json",
        "sdk/conformance/fixtures/feature-discovery.v7.json",
        "sdk/python/pyproject.toml",
        "sdk/python/uv.lock",
        "sdk/go/go.mod",
        "sdk/go/go.sum",
        "go.work",
        "go.work.sum",
    }:
        return True
    return Path(path).name in {"Cargo.toml", "Cargo.lock"}


def prepare(cli: Path, axon: Path, version: str) -> list[str]:
    run(["./tools/scripts/update-project-version.sh", version], cwd=cli)
    run(
        [
            "python3",
            "tools/scripts/update-axon-dependency.py",
            "--write",
            "--axon-root",
            str(axon),
        ],
        cwd=cli,
    )
    run(["uv", "lock", "--project", "sdk/python"], cwd=cli)
    run(
        ["uv", "sync", "--project", "sdk/python", "--extra", "dev", "--locked"],
        cwd=cli,
    )
    run(["go", "work", "sync"], cwd=cli)
    run(["./tools/scripts/update-project-version.sh", "--check", version], cwd=cli)
    run(
        [
            "python3",
            "tools/scripts/update-axon-dependency.py",
            "--check",
            "--axon-root",
            str(axon),
        ],
        cwd=cli,
    )
    run(["uv", "lock", "--project", "sdk/python", "--check"], cwd=cli)
    run(["go", "test", "./..."], cwd=cli / "sdk/go")
    run(
        ["python3", "tools/scripts/check-axon-lock.py", "--axon-root", str(axon)],
        cwd=cli,
    )
    paths = changed_paths(cli)
    unexpected = [path for path in paths if not is_allowed_release_path(path)]
    if unexpected:
        raise ReleaseError(
            f"release generators changed non-release paths: {unexpected}"
        )
    return paths


def commit_isolated(worktree: Path, version: str, paths: list[str]) -> str:
    if paths:
        run(["git", "add", "--", *paths], cwd=worktree)
    message = (
        f"build: synchronize Runtime release {version}\n\n"
        "Update Runtime manifests, generated locks, and the exact Axon dependency coordinate."
    )
    run(
        [
            "git",
            "-c",
            f"user.name={AUTHOR_NAME}",
            "-c",
            f"user.email={AUTHOR_EMAIL}",
            "commit",
            "--allow-empty",
            "-m",
            message,
        ],
        cwd=worktree,
    )
    commit = git(worktree, "rev-parse", "HEAD")
    identity = git(worktree, "show", "-s", "--format=%an <%ae>", commit)
    if identity != f"{AUTHOR_NAME} <{AUTHOR_EMAIL}>":
        raise ReleaseError(
            f"metadata commit has unexpected author identity: {identity}"
        )
    return commit


class ReleaseCoordinateTransaction:
    def __init__(self, options: Options) -> None:
        self.options = options
        self.phase = Phase.CREATED

    def execute(self) -> None:
        root = self.options.root.resolve()
        axon_repo = self.options.axon_root.resolve()
        require_clean(root, "CLI caller")
        branch = git(root, "branch", "--show-current")
        if self.options.action is not Action.CHECK and not branch:
            raise ReleaseError("release mutation requires an attached CLI branch")
        if self.options.action in {Action.PUSH, Action.PUSH_ONLY}:
            require_push_target(root, self.options.remote, branch)
        self.phase = Phase.PREFLIGHTED
        if self.options.action is Action.PUSH_ONLY:
            self._push(root, branch)
            return

        version = resolve_version(root, self.options.version)
        functional_head = git(root, "rev-parse", "HEAD")
        axon_head = git(axon_repo, "rev-parse", "HEAD")
        self.phase = Phase.UPSTREAM_PINNED
        temporary_root = Path(tempfile.mkdtemp(prefix="runtime-release-coordinate-"))
        cli_worktree = temporary_root / "EasyNet-Cli"
        axon_worktree = temporary_root / "EasyNet-Axon"
        cli_added = False
        axon_added = False
        commit: str | None = None
        paths: list[str] = []
        try:
            run(
                ["git", "worktree", "add", "--detach", str(axon_worktree), axon_head],
                cwd=axon_repo,
            )
            axon_added = True
            run(
                [
                    "git",
                    "worktree",
                    "add",
                    "--detach",
                    str(cli_worktree),
                    functional_head,
                ],
                cwd=root,
            )
            cli_added = True
            self.phase = Phase.ISOLATED
            run(
                [
                    "python3",
                    "scripts/checks/check_compatibility_contract.py",
                    "--check",
                ],
                cwd=axon_worktree,
            )
            paths = prepare(cli_worktree, axon_worktree, version)
            self.phase = Phase.SYNCHRONIZED
            self.phase = Phase.VERIFIED
            if self.options.action is not Action.CHECK:
                commit = commit_isolated(cli_worktree, version, paths)
                self.phase = Phase.COMMITTED
        finally:
            if cli_added:
                run(
                    ["git", "worktree", "remove", "--force", str(cli_worktree)],
                    cwd=root,
                )
            if axon_added:
                run(
                    ["git", "worktree", "remove", "--force", str(axon_worktree)],
                    cwd=axon_repo,
                )
            shutil.rmtree(temporary_root, ignore_errors=True)

        if self.options.action is Action.CHECK:
            rendered = ", ".join(paths) if paths else "no file changes"
            print(f"Runtime release {version} is preparable ({rendered}).")
            return
        if commit is None:
            raise ReleaseError("metadata commit was not created")
        if git(root, "rev-parse", "HEAD") != functional_head:
            raise ReleaseError("CLI branch moved during isolated preparation")
        require_clean(root, "CLI caller")
        run(["git", "merge", "--ff-only", commit], cwd=root)
        if self.options.action is Action.PUSH:
            self._push(root, branch)
        else:
            print(f"Created Runtime release metadata commit {commit} for {version}.")

    def _push(self, root: Path, branch: str) -> None:
        run(
            [
                "git",
                "push",
                "--porcelain",
                self.options.remote,
                f"HEAD:refs/heads/{branch}",
            ],
            cwd=root,
        )
        self.phase = Phase.PUSHED
        print(
            f"Pushed {git(root, 'rev-parse', 'HEAD')} to {self.options.remote}/{branch}."
        )


def parse_args(argv: list[str] | None = None) -> Options:
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--check", action="store_true")
    action.add_argument("--commit", action="store_true")
    action.add_argument("--push", action="store_true")
    action.add_argument("--push-only", action="store_true")
    parser.add_argument("--version", help="explicit frozen Runtime version")
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--axon-root", type=Path)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help=argparse.SUPPRESS,
    )
    arguments = parser.parse_args(argv)
    if arguments.push_only and arguments.version is not None:
        parser.error("--push-only does not accept --version")
    root = arguments.root.resolve()
    selected = (
        Action.CHECK
        if arguments.check
        else Action.COMMIT
        if arguments.commit
        else Action.PUSH
        if arguments.push
        else Action.PUSH_ONLY
    )
    return Options(
        root,
        (arguments.axon_root or root.parent / "EasyNet-Axon").resolve(),
        selected,
        arguments.version,
        arguments.remote,
    )


def main(argv: list[str] | None = None) -> int:
    try:
        ReleaseCoordinateTransaction(parse_args(argv)).execute()
        return 0
    except (OSError, ReleaseError, subprocess.TimeoutExpired) as error:
        print(f"release-coordinate: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
