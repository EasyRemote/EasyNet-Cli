import concurrent.futures
import json
import tempfile
import unittest
from collections.abc import Iterator
from dataclasses import replace
from pathlib import Path
from typing import Any, cast

import grpc

from easynet_sdk import (
    ConnectOptions,
    DaemonInvocationTransport,
    ErrorCode,
    InvocationSignature,
    SDKError,
    is_code,
)
from easynet_sdk._axon_pb.axon.v1 import (
    invoke_pb2 as _invoke_pb2,
    invoke_pb2_grpc as _invoke_pb2_grpc,
    types_pb2 as _types_pb2,
)
from easynet_sdk.control_ipc import ControlDiscovery, IpcVersionRange
from easynet_sdk.direct_runtime import (
    DirectDaemonRuntimeConnector,
    DirectDaemonRuntimeTransport,
)
from easynet_sdk.identity import AbilityAddress

from test_runtime import complete_draft

invoke_pb2: Any = _invoke_pb2
invoke_pb2_grpc: Any = _invoke_pb2_grpc
types_pb2: Any = _types_pb2

DESCRIPTOR_REF = "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
ABILITY_URA = "easynet:///r/example/ability/device.dev-a.observe.health"
ABILITY_PUBLIC_NAME = "observe.health"
CALLEE_URA = "easynet:///r/example/device/dev-a"
USER_SUBJECT_URA = "easynet:///r/example/user/alice"
PROJECTED_USER_SUBJECT_URA = (
    "easynet:///r/example/resource/user.alice/invoke/observe.health"
)


class RecordingInvocationServicer(invoke_pb2_grpc.InvocationServicer):
    def __init__(self) -> None:
        self.requests: list[Any] = []
        self.stream_requests: list[Any] = []
        self.bidi_up_frames: list[Any] = []
        self.stream_chunks: list[Any] = [
            invoke_pb2.InvokeStreamChunk(
                invocation_id="inv-stream",
                state=types_pb2.INVOCATION_STATE_RUNNING,
                selected_node_id="node-direct",
                scheduling_reason="fake-stream",
                payload=b'{"chunk":1}',
                content_type="application/json",
                sequence=0,
                terminal=False,
            ),
            invoke_pb2.InvokeStreamChunk(
                invocation_id="inv-stream",
                state=types_pb2.INVOCATION_STATE_COMPLETED,
                selected_node_id="node-direct",
                scheduling_reason="fake-stream",
                payload=b'{"done":true}',
                content_type="application/json",
                sequence=1,
                terminal=True,
                elapsed_ms=11,
                terminal_receipt=invoke_pb2.InvocationReceipt(
                    index=1,
                    invocation_id="inv-stream",
                    receipt_type="terminal",
                    state=types_pb2.INVOCATION_STATE_COMPLETED,
                    timestamp_unix_ms=1783100000123,
                    self_hash=bytes.fromhex("22" * 32),
                    cleanup_complete=True,
                ),
            ),
        ]

    def Invoke(self, request, context):
        self.requests.append(request)
        return invoke_pb2.InvokeResponse(
            state=types_pb2.INVOCATION_STATE_COMPLETED,
            selected_node_id="node-direct",
            scheduling_reason="fake-daemon",
            result=b'{"ready":true}',
            result_content_type="application/json",
            elapsed_ms=9,
            terminal_receipt=invoke_pb2.InvocationReceipt(
                index=1,
                invocation_id="inv-direct",
                receipt_type="terminal",
                state=types_pb2.INVOCATION_STATE_COMPLETED,
                timestamp_unix_ms=1783100000123,
                self_hash=bytes.fromhex("11" * 32),
                cleanup_complete=True,
            ),
        )

    def InvokeStream(self, request, context):
        self.stream_requests.append(request)
        yield from self.stream_chunks

    def InvokeBidi(self, request_iterator, context) -> Iterator[Any]:
        first = next(request_iterator)
        self.bidi_up_frames.append(first)
        yield invoke_pb2.InvokeBidiDown(
            sequence=0,
            receipt=invoke_pb2.InvocationReceipt(
                index=1,
                invocation_id="inv-bidi",
                receipt_type="admission",
                state=types_pb2.INVOCATION_STATE_ACCEPTED,
                timestamp_unix_ms=1783100000123,
                self_hash=bytes.fromhex("33" * 32),
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
                    receipt=invoke_pb2.InvocationReceipt(
                        index=2,
                        invocation_id="inv-bidi",
                        receipt_type="terminal",
                        state=types_pb2.INVOCATION_STATE_COMPLETED,
                        timestamp_unix_ms=1783100000456,
                        self_hash=bytes.fromhex("44" * 32),
                        cleanup_complete=True,
                    ),
                )
                return


class DirectRuntimeTests(unittest.TestCase):
    def test_direct_connector_resolves_invocation_endpoint_from_discovery(self) -> None:
        connector = DirectDaemonRuntimeConnector(
            control_path="/tmp/control.json",
            discovery_reader=lambda path: ControlDiscovery(
                socket_path="/tmp/control.sock",
                invocation_endpoint="/tmp/invoke.sock",
                daemon_version="1.2.3",
                supported_ipc_versions=IpcVersionRange(1, 1),
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
        self.assertEqual(resolved["control_endpoint"], "/tmp/control.sock")
        self.assertEqual(resolved["daemon_version"], "1.2.3")
        self.assertEqual(
            resolved["capability_flags"],
            ["runtime.invoke", "direct.grpc"],
        )
        self.assertEqual(resolved["dial_timeout_ms"], 500)

    def test_direct_connector_reports_control_only_without_invocation_endpoint(
        self,
    ) -> None:
        connector = DirectDaemonRuntimeConnector(
            discovery_reader=lambda path: ControlDiscovery(
                socket_path="/tmp/control.sock",
                supported_ipc_versions=IpcVersionRange(1, 1),
            )
        )

        with self.assertRaises(SDKError) as raised:
            connector.resolve(b"{}")

        self.assertTrue(is_code(raised.exception, ErrorCode.CONTROL_ONLY))
        self.assertEqual(raised.exception.stage, "direct_runtime.resolve")

    def test_direct_connector_handshake_reports_runtime_capabilities(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            connector = DirectDaemonRuntimeConnector(identity=_identity())
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
            connector = DirectDaemonRuntimeConnector(
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
                awaited = transport.await_handle(7)
                cancelled = transport.cancel_handle(7, "stop")
                events = transport.handle_events(7)
                transport.free_handle(7)
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

    def test_direct_connector_projects_delegated_prepare_subject(self) -> None:
        servicer = RecordingInvocationServicer()
        handle_transport = _RecordingHandleTransport()
        draft_json = _user_subject_draft_json()
        with _fake_daemon(servicer) as endpoint:
            connector = DirectDaemonRuntimeConnector(
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
                transport.prepare(draft_json, b"{}")
            finally:
                transport.close()
                connector.close()

        _, projected_json, _ = handle_transport.calls[0]
        projected = json.loads(projected_json.decode("utf-8"))
        self.assertEqual(projected["subject_ura"], PROJECTED_USER_SUBJECT_URA)

    def test_direct_connector_closes_owned_handle_transport_once(self) -> None:
        servicer = RecordingInvocationServicer()
        handle_transport = _RecordingHandleTransport()
        identity = _identity()
        with _fake_daemon(servicer) as endpoint:
            connector = (
                DirectDaemonRuntimeConnector()
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
            transport = DirectDaemonRuntimeTransport.open(
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
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=identity,
            )
            try:
                result = transport.invoke(complete_draft())
            finally:
                transport.close()

        self.assertEqual(result["ok"], True)
        self.assertEqual(result["terminal_state"], "Completed")
        self.assertEqual(result["output_json"], {"ready": True})
        self.assertEqual(result["selected_node_id"], "node-direct")
        receipt = cast(dict[str, object], result["receipt"])
        self.assertEqual(receipt["invocation_id"], "inv-direct")

        self.assertEqual(len(servicer.requests), 1)
        request = servicer.requests[0]
        self.assertEqual(request.function_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(request.target.ability_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(request.target.WhichOneof("typed_target"), "ability")
        self.assertEqual(request.target.ability.ability_name, ABILITY_PUBLIC_NAME)
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

    def test_direct_transport_projects_user_subject_before_daemon_invoke(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                result = transport.invoke(_user_subject_draft_dict())
            finally:
                transport.close()

        self.assertEqual(len(servicer.requests), 1)
        self.assertEqual(
            servicer.requests[0].envelope.subject.ura,
            PROJECTED_USER_SUBJECT_URA,
        )
        tuple_json = cast(dict[str, object], result["tuple"])
        self.assertEqual(tuple_json["subject_ura"], PROJECTED_USER_SUBJECT_URA)

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
            transport = DaemonInvocationTransport.connect_direct(
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

    def test_direct_transport_rejects_descriptor_not_owned_by_callee(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectDaemonRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(owner_ura="easynet:///r/example/device/other"),
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(complete_draft().to_json().encode("utf-8"))
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_INVOCATION))
        self.assertEqual(servicer.requests, [])

    def test_direct_transport_projects_failed_terminal_state_to_admission_denied(self) -> None:
        class FailedServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_FAILED,
                    elapsed_ms=4,
                )

        servicer = FailedServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                result = transport.invoke(complete_draft())
            finally:
                transport.close()

        error = cast(dict[str, object], result["error"])
        self.assertEqual(result["ok"], False)
        self.assertEqual(result["terminal_state"], "Failed")
        self.assertEqual(error["code"], ErrorCode.ADMISSION_DENIED.value)

    def test_direct_transport_projects_cancelled_terminal_state_to_cancelled(self) -> None:
        class CancelledServicer(RecordingInvocationServicer):
            def Invoke(self, request, context):
                self.requests.append(request)
                return invoke_pb2.InvokeResponse(
                    state=types_pb2.INVOCATION_STATE_CANCELLED,
                    elapsed_ms=4,
                )

        servicer = CancelledServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                result = transport.invoke(complete_draft())
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
            transport = DirectDaemonRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
                identity=_identity(),
            )
            try:
                draft = complete_draft().to_json_dict()
                draft["metadata"] = {
                    "attempt": 1,
                    "dry_run": False,
                    "shape": {"b": 2, "a": 1},
                    "empty": None,
                }
                transport.invoke(json.dumps(draft, separators=(",", ":")).encode("utf-8"))
            finally:
                transport.close()

        self.assertEqual(len(servicer.requests), 1)
        self.assertEqual(servicer.requests[0].metadata["attempt"], "1")
        self.assertEqual(servicer.requests[0].metadata["dry_run"], "false")
        self.assertEqual(servicer.requests[0].metadata["shape"], '{"a":1,"b":2}')
        self.assertNotIn("empty", servicer.requests[0].metadata)
        self.assertEqual(
            servicer.requests[0].metadata["x-easynet-signed-descriptor-ref"],
            DESCRIPTOR_REF,
        )

    def test_direct_transport_streams_daemon_chunks_over_axon_grpc_uds(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                ),
                identity=_identity(),
            )
            try:
                stream = transport.stream(complete_draft())
                first = stream.recv()
                terminal = stream.recv()
                stream.close()
            finally:
                transport.close()

        self.assertEqual(first["sequence"], 1)
        self.assertEqual(first["kind"], "chunk")
        self.assertEqual(first["payload_json"], {"chunk": 1})
        self.assertFalse(first["terminal"])
        self.assertEqual(terminal["sequence"], 2)
        self.assertEqual(terminal["kind"], "terminal")
        self.assertEqual(terminal["payload_json"], {"done": True})
        self.assertTrue(terminal["terminal"])
        self.assertEqual(len(servicer.stream_requests), 1)
        request = servicer.stream_requests[0]
        self.assertEqual(request.function_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(request.target.ability_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(request.target.WhichOneof("typed_target"), "ability")
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
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                ),
                identity=_identity(),
            )
            try:
                stream = transport.stream(complete_draft())
                event = stream.recv(timeout=1)
                stream.close()
            finally:
                transport.close()

        self.assertEqual(event["sequence"], 1)
        self.assertEqual(event["kind"], "chunk")
        self.assertFalse(event["terminal"])

    def test_direct_transport_opens_bidi_over_axon_grpc_uds(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                ),
                identity=_identity(),
            )
            try:
                bidi = transport.bidi(
                    complete_draft(),
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
        self.assertEqual(outcome["state"], "HalfClosedLocal")
        self.assertFalse(outcome["terminal"])
        self.assertEqual(terminal["sequence"], 3)
        self.assertEqual(terminal["kind"], "terminal")
        self.assertTrue(terminal["terminal"])

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
        self.assertEqual(envelope_open.target.ability_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(envelope_open.target.WhichOneof("typed_target"), "ability")
        self.assertEqual(envelope_open.target.ability.ability_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(envelope_open.target.ability.function_name, ABILITY_PUBLIC_NAME)
        self.assertEqual(envelope_open.initial_args, b"{}")
        self.assertEqual(envelope_open.args_content_type, "application/json")
        self.assertEqual(envelope_open.content_envelope.encoding, "identity")
        self.assertEqual(len(envelope_open.streams), 1)
        self.assertEqual(envelope_open.streams[0].stream_id, 1)
        self.assertEqual(envelope_open.streams[0].content_type, "application/json")
        self.assertEqual(envelope_open.streams[0].ordering, "STRICT")
        self.assertEqual(servicer.bidi_up_frames[1].sequence, 1)
        self.assertEqual(servicer.bidi_up_frames[1].WhichOneof("payload"), "binary_chunk")
        self.assertEqual(servicer.bidi_up_frames[2].sequence, 2)
        self.assertEqual(servicer.bidi_up_frames[2].WhichOneof("payload"), "control")
        self.assertEqual(servicer.bidi_up_frames[2].control.WhichOneof("control"), "eof")

    def test_direct_transport_rejects_non_contiguous_bidi_up_sequence(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=5000,
                ),
                identity=_identity(),
            )
            try:
                bidi = transport.bidi(
                    complete_draft(),
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
            transport = DirectDaemonRuntimeTransport.open(
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
            transport = DirectDaemonRuntimeTransport.open(
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

    def test_direct_transport_maps_missing_endpoint_to_daemon_offline(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            endpoint = str(Path(tmp) / "missing.sock")
            with self.assertRaises(SDKError) as raised:
                DirectDaemonRuntimeTransport.open(
                    endpoint,
                    dial_timeout_seconds=0.05,
                    identity=_identity(),
                )

        self.assertTrue(is_code(raised.exception, ErrorCode.DAEMON_OFFLINE))

    def test_direct_transport_requires_identity_projection_before_open(self) -> None:
        with self.assertRaises(SDKError) as raised:
            DirectDaemonRuntimeTransport.open(
                "/tmp/direct-runtime-unused.sock",
                dial_timeout_seconds=0.05,
            )

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(raised.exception.stage, "direct_runtime")

    def test_direct_transport_rejects_use_after_close(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectDaemonRuntimeTransport.open(
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

    def ability_address(self, ability_ura: str) -> AbilityAddress:
        self.ability_uras.append(ability_ura)
        if ability_ura != ABILITY_URA:
            raise AssertionError(f"unexpected ability_ura: {ability_ura}")
        return AbilityAddress(
            ability_ura=ability_ura,
            owner_ura=self.owner_ura,
            owner_kind="device",
            public_name=ABILITY_PUBLIC_NAME,
            subject_ura=ability_ura,
            local_registry_ability=ABILITY_PUBLIC_NAME,
            namespace="observe",
            local_name="health",
            profile="easynet-strict-v2",
            metadata={"grammar_owner": "axon"},
        )

    def descriptor_bound_resource_subject_ura(self, owner_ura: str, path: str) -> str:
        if owner_ura != USER_SUBJECT_URA:
            raise AssertionError(f"unexpected descriptor-bound owner: {owner_ura}")
        if path != "invoke/observe.health":
            raise AssertionError(f"unexpected descriptor-bound path: {path}")
        return PROJECTED_USER_SUBJECT_URA

    def close(self) -> None:
        self.close_count += 1


def _identity(*, owner_ura: str = CALLEE_URA) -> _RecordingIdentity:
    return _RecordingIdentity(owner_ura=owner_ura)


def _user_subject_draft_json() -> bytes:
    draft = _user_subject_draft_dict()
    return json.dumps(draft, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _user_subject_draft_dict() -> dict[str, object]:
    draft = complete_draft().to_json_dict()
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

    def await_handle(self, handle_id: int) -> bytes:
        self.calls.append(("await_handle", handle_id))
        return b'{"ok":true,"terminal_state":"Completed"}'

    def cancel_handle(self, handle_id: int, reason: str) -> bytes:
        self.calls.append(("cancel_handle", handle_id, reason))
        return b'{"handle_id":7,"cancelled":true}'

    def handle_events(self, handle_id: int) -> bytes:
        self.calls.append(("handle_events", handle_id))
        return b'{"handle_id":7,"events":[]}'

    def free_handle(self, handle_id: int) -> None:
        self.calls.append(("free_handle", handle_id))

    def close(self) -> None:
        self.close_count += 1
