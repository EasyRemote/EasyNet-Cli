#!/usr/bin/env python3
"""Check README facts derived from the Runtime installer and repository set."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


REPOSITORIES = ("EasyNet-Axon", "EasyNet-Cli", "EasyNet", "EasyRemote")


def installed_binaries(installer: str) -> list[str]:
    block = installer.split("cargo_args_cli=(", 1)[1].split(")", 1)[0]
    return list(dict.fromkeys(re.findall(r"--bin\s+([A-Za-z0-9_-]+)", block)))


def errors(readme: str, installer: str) -> list[str]:
    problems: list[str] = []
    for binary in installed_binaries(installer):
        if f"`{binary}`" not in readme:
            problems.append(f"README omits installed binary: {binary}")
    for repository in REPOSITORIES:
        if f"github.com/EasyRemote/{repository}" not in readme:
            problems.append(f"README omits repository: {repository}")
    required = (
        "Capability-native network",
        "packaging/release/dev-install-local.sh --debug",
        "curl -sSf https://easynet.run/install | sudo sh",
        "easynet --help",
    )
    for fact in required:
        if fact not in readme:
            problems.append(f"README omits current fact: {fact}")
    if "Four things in one binary" in readme:
        problems.append("README still claims one binary owns every surface")
    if "```bash\ncargo install --path ." in readme:
        problems.append("README presents partial cargo install as complete")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    readme = (root / "README.md").read_text(encoding="utf-8")
    installer = (root / "packaging/release/dev-install-local.sh").read_text(
        encoding="utf-8"
    )
    if args.self_test:
        mutated = readme.replace("`easynet-keyring`", "key helper")
        if not any("easynet-keyring" in item for item in errors(mutated, installer)):
            raise SystemExit("self-test failed to detect an omitted binary")
        print("README fact checker self-test passed")
        return 0
    problems = errors(readme, installer)
    if problems:
        raise SystemExit("\n".join(problems))
    print("README facts match the Runtime installer")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
