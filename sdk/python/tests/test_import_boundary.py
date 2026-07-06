from pathlib import Path
import unittest

import easynet_sdk
import easynet_sdk.direct_runtime as direct_runtime


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

    def test_python_sdk_root_does_not_export_product_aliases(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        self.assertFalse((root / "easyremote_profiles.py").exists())
        exported = set(getattr(easynet_sdk, "__all__", ()))
        leaked = sorted(name for name in exported if name.startswith("EasyRemote"))
        self.assertEqual(leaked, [])

    def test_python_sdk_root_does_not_export_direct_runtime_internals(self) -> None:
        exported = set(getattr(easynet_sdk, "__all__", ()))
        self.assertNotIn("DirectDaemonRuntimeConnector", exported)
        self.assertNotIn("DirectDaemonRuntimeTransport", exported)
        self.assertFalse(hasattr(easynet_sdk, "DirectDaemonRuntimeConnector"))
        self.assertFalse(hasattr(easynet_sdk, "DirectDaemonRuntimeTransport"))

    def test_direct_runtime_does_not_export_axon_protobuf_modules(self) -> None:
        self.assertFalse(hasattr(direct_runtime, "invoke_pb2"))
        self.assertFalse(hasattr(direct_runtime, "invoke_pb2_grpc"))
        self.assertFalse(hasattr(direct_runtime, "types_pb2"))


if __name__ == "__main__":
    unittest.main()
