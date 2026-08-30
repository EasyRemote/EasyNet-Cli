#!/usr/bin/env python3
"""Reject workflows that reference missing repository-local scripts or actions."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import shlex
import stat
import sys


LOCAL_ROOTS = ("packaging/", "sdk/", "tests/", "tools/")
SCRIPT_SUFFIXES = (".sh", ".py", ".ps1")


@dataclass(frozen=True)
class Reference:
    workflow: Path
    line: int
    path: Path
    requires_executable: bool


def run_lines(lines: list[str]) -> list[tuple[int, str]]:
    commands: list[tuple[int, str]] = []
    index = 0
    while index < len(lines):
        match = re.match(r"^(?P<indent>\s*)(?:-\s*)?run:\s*(?P<body>.*)$", lines[index])
        if match is None:
            index += 1
            continue
        body = match.group("body").strip()
        if body not in {"|", ">", "|-", ">-"}:
            commands.append((index + 1, body))
            index += 1
            continue
        indent = len(match.group("indent"))
        index += 1
        while index < len(lines):
            candidate = lines[index]
            if candidate.strip() and len(candidate) - len(candidate.lstrip()) <= indent:
                break
            commands.append((index + 1, candidate.strip()))
            index += 1
    return commands


def normalize_script(token: str) -> str | None:
    token = token.strip("'\"(){}[],:;")
    if not token or "$" in token or "*" in token:
        return None
    while token.startswith("./"):
        token = token[2:]
    if token.startswith("/") or not token.endswith(SCRIPT_SUFFIXES):
        return None
    if token.startswith("../") or token.startswith(LOCAL_ROOTS):
        return token
    return None


def references(root: Path) -> list[Reference]:
    found: list[Reference] = []
    for workflow in sorted((root / ".github/workflows").glob("*.y*ml")):
        lines = workflow.read_text(encoding="utf-8").splitlines()
        for line_number, line in enumerate(lines, start=1):
            match = re.match(r"^\s*-?\s*uses:\s*['\"]?(\./[^'\"\s]+)", line)
            if match:
                local = Path(match.group(1)[2:])
                if local.suffix not in {".yml", ".yaml"}:
                    local /= "action.yml"
                found.append(Reference(workflow, line_number, local, False))
        for line_number, command in run_lines(lines):
            try:
                tokens = shlex.split(command, comments=True, posix=True)
            except ValueError:
                tokens = command.split()
            for token in tokens:
                normalized = normalize_script(token)
                if normalized is not None:
                    found.append(
                        Reference(
                            workflow,
                            line_number,
                            Path(normalized),
                            token.startswith("./"),
                        )
                    )
    return found


def validate(root: Path) -> list[str]:
    root = root.resolve()
    failures: list[str] = []
    for reference in references(root):
        target = (root / reference.path).resolve()
        try:
            target.relative_to(root)
        except ValueError:
            failures.append(
                f"{reference.workflow.relative_to(root)}:{reference.line}: "
                f"local reference escapes repository: {reference.path}"
            )
            continue
        if not target.is_file():
            failures.append(
                f"{reference.workflow.relative_to(root)}:{reference.line}: "
                f"local reference is missing: {reference.path}"
            )
        elif reference.requires_executable and not target.stat().st_mode & stat.S_IXUSR:
            failures.append(
                f"{reference.workflow.relative_to(root)}:{reference.line}: "
                f"directly invoked script is not executable: {reference.path}"
            )
    return failures


def repository_root() -> Path:
    return Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=repository_root(), help=argparse.SUPPRESS)
    arguments = parser.parse_args(argv)
    failures = validate(arguments.root)
    if failures:
        for failure in failures:
            print(f"check-workflow-integrity: {failure}", file=sys.stderr)
        return 1
    print("Workflow integrity checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
