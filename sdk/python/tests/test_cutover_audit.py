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

    def test_accepts_sdk_only_pyproject_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "pyproject.toml").write_text(
                textwrap.dedent(
                    """
                    [project]
                    name = "consumer"
                    dependencies = ["easynet-sdk>=0.91.30"]

                    [project.optional-dependencies]
                    dev = ["pytest>=8"]
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

    def test_flags_raw_lower_layer_pyproject_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "pyproject.toml").write_text(
                textwrap.dedent(
                    """
                    [project]
                    name = "consumer"
                    dependencies = [
                        "easynet-sdk>=0.91.30",
                        "easynet-run-axon>=0.4",
                    ]
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_lower_layer_dependency", _rules(result))

    def test_flags_raw_lower_layer_requirements_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "requirements.txt").write_text(
                "easynet-sdk>=0.91\n# easynet-run-axon in comment\naxon==1.0\n",
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_lower_layer_dependency", _rules(result))

    def test_flags_raw_lower_layer_setup_py_dependency_without_execution(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "setup.py").write_text(
                textwrap.dedent(
                    '''
                    """Docstring mention: easynet-run-axon."""
                    from setuptools import setup

                    setup(
                        name="consumer",
                        install_requires=["easynet_axon>=1"],
                    )
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_lower_layer_dependency", _rules(result))

    def test_ignores_consumer_test_fixtures_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "easyremote").mkdir()
            (root / "easyremote" / "client.py").write_text(
                "from easynet_sdk import DaemonInvocationTransport\n",
                encoding="utf-8",
            )
            (root / "tests").mkdir()
            (root / "tests" / "test_legacy.py").write_text(
                textwrap.dedent(
                    """
                    import json

                    raw = json.dumps({
                        "caller_ura": "a",
                        "callee_ura": "b",
                        "descriptor_ref": "c",
                        "subject_ura": "d",
                        "nonce_base64": "e",
                        "causal_context": {},
                    })
                    symbol = "easynet_invocation_invoke"
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

    def test_ignores_docstrings_and_comments_about_old_raw_symbols(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs.py").write_text(
                textwrap.dedent(
                    '''
                    """Legacy notes mention easynet_daemon_start and ctypes.CDLL."""

                    def explain() -> str:
                        """This docstring says easynet_invocation_invoke."""
                        # A comment also mentions easynet_last_error.
                        return "sdk-only"
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

    def test_flags_executable_raw_symbol_strings_and_attributes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "abi.py").write_text(
                textwrap.dedent(
                    """
                    class Lib:
                        pass

                    lib = Lib()
                    symbol = "easynet_invocation_invoke"
                    getattr(lib, "easynet_last_error")
                    lib.easynet_daemon_start
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
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
        self.assertNotIn("raw_c_abi_symbol", _rules(result))

    def test_flags_raw_publication_carrier_literals(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "control.py").write_text(
                textwrap.dedent(
                    '''
                    def install(client):
                        """A docstring can mention ability.deploy safely."""
                        return client.invoke("ability.deploy", node_id="local")

                    def list_abilities(client):
                        return client.invoke("meta.list_abilities")
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_publication_carrier", _rules(result))

    def test_flags_raw_admin_carrier_literals(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "control.py").write_text(
                textwrap.dedent(
                    '''
                    def add(client):
                        """A docstring can mention agent.start safely."""
                        return client.invoke("agent.start", name="codex")

                    def refresh(client):
                        return client.invoke("agent.refresh")
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_admin_carrier", _rules(result))

    def test_ignores_comments_and_docstrings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs_only.py").write_text(
                textwrap.dedent(
                    '''
                    """Mentions easynet_daemon_start and dlopen in prose only."""

                    # ctypes.CDLL("libeasynet_cli.dylib")
                    # symbol = "easynet_runtime_invoke"
                    from easynet_sdk import RuntimeClient

                    def use(client: RuntimeClient) -> None:
                        """References easynet_last_error in documentation."""
                        client.close()
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

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

    def test_flags_invocation_tuple_dict_without_json_call(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "invocation.py").write_text(
                textwrap.dedent(
                    """
                    def encode():
                        return {
                            "caller_ura": "easynet:///r/example/agent/alice",
                            "callee_ura": "easynet:///r/example/device/dev-a",
                            "descriptor_ref": "easynet:///r/example/ability/a@1.0.0",
                            "subject_ura": "easynet:///r/example/device/dev-a",
                            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                            "causal_context": {"form": "none"},
                        }
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
