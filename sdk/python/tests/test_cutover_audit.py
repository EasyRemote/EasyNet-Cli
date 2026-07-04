import tempfile
import textwrap
import unittest
from pathlib import Path

from easynet_sdk import EasyRemoteCutoverAuditor, audit_easyremote_cutover


class EasyRemoteCutoverAuditTests(unittest.TestCase):
    def test_accepts_sdk_only_consumer_facade(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "client.py").write_text(
                textwrap.dedent(
                    """
                    from easynet_sdk import (
                        AbilityInvocationClient,
                        InvocationDraft,
                        ReceiptClient,
                    )

                    def invoke(client: AbilityInvocationClient, draft: InvocationDraft):
                        return client.runtime.invoke(draft)
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

    def test_flags_raw_ffi_and_abi_symbols(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "abi.py").write_text(
                textwrap.dedent(
                    """
                    import ctypes

                    lib = ctypes.CDLL("libeasynet_cli.dylib")
                    symbol = "easynet_runtime_invoke"
                    """
                ),
                encoding="utf-8",
            )

            result = EasyRemoteCutoverAuditor().audit_path(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_lower_layer_import", _rules(result))
        self.assertIn("raw_ffi_loader", _rules(result))
        self.assertIn("raw_c_abi_symbol", _rules(result))

    def test_flags_raw_axon_imports_and_invocation_json_codec(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "invocation.py").write_text(
                textwrap.dedent(
                    '''
                    import json
                    from easynet_axon import invocation

                    raw = json.dumps({"caller_ura": "a", "descriptor_ref": "b"})
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_lower_layer_import", _rules(result))
        self.assertIn("raw_invocation_json_codec", _rules(result))

    def test_flags_multiline_raw_invocation_json_codec(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "invocation.py").write_text(
                textwrap.dedent(
                    """
                    import json

                    raw = json.dumps(
                        {
                            "caller_ura": "easynet:///r/example/agent/alice",
                            "callee_ura": "easynet:///r/example/device/dev-a",
                            "descriptor_ref": "easynet:///r/example/ability/a@1.0.0",
                            "subject_ura": "easynet:///r/example/device/dev-a",
                            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                            "causal_context": {"form": "none"},
                        }
                    )
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_invocation_json_codec", _rules(result))

    def test_require_ok_reports_all_violations(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bad.py").write_text(
                "import ctypes\nsymbol = 'easynet_last_error'\n",
                encoding="utf-8",
            )
            result = audit_easyremote_cutover(root)

        with self.assertRaises(AssertionError) as caught:
            result.require_ok()
        self.assertIn("raw_lower_layer_import", str(caught.exception))
        self.assertIn("raw_c_abi_symbol", str(caught.exception))


def _rules(result) -> set[str]:
    return {item.rule for item in result.violations}


if __name__ == "__main__":
    unittest.main()
