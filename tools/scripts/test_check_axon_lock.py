#!/usr/bin/env python3
"""Focused tests for the CLI Axon lock checker."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
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
        axon_root = self.axon_fixture(changed["axon"])
        with (
            mock.patch.object(MODULE, "require_clean_checkout"),
            mock.patch.object(
                MODULE, "git_head", return_value=changed["axon"]["git_revision"]
            ),
            self.assertRaisesRegex(MODULE.LockError, "contract digest mismatch"),
        ):
            MODULE.verify_axon_checkout(axon_root, changed["axon"])

    def test_dirty_axon_checkout_is_rejected(self) -> None:
        completed = mock.Mock(returncode=0, stdout=" M sdk/rust/src/lib.rs\n", stderr="")
        with mock.patch.object(MODULE.subprocess, "run", return_value=completed):
            with self.assertRaisesRegex(MODULE.LockError, "must be clean"):
                MODULE.require_clean_checkout(self.root.parent / "EasyNet-Axon")

    def test_committed_cli_and_axon_checkout_match(self) -> None:
        validated = MODULE.validate_lock(self.lock)
        MODULE.verify_cli_sources(self.root, validated)
        axon = copy.deepcopy(validated["axon"])
        axon_root = self.axon_fixture(axon)
        axon["contract_sha256"] = hashlib.sha256(
            (axon_root / MODULE.AXON_CONTRACT_PATH).read_bytes()
        ).hexdigest()
        with (
            mock.patch.object(MODULE, "require_clean_checkout"),
            mock.patch.object(
                MODULE, "git_head", return_value=axon["git_revision"]
            ),
        ):
            MODULE.verify_axon_checkout(axon_root, axon)

    def axon_fixture(self, axon: dict[str, object]) -> Path:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        contract_path = root / MODULE.AXON_CONTRACT_PATH
        contract_path.parent.mkdir(parents=True)
        contract = {
            "axon_release_version": axon["release_version"],
            "protocol": {
                "descriptor_set_sha256": axon["protocol"]["descriptor_set_sha256"]
            },
            "ffi": {
                "dendrite_abi_version": axon["ffi"]["dendrite_abi_version"],
                "public_header_sha256": axon["ffi"]["public_header_sha256"],
            },
            "sdks": axon["sdks"],
        }
        contract_path.write_text(json.dumps(contract), encoding="utf-8")
        return root


if __name__ == "__main__":
    unittest.main()
