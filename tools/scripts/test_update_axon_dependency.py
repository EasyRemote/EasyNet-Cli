#!/usr/bin/env python3
"""Tests for exact Axon dependency projection."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("update-axon-dependency.py")
SPEC = importlib.util.spec_from_file_location("update_axon_dependency", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class DependencyProjectionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="cli-axon-dependency-")
        base = Path(self.temporary.name)
        self.root = base / "EasyNet-Cli"
        self.axon = base / "EasyNet-Axon"
        (self.root / "compatibility").mkdir(parents=True)
        (self.root / "sdk/python").mkdir(parents=True)
        (self.root / "sdk/node").mkdir(parents=True)
        (self.root / "sdk/go").mkdir(parents=True)
        (self.axon / "compatibility").mkdir(parents=True)
        (self.root / "VERSION").write_text("2.3.4\n", encoding="utf-8")
        (self.root / "compatibility/axon.lock.json").write_text(
            "{}\n", encoding="utf-8"
        )
        (self.root / "sdk/python/pyproject.toml").write_text(
            '[project]\nversion = "1.4.5"\ndependencies = [\n    "axon-runtime-sdk>=0.1.0,<0.2",\n]\n',
            encoding="utf-8",
        )
        (self.root / "sdk/node/package.json").write_text(
            json.dumps({"version": "0.0.0-seam"}), encoding="utf-8"
        )
        (self.root / "sdk/go/go.mod").write_text(
            "module example\n\nrequire axon.run/sdk/go v0.1.0\n", encoding="utf-8"
        )
        contract = {
            "axon_release_version": "0.193.2",
            "protocol": {"descriptor_set_sha256": "a" * 64},
            "ffi": {"dendrite_abi_version": 1, "public_header_sha256": "b" * 64},
            "sdks": {
                name: "0.193.2"
                for name in ("rust", "python", "go", "node", "react", "java", "swift")
            },
        }
        (self.axon / "compatibility/contract.json").write_text(
            json.dumps(contract), encoding="utf-8"
        )
        subprocess.run(
            ["git", "init", "-b", "release/test"],
            cwd=self.axon,
            check=True,
            capture_output=True,
        )
        subprocess.run(["git", "add", "."], cwd=self.axon, check=True)
        subprocess.run(
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
            cwd=self.axon,
            check=True,
            capture_output=True,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_write_then_check_projects_every_owned_fact(self) -> None:
        MODULE.check_or_write(self.root, self.axon, True)
        MODULE.check_or_write(self.root, self.axon, False)
        pyproject = (self.root / "sdk/python/pyproject.toml").read_text()
        self.assertIn("axon-runtime-sdk>=0.193.2,<0.194", pyproject)
        self.assertIn(
            "axon.run/sdk/go v0.193.2", (self.root / "sdk/go/go.mod").read_text()
        )
        lock = json.loads((self.root / "compatibility/axon.lock.json").read_text())
        self.assertEqual(lock["cli"]["runtime_version"], "2.3.4")
        self.assertEqual(lock["cli"]["sdks"]["node"], "0.0.0-seam")

    def test_check_reports_drift_without_writing(self) -> None:
        before = (self.root / "compatibility/axon.lock.json").read_bytes()
        with self.assertRaises(MODULE.DependencyError):
            MODULE.check_or_write(self.root, self.axon, False)
        self.assertEqual(
            (self.root / "compatibility/axon.lock.json").read_bytes(), before
        )

    def test_rejects_local_go_replace_and_malformed_version(self) -> None:
        go_mod = self.root / "sdk/go/go.mod"
        go_mod.write_text(go_mod.read_text() + "replace axon.run/sdk/go => ../Axon\n")
        with self.assertRaises(MODULE.DependencyError):
            MODULE.check_or_write(self.root, self.axon, True)
        with self.assertRaises(MODULE.DependencyError):
            MODULE.next_minor("bad")


if __name__ == "__main__":
    unittest.main()
