from pathlib import Path
import json
import subprocess
import sys
import unittest

import easynet_sdk
import easynet_sdk.direct_runtime as direct_runtime


class ImportBoundaryTests(unittest.TestCase):
    def test_public_python_sdk_does_not_import_forbidden_runtime_boundaries(
        self,
    ) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        private_cabi = root / "_cabi.py"
        provider_root = root / "providers"
        axon_addressing = root / "axon_addressing.py"
        direct_runtime_path = root / "direct_runtime.py"
        directory = root / "directory.py"
        receipt = root / "receipt.py"
        runtime = root / "runtime.py"
        axon_adapters = {axon_addressing, direct_runtime_path, receipt, runtime}
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
            if path.is_relative_to(provider_root):
                continue
            body = path.read_text()
            for needle in forbidden:
                if (
                    path in {axon_addressing, directory, receipt}
                    and needle == "easynet_"
                ):
                    continue
                if path in axon_adapters and needle in {"import axon", "from axon"}:
                    continue
                self.assertNotIn(needle, body, f"{path} contains {needle!r}")

        addressing_body = axon_addressing.read_text()
        self.assertIn("from axon_sdk", addressing_body)
        self.assertNotIn("service_locator", addressing_body)
        self.assertNotIn("open_cabi", addressing_body)

        receipt_body = receipt.read_text()
        self.assertIn("from axon_sdk.invocation import", receipt_body)
        self.assertIn("parse_invocation_ledger_record", receipt_body)
        self.assertIn("parse_invocation_trace_graph", receipt_body)
        self.assertIn("verify_receipt_chain", receipt_body)
        self.assertNotIn("_axon_pb", receipt_body)
        self.assertNotIn("direct_runtime", receipt_body)
        self.assertNotIn("_cabi", receipt_body)

        direct_runtime_body = direct_runtime_path.read_text()
        self.assertIn("from axon_sdk.invocation import", direct_runtime_body)
        self.assertIn("_AxonDescriptorBoundEnvelope", direct_runtime_body)
        self.assertIn("_AxonDescriptorBoundInvocationRequest", direct_runtime_body)

    def test_raw_cabi_is_confined_to_private_transport_adapter(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        private_cabi = root / "_cabi.py"
        self.assertTrue(private_cabi.exists())
        body = private_cabi.read_text()
        self.assertIn("import ctypes", body)
        self.assertIn("easynet_abi_version", body)

    def test_python_sdk_root_exports_only_runtime_concepts(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        self.assertFalse((root / "easyremote_profiles.py").exists())
        exported = set(getattr(easynet_sdk, "__all__", ()))
        leaked = sorted(name for name in exported if name.startswith("EasyRemote"))
        self.assertEqual(leaked, [])
        for name in (
            "ListModelsRequest",
            "ChatCompletionRequest",
            "StreamChatCompletionRequest",
            "GatewayLifecycleFacade",
            "MissionClient",
        ):
            self.assertNotIn(name, exported)
            self.assertFalse(hasattr(easynet_sdk, name), name)
        for name in (
            "AddressingClient",
            "DirectoryClient",
            "InvocationDraft",
            "ReceiptClient",
            "ReceiptReference",
            "RuntimeClient",
            "RuntimeReceiptProvider",
        ):
            self.assertIn(name, exported)
            self.assertTrue(hasattr(easynet_sdk, name), name)

    def test_python_sdk_root_namespace_matches_public_exports(self) -> None:
        script = """
import json
import easynet_sdk
exported = set(getattr(easynet_sdk, "__all__", ()))
allowed_metadata = {"annotations", "direct_runtime", "providers"}
leaked = sorted(
    name
    for name in dir(easynet_sdk)
    if not name.startswith("_")
    and name not in exported
    and name not in allowed_metadata
)
print(json.dumps(leaked))
"""
        completed = subprocess.run(
            [sys.executable, "-c", script],
            check=True,
            capture_output=True,
            text=True,
        )
        leaked = json.loads(completed.stdout)
        self.assertEqual(leaked, [])

    def test_python_sdk_lifecycle_helper_uses_runtime_capability_terms(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        body = (root / "_lifecycle.py").read_text()
        self.assertIn("_capability", body)
        self.assertNotIn("_profile", body)
        self.assertNotIn("profile: str", body)

    def test_python_sdk_root_does_not_export_direct_runtime_internals(self) -> None:
        exported = set(getattr(easynet_sdk, "__all__", ()))
        self.assertNotIn("DirectRuntimeConnector", exported)
        self.assertNotIn("DirectRuntimeTransport", exported)
        self.assertFalse(hasattr(easynet_sdk, "DirectRuntimeConnector"))
        self.assertFalse(hasattr(easynet_sdk, "DirectRuntimeTransport"))

    def test_direct_runtime_does_not_export_axon_protobuf_modules(self) -> None:
        self.assertFalse(hasattr(direct_runtime, "invoke_pb2"))
        self.assertFalse(hasattr(direct_runtime, "invoke_pb2_grpc"))
        self.assertFalse(hasattr(direct_runtime, "types_pb2"))

    def test_descriptor_ref_grammar_stays_out_of_python_runtime_core(self) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        guarded = (
            root / "ability_descriptor.py",
            root / "invocation.py",
            root / "__init__.py",
        )
        forbidden = (
            "validate_ability_descriptor_ref_shape",
            '.split("@',
            ".split('@",
            '.rsplit("@',
            ".rsplit('@",
            '.partition("@',
            ".partition('@",
            '.rpartition("@',
            ".rpartition('@",
            '.count("@',
            ".count('@",
        )
        for path in guarded:
            body = path.read_text()
            for needle in forbidden:
                self.assertNotIn(needle, body, f"{path} contains {needle!r}")

    def test_descriptor_ref_public_helper_delegates_to_addressing_projection(
        self,
    ) -> None:
        root = Path(__file__).resolve().parents[1] / "easynet_sdk"
        body = (root / "ability_descriptor.py").read_text()
        self.assertIn(".axon_addressing", body)
        self.assertIn("project_descriptor_ref", body)


if __name__ == "__main__":
    unittest.main()
