import base64
import concurrent.futures
import hashlib
import json
import tempfile
import time
import unittest
from collections.abc import Iterator
from dataclasses import replace
from pathlib import Path
from typing import Any, cast

import grpc

from easynet_sdk import (
    AbilityCallRequest,
    AbilityInvocationClient,
    AddressingClient,
    AddressingProjection,
    ConnectOptions,
    RuntimeInvocationTransport,
    ErrorCode,
    ErrorClass,
    InvocationHandle,
    InvocationSignature,
    RetryHint,
    RuntimeClient,
    RuntimeReceipt,
    SDKError,
    is_code,
)
from easynet_sdk._axon_pb.axon.v1 import (
    invoke_pb2 as _invoke_pb2,
    invoke_pb2_grpc as _invoke_pb2_grpc,
    types_pb2 as _types_pb2,
)
from easynet_sdk.providers.runtime.control import _ControlDiscovery, _IpcVersionRange
from easynet_sdk.providers.runtime.direct import (
    DirectRuntimeBidiTransport,
    DirectRuntimeConnector,
    DirectRuntimeTransport,
    _canonical_receipt_document,
    _canonical_receipt_projection,
    _axon_causal_context,
    _grpc_error,
    _invoke_response_json,
    _response_error_code,
    _stream_chunk_json,
)
from test_runtime import complete_draft
from addressing_fake import MemoryAddressingTransport

invoke_pb2: Any = _invoke_pb2
invoke_pb2_grpc: Any = _invoke_pb2_grpc
types_pb2: Any = _types_pb2

DESCRIPTOR_REF = "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
ABILITY_URA = "easynet:///r/example/ability/device.dev-a.observe.health"
ABILITY_PUBLIC_NAME = "observe.health"
CALLEE_URA = "easynet:///r/example/device/dev-a"
USER_SUBJECT_URA = "easynet:///r/example/user/alice"
RESOURCE_SUBJECT_URA = "easynet:///r/example/resource/job-42"


def _caller_signature() -> InvocationSignature:
    return InvocationSignature(
        algorithm="ed25519",
        signature_base64=base64.b64encode(bytes.fromhex("71" * 64)).decode("ascii"),
        key_id_hint="caller-key",
    )


def _signed_draft():
    return replace(complete_draft(), caller_signature=_caller_signature())


class _FakeRpcError(grpc.RpcError):
    def __init__(self, status_code: grpc.StatusCode, details: str) -> None:
        super().__init__()
        self._status_code = status_code
        self._details = details

    def code(self) -> grpc.StatusCode:
        return self._status_code

    def details(self) -> str:
        return self._details


def _canonical_receipt(
    *,
    index: int,
    invocation_id: str,
    receipt_type: str,
    state: int,
    cleanup_complete: bool = False,
    parent_receipts: tuple[Any, ...] = (),
) -> Any:
    caller_ura = "easynet:///r/example/agent/alice.sdk"
    proof_payload = b'{"authority":"self"}'
    payload = b'{"ready":true}' if cleanup_complete else b'{"admitted":true}'
    authority = types_pb2.AuthorityBinding(
        self_authority=types_pb2.SelfAuthority(principal_ura=caller_ura)
    )
    return invoke_pb2.InvocationReceipt(
        index=index,
        invocation_id=invocation_id,
        receipt_type=receipt_type,
        state=state,
        timestamp_unix_ms=1783100000123 + index,
        prev_receipt_hash=(bytes(32) if index == 0 else bytes.fromhex("01" * 32)),
        self_hash=bytes.fromhex(f"{index + 1:02x}" * 32),
        payload=payload,
        payload_content_type="application/json",
        cleanup_complete=cleanup_complete,
        caller_binding=types_pb2.AgentIdentity(
            ura=caller_ura,
            profile="axon-strict-v2",
        ),
        callee_binding=types_pb2.AgentIdentity(
            ura=CALLEE_URA,
            profile="axon-strict-v2",
        ),
        subject_binding=types_pb2.SubjectIdentity(
            ura=CALLEE_URA,
            profile="axon-strict-v2",
        ),
        invocation_nonce=bytes(range(1, 17)),
        causal_binding=types_pb2.CausalContext(none=types_pb2.Empty()),
        callee_signature=types_pb2.CalleeSignature(
            algorithm="ed25519",
            signature=bytes.fromhex("91" * 64),
            key_id_hint="callee-key",
        ),
        authority_binding=authority,
        ability_binding=DESCRIPTOR_REF,
        usage=invoke_pb2.InvocationUsage(
            tokens_in=3,
            tokens_out=5,
            duration_ms=11,
            external_calls=1,
        ),
        subject_ref=types_pb2.EntityRef(
            kind=types_pb2.ENTITY_REF_KIND_DEVICE,
            ura=CALLEE_URA,
            profile="axon-strict-v2",
        ),
        descriptor_version="1.0.0",
        schema_hash=bytes.fromhex("21" * 32),
        impl_hash=bytes.fromhex("31" * 32),
        runtime_env="python-test",
        authority_proof=types_pb2.InvocationAuthorityProof(
            proof_type="self",
            binding=authority,
            proof_payload=proof_payload,
            proof_hash=hashlib.sha256(proof_payload).digest(),
            issuer=types_pb2.AgentIdentity(
                ura=CALLEE_URA,
                profile="axon-strict-v2",
            ),
            signature=types_pb2.CalleeSignature(
                algorithm="ed25519",
                signature=bytes.fromhex("a1" * 64),
                key_id_hint="authority-key",
            ),
            admission_hook="self-authority",
        ),
        input_hash=hashlib.sha256(b"{}").digest(),
        output_hash=hashlib.sha256(payload).digest(),
        parent_receipts=parent_receipts,
    )


def _admission_receipt(invocation_id: str) -> Any:
    return _canonical_receipt(
        index=0,
        invocation_id=invocation_id,
        receipt_type="admitted",
        state=types_pb2.INVOCATION_STATE_ADMITTED,
    )


def _assert_complete_receipt_projection(
    test: unittest.TestCase,
    receipt: dict[str, object],
) -> None:
    test.assertRegex(
        cast(str, receipt["receipt_ura"]),
        r"^easynet:///r/example/resource/runtime/invocation/.+/receipt/[0-9]+$",
    )
    test.assertEqual(receipt["descriptor_version"], "1.0.0")
    test.assertEqual(receipt["schema_hash_hex"], "21" * 32)
    test.assertEqual(receipt["impl_hash_hex"], "31" * 32)
    test.assertEqual(receipt["runtime_env"], "python-test")
    test.assertEqual(receipt["input_hash_hex"], hashlib.sha256(b"{}").hexdigest())
    payload = base64.b64decode(cast(str, receipt["payload_base64"]), validate=True)
    test.assertEqual(
        receipt["output_hash_hex"],
        hashlib.sha256(payload).hexdigest(),
    )
    test.assertEqual(receipt["causal_binding_kind"], "none")
    test.assertEqual(receipt["causal_binding"], {"form": "none"})
    test.assertEqual(receipt["authority_binding_kind"], "self")
    authority = cast(dict[str, object], receipt["authority_binding"])
    test.assertEqual(
        authority["principal_ura"],
        "easynet:///r/example/agent/alice.sdk",
    )
    signature = cast(dict[str, object], receipt["callee_signature"])
    test.assertEqual(signature["algorithm"], "ed25519")
    test.assertTrue(signature["signature_base64"])
    signer = cast(dict[str, object], receipt["signer_binding"])
    test.assertEqual(signer["ura"], CALLEE_URA)
    proof = cast(dict[str, object], receipt["authority_proof"])
    test.assertEqual(proof["proof_type"], "self")
    test.assertEqual(proof["binding_kind"], "self")
    test.assertIsInstance(proof["binding"], dict)
    test.assertTrue(proof["proof_payload_base64"])
    test.assertTrue(proof["proof_hash_hex"])
    test.assertIsInstance(proof["issuer"], dict)
    test.assertIsInstance(proof["signature"], dict)
    test.assertEqual(proof["admission_hook"], "self-authority")
    test.assertIn("parent_receipts", receipt)
    RuntimeReceipt.from_required_mapping(receipt)


class RecordingInvocationServicer(invoke_pb2_grpc.InvocationServicer):
    def __init__(self) -> None:
        self.requests: list[Any] = []
        self.stream_requests: list[Any] = []
        self.bidi_up_frames: list[Any] = []
        self.invoke_delay_seconds = 0.0
        self.stream_delay_seconds = 0.0
        self.bidi_delay_seconds = 0.0
        self.stream_chunks: list[Any] = [
            invoke_pb2.InvokeStreamChunk(
                invocation_id="inv-stream",
                state=types_pb2.INVOCATION_STATE_RUNNING,
                payload=b'{"chunk":1}',
                content_type="application/json",
                sequence=0,
                terminal=False,
            ),
            invoke_pb2.InvokeStreamChunk(
                invocation_id="inv-stream",
                state=types_pb2.INVOCATION_STATE_COMPLETED,
                payload=b'{"done":true}',
                content_type="application/json",
                sequence=1,
                terminal=True,
                elapsed_ms=11,
                terminal_receipt=_canonical_receipt(
                    index=1,
                    invocation_id="inv-stream",
                    receipt_type="completed",
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    cleanup_complete=True,
                ),
            ),
        ]

    def Invoke(self, request, context):
        self.requests.append(request)
        if self.invoke_delay_seconds:
            time.sleep(self.invoke_delay_seconds)
        return invoke_pb2.InvokeResponse(
            state=types_pb2.INVOCATION_STATE_COMPLETED,
            result=b'{"ready":true}',
            result_content_type="application/json",
            elapsed_ms=9,
            admission_receipt=_admission_receipt("inv-direct"),
            terminal_receipt=_canonical_receipt(
                index=1,
                invocation_id="inv-direct",
                receipt_type="completed",
                state=types_pb2.INVOCATION_STATE_COMPLETED,
                cleanup_complete=True,
            ),
        )

    def InvokeStream(self, request, context):
        self.stream_requests.append(request)
        if self.stream_delay_seconds:
            time.sleep(self.stream_delay_seconds)
        yield from self.stream_chunks

    def InvokeBidi(self, request_iterator, context) -> Iterator[Any]:
        first = next(request_iterator)
        self.bidi_up_frames.append(first)
        if self.bidi_delay_seconds:
            time.sleep(self.bidi_delay_seconds)
        yield invoke_pb2.InvokeBidiDown(
            sequence=0,
            receipt=_canonical_receipt(
                index=0,
                invocation_id="inv-bidi",
                receipt_type="admitted",
                state=types_pb2.INVOCATION_STATE_ADMITTED,
            ),
        )
        for frame in request_iterator:
            self.bidi_up_frames.append(frame)
            payload = frame.WhichOneof("payload")
            if payload == "binary_chunk":
                yield invoke_pb2.InvokeBidiDown(
                    sequence=frame.sequence,
                    binary_chunk=invoke_pb2.BinaryChunk(
                        stream_id=frame.binary_chunk.stream_id,
                        data=frame.binary_chunk.data,
                    ),
                )
            elif payload == "control" and frame.control.WhichOneof("control") == "eof":
                yield invoke_pb2.InvokeBidiDown(
                    sequence=frame.sequence,
                    receipt=_canonical_receipt(
                        index=1,
                        invocation_id="inv-bidi",
                        receipt_type="completed",
                        state=types_pb2.INVOCATION_STATE_COMPLETED,
                        cleanup_complete=True,
                    ),
                )
                return


class DirectRuntimeTests(unittest.TestCase):
    def test_direct_runtime_grpc_not_found_projects_descriptor_not_found(self) -> None:
        error = _grpc_error(
            _FakeRpcError(grpc.StatusCode.NOT_FOUND, "descriptor_ref not found"),
            endpoint="unix:///tmp/easynet-daemon.sock",
        )

        self.assertEqual(error.code, ErrorCode.DESCRIPTOR_NOT_FOUND)
        self.assertEqual(error.error_class, ErrorClass.ROUTING)
        self.assertEqual(error.stage, "direct_runtime")

    def test_direct_runtime_grpc_owner_offline_projects_descriptor_owner_offline(
        self,
    ) -> None:
        for status_code in (grpc.StatusCode.NOT_FOUND, grpc.StatusCode.UNAVAILABLE):
            with self.subTest(status_code=status_code):
                error = _grpc_error(
                    _FakeRpcError(
                        status_code,
                        "ROUTE_NEGATIVE: namespace.resolve negative for `easynet:///r/localhost/ability/device.dev-a.meta.list_abilities`: NEGATIVE_REASON_NXDOMAIN: owner is not online",
                    ),
                    endpoint="unix:///tmp/easynet-daemon.sock",
                )

                self.assertEqual(error.code, ErrorCode.DESCRIPTOR_OWNER_OFFLINE)
                self.assertEqual(
                    error.message,
                    "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online",
                )
                self.assertEqual(error.error_class, ErrorClass.ROUTING)
                self.assertEqual(error.retry, RetryHint.SAFE)
                self.assertTrue(error.retryable)

    def test_direct_connector_resolves_invocation_endpoint_from_discovery(self) -> None:
        connector = DirectRuntimeConnector(
            control_path="/tmp/control.json",
            discovery_reader=lambda path: _ControlDiscovery(
                socket_path="/tmp/control.sock",
                invocation_endpoint="/tmp/invoke.sock",
                runtime_host_version="1.2.3",
                supported_ipc_versions=_IpcVersionRange(1, 1),
                capability_flags=("runtime.invoke", "direct.grpc"),
            ),
        )

        resolved = json.loads(
            connector.resolve(
                json.dumps({"dial_timeout_ms": 500}, separators=(",", ":")).encode(
                    "utf-8"
                )
            ).decode("utf-8")
        )

        self.assertEqual(resolved["endpoint"], "/tmp/invoke.sock")
        self.assertEqual(resolved["control_path"], "/tmp/control.json")
        self.assertNotIn("control_endpoint", resolved)
        self.assertNotIn("daemon_version", resolved)
        self.assertNotIn("capability_flags", resolved)
        self.assertEqual(resolved["dial_timeout_ms"], 500)

    def test_direct_connector_reports_control_only_without_invocation_endpoint(
        self,
    ) -> None:
        connector = DirectRuntimeConnector(
            discovery_reader=lambda path: _ControlDiscovery(
                socket_path="/tmp/control.sock",
                supported_ipc_versions=_IpcVersionRange(1, 1),
            )
        )

        with self.assertRaises(SDKError) as raised:
            connector.resolve(b"{}")

        self.assertTrue(is_code(raised.exception, ErrorCode.CONTROL_ONLY))
        self.assertEqual(raised.exception.stage, "direct_runtime.resolve")

    def test_direct_connector_handshake_reports_runtime_capabilities(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            connector = DirectRuntimeConnector(identity=_identity())
            transport, facts_json = connector.handshake(
                json.dumps(
                    {
                        "endpoint": endpoint,
                        "dial_timeout_ms": 1000,
                        "invoke_timeout_ms": 1000,
                    },
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            try:
                facts = json.loads(facts_json.decode("utf-8"))
            finally:
                transport.close()
                connector.close()

        self.assertEqual(facts["transport"], "direct-axon-grpc-uds")
        self.assertEqual(facts["protocol"], "axon.v1.Invocation")
        self.assertEqual(facts["unary"], True)
        self.assertEqual(facts["stream"], True)
        self.assertEqual(facts["bidi"], True)
        self.assertEqual(facts["prepare"], False)
        self.assertEqual(facts["submit_signed"], False)

    def test_direct_connector_delegates_handle_transport_when_configured(self) -> None:
        servicer = RecordingInvocationServicer()
        handle_transport = _RecordingHandleTransport()
        draft_json = complete_draft().to_json().encode("utf-8")
        with _fake_daemon(servicer) as endpoint:
            connector = DirectRuntimeConnector(
                handle_transport=handle_transport,
                identity=_identity(),
            )
            transport, facts_json = connector.handshake(
                json.dumps(
                    {
                        "endpoint": endpoint,
                        "dial_timeout_ms": 1000,
                        "invoke_timeout_ms": 1000,
                    },
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            try:
                facts = json.loads(facts_json.decode("utf-8"))
                prepared = transport.prepare(draft_json, b'{"resolve":true}')
                submitted = transport.submit_signed(b'{"signed":true}')
                control = InvocationHandle._from_runtime_json(
                    b'{"handle_id":7,"state":"Submitted","terminal":false}'
                ).control_capability()
                awaited = transport.await_handle(control)
                cancelled = transport.cancel_handle(control, "stop")
                events = transport.handle_events(control)
                transport.free_handle(control)
            finally:
                transport.close()
                connector.close()

        self.assertEqual(facts["prepare"], True)
        self.assertEqual(facts["submit_signed"], True)
        self.assertEqual(prepared, b'{"prepared":true}')
        self.assertEqual(submitted, b'{"handle_id":7,"state":"Submitted"}')
        self.assertEqual(awaited, b'{"ok":true,"terminal_state":"Completed"}')
        self.assertEqual(cancelled, b'{"handle_id":7,"cancelled":true}')
        self.assertEqual(events, b'{"handle_id":7,"events":[]}')
        self.assertEqual(
            handle_transport.calls,
            [
                ("prepare", draft_json, b'{"resolve":true}'),
                ("submit_signed", b'{"signed":true}'),
                ("await_handle", 7),
                ("cancel_handle", 7, "stop"),
                ("handle_events", 7),
                ("free_handle", 7),
            ],
        )
        self.assertEqual(handle_transport.close_count, 0)

    def test_direct_connector_rejects_non_entity_prepare_subject(self) -> None:
        servicer = RecordingInvocationServicer()
        handle_transport = _RecordingHandleTransport()
        draft_json = _user_subject_draft_json()
        with _fake_daemon(servicer) as endpoint:
            connector = DirectRuntimeConnector(
                handle_transport=handle_transport,
                identity=_identity(),
            )
            transport, _ = connector.handshake(
                json.dumps(
                    {
                        "endpoint": endpoint,
                        "dial_timeout_ms": 1000,
                        "invoke_timeout_ms": 1000,
                    },
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.prepare(draft_json, b"{}")
            finally:
                transport.close()
                connector.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_INVOCATION))
        self.assertIn("subject_ref_kind_unsupported:user", raised.exception.message)
        self.assertEqual(handle_transport.calls, [])

    def test_direct_connector_closes_owned_handle_transport_once(self) -> None:
        servicer = RecordingInvocationServicer()
        handle_transport = _RecordingHandleTransport()
        identity = _identity()
        with _fake_daemon(servicer) as endpoint:
            connector = (
                DirectRuntimeConnector()
                .with_identity(identity, close_on_connector_close=True)
                .with_handle_transport(
                    handle_transport,
                    close_on_connector_close=True,
                )
            )
            transport, facts_json = connector.handshake(
                json.dumps(
                    {
                        "endpoint": endpoint,
                        "dial_timeout_ms": 1000,
                        "invoke_timeout_ms": 1000,
                    },
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            facts = json.loads(facts_json.decode("utf-8"))
            transport.close()
            connector.close()
            connector.close()

        self.assertEqual(facts["prepare"], True)
        self.assertEqual(facts["submit_signed"], True)
        self.assertEqual(handle_transport.close_count, 1)
        self.assertEqual(identity.close_count, 1)

    def test_direct_transport_closes_owned_handle_transport_once(self) -> None:
        servicer = RecordingInvocationServicer()
        handle_transport = _RecordingHandleTransport()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                handle_transport=handle_transport,
                identity=_identity(),
                close_handle_transport=True,
            )
            transport.close()
            transport.close()

        self.assertEqual(handle_transport.close_count, 1)

    def test_direct_transport_invokes_daemon_over_axon_grpc_uds(self) -> None:
        servicer = RecordingInvocationServicer()
        identity = _identity()
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=identity,
            )
            try:
                result = transport.invoke(_signed_draft())
            finally:
                transport.close()

        self.assertEqual(result["ok"], True)
        self.assertEqual(result["terminal_state"], "Completed")
        self.assertEqual(result["output_json"], {"ready": True})
        receipt = cast(dict[str, object], result["terminal_receipt"])
        self.assertEqual(receipt["invocation_id"], "inv-direct")
        _assert_complete_receipt_projection(self, receipt)

        self.assertEqual(len(servicer.requests), 1)
        request = servicer.requests[0]
        self.assertEqual(request.target.WhichOneof("typed_target"), "ability")
        self.assertEqual(request.target.ability.ability_name, DESCRIPTOR_REF)
        self.assertEqual(request.target.ability.function_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(request.content_type, "application/json")
        self.assertEqual(request.arguments, b"{}")
        self.assertEqual(request.content_envelope.encoding, "identity")
        self.assertEqual(
            request.envelope.caller.ura,
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(
            request.envelope.callee.ura,
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            request.envelope.subject.ura,
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(request.envelope.causal_context.WhichOneof("form"), "none")
        self.assertEqual(identity.descriptor_refs, [DESCRIPTOR_REF])
        self.assertEqual(identity.ability_uras, [ABILITY_URA])

    def test_direct_dispatch_rejects_unsigned_drafts_before_wire(self) -> None:
        servicer = RecordingInvocationServicer()
        draft_json = complete_draft().to_json().encode("utf-8")
        streams_json = b'[{"stream_id":1,"content_type":"application/json"}]'
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                operations = (
                    ("unary", lambda: transport.invoke(draft_json)),
                    ("stream", lambda: transport.open_stream(draft_json)),
                    (
                        "bidi",
                        lambda: transport.open_bidi(draft_json, streams_json),
                    ),
                )
                for mode, operation in operations:
                    with self.subTest(mode=mode):
                        with self.assertRaises(SDKError) as raised:
                            operation()
                        self.assertTrue(
                            is_code(raised.exception, ErrorCode.INVALID_INVOCATION)
                        )
                        self.assertIn(
                            "requires caller_signature",
                            raised.exception.message,
                        )
            finally:
                transport.close()

        self.assertEqual(servicer.requests, [])
        self.assertEqual(servicer.stream_requests, [])
        self.assertEqual(servicer.bidi_up_frames, [])

    def test_runtime_ability_deadline_is_provider_owned(self) -> None:
        servicer = RecordingInvocationServicer()
        servicer.invoke_delay_seconds = 0.2
        with _fake_daemon(servicer) as endpoint:
            direct = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=0.05,
                identity=_identity(),
            )
            client = AbilityInvocationClient(
                runtime=RuntimeClient(_DirectAbilityRuntimeTransport(direct)),
                addressing=AddressingClient(_ability_addressing_transport()),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    client.invoke(_ability_request())

                servicer.invoke_delay_seconds = 0.0
                retry = client.invoke(_ability_request())
            finally:
                client.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.TIMEOUT))
        self.assertEqual(raised.exception.stage, "direct_runtime")
        self.assertEqual(
            raised.exception.details["grpc_status"],
            str(grpc.StatusCode.DEADLINE_EXCEEDED),
        )
        self.assertGreaterEqual(len(servicer.requests), 2)
        self.assertTrue(retry.ok)
        self.assertIsNotNone(retry.terminal_receipt_summary)

    def test_direct_transport_rejects_user_subject_instead_of_rewriting(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(_user_subject_draft_dict())
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_INVOCATION))
        self.assertIn("subject_ref_kind_unsupported:user", raised.exception.message)
        self.assertEqual(servicer.requests, [])

    def test_direct_transport_projects_signer_pubkey_as_wire_key_hint(self) -> None:
        servicer = RecordingInvocationServicer()
        public_key_b64 = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA="
        draft = replace(
            complete_draft(),
            caller_signature=InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
                signer_public_key_base64=public_key_b64,
            ),
        )
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                transport.invoke(draft)
            finally:
                transport.close()

        self.assertEqual(len(servicer.requests), 1)
        self.assertEqual(
            servicer.requests[0].envelope.caller_signature.key_id_hint,
            public_key_b64,
        )

    def test_direct_transport_preserves_complete_caller_supplied_tuple(self) -> None:
        servicer = RecordingInvocationServicer()
        nonce = bytes.fromhex("51" * 16)
        parent_hash = bytes.fromhex("61" * 32)
        signature = bytes.fromhex("71" * 64)
        draft = replace(
            complete_draft(),
            subject_ura=RESOURCE_SUBJECT_URA,
            nonce_base64=base64.b64encode(nonce).decode("ascii"),
            causal_context={
                "form": "scalar",
                "receipt_hash_hex": parent_hash.hex(),
                "receipt_ura": "easynet:///r/example/resource/job-41/receipt/terminal",
            },
            args=None,
            arguments_base64=base64.b64encode(b"\x00\x01\x02").decode("ascii"),
            content_type="application/octet-stream",
            metadata={"trace": "caller-owned"},
            caller_signature=InvocationSignature(
                algorithm="ed25519",
                signature_base64=base64.b64encode(signature).decode("ascii"),
                key_id_hint="caller-key",
            ),
            _has_args=False,
        )
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                result = transport.invoke(draft)
            finally:
                transport.close()

        request = servicer.requests[0]
        self.assertEqual(request.envelope.caller.ura, draft.caller_ura)
        self.assertEqual(request.envelope.callee.ura, draft.callee_ura)
        self.assertEqual(request.envelope.subject.ura, RESOURCE_SUBJECT_URA)
        self.assertEqual(request.envelope.invocation_nonce, nonce)
        self.assertEqual(
            request.envelope.causal_context.scalar.receipt_hash,
            parent_hash,
        )
        self.assertEqual(
            request.envelope.causal_context.scalar.receipt_ura,
            draft.causal_context["receipt_ura"],
        )
        self.assertEqual(request.arguments, b"\x00\x01\x02")
        self.assertEqual(request.envelope.caller_signature.signature, signature)
        self.assertEqual(request.envelope.caller_signature.key_id_hint, "caller-key")
        self.assertEqual(request.target.ability.ability_name, DESCRIPTOR_REF)
        self.assertEqual(request.target.ability.function_name, "observe.health")
        self.assertNotIn("x-easynet-signed-descriptor-ref", request.metadata)
        tuple_projection = cast(dict[str, object], result["tuple"])
        self.assertEqual(tuple_projection, draft.to_json_dict())

    def test_direct_runtime_causal_context_rejects_retired_dag_proof_alias(self) -> None:
        canonical = _axon_causal_context(
            {
                "form": "merkle",
                "root_hex": "ab" * 32,
                "proof_ura": "easynet:///r/example/resource/agent.alice/proof/causal",
            }
        )
        self.assertIsNotNone(canonical.as_merkle())

        for causal_context in (
            {
                "form": "merkle",
                "root_hex": "ab" * 32,
                "dag_proof_ura": (
                    "easynet:///r/example/resource/agent.alice/proof/causal"
                ),
            },
            {
                "form": "merkle",
                "dag_root_hex": "ab" * 32,
                "proof_ura": "easynet:///r/example/resource/agent.alice/proof/causal",
            },
        ):
            with self.subTest(causal_context=causal_context):
                with self.assertRaises(SDKError) as raised:
                    _axon_causal_context(causal_context)
                self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_INVOCATION))

    def test_direct_transport_rejects_descriptor_not_owned_by_callee(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(owner_ura="easynet:///r/example/device/other"),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(_signed_draft().to_json().encode("utf-8"))
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_INVOCATION))
        self.assertEqual(servicer.requests, [])

    def test_direct_transport_projects_failed_terminal_state_to_admission_denied(
        self,
    ) -> None:
        class FailedServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_FAILED,
                    elapsed_ms=4,
                    admission_receipt=_admission_receipt("inv-failed"),
                    terminal_receipt=_canonical_receipt(
                        index=1,
                        invocation_id="inv-failed",
                        receipt_type="failed",
                        state=types_pb2.INVOCATION_STATE_FAILED,
                        cleanup_complete=True,
                    ),
                )

        servicer = FailedServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                result = transport.invoke(_signed_draft())
            finally:
                transport.close()

        error = cast(dict[str, object], result["error"])
        self.assertEqual(result["ok"], False)
        self.assertEqual(result["terminal_state"], "Failed")
        self.assertEqual(error["code"], ErrorCode.ADMISSION_DENIED.value)

    def test_direct_transport_projects_missing_error_code_to_protocol_mismatch(
        self,
    ) -> None:
        self.assertEqual(_response_error_code(""), ErrorCode.PROTOCOL_MISMATCH)

    def test_direct_runtime_unary_rejects_unsupported_invocation_state(self) -> None:
        with self.assertRaises(SDKError) as raised:
            _invoke_response_json(
                _signed_draft(),
                invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_UNSPECIFIED,
                ),
            )

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_direct_runtime_stream_rejects_unsupported_invocation_state(self) -> None:
        with self.assertRaises(SDKError) as raised:
            _stream_chunk_json(
                invoke_pb2.InvokeStreamChunk(
                    state=types_pb2.INVOCATION_STATE_UNSPECIFIED,
                )
            )

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_direct_transport_projects_cancelled_terminal_state_to_cancelled(
        self,
    ) -> None:
        class CancelledServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_CANCELLED,
                    elapsed_ms=4,
                    admission_receipt=_admission_receipt("inv-cancelled"),
                    terminal_receipt=_canonical_receipt(
                        index=1,
                        invocation_id="inv-cancelled",
                        receipt_type="cancelled",
                        state=types_pb2.INVOCATION_STATE_CANCELLED,
                        cleanup_complete=True,
                    ),
                )

        servicer = CancelledServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                result = transport.invoke(_signed_draft())
            finally:
                transport.close()

        error = cast(dict[str, object], result["error"])
        self.assertEqual(result["ok"], False)
        self.assertEqual(result["terminal_state"], "Cancelled")
        self.assertEqual(error["code"], ErrorCode.CANCELLED.value)

    def test_direct_transport_projects_metadata_to_axon_string_map(
        self,
    ) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                draft = _signed_draft().to_json_dict()
                draft["metadata"] = {
                    "attempt": 1,
                    "dry_run": False,
                    "shape": {"b": 2, "a": 1},
                    "empty": None,
                }
                transport.invoke(
                    json.dumps(draft, separators=(",", ":")).encode("utf-8")
                )
            finally:
                transport.close()

        self.assertEqual(len(servicer.requests), 1)
        self.assertEqual(servicer.requests[0].metadata["attempt"], "1")
        self.assertEqual(servicer.requests[0].metadata["dry_run"], "false")
        self.assertEqual(servicer.requests[0].metadata["shape"], '{"a":1,"b":2}')
        self.assertNotIn("empty", servicer.requests[0].metadata)
        self.assertEqual(
            servicer.requests[0].target.ability.function_name,
            "observe.health",
        )
        self.assertNotIn(
            "x-easynet-signed-descriptor-ref",
            servicer.requests[0].metadata,
        )

    def test_direct_transport_projects_daemon_stream_events_over_axon_grpc_uds(
        self,
    ) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                ),
                identity=_identity(),
            )
            try:
                stream = transport.stream(_signed_draft())
                first = stream.recv()
                terminal = stream.recv()
                stream.close()
            finally:
                transport.close()

        self.assertEqual(first["sequence"], 1)
        self.assertEqual(first["kind"], "data")
        self.assertEqual(first["payload_json"], {"chunk": 1})
        self.assertFalse(first["terminal"])
        self.assertEqual(terminal["sequence"], 2)
        self.assertEqual(terminal["kind"], "terminal")
        self.assertEqual(terminal["payload_json"], {"done": True})
        self.assertTrue(terminal["terminal"])
        self.assertEqual(len(servicer.stream_requests), 1)
        request = servicer.stream_requests[0]
        self.assertEqual(request.target.WhichOneof("typed_target"), "ability")
        self.assertEqual(request.target.ability.ability_name, DESCRIPTOR_REF)
        self.assertEqual(request.target.ability.function_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(request.content_type, "application/json")
        self.assertEqual(request.arguments, b"{}")
        self.assertEqual(
            request.envelope.caller.ura,
            "easynet:///r/example/agent/alice.sdk",
        )

    def test_direct_transport_projects_zero_based_stream_sequence(self) -> None:
        servicer = RecordingInvocationServicer()
        servicer.stream_chunks = [
            invoke_pb2.InvokeStreamChunk(
                invocation_id="inv-stream",
                state=types_pb2.INVOCATION_STATE_RUNNING,
                payload=b"{}",
                content_type="application/json",
                sequence=0,
                terminal=False,
            )
        ]
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                ),
                identity=_identity(),
            )
            try:
                stream = transport.stream(_signed_draft())
                event = stream.recv(timeout=1)
                stream.close()
            finally:
                transport.close()

        self.assertEqual(event["sequence"], 1)
        self.assertEqual(event["kind"], "data")
        self.assertFalse(event["terminal"])

    def test_direct_transport_opens_bidi_over_axon_grpc_uds(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                ),
                identity=_identity(),
            )
            try:
                bidi = transport.bidi(
                    _signed_draft(),
                    (
                        {
                            "stream_id": 1,
                            "content_type": "application/json",
                            "ordering": "STRICT",
                        },
                    ),
                )
                ack = bidi.send(
                    {
                        "sequence": 1,
                        "kind": "data",
                        "stream_id": 1,
                        "payload_base64": "eyJwaW5nIjp0cnVlfQ==",
                    }
                )
                echoed = bidi.recv(timeout=5)
                outcome = bidi.close_send()
                terminal = bidi.recv(timeout=5)
                bidi.close()
            finally:
                transport.close()

        self.assertEqual(ack["sequence"], 1)
        self.assertEqual(echoed["sequence"], 2)
        self.assertEqual(echoed["kind"], "data")
        self.assertEqual(echoed["stream_id"], 1)
        self.assertEqual(echoed["payload_base64"], "eyJwaW5nIjp0cnVlfQ==")
        self.assertNotIn("admission_receipt", echoed)
        self.assertEqual(outcome["state"], "HalfClosedLocal")
        self.assertFalse(outcome["terminal"])
        self.assertEqual(terminal["sequence"], 3)
        self.assertEqual(terminal["kind"], "terminal")
        self.assertTrue(terminal["terminal"])
        self.assertNotIn("receipt", terminal)
        payload = cast(dict[str, object], terminal.get("payload_json") or {})
        self.assertNotIn("receipt", payload)
        terminal_receipt = cast(dict[str, object], terminal["terminal_receipt"])
        self.assertEqual(terminal_receipt["invocation_id"], "inv-bidi")
        _assert_complete_receipt_projection(self, terminal_receipt)

        self.assertEqual(len(servicer.bidi_up_frames), 3)
        open_frame = servicer.bidi_up_frames[0]
        self.assertEqual(open_frame.sequence, 0)
        self.assertEqual(open_frame.WhichOneof("payload"), "envelope_open")
        envelope_open = open_frame.envelope_open
        self.assertEqual(
            envelope_open.envelope.caller.ura,
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(
            envelope_open.envelope.callee.ura,
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            envelope_open.envelope.subject.ura,
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(envelope_open.target.WhichOneof("typed_target"), "ability")
        self.assertEqual(envelope_open.target.ability.ability_name, DESCRIPTOR_REF)
        self.assertEqual(
            envelope_open.target.ability.function_name, ABILITY_PUBLIC_NAME
        )
        self.assertEqual(envelope_open.initial_args, b"{}")
        self.assertEqual(envelope_open.args_content_type, "application/json")
        self.assertEqual(envelope_open.content_envelope.encoding, "identity")
        self.assertEqual(len(envelope_open.streams), 1)
        self.assertEqual(envelope_open.streams[0].stream_id, 1)
        self.assertEqual(envelope_open.streams[0].content_type, "application/json")
        self.assertEqual(envelope_open.streams[0].ordering, "STRICT")
        self.assertEqual(servicer.bidi_up_frames[1].sequence, 1)
        self.assertEqual(
            servicer.bidi_up_frames[1].WhichOneof("payload"), "binary_chunk"
        )
        self.assertEqual(servicer.bidi_up_frames[2].sequence, 2)
        self.assertEqual(servicer.bidi_up_frames[2].WhichOneof("payload"), "control")
        self.assertEqual(
            servicer.bidi_up_frames[2].control.WhichOneof("control"), "eof"
        )

    def test_direct_runtime_provider_json_uses_canonical_receipt_fields(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                raw = transport.invoke(_signed_draft().to_json().encode("utf-8"))
            finally:
                transport.close()

        result = json.loads(raw.decode("utf-8"))
        self.assertNotIn("receipt", result)
        receipt = cast(dict[str, object], result["terminal_receipt"])
        self.assertEqual(receipt["invocation_id"], "inv-direct")
        _assert_complete_receipt_projection(self, receipt)

    def test_receipt_projection_rejects_every_required_proof_fact_omission(
        self,
    ) -> None:
        omissions = (
            ("self_hash",),
            ("caller_binding",),
            ("callee_binding",),
            ("subject_binding",),
            ("invocation_nonce",),
            ("causal_binding",),
            ("callee_signature",),
            ("authority_binding",),
            ("authority_binding", "self_authority", "principal_ura"),
            ("ability_binding",),
            ("usage",),
            ("subject_ref",),
            ("descriptor_version",),
            ("schema_hash",),
            ("impl_hash",),
            ("runtime_env",),
            ("authority_proof",),
            ("input_hash",),
            ("output_hash",),
            ("authority_proof", "binding"),
            (
                "authority_proof",
                "binding",
                "self_authority",
                "principal_ura",
            ),
            ("authority_proof", "proof_payload"),
            ("authority_proof", "proof_hash"),
            ("authority_proof", "issuer"),
            ("authority_proof", "signature"),
            ("authority_proof", "admission_hook"),
        )
        for path in omissions:
            with self.subTest(field=".".join(path)):
                receipt = _canonical_receipt(
                    index=1,
                    invocation_id="inv-omission",
                    receipt_type="completed",
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    cleanup_complete=True,
                )
                target = receipt
                for part in path[:-1]:
                    target = getattr(target, part)
                target.ClearField(path[-1])

                with self.assertRaises(SDKError) as raised:
                    _canonical_receipt_projection(receipt)

                self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))
                self.assertIn("canonical receipt rejected", raised.exception.message)

    def test_receipt_projection_retains_parent_and_causal_proof_facts(self) -> None:
        parent = types_pb2.ReceiptRef(
            receipt_hash=bytes.fromhex("81" * 32),
            receipt_ura="easynet:///r/example/resource/job-41/receipt/terminal",
        )
        receipt = _canonical_receipt(
            index=1,
            invocation_id="inv-causal",
            receipt_type="completed",
            state=types_pb2.INVOCATION_STATE_COMPLETED,
            cleanup_complete=True,
            parent_receipts=(parent,),
        )
        receipt.causal_binding.CopyFrom(types_pb2.CausalContext(scalar=parent))

        projection = _canonical_receipt_projection(receipt)

        self.assertEqual(projection["causal_binding_kind"], "scalar")
        self.assertEqual(
            projection["causal_binding"],
            {
                "form": "scalar",
                "receipt": {
                    "receipt_hash_hex": "81" * 32,
                    "receipt_ura": parent.receipt_ura,
                },
            },
        )
        self.assertEqual(
            projection["parent_receipts"],
            [
                {
                    "receipt_hash_hex": "81" * 32,
                    "receipt_ura": parent.receipt_ura,
                }
            ],
        )

    def test_receipt_projection_uses_canonical_timed_out_state(self) -> None:
        receipt = _canonical_receipt(
            index=1,
            invocation_id="inv-timed-out",
            receipt_type="timed_out",
            state=types_pb2.INVOCATION_STATE_TIMED_OUT,
            cleanup_complete=True,
        )

        projection = _canonical_receipt_projection(receipt)
        canonical = _canonical_receipt_document(receipt)

        self.assertEqual(projection["state"], "TimedOut")
        self.assertEqual(canonical["state"], "TIMED_OUT")

    def test_receipt_projection_accepts_every_complete_authority_binding(
        self,
    ) -> None:
        signature = bytes.fromhex("c1" * 64)
        cases = (
            (
                "self",
                types_pb2.AuthorityBinding(
                    self_authority=types_pb2.SelfAuthority(
                        principal_ura="easynet:///r/example/agent/alice.sdk"
                    )
                ),
                "self_authority",
                "principal_ura",
            ),
            (
                "delegation",
                types_pb2.AuthorityBinding(
                    delegated_authority=types_pb2.DelegationProof(
                        issuer_ura="easynet:///r/example/agent/authority",
                        subject_ura=RESOURCE_SUBJECT_URA,
                        caller_ura="easynet:///r/example/agent/alice.sdk",
                        audience=DESCRIPTOR_REF,
                        scopes=("invoke",),
                        issued_at_ms=1,
                        expires_at_ms=2,
                        signature=signature,
                    )
                ),
                "delegated_authority",
                "audience",
            ),
            (
                "capability",
                types_pb2.AuthorityBinding(
                    capability_grant=types_pb2.CapabilityGrant(
                        capability_ura="easynet:///r/example/resource/capability-1"
                    )
                ),
                "capability_grant",
                "capability_ura",
            ),
            (
                "policy",
                types_pb2.AuthorityBinding(
                    policy_grant=types_pb2.PolicyGrant(
                        policy_ura="easynet:///r/example/resource/policy-1"
                    )
                ),
                "policy_grant",
                "policy_ura",
            ),
            (
                "session",
                types_pb2.AuthorityBinding(
                    session_authority=types_pb2.SessionAuthority(
                        backend_ura="easynet:///r/example/agent/backend",
                        user_ura="easynet:///r/example/agent/alice",
                        session_id="session-1",
                        scopes=("invoke",),
                        audiences=(DESCRIPTOR_REF,),
                        issued_at_ms=1,
                        expires_at_ms=2,
                        signature=signature,
                    )
                ),
                "session_authority",
                "session_id",
            ),
            (
                "bootstrap",
                types_pb2.AuthorityBinding(
                    bootstrap_authority=types_pb2.BootstrapAuthority(
                        principal_ura="easynet:///r/example/agent/alice.sdk",
                        realm="example",
                        ability=DESCRIPTOR_REF,
                    )
                ),
                "bootstrap_authority",
                "realm",
            ),
        )
        for kind, binding, arm, required_field in cases:
            with self.subTest(kind=kind):
                receipt = _canonical_receipt(
                    index=1,
                    invocation_id=f"inv-authority-{kind}",
                    receipt_type="completed",
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    cleanup_complete=True,
                )
                receipt.authority_binding.CopyFrom(binding)
                receipt.authority_proof.binding.CopyFrom(binding)

                projection = _canonical_receipt_projection(receipt)

                self.assertEqual(projection["authority_binding_kind"], kind)
                if kind == "session":
                    binding_projection = cast(
                        dict[str, object], projection["authority_binding"]
                    )
                    self.assertEqual(
                        binding_projection["issuer_ura"],
                        "easynet:///r/example/agent/backend",
                    )
                    self.assertEqual(
                        binding_projection["subject_ura"],
                        "easynet:///r/example/agent/alice",
                    )
                    self.assertNotIn("backend_ura", binding_projection)
                    self.assertNotIn("user_ura", binding_projection)
                proof = cast(dict[str, object], projection["authority_proof"])
                self.assertEqual(proof["binding_kind"], kind)

                getattr(receipt.authority_binding, arm).ClearField(required_field)
                with self.assertRaises(SDKError) as raised:
                    _canonical_receipt_projection(receipt)
                self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_receipt_projection_rejects_authority_proof_binding_mismatch(
        self,
    ) -> None:
        receipt = _canonical_receipt(
            index=1,
            invocation_id="inv-authority-mismatch",
            receipt_type="completed",
            state=types_pb2.INVOCATION_STATE_COMPLETED,
            cleanup_complete=True,
        )
        receipt.authority_proof.binding.CopyFrom(
            types_pb2.AuthorityBinding(
                policy_grant=types_pb2.PolicyGrant(
                    policy_ura="easynet:///r/example/resource/policy-1"
                )
            )
        )

        with self.assertRaises(SDKError) as raised:
            _canonical_receipt_projection(receipt)

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))
        self.assertIn("does not match", raised.exception.message)

    def test_receipt_projection_rejects_incomplete_parent_receipt(self) -> None:
        for parent in (
            types_pb2.ReceiptRef(
                receipt_ura="easynet:///r/example/resource/job-41/receipt/terminal"
            ),
            types_pb2.ReceiptRef(receipt_hash=bytes.fromhex("81" * 32)),
        ):
            with self.subTest(parent=parent):
                receipt = _canonical_receipt(
                    index=1,
                    invocation_id="inv-incomplete-parent",
                    receipt_type="completed",
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    cleanup_complete=True,
                    parent_receipts=(parent,),
                )

                with self.assertRaises(SDKError) as raised:
                    _canonical_receipt_projection(receipt)

                self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_receipt_projection_requires_host_attestation_for_hosted_signer(
        self,
    ) -> None:
        receipt = _canonical_receipt(
            index=1,
            invocation_id="inv-hosted",
            receipt_type="completed",
            state=types_pb2.INVOCATION_STATE_COMPLETED,
            cleanup_complete=True,
        )
        receipt.signer_binding.CopyFrom(
            types_pb2.AgentIdentity(
                ura="easynet:///r/example/agent/runtime-host",
                profile="axon-strict-v2",
            )
        )

        with self.assertRaises(SDKError) as raised:
            _canonical_receipt_projection(receipt)

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

        receipt.host_attestation = bytes.fromhex("b1" * 64)
        projection = _canonical_receipt_projection(receipt)
        signer = cast(dict[str, object], projection["signer_binding"])
        self.assertEqual(
            signer["ura"],
            "easynet:///r/example/agent/runtime-host",
        )
        self.assertEqual(
            projection["host_attestation_base64"],
            base64.b64encode(receipt.host_attestation).decode("ascii"),
        )

    def test_unary_allows_only_typed_pre_admission_receipt_free_failures(
        self,
    ) -> None:
        allowed_stages = (
            types_pb2.ERROR_STAGE_GLOBAL_ADMISSION,
            types_pb2.ERROR_STAGE_CALLER_AUTHENTICATION,
            types_pb2.ERROR_STAGE_AUTHORITY_VALIDATION,
            types_pb2.ERROR_STAGE_BOOTSTRAP_AUTHORIZATION,
            types_pb2.ERROR_STAGE_QUOTA,
            types_pb2.ERROR_STAGE_ABILITY_RESOLUTION,
            types_pb2.ERROR_STAGE_ABILITY_POLICY,
            types_pb2.ERROR_STAGE_REQUEST_VALIDATION,
        )

        class PreAdmissionFailureServicer(RecordingInvocationServicer):
            stage = types_pb2.ERROR_STAGE_UNSPECIFIED

            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_FAILED,
                    error=types_pb2.Error(
                        code="AXON_ADMISSION_REJECTED",
                        message="rejected before admission",
                        stage=self.stage,
                    ),
                )

        servicer = PreAdmissionFailureServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                for stage in allowed_stages:
                    with self.subTest(stage=types_pb2.ErrorStage.Name(stage)):
                        servicer.stage = stage
                        result = json.loads(
                            transport.invoke(
                                _signed_draft().to_json().encode("utf-8")
                            ).decode("utf-8")
                        )
                        self.assertFalse(result["ok"])
                        self.assertEqual(result["terminal_state"], "Failed")
                        self.assertIsNone(result["admission_receipt"])
                        self.assertIsNone(result["terminal_receipt"])
            finally:
                transport.close()

        self.assertEqual(len(servicer.requests), len(allowed_stages))

    def test_unary_rejects_receipt_free_failures_outside_pre_admission(
        self,
    ) -> None:
        rejected_stages = (
            types_pb2.ERROR_STAGE_UNSPECIFIED,
            types_pb2.ERROR_STAGE_TRANSPORT,
            types_pb2.ERROR_STAGE_EXECUTION,
        )

        class ReceiptFreeFailureServicer(RecordingInvocationServicer):
            stage = types_pb2.ERROR_STAGE_UNSPECIFIED

            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_FAILED,
                    error=types_pb2.Error(
                        code="AXON_EXECUTION_FAILED",
                        message="receipt-free failure",
                        stage=self.stage,
                    ),
                )

        servicer = ReceiptFreeFailureServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                for stage in rejected_stages:
                    with self.subTest(stage=types_pb2.ErrorStage.Name(stage)):
                        servicer.stage = stage
                        with self.assertRaises(SDKError) as raised:
                            transport.invoke(_signed_draft().to_json().encode("utf-8"))
                        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))
            finally:
                transport.close()

    def test_unary_rejects_receipt_free_failure_with_proof_error(self) -> None:
        class ProofFailureServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_FAILED,
                    error=types_pb2.Error(
                        code="AXON_ADMISSION_REJECTED",
                        message="pre-admission failure",
                        stage=types_pb2.ERROR_STAGE_GLOBAL_ADMISSION,
                    ),
                    proof_error=types_pb2.Error(
                        code="AXON_RECEIPT_CONSTRUCTION_FAILED",
                        message="proof plane failed",
                        stage=types_pb2.ERROR_STAGE_EXECUTION,
                    ),
                )

        servicer = ProofFailureServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(_signed_draft().to_json().encode("utf-8"))
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_unary_rejects_partial_checkpoint_pairs(self) -> None:
        class PartialCheckpointServicer(RecordingInvocationServicer):
            admission = True
            terminal = False

            def Invoke(self, request, context):
                self.requests.append(request)
                response = invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                )
                if self.admission:
                    response.admission_receipt.CopyFrom(
                        _admission_receipt("inv-partial")
                    )
                if self.terminal:
                    response.terminal_receipt.CopyFrom(
                        _canonical_receipt(
                            index=1,
                            invocation_id="inv-partial",
                            receipt_type="completed",
                            state=types_pb2.INVOCATION_STATE_COMPLETED,
                            cleanup_complete=True,
                        )
                    )
                return response

        servicer = PartialCheckpointServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                for admission, terminal in ((True, False), (False, True)):
                    with self.subTest(admission=admission, terminal=terminal):
                        servicer.admission = admission
                        servicer.terminal = terminal
                        with self.assertRaises(SDKError) as raised:
                            transport.invoke(_signed_draft().to_json().encode("utf-8"))
                        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))
                        self.assertIn(
                            "partial checkpoint pair", raised.exception.message
                        )
            finally:
                transport.close()

    def test_unary_requires_complete_checkpoint_pair(self) -> None:
        class MissingReceiptServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                )

        servicer = MissingReceiptServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(_signed_draft().to_json().encode("utf-8"))
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))
        self.assertIn(
            "requires admission_receipt and terminal_receipt",
            raised.exception.message,
        )

    def test_unary_rejects_checkpoint_binding_mismatch(self) -> None:
        class MismatchedCheckpointServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    admission_receipt=_admission_receipt("inv-admission"),
                    terminal_receipt=_canonical_receipt(
                        index=1,
                        invocation_id="inv-terminal",
                        receipt_type="completed",
                        state=types_pb2.INVOCATION_STATE_COMPLETED,
                        cleanup_complete=True,
                    ),
                )

        servicer = MismatchedCheckpointServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(_signed_draft().to_json().encode("utf-8"))
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))
        self.assertIn("binding mismatch", raised.exception.message)

    def test_unary_receipt_projection_fails_closed_on_missing_schema_hash(
        self,
    ) -> None:
        class InvalidReceiptServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                receipt = _canonical_receipt(
                    index=1,
                    invocation_id="inv-invalid-unary",
                    receipt_type="completed",
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    cleanup_complete=True,
                )
                receipt.ClearField("schema_hash")
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    admission_receipt=_admission_receipt("inv-invalid-unary"),
                    terminal_receipt=receipt,
                )

        servicer = InvalidReceiptServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(_signed_draft().to_json().encode("utf-8"))
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_stream_receipt_projection_fails_closed_on_missing_impl_hash(
        self,
    ) -> None:
        receipt = _canonical_receipt(
            index=1,
            invocation_id="inv-invalid-stream",
            receipt_type="completed",
            state=types_pb2.INVOCATION_STATE_COMPLETED,
            cleanup_complete=True,
        )
        receipt.ClearField("impl_hash")
        servicer = RecordingInvocationServicer()
        servicer.stream_chunks = [
            invoke_pb2.InvokeStreamChunk(
                invocation_id="inv-invalid-stream",
                state=types_pb2.INVOCATION_STATE_COMPLETED,
                sequence=0,
                terminal=True,
                terminal_receipt=receipt,
            )
        ]
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                stream, _ = transport.open_stream(
                    _signed_draft().to_json().encode("utf-8")
                )
                with self.assertRaises(SDKError) as raised:
                    stream.recv(timeout=1)
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_stream_retains_complete_admission_receipt(self) -> None:
        servicer = RecordingInvocationServicer()
        servicer.stream_chunks[0].admission_receipt.CopyFrom(
            _canonical_receipt(
                index=0,
                invocation_id="inv-stream",
                receipt_type="admitted",
                state=types_pb2.INVOCATION_STATE_ADMITTED,
            )
        )
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                stream, _ = transport.open_stream(
                    _signed_draft().to_json().encode("utf-8")
                )
                event = json.loads(stream.recv(timeout=1).decode("utf-8"))
            finally:
                transport.close()

        receipt = cast(dict[str, object], event["admission_receipt"])
        self.assertEqual(receipt["receipt_type"], "admitted")
        _assert_complete_receipt_projection(self, receipt)

    def test_bidi_admission_receipt_fails_closed_before_internal_filtering(
        self,
    ) -> None:
        class InvalidAdmissionServicer(RecordingInvocationServicer):
            def InvokeBidi(self, request_iterator, context):
                self.bidi_up_frames.append(next(request_iterator))
                receipt = _canonical_receipt(
                    index=0,
                    invocation_id="inv-invalid-bidi",
                    receipt_type="admitted",
                    state=types_pb2.INVOCATION_STATE_ADMITTED,
                )
                receipt.ClearField("authority_proof")
                yield invoke_pb2.InvokeBidiDown(sequence=0, receipt=receipt)

        servicer = InvalidAdmissionServicer()
        streams = b'[{"stream_id":1,"content_type":"application/json"}]'
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                bidi, _ = transport.open_bidi(
                    _signed_draft().to_json().encode("utf-8"),
                    streams,
                )
                with self.assertRaises(SDKError) as raised:
                    bidi.recv(timeout=1)
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))

    def test_direct_runtime_stream_provider_json_uses_terminal_receipt(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                stream_transport, _ = transport.open_stream(
                    _signed_draft().to_json().encode("utf-8")
                )
                raw_first = stream_transport.recv(timeout=1)
                raw_terminal = stream_transport.recv(timeout=1)
            finally:
                transport.close()

        first = json.loads(raw_first.decode("utf-8"))
        self.assertFalse(first["terminal"])
        self.assertEqual(first["state"], "Running")
        terminal = json.loads(raw_terminal.decode("utf-8"))
        self.assertNotIn("receipt", terminal)
        receipt = cast(dict[str, object], terminal["terminal_receipt"])
        self.assertEqual(receipt["invocation_id"], "inv-stream")
        _assert_complete_receipt_projection(self, receipt)

    def test_direct_runtime_stream_deadline_is_typed_timeout(self) -> None:
        servicer = RecordingInvocationServicer()
        servicer.stream_delay_seconds = 0.2
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=0.05,
                identity=_identity(),
            )
            try:
                stream_transport, _ = transport.open_stream(
                    _signed_draft().to_json().encode("utf-8")
                )
                with self.assertRaises(SDKError) as raised:
                    stream_transport.recv(timeout=1)
                stream_transport.close()

                servicer.stream_delay_seconds = 0.0
                retry_stream, _ = transport.open_stream(
                    _signed_draft().to_json().encode("utf-8")
                )
                retry_event = json.loads(retry_stream.recv(timeout=1).decode("utf-8"))
                retry_stream.close()
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.TIMEOUT))
        self.assertEqual(raised.exception.stage, "direct_runtime")
        self.assertEqual(
            raised.exception.details["grpc_status"],
            str(grpc.StatusCode.DEADLINE_EXCEEDED),
        )
        self.assertGreaterEqual(len(servicer.stream_requests), 2)
        self.assertFalse(retry_event["terminal"])

    def test_direct_runtime_stream_cancel_is_explicitly_unsupported(
        self,
    ) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                stream_transport, _ = transport.open_stream(
                    _signed_draft().to_json().encode("utf-8")
                )
                with self.assertRaises(SDKError) as raised:
                    stream_transport.cancel("client stop")
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(raised.exception.details["capability"], "stream_cancel")

    def test_direct_runtime_bidi_provider_json_uses_terminal_receipt(self) -> None:
        servicer = RecordingInvocationServicer()
        streams_json = json.dumps(
            [{"stream_id": 1, "content_type": "application/json"}],
            separators=(",", ":"),
        ).encode("utf-8")
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                bidi_transport, _ = transport.open_bidi(
                    _signed_draft().to_json().encode("utf-8"),
                    streams_json,
                )
                bidi_transport.send(
                    b'{"sequence":1,"kind":"data","stream_id":1,'
                    b'"payload_base64":"eyJwaW5nIjp0cnVlfQ=="}'
                )
                raw_frame = bidi_transport.recv(timeout=1)
                bidi_transport.close_send()
                raw_terminal = bidi_transport.recv(timeout=1)
            finally:
                transport.close()

        frame = json.loads(raw_frame.decode("utf-8"))
        self.assertFalse(frame["terminal"])
        self.assertEqual(frame["kind"], "data")
        self.assertEqual(frame["stream_id"], 1)
        self.assertGreaterEqual(len(servicer.bidi_up_frames), 2)
        self.assertEqual(
            servicer.bidi_up_frames[0].WhichOneof("payload"),
            "envelope_open",
        )
        terminal = json.loads(raw_terminal.decode("utf-8"))
        self.assertNotIn("receipt", terminal)
        payload = cast(dict[str, object], terminal.get("payload_json") or {})
        self.assertNotIn("receipt", payload)
        receipt = cast(dict[str, object], terminal["terminal_receipt"])
        self.assertEqual(receipt["invocation_id"], "inv-bidi")
        _assert_complete_receipt_projection(self, receipt)

    def test_direct_runtime_bidi_deadline_is_typed_timeout(self) -> None:
        servicer = RecordingInvocationServicer()
        servicer.bidi_delay_seconds = 0.2
        streams_json = json.dumps(
            [{"stream_id": 1, "content_type": "application/json"}],
            separators=(",", ":"),
        ).encode("utf-8")
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=0.05,
                identity=_identity(),
            )
            try:
                bidi_transport, _ = transport.open_bidi(
                    _signed_draft().to_json().encode("utf-8"),
                    streams_json,
                )
                with self.assertRaises(SDKError) as raised:
                    bidi_transport.recv(timeout=1)
                bidi_transport.close()

                servicer.bidi_delay_seconds = 0.0
                retry_bidi, _ = transport.open_bidi(
                    _signed_draft().to_json().encode("utf-8"),
                    streams_json,
                )
                retry_bidi.send(
                    b'{"sequence":1,"kind":"data","stream_id":1,'
                    b'"payload_base64":"eyJwaW5nIjp0cnVlfQ=="}'
                )
                retry_frame = json.loads(retry_bidi.recv(timeout=1).decode("utf-8"))
                retry_bidi.close()
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.TIMEOUT))
        self.assertEqual(raised.exception.stage, "direct_runtime")
        self.assertEqual(
            raised.exception.details["grpc_status"],
            str(grpc.StatusCode.DEADLINE_EXCEEDED),
        )
        self.assertGreaterEqual(len(servicer.bidi_up_frames), 2)
        self.assertFalse(retry_frame["terminal"])

    def test_direct_runtime_bidi_cancel_is_explicitly_unsupported(
        self,
    ) -> None:
        servicer = RecordingInvocationServicer()
        streams_json = json.dumps(
            [{"stream_id": 1, "content_type": "application/json"}],
            separators=(",", ":"),
        ).encode("utf-8")
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                bidi_transport, _ = transport.open_bidi(
                    _signed_draft().to_json().encode("utf-8"),
                    streams_json,
                )
                with self.assertRaises(SDKError) as raised:
                    bidi_transport.cancel("client stop")
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(raised.exception.details["capability"], "bidi_cancel")

    def test_direct_bidi_rejects_missing_frame0_before_session_entry(self) -> None:
        class Stub:
            called = False

            def InvokeBidi(self, request_iterator: Any, *, timeout: float) -> Any:
                self.called = True
                return iter(())

        stub = Stub()
        session = DirectRuntimeBidiTransport(endpoint="unix:///direct-test")
        with self.assertRaises(SDKError) as raised:
            session.start(stub, None, timeout_seconds=1)

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertFalse(stub.called)

        bad_open = invoke_pb2.InvokeBidiUp(
            sequence=1,
            control=invoke_pb2.BidiControl(eof=True),
        )
        with self.assertRaises(SDKError) as raised:
            session.start(stub, bad_open, timeout_seconds=1)

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertFalse(stub.called)

    def test_direct_transport_rejects_non_contiguous_bidi_up_sequence(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = RuntimeInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                bidi = transport.bidi(
                    _signed_draft(),
                    ({"stream_id": 1, "content_type": "application/json"},),
                )
                with self.assertRaises(SDKError) as raised:
                    bidi.send({"sequence": 2, "kind": "data", "stream_id": 1})
                bidi.close()
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(raised.exception.stage, "direct_runtime")

    def test_direct_transport_rejects_empty_bidi_streams_before_wire_call(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.open_bidi(
                        complete_draft().to_json().encode("utf-8"),
                        b"[]",
                    )
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_INVOCATION))
        self.assertEqual(raised.exception.stage, "direct_runtime")
        self.assertEqual(servicer.bidi_up_frames, [])

    def test_direct_transport_reports_unsupported_modes_explicitly(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.prepare(
                        complete_draft().to_json().encode("utf-8"),
                        b"{}",
                    )
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(raised.exception.stage, "direct_runtime")

    def test_direct_transport_maps_missing_endpoint_to_runtime_offline(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            endpoint = str(Path(tmp) / "missing.sock")
            with self.assertRaises(SDKError) as raised:
                DirectRuntimeTransport.open(
                    endpoint,
                    dial_timeout_seconds=0.05,
                    identity=_identity(),
                )

        self.assertTrue(is_code(raised.exception, ErrorCode.RUNTIME_OFFLINE))

    def test_direct_transport_requires_identity_projection_before_open(self) -> None:
        with self.assertRaises(SDKError) as raised:
            DirectRuntimeTransport.open(
                "/tmp/direct-runtime-unused.sock",
                dial_timeout_seconds=0.05,
            )

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(raised.exception.stage, "direct_runtime")

    def test_direct_transport_rejects_use_after_close(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            transport.close()
            with self.assertRaises(SDKError) as raised:
                transport.invoke(complete_draft().to_json().encode("utf-8"))

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_HANDLE))


class _fake_daemon:
    def __init__(self, servicer: RecordingInvocationServicer) -> None:
        self._servicer = servicer
        self._tmp = tempfile.TemporaryDirectory()
        self._server = grpc.server(concurrent.futures.ThreadPoolExecutor(max_workers=4))
        self.endpoint = str(Path(self._tmp.name) / "daemon.sock")

    def __enter__(self) -> str:
        invoke_pb2_grpc.add_InvocationServicer_to_server(
            self._servicer,
            self._server,
        )
        port = self._server.add_insecure_port(f"unix:{self.endpoint}")
        if port != 1:
            raise RuntimeError(f"failed to bind fake daemon UDS: {port}")
        self._server.start()
        return self.endpoint

    def __exit__(self, *exc_info: object) -> None:
        self._server.stop(0).wait()
        self._tmp.cleanup()


class _RecordingIdentity:
    def __init__(self, *, owner_ura: str = CALLEE_URA) -> None:
        self.owner_ura = owner_ura
        self.descriptor_refs: list[str] = []
        self.ability_uras: list[str] = []
        self.close_count = 0

    def ability_ura_from_descriptor_ref(self, descriptor_ref: str) -> str:
        self.descriptor_refs.append(descriptor_ref)
        if descriptor_ref != DESCRIPTOR_REF:
            raise AssertionError(f"unexpected descriptor_ref: {descriptor_ref}")
        return ABILITY_URA

    def project_ability_ura(self, ability_ura: str) -> AddressingProjection:
        self.ability_uras.append(ability_ura)
        if ability_ura != ABILITY_URA:
            raise AssertionError(f"unexpected ability_ura: {ability_ura}")
        return AddressingProjection(
            kind="ability",
            valid=True,
            profile="axon-strict-v2",
            ura=ability_ura,
            ability_ura=ability_ura,
            components={
                "owner_ura": self.owner_ura,
                "owner_kind": "device",
                "public_name": ABILITY_PUBLIC_NAME,
                "local_registry_ability": ABILITY_PUBLIC_NAME,
                "namespace": "observe",
                "local_name": "health",
            },
            metadata={"grammar_owner": "axon"},
        )

    def close(self) -> None:
        self.close_count += 1


def _identity(*, owner_ura: str = CALLEE_URA) -> _RecordingIdentity:
    return _RecordingIdentity(owner_ura=owner_ura)


class _DirectAbilityRuntimeTransport:
    def __init__(self, delegate: DirectRuntimeTransport) -> None:
        self._delegate = delegate

    def __getattr__(self, name: str) -> Any:
        return getattr(self._delegate, name)

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if request != {
            "ability": ABILITY_URA,
            "callee_ura": CALLEE_URA,
            "call_mode": "rpc",
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "subject_ura": CALLEE_URA,
        }:
            raise AssertionError(f"unexpected ability descriptor request: {request}")
        return json.dumps(
            {"descriptor_ref": DESCRIPTOR_REF},
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")


def _ability_addressing_transport() -> MemoryAddressingTransport:
    transport = MemoryAddressingTransport()
    transport.identity_json = (
        b'{"kind":"ability","valid":true,'
        b'"ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
        b'"profile":"axon-strict-v2",'
        b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
        b'"owner_kind":"device","public_name":"observe.health",'
        b'"local_registry_ability":"observe.health",'
        b'"namespace":"observe","local_name":"health"},'
        b'"metadata":{"grammar_owner":"axon"}}'
    )
    transport.descriptor_json = (
        b'{"kind":"descriptor_ref","valid":true,'
        b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
        b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
        b'"descriptor_version":"1.0.0","profile":"axon-strict-v2",'
        b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
        b'"metadata":{"grammar_owner":"axon"}}'
    )
    return transport


def _ability_request() -> AbilityCallRequest:
    return AbilityCallRequest(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura=CALLEE_URA,
        subject_ura=CALLEE_URA,
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        ability_ura=ABILITY_URA,
        args={"city": "Singapore"},
        caller_signature=_caller_signature(),
    )


def _user_subject_draft_json() -> bytes:
    draft = _user_subject_draft_dict()
    return json.dumps(draft, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _user_subject_draft_dict() -> dict[str, object]:
    draft = _signed_draft().to_json_dict()
    draft["subject_ura"] = USER_SUBJECT_URA
    return draft


class _RecordingHandleTransport:
    def __init__(self) -> None:
        self.calls: list[tuple[Any, ...]] = []
        self.close_count = 0

    def invoke(self, draft_json: bytes) -> bytes:
        raise AssertionError("handle delegate must not receive unary invoke")

    def open_stream(self, draft_json: bytes):
        raise AssertionError("handle delegate must not receive open_stream")

    def open_bidi(self, draft_json: bytes, streams_json: bytes):
        raise AssertionError("handle delegate must not receive open_bidi")

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        self.calls.append(("prepare", draft_json, options_json))
        return b'{"prepared":true}'

    def submit_signed(self, signed_json: bytes) -> bytes:
        self.calls.append(("submit_signed", signed_json))
        return b'{"handle_id":7,"state":"Submitted"}'

    def await_handle(self, control) -> bytes:
        self.calls.append(("await_handle", control._adapter_handle_id()))
        return b'{"ok":true,"terminal_state":"Completed"}'

    def cancel_handle(self, control, reason: str) -> bytes:
        self.calls.append(("cancel_handle", control._adapter_handle_id(), reason))
        return b'{"handle_id":7,"cancelled":true}'

    def handle_events(self, control) -> bytes:
        self.calls.append(("handle_events", control._adapter_handle_id()))
        return b'{"handle_id":7,"events":[]}'

    def free_handle(self, control) -> None:
        self.calls.append(("free_handle", control._adapter_handle_id()))

    def close(self) -> None:
        self.close_count += 1
