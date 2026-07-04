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

    def test_flags_legacy_private_transport_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            transport = root / "easyremote" / "_transport"
            transport.mkdir(parents=True)
            (transport / "abi.py").write_text(
                textwrap.dedent(
                    """
                    class Session:
                        def invoke(self, payload):
                            return payload
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_transport_module", _rules(result))

    def test_flags_legacy_private_transport_imports(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "client.py").write_text(
                textwrap.dedent(
                    """
                    import easyremote._transport.abi
                    from ._transport import session
                    from . import _transport
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        violations = [item for item in result.violations if item.rule == "raw_transport_module"]
        self.assertGreaterEqual(len(violations), 3)

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

    def test_allows_sdk_addressing_helper_imports(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "addressing.py").write_text(
                textwrap.dedent(
                    """
                    from easynet_sdk import (
                        ability_ura_from_descriptor_ref,
                        canonical_ability_descriptor_ref,
                        owner_ability_ura,
                        owner_ura_for_ability,
                        parse_ura,
                    )

                    def build(identity, owner, ability_name):
                        ability = owner_ability_ura(owner, ability_name)
                        descriptor = canonical_ability_descriptor_ref(ability, "1.0.0")
                        return parse_ura(owner_ura_for_ability(ability)), descriptor
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

    def test_allows_sdk_identity_facade_helper_methods(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "_sdk_identity.py").write_text(
                textwrap.dedent(
                    """
                    import easynet_sdk

                    class IdentityFacade:
                        def parse_ura(self, value):
                            raise NotImplementedError

                        def owner_ability_ura(self, owner_ura, ability_name):
                            raise NotImplementedError

                        def owner_ura_for_ability(self, ability_ura):
                            raise NotImplementedError

                        def canonical_ability_descriptor_ref(self, value, version=""):
                            raise NotImplementedError

                    class SdkIdentityFacade:
                        def parse_ura(self, value):
                            return easynet_sdk.parse_ura(value)

                        def owner_ability_ura(self, owner_ura, ability_name):
                            return easynet_sdk.owner_ability_ura(owner_ura, ability_name)

                        def owner_ura_for_ability(self, ability_ura):
                            return easynet_sdk.owner_ura_for_ability(ability_ura)

                        def canonical_ability_descriptor_ref(self, value, version=""):
                            return easynet_sdk.canonical_ability_descriptor_ref(value, version)
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

    def test_flags_non_sdk_identity_facade_helper_methods(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "_sdk_identity.py").write_text(
                textwrap.dedent(
                    """
                    def parse_ura(value):
                        return value
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_addressing_helper", _rules(result))

    def test_flags_raw_addressing_helpers_and_descriptor_ref_assembly(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "_addressing.py").write_text(
                textwrap.dedent(
                    '''
                    def parse_ura(value):
                        """Docstring can mention canonical_ability_descriptor_ref."""
                        return value.split("/")

                    def owner_ability_ura(owner_ura, ability_name):
                        return f"{owner_ura}.ability.{ability_name}"

                    def canonical_ability_descriptor_ref(ability_ura, version):
                        return ability_ura + "@" + version

                    def ability_ura_from_descriptor_ref(descriptor_ref):
                        return descriptor_ref.split("@", 1)[0]
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_addressing_helper", _rules(result))
        self.assertIn("raw_descriptor_ref_assembly", _rules(result))

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

    def test_flags_raw_mission_carrier_literals(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "mission.py").write_text(
                textwrap.dedent(
                    '''
                    def run(client):
                        """A docstring can mention mission.run safely."""
                        return client.invoke("mission.run", source="mission x {}")

                    def cancel(client):
                        return client.invoke("mission.cancel", run_id="run-1")

                    def events(client):
                        return client.invoke("mission.events", run_id="run-1")
                    '''
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_mission_carrier", _rules(result))

    def test_flags_raw_context_causal_ref_construction(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "context_dispatch.py").write_text(
                textwrap.dedent(
                    """
                    from .invocation import CausalRef

                    def child_causal(parent_receipt):
                        return CausalRef(
                            receipt_hash=bytes.fromhex(
                                parent_receipt.raw["self_hash_hex"]
                            ),
                            receipt_ura=parent_receipt.raw["receipt_ura"],
                        )
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_context_causal_ref", _rules(result))

    def test_accepts_context_causal_ref_via_sdk_receipt_projection(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "context_dispatch.py").write_text(
                textwrap.dedent(
                    """
                    import easynet_sdk
                    from .invocation import CausalRef

                    def child_causal(receipt_json):
                        ref = easynet_sdk.ReceiptClient(
                            easynet_sdk.LocalReceiptTransport()
                        ).causal_ref(receipt_json)
                        return CausalRef(
                            receipt_hash=bytes.fromhex(ref.receipt_hash_hex),
                            receipt_ura=ref.receipt_ura,
                        )
                    """
                ),
                encoding="utf-8",
            )

            result = audit_easyremote_cutover(root)

        self.assertTrue(result.ok)

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
