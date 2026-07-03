from pathlib import Path
import unittest


class ImportBoundaryTests(unittest.TestCase):
    def test_public_python_sdk_does_not_import_forbidden_runtime_boundaries(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        forbidden = [
            "import ctypes",
            "from ctypes",
            "import cffi",
            "from cffi",
            "dlopen",
            "libeasynet_cli",
            "easynet_",
            "import axon",
            "from axon",
            "import protobuf",
            "from protobuf",
        ]
        for path in root.rglob("*.py"):
            body = path.read_text()
            for needle in forbidden:
                self.assertNotIn(needle, body, f"{path} contains {needle!r}")


if __name__ == "__main__":
    unittest.main()
