import json
import unittest
from dataclasses import fields

from easynet_sdk import (
    BidiStreamDescriptor,
    ErrorCode,
    InvocationBuilder,
    InvocationHandle,
    InvocationResult,
    InvocationSignature,
    PrepareOptions,
    PreparedInvocation,
    RuntimeClient,
    RuntimeReceipt,
    SDKError,
    Signer,
    is_code,
)

from test_signing import PREPARED_FIXTURE, signer_handle


class MemoryRuntimeTransport:
    def __init__(self) -> None:
        self.seen_draft: dict[str, object] | None = None
        self.seen_options: dict[str, object] | None = None
        self.seen_signed: dict[str, object] | None = None
        self.seen_streams: list[dict[str, object]] | None = None
        self.prepare_error: BaseException | None = None
        self.seen_await_id = 0
        self.seen_free_id = 0
        self.seen_cancel_reason = ""
        self.close_calls = 0
        self.close_error: BaseException | None = None
        self.handle_json = (
            b'{"handle_id":7,"state":"Submitted","terminal":false,'
            b'"events":[{"sequence":1,"kind":"submitted",'
            b'"state":"Submitted","terminal":false}],"result":null}'
        )

    def invoke(self, draft_json: bytes) -> bytes:
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen_draft,
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "eyJyZWFkeSI6dHJ1ZX0=",
                "output_json": {"ready": True},
                "selected_node_id": "node-a",
                "scheduling_reason": "direct",
                "elapsed_ms": 12,
                "terminal_receipt": {"receipt_id": "receipt-1"},
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def open_stream(self, draft_json: bytes):
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        from test_stream import MemoryStreamTransport

        return (
            MemoryStreamTransport(
                [
                    b'{"sequence":1,"kind":"terminal","state":"Completed","terminal":true}'
                ]
            ),
            b'{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}',
        )

    def open_bidi(self, draft_json: bytes, streams_json: bytes):
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        self.seen_streams = json.loads(streams_json.decode("utf-8"))
        from test_bidi import MemoryBidiTransport

        return (
            MemoryBidiTransport(),
            b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}',
        )

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        if self.prepare_error is not None:
            raise self.prepare_error
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return PREPARED_FIXTURE

    def submit_signed(self, signed_json: bytes) -> bytes:
        self.seen_signed = json.loads(signed_json.decode("utf-8"))
        return self.handle_json

    def await_handle(self, control) -> bytes:
        self.seen_await_id = control._adapter_handle_id()
        draft = self.seen_draft or complete_draft().to_json_dict()
        return json.dumps(
            {
                "ok": True,
                "tuple": draft,
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "e30=",
                "output_json": {},
                "elapsed_ms": 8,
                "terminal_receipt": None,
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def cancel_handle(self, control, reason: str) -> bytes:
        self.seen_cancel_reason = reason
        return (
            b'{"handle_id":7,"request_accepted":false,"deduplicated":true,'
            b'"cancelled":false,"state":"Completed","terminal":true}'
        )

    def handle_events(self, control) -> bytes:
        return (
            b'{"handle_id":7,"state":"Cancelled","terminal":true,'
            b'"events":[{"sequence":1,"kind":"submitted",'
            b'"state":"Submitted","terminal":false},{"sequence":2,'
            b'"kind":"cancelled","state":"Cancelled","terminal":true,'
            b'"reason":"client stop"}],"result":null}'
        )

    def free_handle(self, control) -> None:
        self.seen_free_id = control._adapter_handle_id()

    def close(self) -> None:
        self.close_calls += 1
        if self.close_error is not None:
            raise self.close_error


def complete_draft():
    return (
        InvocationBuilder()
        .with_caller_ura("easynet:///r/example/agent/alice.sdk")
        .with_callee_ura("easynet:///r/example/device/dev-a")
        .with_descriptor_ref(
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        )
        .with_subject_ura("easynet:///r/example/device/dev-a")
        .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
        .with_causal_context({"form": "none"})
        .with_json_args({})
        .with_content_type("application/json")
        .build()
    )


def signed_fixture():
    prepared = PreparedInvocation.from_json(PREPARED_FIXTURE)
    return prepared.sign_with_caller_signature(
        InvocationSignature(
            algorithm="ed25519",
            signature_base64="c2lnbmF0dXJl",
            key_id_hint="caller-key",
        )
    )


class RuntimeTests(unittest.TestCase):
    def test_invoke_returns_typed_result(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        result = client.invoke(complete_draft())

        self.assertTrue(result.ok)
        self.assertEqual(result.terminal_state, "Completed")
        self.assertEqual(
            result.tuple.caller_ura,
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(result.output_json, {"ready": True})
        assert transport.seen_draft is not None
        self.assertEqual(
            transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

    def test_invocation_result_projects_runtime_receipt_summary(self) -> None:
        result = InvocationResult.from_json(
            json.dumps(
                {
                    "ok": True,
                    "tuple": complete_draft().to_json_dict(),
                    "terminal_state": "Completed",
                    "output_content_type": "application/json",
                    "output_base64": "e30=",
                    "output_json": {},
                    "elapsed_ms": 8,
                    "terminal_receipt": {
                        "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/opaque/receipt",
                        "invocation_id": "inv-1",
                        "receipt_type": "terminal",
                        "state": "completed",
                        "index": 1,
                        "timestamp_unix_ms": 1783100000123,
                        "prev_receipt_hash_hex": "",
                        "self_hash_hex": "00" * 32,
                        "cleanup_complete": True,
                        "reason": "",
                        "child_invocation_id": "",
                        "extra": {"daemon": "axon"},
                    },
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            )
        )

        self.assertIsInstance(result.terminal_receipt_summary, RuntimeReceipt)
        assert result.terminal_receipt_summary is not None
        self.assertEqual(result.terminal_receipt_summary.invocation_id, "inv-1")
        self.assertTrue(result.terminal_receipt_summary.has_causal_anchor())
        self.assertEqual(
            result.terminal_receipt_summary.raw["extra"], {"daemon": "axon"}
        )
        assert result.terminal_receipt is not None
        self.assertEqual(result.terminal_receipt["invocation_id"], "inv-1")
        self.assertFalse(hasattr(result, "receipt"))
        self.assertFalse(hasattr(result, "receipt_summary"))

    def test_invocation_result_separates_admission_and_terminal_receipts(self) -> None:
        payload = {
            "ok": True,
            "tuple": complete_draft().to_json_dict(),
            "terminal_state": "Completed",
            "admission_receipt": {"index": 0, "state": "Admitted"},
            "terminal_receipt": {"index": 1, "state": "Completed"},
            "error": None,
        }
        result = InvocationResult.from_json(json.dumps(payload))

        self.assertEqual(result.admission_receipt, {"index": 0, "state": "Admitted"})
        assert result.terminal_receipt_summary is not None
        self.assertEqual(result.terminal_receipt_summary.index, 1)
        self.assertEqual(result.terminal_receipt, {"index": 1, "state": "Completed"})

        payload.pop("terminal_receipt")
        payload["receipt"] = {"index": 1, "state": "Completed"}
        legacy = InvocationResult.from_json(json.dumps(payload))
        self.assertIsNone(legacy.terminal_receipt)
        self.assertIsNone(legacy.terminal_receipt_summary)

    def test_invocation_result_preserves_stable_positional_field_prefix(self) -> None:
        self.assertEqual(
            [field.name for field in fields(InvocationResult)][:12],
            [
                "ok",
                "tuple",
                "terminal_state",
                "output_content_type",
                "output_base64",
                "output_json",
                "selected_node_id",
                "scheduling_reason",
                "elapsed_ms",
                "error",
                "admission_receipt",
                "admission_receipt_summary",
            ],
        )

    def test_invocation_result_rejects_malformed_runtime_receipt_fields(self) -> None:
        result = {
            "ok": True,
            "tuple": complete_draft().to_json_dict(),
            "terminal_state": "Completed",
            "output_content_type": "application/json",
            "output_base64": "e30=",
            "output_json": {},
            "elapsed_ms": 8,
            "terminal_receipt": {"cleanup_complete": "yes"},
            "error": None,
        }

        with self.assertRaises(SDKError) as caught:
            InvocationResult.from_json(json.dumps(result))

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_runtime_receipt_required_summary_validates_hashes(self) -> None:
        receipt = RuntimeReceipt.from_required_mapping(
            {
                "index": 1,
                "invocation_id": "inv-1",
                "receipt_type": 1,
                "state": 2,
                "timestamp_unix_ms": 1_700_000_000_000,
                "prev_receipt_hash_hex": "00" * 32,
                "self_hash_hex": "aa" * 32,
            }
        )

        self.assertEqual(receipt.receipt_type, "1")
        self.assertEqual(receipt.state, "2")
        self.assertEqual(receipt.prev_receipt_hash(), bytes(32))
        self.assertEqual(receipt.self_receipt_hash(), b"\xaa" * 32)

    def test_runtime_receipt_projects_complete_typed_facts(self) -> None:
        receipt = RuntimeReceipt.from_mapping(
            {
                "payload_base64": "cGF5bG9hZA==",
                "caller_binding": {"ura": "easynet:///r/local/agent/caller"},
                "callee_binding": {"ura": "easynet:///r/local/agent/callee"},
                "subject_binding": {
                    "ura": "easynet:///r/local/user/owner",
                    "profile": "owner",
                },
                "invocation_nonce_base64": "bm9uY2U=",
                "causal_binding_kind": "scalar",
                "causal_binding": {
                    "form": "scalar",
                    "receipt": {
                        "receipt_hash_hex": "77" * 32,
                        "receipt_ura": "easynet:///r/local/resource/subject/invocation/root/receipt",
                    },
                },
                "callee_signature": {
                    "algorithm": "ed25519",
                    "signature_base64": "c2ln",
                    "key_id_hint": "key-1",
                },
                "signer_binding": {"ura": "easynet:///r/local/agent/signer"},
                "host_attestation_base64": "YXR0ZXN0YXRpb24=",
                "authority_binding_kind": "delegation",
                "authority_binding": {
                    "kind": "delegation",
                    "issuer_ura": "easynet:///r/local/agent/issuer",
                    "subject_ura": "easynet:///r/local/user/owner",
                    "caller_ura": "easynet:///r/local/agent/caller",
                    "audience": "runtime",
                    "scopes": ["invoke"],
                    "issued_at_ms": 1,
                    "expires_at_ms": 2,
                    "signature_base64": "ZGVsZWdhdGlvbg==",
                },
                "ability_binding": "easynet:///r/local/ability/example.run",
                "failure": {
                    "code": "DENIED",
                    "message": "denied",
                    "retryable": False,
                    "stage": 2,
                    "security_class": 3,
                },
                "usage": {
                    "tokens_in": 10,
                    "tokens_out": 20,
                    "duration_ms": 30,
                    "external_calls": 1,
                },
                "subject_ref": {
                    "kind": 4,
                    "ura": "easynet:///r/local/resource/subject",
                },
                "descriptor_version": "1.0.0",
                "schema_hash_hex": "11" * 32,
                "impl_hash_hex": "22" * 32,
                "runtime_env": "native",
                "authority_proof": {
                    "proof_type": "admission",
                    "binding_kind": "delegated",
                    "proof_payload_base64": "cHJvb2Y=",
                    "proof_hash_hex": "33" * 32,
                    "issuer": {"ura": "easynet:///r/local/agent/issuer"},
                    "signature": {
                        "algorithm": "ed25519",
                        "signature_base64": "cHJvb2ZzaWc=",
                    },
                    "admission_hook": "policy.check",
                },
                "input_hash_hex": "44" * 32,
                "output_hash_hex": "55" * 32,
                "parent_receipts": [
                    {
                        "receipt_hash_hex": "66" * 32,
                        "receipt_ura": (
                            "easynet:///r/local/resource/subject/"
                            "invocation/parent/receipt"
                        ),
                    }
                ],
            }
        )

        assert receipt.caller_binding is not None
        assert receipt.subject_binding is not None
        assert receipt.callee_signature is not None
        assert receipt.failure is not None
        assert receipt.usage is not None
        assert receipt.subject_ref is not None
        assert receipt.authority_proof is not None
        assert receipt.authority_proof.issuer is not None
        self.assertEqual(receipt.caller_binding.ura, "easynet:///r/local/agent/caller")
        self.assertEqual(receipt.subject_binding.profile, "owner")
        self.assertEqual(receipt.callee_signature.algorithm, "ed25519")
        self.assertEqual(receipt.causal_binding_kind, "scalar")
        assert receipt.causal_binding is not None
        self.assertEqual(receipt.causal_binding["form"], "scalar")
        self.assertEqual(receipt.authority_binding_kind, "delegation")
        assert receipt.authority_binding is not None
        self.assertEqual(receipt.authority_binding["kind"], "delegation")
        self.assertEqual(receipt.authority_binding["scopes"], ["invoke"])
        self.assertEqual(receipt.failure.code, "DENIED")
        self.assertEqual(receipt.usage.tokens_out, 20)
        self.assertEqual(receipt.subject_ref.kind, 4)
        self.assertEqual(
            receipt.authority_proof.issuer.ura,
            "easynet:///r/local/agent/issuer",
        )
        self.assertEqual(receipt.parent_receipts[0].receipt_hash_hex, "66" * 32)

    def test_runtime_receipt_required_summary_rejects_malformed_hash(self) -> None:
        with self.assertRaises(SDKError) as caught:
            RuntimeReceipt.from_required_mapping(
                {
                    "index": 1,
                    "invocation_id": "inv-1",
                    "receipt_type": "completed",
                    "timestamp_unix_ms": 1_700_000_000_000,
                    "prev_receipt_hash_hex": "00" * 32,
                    "self_hash_hex": "aa",
                }
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_invoke_stream_opens_stream_handle(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        stream = client.invoke_stream(complete_draft())

        self.assertEqual(stream.stream_id, "stream-1")
        assert transport.seen_draft is not None
        self.assertEqual(
            transport.seen_draft["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )

    def test_open_bidi_opens_session(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        session = client.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1,
                    content_type="application/json",
                    ordering="ordered",
                ),
            ),
        )

        self.assertEqual(session.session_id, "bidi-1")
        assert transport.seen_draft is not None
        self.assertEqual(
            transport.seen_draft["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(
            transport.seen_streams,
            [
                {
                    "content_type": "application/json",
                    "ordering": "ordered",
                    "stream_id": 1,
                }
            ],
        )

    def test_prepare_delegates_to_transport(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        prepared, material = client.prepare(
            complete_draft(), PrepareOptions(expires_in_ms=60000)
        )

        self.assertFalse(prepared.submit_ready())
        self.assertTrue(material.canonical_bytes_base64)
        assert transport.seen_draft is not None
        self.assertEqual(
            transport.seen_draft["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(transport.seen_options, {"expires_in_ms": 60000})

    def test_prepare_signing_material_uses_stateless_transport_contract(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        material = client.prepare_signing_material(
            complete_draft(),
            PrepareOptions(expires_in_ms=60000, signer_id="browser-key-1"),
        )

        self.assertTrue(material.canonical_bytes_base64)
        self.assertEqual(
            transport.seen_options,
            {
                "expires_in_ms": 60000,
                "material_only": True,
                "signer_id": "browser-key-1",
            },
        )

    def test_prepare_and_sign_returns_inspectable_signed_envelope(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        signed, material = client.prepare_and_sign(
            complete_draft(),
            signer,
            PrepareOptions(expires_in_ms=60000),
        )

        self.assertTrue(signed.submit_ready())
        self.assertTrue(material.canonical_bytes_base64)
        self.assertIsNone(transport.seen_signed)
        self.assertEqual(transport.seen_options, {"expires_in_ms": 60000})

        handle = client.submit_signed(signed)

        self.assertEqual(handle.control_capability()._adapter_handle_id(), 7)
        assert transport.seen_signed is not None
        self.assertEqual(transport.seen_signed["signer_id"], "signer-alice-key-1")

    def test_bound_object_graph_delegates_full_lifecycle(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )
        builder = (
            client.new_invocation()
            .with_caller_ura("easynet:///r/example/agent/alice.sdk")
            .with_callee_ura("easynet:///r/example/device/dev-a")
            .with_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
            .with_subject_ura("easynet:///r/example/device/dev-a")
            .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
            .with_causal_context({"form": "none"})
            .with_json_args({})
            .with_content_type("application/json")
        )

        prepared, material = builder.prepare(PrepareOptions(expires_in_ms=60000))
        signed = prepared.sign(signer)
        handle = signed.submit()
        result = handle.await_result()
        cancelled = handle.cancel("client stop")
        refreshed = handle.refresh_events()
        handle.close()

        self.assertTrue(material.canonical_bytes_base64)
        self.assertTrue(signed.submit_ready())
        self.assertEqual(handle.control_capability()._adapter_handle_id(), 7)
        self.assertTrue(result.ok)
        self.assertFalse(cancelled.request_accepted)
        self.assertTrue(cancelled.deduplicated)
        self.assertFalse(cancelled.cancelled)
        self.assertTrue(cancelled.terminal)
        self.assertTrue(refreshed.terminal)
        self.assertEqual(transport.seen_options, {"expires_in_ms": 60000})
        self.assertEqual(transport.seen_await_id, 7)
        self.assertEqual(transport.seen_cancel_reason, "client stop")
        self.assertEqual(transport.seen_free_id, 7)
        assert transport.seen_signed is not None
        self.assertEqual(transport.seen_signed["signer_id"], "signer-alice-key-1")
        with self.assertRaises(SDKError) as caught:
            builder.inspect()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))

    def test_bound_draft_invokes_streams_and_opens_bidi(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        draft = (
            client.new_invocation()
            .with_caller_ura("easynet:///r/example/agent/alice.sdk")
            .with_callee_ura("easynet:///r/example/device/dev-a")
            .with_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
            .with_subject_ura("easynet:///r/example/device/dev-a")
            .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
            .with_causal_context({"form": "none"})
            .with_json_args({})
            .with_content_type("application/json")
            .inspect()
        )

        result = draft.invoke()
        stream = draft.open_stream()
        event = stream.next()
        stream.close()
        bidi = draft.open_bidi(
            (
                BidiStreamDescriptor(
                    stream_id=1,
                    content_type="application/json",
                    ordering="ordered",
                ),
            )
        )
        bidi.cancel("test cleanup")
        bidi.close()

        self.assertTrue(result.ok)
        self.assertTrue(event.terminal)
        self.assertEqual(bidi.session_id, "bidi-1")
        self.assertEqual(
            transport.seen_streams,
            [
                {
                    "content_type": "application/json",
                    "ordering": "ordered",
                    "stream_id": 1,
                }
            ],
        )

    def test_unbound_lifecycle_objects_reject_object_methods(self) -> None:
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )
        signed = PreparedInvocation.from_json(PREPARED_FIXTURE).sign(signer)
        handle = InvocationHandle.from_json(
            b'{"handle_id":7,"state":"Submitted","terminal":false}'
        )

        for action in (complete_draft().prepare, signed.submit, handle.await_result):
            with self.subTest(action=action):
                with self.assertRaises(SDKError) as caught:
                    action()
                self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))

    def test_prepare_builder_consumes_only_after_success(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        builder = (
            InvocationBuilder()
            .with_caller_ura("easynet:///r/example/agent/alice.sdk")
            .with_callee_ura("easynet:///r/example/device/dev-a")
            .with_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
            .with_subject_ura("easynet:///r/example/device/dev-a")
            .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
            .with_causal_context({"form": "none"})
            .with_json_args({})
            .with_content_type("application/json")
        )

        prepared, material = client.prepare_builder(builder)

        self.assertEqual(prepared.prepared_id, "prepared-example-1")
        self.assertTrue(material.canonical_bytes_base64)
        with self.assertRaises(SDKError) as caught:
            builder.inspect()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))

    def test_prepare_builder_keeps_builder_on_failure(self) -> None:
        transport = MemoryRuntimeTransport()
        transport.prepare_error = RuntimeError("daemon unavailable")
        client = RuntimeClient(transport)
        builder = (
            InvocationBuilder()
            .with_caller_ura("easynet:///r/example/agent/alice.sdk")
            .with_callee_ura("easynet:///r/example/device/dev-a")
            .with_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
            .with_subject_ura("easynet:///r/example/device/dev-a")
            .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
            .with_causal_context({"form": "none"})
            .with_json_args({})
            .with_content_type("application/json")
        )

        with self.assertRaises(SDKError) as caught:
            client.prepare_builder(builder)
        self.assertTrue(is_code(caught.exception, ErrorCode.ROUTE_UNAVAILABLE))

        builder.inspect()

    def test_submit_signed_preserves_signature(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        handle = client.submit_signed(signed_fixture())

        self.assertEqual(handle.control_capability()._adapter_handle_id(), 7)
        self.assertEqual(handle.state, "Submitted")
        self.assertFalse(handle.terminal)
        self.assertEqual(handle.events[0].sequence, 1)
        assert transport.seen_signed is not None
        self.assertEqual(
            transport.seen_signed["signature"]["signature_base64"],
            "c2lnbmF0dXJl",
        )

    def test_public_handle_json_does_not_grant_control_authority(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        handle = InvocationHandle.from_json(
            b'{"handle_id":7,"state":"Submitted","terminal":false,'
            b'"events":[],"result":null}'
        )

        self.assertEqual(handle.state, "Submitted")
        with self.assertRaises(SDKError) as caught:
            client.await_result(handle)
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.seen_await_id, 0)

    def test_handle_observation_delegates_to_transport(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        handle = client.submit_signed(signed_fixture())

        result = client.await_result(handle)
        cancelled = client.cancel(handle, "client stop")
        events = client.events(handle)
        client.close_handle(handle)

        self.assertTrue(result.ok)
        self.assertEqual(transport.seen_await_id, 7)
        self.assertFalse(cancelled.request_accepted)
        self.assertTrue(cancelled.deduplicated)
        self.assertFalse(cancelled.cancelled)
        self.assertTrue(cancelled.terminal)
        self.assertEqual(cancelled.state, "Completed")
        self.assertEqual(transport.seen_cancel_reason, "client stop")
        self.assertTrue(events.terminal)
        self.assertEqual(len(events.events), 2)
        self.assertEqual(events.events[1].reason, "client stop")
        self.assertEqual(transport.seen_free_id, 7)

    def test_prepare_wraps_transport_failure(self) -> None:
        transport = MemoryRuntimeTransport()
        transport.prepare_error = RuntimeError("daemon unavailable")
        client = RuntimeClient(transport)

        with self.assertRaises(SDKError) as caught:
            client.prepare(complete_draft())

        self.assertTrue(is_code(caught.exception, ErrorCode.ROUTE_UNAVAILABLE))
        self.assertIsInstance(caught.exception.cause, RuntimeError)

    def test_submit_rejects_malformed_handle(self) -> None:
        transport = MemoryRuntimeTransport()
        transport.handle_json = b'{"state":"Submitted"}'
        client = RuntimeClient(transport)

        with self.assertRaises(SDKError) as caught:
            client.submit_signed(signed_fixture())

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_result_rejects_inconsistent_failure(self) -> None:
        result = {
            "ok": False,
            "tuple": complete_draft().to_json_dict(),
            "terminal_state": "Failed",
            "output_content_type": "application/json",
            "output_base64": "",
            "output_json": None,
            "elapsed_ms": 3,
            "receipt": None,
            "error": None,
        }

        with self.assertRaises(SDKError) as caught:
            InvocationResult.from_json(json.dumps(result))

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.invoke(complete_draft())
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_draft)

    def test_close_failure_is_terminal(self) -> None:
        transport = MemoryRuntimeTransport()
        transport.close_error = RuntimeError("close failed")
        client = RuntimeClient(transport)

        with self.assertRaises(SDKError) as close_caught:
            client.close()
        self.assertTrue(is_code(close_caught.exception, ErrorCode.ROUTE_UNAVAILABLE))
        self.assertIsInstance(close_caught.exception.cause, RuntimeError)

        with self.assertRaises(SDKError) as invoke_caught:
            client.invoke(complete_draft())
        self.assertTrue(is_code(invoke_caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_draft)


if __name__ == "__main__":
    unittest.main()
