#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
from pathlib import Path
from typing import Iterable

ROOT = Path(__file__).resolve().parents[2]
AXON_REVISION_ROOTS = (
    "sdk",
    "core/proto",
    "core/runtime-rs/dendrite-bridge/include",
)


def axon_root() -> Path:
    configured = os.environ.get("EASYNET_AXON_ROOT")
    return (
        Path(configured).resolve()
        if configured
        else (ROOT / "../EasyNet-Axon").resolve()
    )


def git_source_revision(repository: Path, roots: Iterable[str]) -> str:
    repository = repository.resolve()
    root_list = tuple(sorted(set(roots)))
    if not root_list:
        raise ValueError("source revision roots must not be empty")
    status = _run(
        [
            "git",
            "-C",
            str(repository),
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            *root_list,
        ]
    )
    if not status:
        revision = _run(
            [
                "git",
                "-C",
                str(repository),
                "log",
                "-1",
                "--format=%H",
                "--",
                *root_list,
            ]
        ).strip()
        if not revision:
            joined = ", ".join(root_list)
            raise ValueError(
                f"source revision roots have no committed history: {joined}"
            )
        return revision

    listed = subprocess.run(
        [
            "git",
            "-C",
            str(repository),
            "ls-files",
            "-co",
            "--exclude-standard",
            "-z",
            "--",
            *root_list,
        ],
        check=True,
        capture_output=True,
    ).stdout
    paths = sorted(raw.decode() for raw in listed.split(b"\0") if raw)
    digest = hashlib.sha256()
    for relative in paths:
        encoded = relative.encode()
        path = repository / relative
        content = path.read_bytes() if path.is_file() else b"<deleted>"
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
    return f"working_tree:{digest.hexdigest()}"


def _run(command: list[str]) -> str:
    return subprocess.run(
        command,
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Attest a Git source surface by commit or bounded working-tree hash."
    )
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--root", action="append", dest="roots", required=True)
    args = parser.parse_args()
    print(git_source_revision(args.repository, args.roots))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
