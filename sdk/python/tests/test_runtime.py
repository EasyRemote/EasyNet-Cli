import base64
import hashlib
import json
import unittest
from dataclasses import fields, replace
from types import MappingProxyType

from axon_sdk.invocation import AuthorityBinding, authority_binding_proof_hash
from easynet_sdk import (
    BidiStreamDescriptor,
    ErrorCode,
    InvocationBuilder,
    InvocationHandle,
    InvocationLifecycleState,
    InvocationResult,
    InvocationSignature,
    PrepareOptions,
    PreparedInvocation,
    RuntimeClient,
    RuntimeRecoveryReport,
    RuntimeRecoveryRequest,
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
        self.seen_descriptor_request: dict[str, object] | None = None
        self.seen_recovery_request: dict[str, object] | None = None
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
        admission, terminal = canonical_runtime_receipt_pair("inv-runtime-1")
        terminal["receipt_id"] = "receipt-1"
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen_draft,
                "invocation_id": "inv-runtime-1",
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "eyJyZWFkeSI6dHJ1ZX0=",
                "output_json": {"ready": True},
                "elapsed_ms": 12,
                "admission_receipt": admission,
                "terminal_receipt": terminal,
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
        self.seen_descriptor_request = json.loads(request_json.decode("utf-8"))
        return b'{"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"}'

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
        if self.seen_options.get("material_only") is True:
            material_only = json.loads(PREPARED_FIXTURE.decode("utf-8"))
            material_only.pop("prepared_id", None)
            return json.dumps(material_only, separators=(",", ":")).encode("utf-8")
        return PREPARED_FIXTURE

    def submit_signed(self, signed_json: bytes) -> bytes:
        self.seen_signed = json.loads(signed_json.decode("utf-8"))
        return self.handle_json

    def recover(self, request_json: bytes) -> bytes:
        self.seen_recovery_request = json.loads(request_json.decode("utf-8"))
        return json.dumps(
            {
                "bounded_scan": True,
                "cleanup_complete": True,
                "events": [
                    {
                        "invocation_id": "inv-orphan",
                        "kind": "orphan_reaped",
                        "reason": "host restart",
                        "receipt_ura": "easynet:///r/example/resource/agent.alice/invocation/inv-orphan/receipt",
                        "sequence": 1,
                        "state": "cancelled",
                        "terminal": True,
                    }
                ],
                "reaped_orphans": 1,
                "recovered_invocations": 2,
                "recovery_id": "recovery-1",
                "replayed_terminal_receipts": 1,
                "state": "runtime_started",
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def await_handle(self, control) -> bytes:
        self.seen_await_id = control._adapter_handle_id()
        draft = self.seen_draft or complete_draft().to_json_dict()
        admission, terminal = canonical_runtime_receipt_pair("inv-await-1")
        return json.dumps(
            {
                "ok": True,
                "tuple": draft,
                "invocation_id": "inv-await-1",
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "e30=",
                "output_json": {},
                "elapsed_ms": 8,
                "admission_receipt": admission,
                "terminal_receipt": terminal,
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


def canonical_runtime_receipt(
    invocation_id: str,
    receipt_type: str,
    state: str,
    index: int,
) -> dict[str, object]:
    proof_payload = b"canonical-runtime-test-proof"
    return {
        "receipt_ura": (
            "easynet:///r/example/resource/runtime/"
            f"invocation/{invocation_id}/receipt/{index}"
        ),
        "invocation_id": invocation_id,
        "receipt_type": receipt_type,
        "state": state,
        "index": index,
        "timestamp_unix_ms": 1_783_100_000_000 + index,
        "prev_receipt_hash_hex": "00" * 32,
        "self_hash_hex": f"{index + 1:064x}",
        "payload_base64": "",
        "payload_content_type": "application/json",
        "cleanup_complete": state.lower() != "admitted",
        "caller_binding": {
            "ura": "easynet:///r/example/agent/alice.sdk",
            "profile": "axon-strict-v2",
        },
        "callee_binding": {
            "ura": "easynet:///r/example/device/dev-a",
            "profile": "axon-strict-v2",
        },
        "subject_binding": {
            "ura": "easynet:///r/example/device/dev-a",
            "profile": "axon-strict-v2",
        },
        "invocation_nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
        "causal_binding_kind": "none",
        "causal_binding": {"form": "none"},
        "callee_signature": {
            "algorithm": "ed25519",
            "signature_base64": base64.b64encode(bytes([0x71]) * 64).decode(),
        },
        "signer_binding": {
            "ura": "easynet:///r/example/device/dev-a",
            "profile": "axon-strict-v2",
        },
        "authority_binding_kind": "self",
        "authority_binding": {
            "kind": "self",
            "principal_ura": "easynet:///r/example/device/dev-a",
        },
        "ability_binding": (
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        ),
        "host_attestation_base64": "",
        "usage": {},
        "subject_ref": {
            "kind": 1,
            "ura": "easynet:///r/example/device/dev-a",
            "profile": "axon-strict-v2",
        },
        "descriptor_version": "1.0.0",
        "schema_hash_hex": "11" * 32,
        "impl_hash_hex": "22" * 32,
        "runtime_env": "python-test",
        "authority_proof": {
            "proof_type": "self",
            "binding_kind": "self",
            "binding": {
                "kind": "self",
                "principal_ura": "easynet:///r/example/device/dev-a",
            },
            "proof_payload_base64": base64.b64encode(proof_payload).decode(),
            "proof_hash_hex": hashlib.sha256(proof_payload).hexdigest(),
            "issuer": {
                "ura": "easynet:///r/example/device/dev-a",
                "profile": "axon-strict-v2",
            },
            "signature": {
                "algorithm": "ed25519",
                "signature_base64": base64.b64encode(bytes([0x72]) * 64).decode(),
            },
            "admission_hook": "test.runtime.admission",
        },
        "input_hash_hex": "33" * 32,
        "output_hash_hex": "44" * 32,
        "parent_receipts": [],
    }


def _runtime_u32(value: int) -> bytes:
    return value.to_bytes(4, byteorder="big", signed=False)


def _runtime_length_prefixed_text(value: str) -> bytes:
    encoded = value.encode()
    return _runtime_u32(len(encoded)) + encoded


def session_authority_binding_hash(binding: dict[str, object]) -> str:
    scopes = list(binding["scopes"])
    audiences = list(binding["audiences"])
    signature = base64.b64decode(str(binding["signature_base64"]))
    canonical = b"".join(
        [
            bytes([0x05]),
            _runtime_length_prefixed_text(str(binding["issuer_ura"])),
            _runtime_length_prefixed_text(str(binding["subject_ura"])),
            _runtime_length_prefixed_text(str(binding["session_id"])),
            _runtime_u32(len(scopes)),
            *( _runtime_length_prefixed_text(str(scope)) for scope in scopes ),
            _runtime_u32(len(audiences)),
            *( _runtime_length_prefixed_text(str(audience)) for audience in audiences ),
            int(binding["issued_at_ms"]).to_bytes(8, byteorder="big", signed=True),
            int(binding["expires_at_ms"]).to_bytes(8, byteorder="big", signed=True),
            _runtime_u32(len(signature)),
            signature,
        ]
    )
    return hashlib.sha256(canonical).hexdigest()


def canonical_runtime_receipt_pair(
    invocation_id: str,
    terminal_state: str = "Completed",
) -> tuple[dict[str, object], dict[str, object]]:
    state = InvocationLifecycleState.from_wire_name(terminal_state)
    if not state.is_terminal:
        raise ValueError(f"unsupported terminal fixture state: {terminal_state}")
    terminal_type = state.name.lower()
    admission = canonical_runtime_receipt(invocation_id, "admitted", "Admitted", 0)
    terminal = canonical_runtime_receipt(
        invocation_id,
        terminal_type,
        terminal_state,
        1,
    )
    terminal["prev_receipt_hash_hex"] = admission["self_hash_hex"]
    return admission, terminal


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
    def test_descriptor_resolution_rejects_blank_call_mode(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        with self.assertRaises(SDKError) as raised:
            client.resolve_descriptor_ref(
                callee_ura="easynet:///r/example/device/dev-a",
                ability="observe.health",
                call_mode="  ",
            )

        self.assertEqual(raised.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertIn("call_mode is required", raised.exception.message)
        self.assertIsNone(transport.seen_descriptor_request)

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

    def test_invocation_result_derives_json_output_from_canonical_payload(self) -> None:
        admission, terminal = canonical_runtime_receipt_pair("inv-derived-output")
        result = InvocationResult.from_json(
            json.dumps(
                {
                    "ok": True,
                    "tuple": complete_draft().to_json_dict(),
                    "invocation_id": "inv-derived-output",
                    "terminal_state": "Completed",
                    "output_content_type": "application/json; charset=utf-8",
                    "output_base64": "eyJyZWFkeSI6dHJ1ZX0=",
                    "output_json": None,
                    "elapsed_ms": 1,
                    "admission_receipt": admission,
                    "terminal_receipt": terminal,
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            )
        )

        self.assertEqual(result.output_json, {"ready": True})

    def test_invocation_result_projects_runtime_receipt_summary(self) -> None:
        admission, terminal = canonical_runtime_receipt_pair("inv-1")
        terminal.update(
            {
                "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/opaque/receipt",
                "self_hash_hex": "aa" * 32,
                "extra": {"daemon": "axon"},
            }
        )
        result = InvocationResult.from_json(
            json.dumps(
                {
                    "ok": True,
                    "tuple": complete_draft().to_json_dict(),
                    "invocation_id": "inv-1",
                    "terminal_state": "Completed",
                    "output_content_type": "application/json",
                    "output_base64": "e30=",
                    "output_json": {},
                    "elapsed_ms": 8,
                    "admission_receipt": admission,
                    "terminal_receipt": terminal,
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
        admission, terminal = canonical_runtime_receipt_pair("inv-1")
        payload = {
            "ok": True,
            "tuple": complete_draft().to_json_dict(),
            "invocation_id": "inv-1",
            "terminal_state": "Completed",
            "admission_receipt": admission,
            "terminal_receipt": terminal,
            "error": None,
        }
        result = InvocationResult.from_json(json.dumps(payload))

        self.assertEqual(result.admission_receipt, admission)
        assert result.terminal_receipt_summary is not None
        self.assertEqual(result.terminal_receipt_summary.index, 1)
        self.assertEqual(result.terminal_receipt, terminal)

        payload.pop("terminal_receipt")
        payload["receipt"] = {"index": 1, "state": "Completed"}
        with self.assertRaises(SDKError) as caught:
            InvocationResult.from_json(json.dumps(payload))

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_invocation_result_allows_only_receipt_free_pre_admission_failure(
        self,
    ) -> None:
        for stage in (
            "global_admission",
            "caller_authentication",
            "authority_validation",
            "bootstrap_authorization",
            "quota",
            "ability_resolution",
            "ability_policy",
            "request_validation",
        ):
            with self.subTest(stage=stage):
                result = InvocationResult.from_json(
                    json.dumps(
                        {
                            "ok": False,
                            "tuple": complete_draft().to_json_dict(),
                            "terminal_state": "Failed",
                            "admission_receipt": None,
                            "terminal_receipt": None,
                            "error": {
                                "code": "ADMISSION_DENIED",
                                "stage": stage,
                                "message": "rejected before admission",
                                "retryable": False,
                            },
                        }
                    )
                )
                self.assertFalse(result.ok)
                self.assertIsNone(result.admission_receipt_summary)
                self.assertIsNone(result.terminal_receipt_summary)

    def test_invocation_result_rejects_conflicting_receipt_topology(self) -> None:
        mutations = (
            ("admission state", "state", "Running"),
            ("admission receipt type", "receipt_type", "completed"),
            ("admission receipt type case", "receipt_type", "Admitted"),
            ("terminal state", "state", "Failed"),
            ("terminal receipt type", "receipt_type", "failed"),
            ("terminal receipt type case", "receipt_type", "Completed"),
            ("terminal index", "index", 0),
            ("terminal cleanup", "cleanup_complete", False),
            ("terminal timestamp", "timestamp_unix_ms", 0),
            ("invocation binding", "invocation_id", "other"),
            (
                "caller binding",
                "caller_binding",
                {"ura": "easynet:///r/example/agent/other"},
            ),
            (
                "host attestation",
                "host_attestation_base64",
                base64.b64encode(b"other-host").decode(),
            ),
        )
        for name, field_name, value in mutations:
            admission, terminal = canonical_runtime_receipt_pair("inv-1")
            target = admission if name.startswith("admission") else terminal
            target[field_name] = value
            with self.subTest(case=name):
                with self.assertRaises(SDKError) as caught:
                    InvocationResult.from_json(
                        json.dumps(
                            {
                                "ok": True,
                                "tuple": complete_draft().to_json_dict(),
                                "invocation_id": "inv-1",
                                "terminal_state": "Completed",
                                "admission_receipt": admission,
                                "terminal_receipt": terminal,
                                "error": None,
                            }
                        )
                    )
                self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        admission, terminal = canonical_runtime_receipt_pair("inv-1")
        with self.assertRaises(SDKError) as caught:
            InvocationResult.from_json(
                json.dumps(
                    {
                        "ok": True,
                        "tuple": complete_draft().to_json_dict(),
                        "invocation_id": "inv-1",
                        "terminal_state": " Completed ",
                        "admission_receipt": admission,
                        "terminal_receipt": terminal,
                        "error": None,
                    }
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_invocation_result_accepts_non_adjacent_finalization_checkpoints(
        self,
    ) -> None:
        admission, terminal = canonical_runtime_receipt_pair("inv-checkpoints")
        admission["index"] = 1
        admission["prev_receipt_hash_hex"] = "aa" * 32
        terminal["index"] = 7
        terminal["prev_receipt_hash_hex"] = "bb" * 32

        result = InvocationResult.from_json(
            json.dumps(
                {
                    "ok": True,
                    "tuple": complete_draft().to_json_dict(),
                    "invocation_id": "inv-checkpoints",
                    "terminal_state": "Completed",
                    "admission_receipt": admission,
                    "terminal_receipt": terminal,
                    "error": None,
                }
            )
        )
        assert result.admission_receipt_summary is not None
        assert result.terminal_receipt_summary is not None
        self.assertEqual(result.admission_receipt_summary.index, 1)
        self.assertEqual(result.terminal_receipt_summary.index, 7)

    def test_runtime_receipt_rejects_malformed_canonical_proof_facts(self) -> None:
        cases = {
            "invalid nonce": (
                "invocation_nonce_base64",
                "not-base64",
            ),
            "missing parent binding": ("parent_receipts", None),
            "malformed parent hash": (
                "parent_receipts",
                [
                    {
                        "receipt_hash_hex": "aa",
                        "receipt_ura": "easynet:///r/example/resource/parent",
                    }
                ],
            ),
        }
        for name, (field_name, value) in cases.items():
            receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
            receipt[field_name] = value
            with self.subTest(case=name):
                with self.assertRaises(SDKError):
                    RuntimeReceipt.from_mapping(receipt)

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        proof = receipt["authority_proof"]
        assert isinstance(proof, dict)
        proof["proof_hash_hex"] = "ff" * 32
        with self.assertRaises(SDKError, msg="mismatched proof hash"):
            RuntimeReceipt.from_mapping(receipt)

        for missing_field in (
            "payload_base64",
            "payload_content_type",
            "host_attestation_base64",
            "usage",
        ):
            receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
            receipt.pop(missing_field)
            with self.subTest(missing_field=missing_field):
                with self.assertRaises(SDKError) as raised:
                    RuntimeReceipt.from_mapping(receipt)
                self.assertIn(
                    f"runtime receipt summary is missing runtime_receipt.{missing_field}",
                    raised.exception.message,
                )

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        authority = receipt["authority_binding"]
        assert isinstance(authority, dict)
        authority["legacy_authority"] = "compat-carrier"
        proof = receipt["authority_proof"]
        assert isinstance(proof, dict)
        proof["binding"] = dict(authority)
        with self.assertRaises(SDKError) as raised:
            RuntimeReceipt.from_mapping(receipt)
        self.assertIn(
            "authority_binding contains noncanonical field legacy_authority",
            raised.exception.message,
        )

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        proof = receipt["authority_proof"]
        assert isinstance(proof, dict)
        proof["legacy_proof_fact"] = "compat-carrier"
        with self.assertRaises(SDKError) as raised:
            RuntimeReceipt.from_mapping(receipt)
        self.assertIn(
            "authority_proof contains noncanonical field legacy_proof_fact",
            raised.exception.message,
        )

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        proof = receipt["authority_proof"]
        assert isinstance(proof, dict)
        proof.pop("proof_payload_base64")
        with self.assertRaises(SDKError) as raised:
            RuntimeReceipt.from_mapping(receipt)
        self.assertIn("proof_payload_base64", raised.exception.message)

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        proof = receipt["authority_proof"]
        assert isinstance(proof, dict)
        issuer = proof["issuer"]
        assert isinstance(issuer, dict)
        proof["issuer"] = {**issuer, "legacy_profile": "opaque"}
        with self.assertRaises(SDKError) as raised:
            RuntimeReceipt.from_mapping(receipt)
        self.assertIn(
            "authority_proof.issuer contains noncanonical field legacy_profile",
            raised.exception.message,
        )

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        proof = receipt["authority_proof"]
        assert isinstance(proof, dict)
        proof["binding_kind"] = "delegation"
        with self.assertRaises(SDKError, msg="mismatched authority kind"):
            RuntimeReceipt.from_mapping(receipt)

        mutations = (
            ("missing proof binding", lambda value: value.pop("binding")),
            (
                "mismatched proof binding",
                lambda value: value.update(
                    {
                        "binding": {
                            "kind": "self",
                            "principal_ura": "easynet:///r/example/device/other",
                        }
                    }
                ),
            ),
            ("missing admission hook", lambda value: value.pop("admission_hook")),
            (
                "issuer does not match callee",
                lambda value: value.update(
                    {
                        "issuer": {
                            "ura": "easynet:///r/example/device/other",
                            "profile": "axon-strict-v2",
                        }
                    }
                ),
            ),
        )
        for name, mutate in mutations:
            receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
            proof = receipt["authority_proof"]
            assert isinstance(proof, dict)
            mutate(proof)
            with self.subTest(case=name):
                with self.assertRaises(SDKError):
                    RuntimeReceipt.from_mapping(receipt)

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        caller = receipt["caller_binding"]
        assert isinstance(caller, dict)
        caller["profile"] = "test"
        with self.assertRaises(SDKError, msg="invalid identity profile"):
            RuntimeReceipt.from_mapping(receipt)

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        receipt["signer_binding"] = {
            "ura": "easynet:///r/example/device/runtime-host",
            "profile": "axon-strict-v2",
        }
        with self.assertRaises(SDKError, msg="hosted signer without attestation"):
            RuntimeReceipt.from_mapping(receipt)

        receipt = canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        receipt["host_attestation_base64"] = base64.b64encode(
            bytes([0x73]) * 64
        ).decode()
        with self.assertRaises(SDKError, msg="self signer with attestation"):
            RuntimeReceipt.from_mapping(receipt)

        canonical = RuntimeReceipt.from_mapping(
            canonical_runtime_receipt("inv-1", "completed", "Completed", 1)
        )
        with self.assertRaises(SDKError, msg="raw projection mismatch"):
            replace(canonical, raw={})

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
                "elapsed_ms",
                "error",
                "admission_receipt",
                "admission_receipt_summary",
                "terminal_receipt",
                "terminal_receipt_summary",
            ],
        )

    def test_invocation_result_direct_constructor_enforces_receipt_topology(
        self,
    ) -> None:
        with self.assertRaises(SDKError):
            InvocationResult(
                ok=True,
                tuple=complete_draft(),
                terminal_state="Completed",
            )

        admission, terminal = canonical_runtime_receipt_pair("inv-direct")
        result = InvocationResult(
            ok=True,
            tuple=complete_draft(),
            terminal_state="Completed",
            admission_receipt=admission,
            admission_receipt_summary=RuntimeReceipt.from_mapping(admission),
            terminal_receipt=terminal,
            terminal_receipt_summary=RuntimeReceipt.from_mapping(terminal),
        )
        self.assertTrue(result.ok)
        self.assertIs(
            result.lifecycle_state,
            InvocationLifecycleState.COMPLETED,
        )

        with self.assertRaises(SDKError):
            InvocationResult(
                ok=True,
                tuple=complete_draft(),
                terminal_state="Completed",
                admission_receipt={},
                admission_receipt_summary=RuntimeReceipt.from_mapping(admission),
                terminal_receipt=terminal,
                terminal_receipt_summary=RuntimeReceipt.from_mapping(terminal),
            )

    def test_invocation_result_rejects_malformed_runtime_receipt_fields(self) -> None:
        admission, terminal = canonical_runtime_receipt_pair("inv-1")
        terminal["cleanup_complete"] = "yes"
        result = {
            "ok": True,
            "tuple": complete_draft().to_json_dict(),
            "invocation_id": "inv-1",
            "terminal_state": "Completed",
            "output_content_type": "application/json",
            "output_base64": "e30=",
            "output_json": {},
            "elapsed_ms": 8,
            "admission_receipt": admission,
            "terminal_receipt": terminal,
            "error": None,
        }

        with self.assertRaises(SDKError) as caught:
            InvocationResult.from_json(json.dumps(result))

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_runtime_receipt_proof_facts_required(self) -> None:
        complete = canonical_runtime_receipt("inv-1", "completed", "completed", 1)
        complete["self_hash_hex"] = "aa" * 32
        receipt = RuntimeReceipt.from_required_mapping(complete)

        self.assertEqual(receipt.receipt_type, "completed")
        self.assertEqual(receipt.state, "completed")
        self.assertEqual(receipt.prev_receipt_hash(), bytes(32))
        self.assertEqual(receipt.self_receipt_hash(), b"\xaa" * 32)

        with self.assertRaises(SDKError) as caught:
            incomplete = canonical_runtime_receipt("inv-1", "completed", "completed", 1)
            incomplete.pop("authority_proof")
            RuntimeReceipt.from_required_mapping(incomplete)

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_runtime_receipt_projection_is_deep_immutable(self) -> None:
        complete = canonical_runtime_receipt(
            "inv-proof-immutable", "completed", "completed", 1
        )
        receipt = RuntimeReceipt.from_mapping(complete)

        authority_binding = complete["authority_binding"]
        authority_proof = complete["authority_proof"]
        assert isinstance(authority_binding, dict)
        assert isinstance(authority_proof, dict)
        proof_binding = authority_proof["binding"]
        assert isinstance(proof_binding, dict)
        authority_binding["legacy_authority"] = "post-validation-mutation"
        proof_binding["legacy_proof_fact"] = "post-validation-mutation"

        self.assertIsInstance(receipt.raw, MappingProxyType)
        self.assertIsInstance(receipt.raw["authority_binding"], MappingProxyType)
        self.assertIsInstance(receipt.raw["authority_proof"], MappingProxyType)
        proof = receipt.raw["authority_proof"]
        assert isinstance(proof, MappingProxyType)
        self.assertIsInstance(proof["binding"], MappingProxyType)

        first_projection = receipt.to_json_dict()
        projected_authority = first_projection["authority_binding"]
        projected_proof = first_projection["authority_proof"]
        projected_parents = first_projection["parent_receipts"]
        assert isinstance(projected_authority, dict)
        assert isinstance(projected_proof, dict)
        assert isinstance(projected_proof["binding"], dict)
        self.assertIsInstance(projected_parents, list)
        self.assertNotIn("legacy_authority", projected_authority)
        self.assertNotIn("legacy_proof_fact", projected_proof["binding"])

        projected_authority["legacy_authority"] = "raw-projection-mutation"
        projected_proof["binding"]["legacy_proof_fact"] = "raw-projection-mutation"

        second_projection = receipt.to_json_dict()
        second_authority = second_projection["authority_binding"]
        second_proof = second_projection["authority_proof"]
        assert isinstance(second_authority, dict)
        assert isinstance(second_proof, dict)
        assert isinstance(second_proof["binding"], dict)
        self.assertNotIn("legacy_authority", second_authority)
        self.assertNotIn("legacy_proof_fact", second_proof["binding"])

    def test_runtime_receipt_owns_fail_closed_lifecycle_projection(self) -> None:
        receipt = RuntimeReceipt.from_required_mapping(
            canonical_runtime_receipt("inv-state", "completed", "completed", 1)
        )

        self.assertIs(receipt.lifecycle_state, InvocationLifecycleState.COMPLETED)
        self.assertTrue(receipt.lifecycle_state.is_terminal)

        for invalid in ("invented_state", " completed ", "5", "UNSPECIFIED", 5):
            malformed = canonical_runtime_receipt(
                "inv-state",
                "completed",
                "completed",
                1,
            )
            malformed["state"] = invalid
            with self.subTest(state=invalid), self.assertRaises(SDKError) as caught:
                RuntimeReceipt.from_required_mapping(malformed)
            self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        for invalid in ("terminal", "failed", "Completed"):
            malformed = canonical_runtime_receipt(
                "inv-state",
                invalid,
                "completed",
                1,
            )
            with self.subTest(receipt_type=invalid), self.assertRaises(SDKError) as caught:
                RuntimeReceipt.from_required_mapping(malformed)
            self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_runtime_receipt_projects_complete_typed_facts(self) -> None:
        complete = canonical_runtime_receipt("inv-typed", "completed", "completed", 1)
        proof_payload = b"typed-proof"
        delegation_signature = base64.b64encode(bytes([0x73]) * 64).decode()
        strict_profile = "axon-strict-v2"
        delegation_binding = {
            "kind": "delegation",
            "issuer_ura": "easynet:///r/local/agent/issuer",
            "subject_ura": "easynet:///r/local/resource/subject",
            "caller_ura": "easynet:///r/local/agent/caller",
            "audience": "runtime",
            "scopes": ["invoke"],
            "issued_at_ms": 1,
            "expires_at_ms": 2,
            "signature_base64": delegation_signature,
        }
        complete.update(
            {
                "payload_base64": "cGF5bG9hZA==",
                "caller_binding": {
                    "ura": "easynet:///r/local/agent/caller",
                    "profile": strict_profile,
                },
                "callee_binding": {
                    "ura": "easynet:///r/local/agent/callee",
                    "profile": strict_profile,
                },
                "subject_binding": {
                    "ura": "easynet:///r/local/resource/subject",
                    "profile": strict_profile,
                },
                "invocation_nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
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
                "signer_binding": {
                    "ura": "easynet:///r/local/agent/signer",
                    "profile": strict_profile,
                },
                "host_attestation_base64": base64.b64encode(
                    bytes([0x74]) * 64
                ).decode(),
                "authority_binding_kind": "delegation",
                "authority_binding": delegation_binding,
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
                    "kind": 1,
                    "ura": "easynet:///r/local/resource/subject",
                    "profile": strict_profile,
                },
                "descriptor_version": "1.0.0",
                "schema_hash_hex": "11" * 32,
                "impl_hash_hex": "22" * 32,
                "runtime_env": "native",
                "authority_proof": {
                    "proof_type": "admission",
                    "binding_kind": "delegation",
                    "binding": delegation_binding,
                    "proof_payload_base64": base64.b64encode(proof_payload).decode(),
                    "proof_hash_hex": hashlib.sha256(proof_payload).hexdigest(),
                    "issuer": {
                        "ura": "easynet:///r/local/agent/callee",
                        "profile": strict_profile,
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
        receipt = RuntimeReceipt.from_mapping(complete)

        assert receipt.caller_binding is not None
        assert receipt.subject_binding is not None
        assert receipt.callee_signature is not None
        assert receipt.failure is not None
        assert receipt.usage is not None
        assert receipt.subject_ref is not None
        assert receipt.authority_proof is not None
        assert receipt.authority_proof.issuer is not None
        self.assertEqual(receipt.caller_binding.ura, "easynet:///r/local/agent/caller")
        self.assertEqual(receipt.subject_binding.profile, strict_profile)
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
        self.assertEqual(receipt.subject_ref.kind, 1)
        self.assertEqual(
            receipt.authority_proof.issuer.ura,
            "easynet:///r/local/agent/callee",
        )
        self.assertEqual(receipt.parent_receipts[0].receipt_hash_hex, "66" * 32)

    def test_runtime_receipt_accepts_binding_hash_proof_without_payload_or_signature(
        self,
    ) -> None:
        complete = canonical_runtime_receipt(
            "inv-empty-proof", "completed", "completed", 1
        )
        proof = complete["authority_proof"]
        assert isinstance(proof, dict)
        proof["proof_payload_base64"] = ""
        proof["proof_hash_hex"] = authority_binding_proof_hash(
            AuthorityBinding.self_("easynet:///r/example/device/dev-a")
        ).hex()
        proof.pop("signature")

        receipt = RuntimeReceipt.from_mapping(complete)

        assert receipt.authority_proof is not None
        self.assertEqual(receipt.authority_proof.proof_payload_base64, "")
        self.assertIsNone(receipt.authority_proof.signature)

    def test_runtime_receipt_session_authority_facade_uses_generic_fields(
        self,
    ) -> None:
        session_binding: dict[str, object] = {
            "kind": "session",
            "issuer_ura": "easynet:///r/example/agent/backend",
            "subject_ura": "easynet:///r/example/agent/alice",
            "session_id": "session-1",
            "scopes": ["invoke"],
            "audiences": [
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            ],
            "issued_at_ms": 1,
            "expires_at_ms": 2,
            "signature_base64": base64.b64encode(bytes([0x73]) * 64).decode(),
        }
        receipt = canonical_runtime_receipt(
            "inv-session-authority", "completed", "Completed", 1
        )
        receipt["authority_binding_kind"] = "session"
        receipt["authority_binding"] = session_binding
        proof = receipt["authority_proof"]
        assert isinstance(proof, dict)
        proof["proof_type"] = "session"
        proof["binding_kind"] = "session"
        proof["binding"] = session_binding
        proof["proof_payload_base64"] = ""
        proof["proof_hash_hex"] = session_authority_binding_hash(session_binding)
        proof.pop("signature")

        RuntimeReceipt.from_mapping(receipt)

        retired_binding: dict[str, object] = {
            "kind": "session",
            "backend_ura": "easynet:///r/example/agent/backend",
            "user_ura": "easynet:///r/example/agent/alice",
            "session_id": "session-1",
            "scopes": ["invoke"],
            "audiences": [
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            ],
            "issued_at_ms": 1,
            "expires_at_ms": 2,
            "signature_base64": base64.b64encode(bytes([0x73]) * 64).decode(),
        }
        retired = canonical_runtime_receipt(
            "inv-retired-session-authority", "completed", "Completed", 1
        )
        retired["authority_binding_kind"] = "session"
        retired["authority_binding"] = retired_binding
        retired_proof = retired["authority_proof"]
        assert isinstance(retired_proof, dict)
        retired_proof["proof_type"] = "session"
        retired_proof["binding_kind"] = "session"
        retired_proof["binding"] = retired_binding
        retired_proof["proof_payload_base64"] = ""
        with self.assertRaises(SDKError) as raised:
            RuntimeReceipt.from_mapping(retired)
        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn(
            "authority_binding contains noncanonical field",
            raised.exception.message,
        )

    def test_runtime_receipt_required_summary_rejects_malformed_hash(self) -> None:
        malformed = canonical_runtime_receipt("inv-1", "completed", "completed", 1)
        malformed["self_hash_hex"] = "aa"
        with self.assertRaises(SDKError) as caught:
            RuntimeReceipt.from_required_mapping(malformed)

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
        self.assertEqual(
            transport.seen_signed["prepared"]["tuple"]["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(
            transport.seen_signed["prepared"]["tuple"]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

    def test_open_signed_stream_preserves_signature(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        stream = client.open_signed_stream(signed_fixture())

        self.assertEqual(stream.stream_id, "stream-1")
        assert transport.seen_draft is not None
        self.assertEqual(transport.seen_draft["signer_id"], "caller-key")
        self.assertEqual(
            transport.seen_draft["signature"]["signature_base64"],
            "c2lnbmF0dXJl",
        )

    def test_open_signed_bidi_preserves_signature_and_streams(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        session = client.open_signed_bidi(
            signed_fixture(),
            (
                BidiStreamDescriptor(
                    stream_id=9,
                    content_type="application/json",
                    ordering="ordered",
                ),
            ),
        )

        self.assertEqual(session.session_id, "bidi-1")
        assert transport.seen_draft is not None
        self.assertEqual(transport.seen_draft["signer_id"], "caller-key")
        self.assertEqual(
            transport.seen_draft["signature"]["signature_base64"],
            "c2lnbmF0dXJl",
        )
        self.assertEqual(
            transport.seen_streams,
            [{"content_type": "application/json", "ordering": "ordered", "stream_id": 9}],
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

    def test_recover_delegates_to_provider(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        report = client.recover(
            RuntimeRecoveryRequest(
                recovery_id="recovery-1",
                deadline_unix_ms=1783100009999,
                max_invocations=32,
            )
        )

        self.assertEqual(
            transport.seen_recovery_request,
            {
                "deadline_unix_ms": 1783100009999,
                "max_invocations": 32,
                "recovery_id": "recovery-1",
            },
        )
        self.assertEqual(report.state, "runtime_started")
        self.assertTrue(report.bounded_scan)
        self.assertTrue(report.cleanup_complete)
        self.assertEqual(report.recovered_invocations, 2)
        self.assertEqual(report.reaped_orphans, 1)
        self.assertEqual(report.replayed_terminal_receipts, 1)
        self.assertEqual(report.events[0].kind, "orphan_reaped")
        self.assertTrue(report.events[0].terminal)
        self.assertTrue(report.events[0].receipt_ura.startswith("easynet:///"))

        with self.assertRaises(SDKError) as state_caught:
            RuntimeRecoveryReport.from_json(
                b'{"recovery_id":"recovery-1","state":"recovering",'
                b'"bounded_scan":true,"cleanup_complete":true,"events":[]}'
            )
        self.assertTrue(is_code(state_caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as bounded_caught:
            RuntimeRecoveryReport.from_json(
                b'{"recovery_id":"recovery-1","state":"runtime_started",'
                b'"bounded_scan":false,"cleanup_complete":true,"events":[]}'
            )
        self.assertTrue(is_code(bounded_caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as cleanup_caught:
            RuntimeRecoveryReport.from_json(
                b'{"recovery_id":"recovery-1","state":"runtime_started",'
                b'"bounded_scan":true,"cleanup_complete":false,"events":[]}'
            )
        self.assertTrue(is_code(cleanup_caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as terminal_caught:
            RuntimeRecoveryReport.from_json(
                b'{"recovery_id":"recovery-1","state":"runtime_started",'
                b'"bounded_scan":true,"cleanup_complete":true,'
                b'"events":[{"sequence":1,"kind":"orphan_reaped"}]}'
            )
        self.assertTrue(is_code(terminal_caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as missing_counter_caught:
            RuntimeRecoveryReport.from_json(
                b'{"recovery_id":"recovery-1","state":"runtime_started",'
                b'"reaped_orphans":0,"replayed_terminal_receipts":0,'
                b'"bounded_scan":true,"cleanup_complete":true,"events":[]}'
            )
        self.assertTrue(
            is_code(missing_counter_caught.exception, ErrorCode.INVALID_ARGUMENT)
        )

        with self.assertRaises(SDKError) as negative_counter_caught:
            RuntimeRecoveryReport.from_json(
                b'{"recovery_id":"recovery-1","state":"runtime_started",'
                b'"recovered_invocations":-1,"reaped_orphans":0,'
                b'"replayed_terminal_receipts":0,'
                b'"bounded_scan":true,"cleanup_complete":true,"events":[]}'
            )
        self.assertTrue(
            is_code(negative_counter_caught.exception, ErrorCode.INVALID_ARGUMENT)
        )

        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        with self.assertRaises(SDKError) as request_caught:
            client.recover(
                RuntimeRecoveryRequest(
                    recovery_id="",
                    deadline_unix_ms=1783100009999,
                    max_invocations=32,
                )
            )
        self.assertTrue(is_code(request_caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_recovery_request)

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
