#!/usr/bin/env python3
"""Focused tests for workflow referential integrity."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("check-workflow-integrity.py")
SPEC = importlib.util.spec_from_file_location("workflow_integrity", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class WorkflowIntegrityTest(unittest.TestCase):
    def fixture(self, workflow: str, *, create_script: bool = True, executable: bool = True) -> Path:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        (root / ".github/workflows").mkdir(parents=True)
        (root / ".github/workflows/test.yml").write_text(workflow, encoding="utf-8")
        if create_script:
            script = root / "tools/scripts/check.sh"
            script.parent.mkdir(parents=True)
            script.write_text("#!/usr/bin/env bash\n", encoding="utf-8")
            os.chmod(script, 0o755 if executable else 0o644)
        return root

    def test_existing_interpreter_invocation_passes(self) -> None:
        root = self.fixture("jobs:\n  gate:\n    steps:\n      - run: bash tools/scripts/check.sh\n")
        self.assertEqual(MODULE.validate(root), [])

    def test_missing_multiline_reference_fails(self) -> None:
        root = self.fixture(
            "jobs:\n  gate:\n    steps:\n      - run: |\n          bash tools/scripts/check.sh\n",
            create_script=False,
        )
        self.assertRegex(MODULE.validate(root)[0], "local reference is missing")

    def test_direct_non_executable_reference_fails(self) -> None:
        root = self.fixture(
            "jobs:\n  gate:\n    steps:\n      - run: ./tools/scripts/check.sh\n",
            executable=False,
        )
        self.assertRegex(MODULE.validate(root)[0], "not executable")

    def test_repository_escape_fails(self) -> None:
        root = self.fixture(
            "jobs:\n  gate:\n    steps:\n      - run: ../tools/scripts/check.sh\n"
        )
        self.assertRegex(MODULE.validate(root)[0], "escapes repository")


if __name__ == "__main__":
    unittest.main()
