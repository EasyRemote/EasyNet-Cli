import json
import tempfile
import unittest

from dataclasses import dataclass
from pathlib import Path

from easynet_sdk import (
    AbilityCallRequest,
    AbilityInvocationClient,
    AbilityTargetRequest,
    AddressingClient,
    ErrorCode,
    InvocationObjectAdapter,
    InvocationResult,
    ReceiptClient,
    RuntimeClient,
    is_code,
    ability_address,
    audit_consumer_boundary,
)

from test_identity import MemoryIdentityTransport
from test_receipt import MemoryReceiptTransport
from test_runtime import MemoryRuntimeTransport


class EasyRemoteCutoverTests(unittest.TestCase):
    def test_invocation_object_adapter_rejects_route_name_alias(
        self,
    ) -> None:
        identity_transport = RejectBareDescriptorTransport()
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity_transport),
        )
        adapter = InvocationObjectAdapter(client)

        with self.assertRaises(Exception) as caught:
            adapter.build_invocation(
                EasyRemoteTuple(
                    caller="easynet:///r/example/agent/alice.sdk",
                    callee="easynet:///r/example/device/dev-a",
                    ability="er.weather",
                    subject="easynet:///r/example/device/dev-a",
                    nonce=bytes(range(1, 17)),
                    causal=None,
                    arguments=EasyRemoteArguments.from_json({"city": "Singapore"}),
                ),
                metadata={"trace_id": "er-1"},
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(
            identity_transport.seen_requests,
            [{"descriptor_ref": "er.weather"}],
        )

    def test_invocation_object_adapter_returns_wire_dict(
        self,
    ) -> None:
        identity_transport = MemoryIdentityTransport()
        identity_transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        client = AbilityInvocationClient(
            runtime=RuntimeClient(MemoryRuntimeTransport()),
            addressing=AddressingClient(identity_transport),
        )
        adapter = InvocationObjectAdapter(client)

        wire = adapter.to_wire_dict(
            EasyRemoteTuple(
                caller="easynet:///r/example/agent/alice.sdk",
                callee="easynet:///r/example/device/dev-a",
                ability=(
                    "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
                ),
                subject="easynet:///r/example/device/dev-a",
                nonce=bytes(range(1, 17)),
                causal=[
                    EasyRemoteCausalRef(
                        receipt_ura="easynet:///r/example/resource/agent.alice.sdk/invocation/parent-1/receipt",
                        receipt_hash=bytes.fromhex("aa" * 32),
                    )
                ],
                arguments=EasyRemoteArguments.from_bytes(
                    b"abc", "application/octet-stream"
                ),
            ),
            caller_signature=EasyRemoteSignature(),
            bidi_streams=(
                EasyRemoteStreamSpec(
                    stream_id=7,
                    content_type="application/octet-stream",
                    ordering="STRICT",
                    codec_params="chunk=4096",
                ),
            ),
        )

        self.assertEqual(wire["arguments_base64"], "YWJj")
        self.assertEqual(wire["content_type"], "application/octet-stream")
        self.assertEqual(
            wire["causal_context"],
            {
                "form": "list",
                "prior": [
                    {
                        "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/parent-1/receipt",
                        "receipt_hash_hex": "aa" * 32,
                    }
                ],
            },
        )
        self.assertEqual(
            wire["caller_signature"],
            {
                "algorithm": "ed25519",
                "signature_base64": "c2ln",
                "key_id_hint": "alice-key-1",
            },
        )
        self.assertEqual(
            wire["bidi_streams"],
            [
                {
                    "stream_id": 7,
                    "content_type": "application/octet-stream",
                    "ordering": "STRICT",
                    "codec_params": "chunk=4096",
                }
            ],
        )
        empty_codec_wire = adapter.to_wire_dict(
            EasyRemoteTuple(
                caller="easynet:///r/example/agent/alice.sdk",
                callee="easynet:///r/example/device/dev-a",
                ability=(
                    "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
                ),
                subject="easynet:///r/example/device/dev-a",
                nonce=bytes(range(1, 17)),
                causal=None,
                arguments=EasyRemoteArguments.from_json({"city": "Singapore"}),
            ),
            bidi_streams=(
                EasyRemoteStreamSpec(
                    stream_id=8,
                    content_type="text/pty",
                    ordering="STRICT",
                    codec_params="",
                ),
            ),
        )
        self.assertEqual(
            empty_codec_wire["bidi_streams"],
            [
                {
                    "stream_id": 8,
                    "content_type": "text/pty",
                    "ordering": "STRICT",
                }
            ],
        )
        self.assertNotIn("ability", wire)
        self.assertEqual(
            identity_transport.seen_requests,
            [
                {
                    "descriptor_ref": (
                        "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
                    )
                },
                {
                    "descriptor_ref": (
                        "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
                    )
                }
            ],
        )

    def test_invocation_object_adapter_dispatches_through_runtime_client(
        self,
    ) -> None:
        identity_transport = RejectBareDescriptorTransport()
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity_transport),
        )
        adapter = InvocationObjectAdapter(client)

        result = adapter.invoke(
            EasyRemoteTuple(
                caller="easynet:///r/example/agent/alice.sdk",
                callee="easynet:///r/example/device/dev-a",
                ability="easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
                subject="easynet:///r/example/device/dev-a",
                nonce=bytes(range(1, 17)),
                causal=None,
                arguments=EasyRemoteArguments.from_json({"city": "Singapore"}),
            )
        )

        self.assertTrue(result.ok)
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )

    def test_boundary_audit_rejects_raw_host_stream_codec(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "host.py").write_text(
                """
import hashlib

class _RollingHash:
    def fold(self, seq, frame):
        hashlib.sha256()
        return {"stream_item": frame, "seq": seq}
""",
                encoding="utf-8",
            )

            result = audit_consumer_boundary(root)

        self.assertFalse(result.ok)
        self.assertIn("raw_host_stream_codec", {item.rule for item in result.violations})

    def test_boundary_audit_rejects_raw_receipt_chain_semantics(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "receipts.py").write_text(
                """
def verify_continuity(previous, current):
    if current.prev_receipt_hash != previous.self_hash:
        raise RuntimeError("broken")
""",
                encoding="utf-8",
            )

            result = audit_consumer_boundary(root)

        self.assertFalse(result.ok)
        self.assertIn(
            "raw_receipt_chain_semantics", {item.rule for item in result.violations}
        )

    def test_easyremote_style_unary_invoke_uses_sdk_addressing_and_transport(
        self,
    ) -> None:
        identity_transport = MemoryIdentityTransport()
        identity_transport.identity_json = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"er.weather",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:er.weather",'
            b'"namespace":"er","local_name":"weather"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        identity_transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity_transport),
        )
        address = client.addressing.ability_address(
            "easynet:///r/example/ability/device.dev-a.er.weather"
        )

        result = client.invoke(
            AbilityCallRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                callee_ura=address.owner_ura,
                subject_ura=address.subject_ura,
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                ability_ura=address.ability_ura,
                args={"city": "Singapore"},
            )
        )

        self.assertTrue(result.ok)
        self.assertEqual(address.public_name, "er.weather")
        self.assertEqual(address.owner_kind, "device")
        self.assertEqual(address.namespace, "er")
        self.assertEqual(address.local_name, "weather")
        self.assertEqual(
            identity_transport.seen_requests,
            [
                {"ura": "easynet:///r/example/ability/device.dev-a.er.weather"},
                {
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                    "descriptor_version": "1.0.0",
                },
            ],
        )
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        self.assertEqual(runtime_transport.seen_draft["args"], {"city": "Singapore"})

    def test_package_level_ability_address_uses_default_sdk_facade(self) -> None:
        from test_environment import FakeRawCABI, _load_patch

        raw = FakeRawCABI()
        with _load_patch(raw):
            address = ability_address(
                "easynet:///r/example/ability/device.dev-a.observe.health",
                control_path="/tmp/control.json",
            )

        self.assertEqual(
            address.ability_ura,
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        self.assertEqual(address.subject_ura, address.ability_ura)
        self.assertEqual(address.owner_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(address.owner_kind, "device")
        self.assertEqual(address.public_name, "observe.health")
        self.assertEqual(raw.init_paths, ["/tmp/control.json"])
        self.assertEqual(raw.shutdown_handles, [42])
        self.assertEqual([entry[0] for entry in raw.identity_requests], ["project_ura"])

    def test_easyremote_style_target_invoke_uses_sdk_target_facade(self) -> None:
        identity_transport = MemoryIdentityTransport()
        identity_transport.identity_json = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"er.weather",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:er.weather",'
            b'"namespace":"er","local_name":"weather"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        identity_transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity_transport),
        )

        result = client.invoke_target(
            AbilityTargetRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
                args={"city": "Singapore"},
            )
        )

        self.assertTrue(result.ok)
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["callee_ura"],
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            runtime_transport.seen_draft["subject_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather",
        )
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )

    def test_easyremote_style_child_context_keeps_receipt_causal_anchor(self) -> None:
        identity_transport = MemoryIdentityTransport()
        identity_transport.identity_json = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"er.weather",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:er.weather",'
            b'"namespace":"er","local_name":"weather"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        identity_transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity_transport),
        )

        child = client.child_context(
            _parent_result_with_receipt(),
            ReceiptClient(MemoryReceiptTransport()),
            caller_ura="easynet:///r/example/agent/alice.sdk",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        )
        result = child.invoke_target(
            ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
            args={"city": "Singapore"},
        )

        self.assertTrue(result.ok)
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        self.assertEqual(
            runtime_transport.seen_draft["callee_ura"],
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            runtime_transport.seen_draft["causal_context"],
            {
                "form": "scalar",
                "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt",
                "receipt_hash_hex": (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
            },
        )
        self.assertEqual(runtime_transport.seen_draft["args"], {"city": "Singapore"})


def _parent_result_with_receipt() -> InvocationResult:
    return InvocationResult.from_json(
        json.dumps(
            {
                "ok": True,
                "tuple": {
                    "caller_ura": "easynet:///r/example/agent/alice.sdk",
                    "callee_ura": "easynet:///r/example/device/dev-a",
                    "descriptor_ref": (
                        "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
                    ),
                    "subject_ura": "easynet:///r/example/device/dev-a",
                    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                    "causal_context": {"form": "none"},
                    "content_type": "application/json",
                    "args": {},
                },
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "e30=",
                "output_json": {},
                "receipt": {
                    "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt",
                    "invocation_id": "inv-parent-1",
                    "self_hash_hex": (
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    ),
                },
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        )
    )


class RejectBareDescriptorTransport(MemoryIdentityTransport):
    def __init__(self) -> None:
        super().__init__()
        self.identity_json = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"er.weather",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:er.weather",'
            b'"namespace":"er","local_name":"weather"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        self.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        self.seen_request = request
        self.seen_requests.append(request)
        if request.get("descriptor_ref") == "er.weather":
            return b"{}"
        return self.descriptor_json


@dataclass(frozen=True)
class EasyRemoteTuple:
    caller: str
    callee: str
    ability: str
    subject: str
    nonce: bytes
    causal: object
    arguments: "EasyRemoteArguments"


@dataclass(frozen=True)
class EasyRemoteArguments:
    content_type: str
    json_value: object = None
    raw: bytes | None = None

    @classmethod
    def from_json(cls, value: object) -> "EasyRemoteArguments":
        return cls(content_type="application/json", json_value=value)

    @classmethod
    def from_bytes(cls, value: bytes, content_type: str) -> "EasyRemoteArguments":
        return cls(content_type=content_type, raw=value)

    @property
    def is_json(self) -> bool:
        return self.raw is None


@dataclass(frozen=True)
class EasyRemoteCausalRef:
    receipt_ura: str
    receipt_hash: bytes

    def to_wire(self) -> dict[str, object]:
        return {
            "receipt_ura": self.receipt_ura,
            "receipt_hash_hex": self.receipt_hash.hex(),
        }


@dataclass(frozen=True)
class EasyRemoteSignature:
    def to_wire(self) -> dict[str, object]:
        return {
            "algorithm": "ed25519",
            "signature_base64": "c2ln",
            "key_id_hint": "alice-key-1",
        }


@dataclass(frozen=True)
class EasyRemoteStreamSpec:
    stream_id: int
    content_type: str
    ordering: str
    codec_params: str

    def to_wire(self) -> dict[str, object]:
        return {
            "stream_id": self.stream_id,
            "content_type": self.content_type,
            "ordering": self.ordering,
            "codec_params": self.codec_params,
        }


if __name__ == "__main__":
    unittest.main()
