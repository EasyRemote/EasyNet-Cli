#!/usr/bin/env python3
"""Focused tests for Runtime-only version synchronization."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("update-project-version.sh")


def run(
    command: list[str],
    cwd: Path,
    *,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command, cwd=cwd, check=check, capture_output=True, text=True, env=env
    )


class RuntimeVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="runtime-version-test-")
        self.root = Path(self.temporary.name) / "repo"
        (self.root / "tools/scripts").mkdir(parents=True)
        (self.root / "compatibility").mkdir(parents=True)
        (self.root / "sdk/conformance/fixtures").mkdir(parents=True)
        (self.root / "sdk/node").mkdir(parents=True)
        (self.root / "sdk/python").mkdir(parents=True)
        shutil.copy2(SCRIPT, self.root / "tools/scripts/update-project-version.sh")
        (self.root / "VERSION").write_text("1.0.0\n", encoding="utf-8")
        (self.root / "Cargo.toml").write_text(
            '[package]\nname = "fixture"\nversion = "1.0.0"\nedition = "2021"\n',
            encoding="utf-8",
        )
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_text("pub fn fixture() {}\n", encoding="utf-8")
        (self.root / "compatibility/axon.lock.json").write_text(
            json.dumps({"cli": {"runtime_version": "1.0.0"}}), encoding="utf-8"
        )
        (self.root / "sdk/conformance/fixtures/feature-discovery.v7.json").write_text(
            json.dumps({"sdk_version": "1.0.0"}), encoding="utf-8"
        )
        (self.root / "sdk/node/package.json").write_text(
            json.dumps({"version": "0.0.0-seam"}), encoding="utf-8"
        )
        (self.root / "sdk/python/pyproject.toml").write_text(
            '[project]\nversion = "0.7.0"\n', encoding="utf-8"
        )
        run(["cargo", "generate-lockfile", "--quiet"], self.root)
        run(["git", "init", "-b", "release/test"], self.root)
        run(["git", "add", "."], self.root)
        run(
            [
                "git",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=f@example.com",
                "commit",
                "-m",
                "fixture",
            ],
            self.root,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def invoke(
        self, *arguments: str, env: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess[str]:
        return run(
            [str(self.root / "tools/scripts/update-project-version.sh"), *arguments],
            self.root,
            check=False,
            env=env,
        )

    def test_check_is_read_only_and_update_is_runtime_scoped(self) -> None:
        before = run(["git", "status", "--porcelain"], self.root).stdout
        checked = self.invoke("--check", "1.0.0")
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(
            run(["git", "status", "--porcelain"], self.root).stdout, before
        )
        updated = self.invoke("1.2.3")
        self.assertEqual(updated.returncode, 0, updated.stderr)
        self.assertIn('version = "1.2.3"', (self.root / "Cargo.toml").read_text())
        self.assertEqual(
            json.loads((self.root / "sdk/node/package.json").read_text())["version"],
            "0.0.0-seam",
        )
        self.assertIn(
            'version = "0.7.0"', (self.root / "sdk/python/pyproject.toml").read_text()
        )

    def test_generator_failure_restores_every_target(self) -> None:
        before = {
            path: (self.root / path).read_bytes()
            for path in (
                "VERSION",
                "Cargo.toml",
                "Cargo.lock",
                "compatibility/axon.lock.json",
                "sdk/conformance/fixtures/feature-discovery.v7.json",
            )
        }
        fake_bin = self.root / "fake-bin"
        fake_bin.mkdir()
        fake_cargo = fake_bin / "cargo"
        fake_cargo.write_text("#!/usr/bin/env bash\nexit 9\n", encoding="utf-8")
        fake_cargo.chmod(0o755)
        environment = dict(os.environ)
        environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
        failed = self.invoke("1.2.3", env=environment)
        self.assertNotEqual(failed.returncode, 0)
        for path, content in before.items():
            self.assertEqual((self.root / path).read_bytes(), content)

    def test_malformed_version_and_dirty_target_are_rejected(self) -> None:
        self.assertNotEqual(self.invoke("bad").returncode, 0)
        (self.root / "VERSION").write_text("dirty\n", encoding="utf-8")
        self.assertNotEqual(self.invoke("1.2.3").returncode, 0)


if __name__ == "__main__":
    unittest.main()
