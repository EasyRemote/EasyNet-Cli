#!/usr/bin/env python3
"""Focused tests for the CLI Axon lock checker."""

from __future__ import annotations

import copy
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("check-axon-lock.py")
SPEC = importlib.util.spec_from_file_location("check_axon_lock", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class AxonLockTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.root = SCRIPT.resolve().parents[2]
        cls.lock = MODULE.load_json_object(
            cls.root / MODULE.LOCK_PATH, "Axon lock"
        )

    def test_committed_lock_schema_is_closed_and_valid(self) -> None:
        self.assertEqual(MODULE.validate_lock(self.lock), self.lock)

    def test_unknown_top_level_fact_is_rejected(self) -> None:
        changed = copy.deepcopy(self.lock)
        changed["legacy_revision"] = changed["axon"]["git_revision"]
        with self.assertRaises(MODULE.LockError):
            MODULE.validate_lock(changed)

    def test_short_revision_is_rejected(self) -> None:
        changed = copy.deepcopy(self.lock)
        changed["axon"]["git_revision"] = "deadbeef"
        with self.assertRaises(MODULE.LockError):
            MODULE.validate_lock(changed)

    def test_contract_digest_drift_is_rejected(self) -> None:
        changed = copy.deepcopy(self.lock)
        changed["axon"]["contract_sha256"] = "0" * 64
        with (
            mock.patch.object(MODULE, "require_clean_checkout"),
            mock.patch.object(
                MODULE, "git_head", return_value=changed["axon"]["git_revision"]
            ),
            self.assertRaisesRegex(MODULE.LockError, "contract digest mismatch"),
        ):
            MODULE.verify_axon_checkout(
                self.root.parent / "EasyNet-Axon", changed["axon"]
            )

    def test_dirty_axon_checkout_is_rejected(self) -> None:
        completed = mock.Mock(returncode=0, stdout=" M sdk/rust/src/lib.rs\n", stderr="")
        with mock.patch.object(MODULE.subprocess, "run", return_value=completed):
            with self.assertRaisesRegex(MODULE.LockError, "must be clean"):
                MODULE.require_clean_checkout(self.root.parent / "EasyNet-Axon")

    def test_committed_cli_and_axon_checkout_match(self) -> None:
        validated = MODULE.validate_lock(self.lock)
        MODULE.verify_cli_sources(self.root, validated)
        with (
            mock.patch.object(MODULE, "require_clean_checkout"),
            mock.patch.object(
                MODULE, "git_head", return_value=validated["axon"]["git_revision"]
            ),
        ):
            MODULE.verify_axon_checkout(
                self.root.parent / "EasyNet-Axon", validated["axon"]
            )


if __name__ == "__main__":
    unittest.main()
