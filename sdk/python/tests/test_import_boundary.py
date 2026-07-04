from pathlib import Path
import unittest


class ImportBoundaryTests(unittest.TestCase):
    def test_public_python_sdk_does_not_import_forbidden_runtime_boundaries(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        private_cabi = root / "_cabi.py"
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
            if path == private_cabi:
                continue
            body = path.read_text()
            for needle in forbidden:
                self.assertNotIn(needle, body, f"{path} contains {needle!r}")

    def test_raw_cabi_is_confined_to_private_transport_adapter(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        private_cabi = root / "_cabi.py"
        self.assertTrue(private_cabi.exists())
        body = private_cabi.read_text()
        self.assertIn("import ctypes", body)
        self.assertIn("easynet_abi_version", body)


if __name__ == "__main__":
    unittest.main()
