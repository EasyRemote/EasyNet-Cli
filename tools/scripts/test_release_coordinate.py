#!/usr/bin/env python3
"""Focused integration tests for the Runtime release-coordinate transaction."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("release-coordinate.py")


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


class ReleaseCoordinateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="runtime-release-test-")
        base = Path(self.temporary.name)
        self.cli = base / "EasyNet-Cli-source"
        self.axon = base / "EasyNet-Axon-source"
        for path in (
            self.cli / "tools/scripts",
            self.cli / "compatibility",
            self.cli / "sdk/go",
            self.axon / "scripts/checks",
            self.axon / "compatibility",
        ):
            path.mkdir(parents=True)
        (self.cli / "VERSION").write_text("1.0.0\n", encoding="utf-8")
        (self.cli / "compatibility/axon.lock.json").write_text("{}\n", encoding="utf-8")
        (self.cli / "sdk/go/go.mod").write_text("module fixture\n", encoding="utf-8")
        self._write_executable(
            self.cli / "tools/scripts/update-axon-dependency.py",
            """#!/usr/bin/env python3
from pathlib import Path
import sys
if '--write' in sys.argv:
    Path('compatibility/axon.lock.json').write_text('{"pinned": true}\\n')
""",
        )
        self._write_executable(
            self.cli / "tools/scripts/update-project-version.sh",
            """#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" != "--check" ]]; then printf '%s\\n' "$1" > VERSION; fi
""",
        )
        self._write_executable(
            self.cli / "tools/scripts/check-axon-lock.py", "#!/usr/bin/env python3\n"
        )
        self._write_executable(
            self.axon / "scripts/checks/check_compatibility_contract.py",
            "#!/usr/bin/env python3\n",
        )
        (self.axon / "compatibility/contract.json").write_text("{}\n", encoding="utf-8")
        self._init_repo(self.cli)
        self._init_repo(self.axon)
        self.remote = base / "remote.git"
        run(["git", "init", "--bare", str(self.remote)], self.cli)
        run(["git", "remote", "add", "origin", str(self.remote)], self.cli)
        fake_bin = base / "fake-bin"
        fake_bin.mkdir()
        for name in ("uv", "go"):
            self._write_executable(fake_bin / name, "#!/usr/bin/env bash\nexit 0\n")
        self.environment = dict(os.environ)
        self.environment["PATH"] = f"{fake_bin}:{self.environment['PATH']}"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def _write_executable(path: Path, content: str) -> None:
        path.write_text(content, encoding="utf-8")
        path.chmod(0o755)

    @staticmethod
    def _init_repo(root: Path) -> None:
        run(["git", "init", "-b", "release/test"], root)
        run(["git", "add", "."], root)
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
            root,
        )

    def invoke(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return run(
            [
                "python3",
                str(SCRIPT),
                "--root",
                str(self.cli),
                "--axon-root",
                str(self.axon),
                *arguments,
            ],
            self.cli,
            check=False,
            env=self.environment,
        )

    def test_check_is_read_only(self) -> None:
        head = run(["git", "rev-parse", "HEAD"], self.cli).stdout
        result = self.invoke("--check", "--version", "1.2.3")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(run(["git", "rev-parse", "HEAD"], self.cli).stdout, head)
        self.assertEqual((self.cli / "VERSION").read_text(), "1.0.0\n")

    def test_check_accepts_detached_ci_worktree(self) -> None:
        run(["git", "checkout", "--detach"], self.cli)
        result = self.invoke("--check", "--version", "1.2.3")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_commit_has_exact_identity_and_coordinate(self) -> None:
        result = self.invoke("--commit", "--version", "1.2.3")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.cli / "VERSION").read_text(), "1.2.3\n")
        identity = run(
            ["git", "show", "-s", "--format=%an <%ae>"], self.cli
        ).stdout.strip()
        self.assertEqual(identity, "Silan.Hu <silan.hu@u.nus.edu>")

    def test_dirty_and_protected_push_fail_before_preparation(self) -> None:
        (self.cli / "dirty").write_text("dirty", encoding="utf-8")
        self.assertNotEqual(self.invoke("--commit", "--version", "1.2.3").returncode, 0)
        (self.cli / "dirty").unlink()
        run(["git", "branch", "-M", "main"], self.cli)
        result = self.invoke("--push", "--version", "1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected branch", result.stderr)

    def test_push_advances_exact_remote_branch(self) -> None:
        result = self.invoke("--push", "--version", "1.2.3")
        self.assertEqual(result.returncode, 0, result.stderr)
        local = run(["git", "rev-parse", "HEAD"], self.cli).stdout.strip()
        remote = run(
            [
                "git",
                "--git-dir",
                str(self.remote),
                "rev-parse",
                "refs/heads/release/test",
            ],
            self.cli,
        ).stdout.strip()
        self.assertEqual(remote, local)


if __name__ == "__main__":
    unittest.main()
