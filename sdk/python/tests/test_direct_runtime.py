import concurrent.futures
import json
import tempfile
import unittest
from pathlib import Path
from typing import Any, cast

import grpc

from easynet_sdk import (
    ConnectOptions,
    DaemonInvocationTransport,
    ErrorCode,
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

from test_runtime import complete_draft

invoke_pb2: Any = _invoke_pb2
invoke_pb2_grpc: Any = _invoke_pb2_grpc
types_pb2: Any = _types_pb2


class RecordingInvocationServicer(invoke_pb2_grpc.InvocationServicer):
    def __init__(self) -> None:
        self.requests: list[Any] = []
        self.stream_requests: list[Any] = []
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

    def test_direct_connector_handshake_reports_unary_only_capabilities(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            connector = DirectDaemonRuntimeConnector()
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
        self.assertEqual(facts["bidi"], False)
        self.assertEqual(facts["prepare"], False)
        self.assertEqual(facts["submit_signed"], False)

    def test_direct_transport_invokes_daemon_over_axon_grpc_uds(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                )
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
        self.assertEqual(request.function_name, complete_draft().descriptor_ref)
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

    def test_direct_transport_rejects_non_string_metadata_before_wire_call(
        self,
    ) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectDaemonRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
            )
            try:
                draft = complete_draft().to_json_dict()
                draft["metadata"] = {"attempt": 1}
                with self.assertRaises(SDKError) as raised:
                    transport.invoke(
                        json.dumps(draft, separators=(",", ":")).encode("utf-8")
                    )
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_INVOCATION))
        self.assertEqual(len(servicer.requests), 0)

    def test_direct_transport_streams_daemon_chunks_over_axon_grpc_uds(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DaemonInvocationTransport.connect_direct(
                options=ConnectOptions(
                    endpoint=endpoint,
                    dial_timeout_ms=1000,
                    invoke_timeout_ms=1000,
                )
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
        self.assertEqual(request.function_name, complete_draft().descriptor_ref)
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
                )
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

    def test_direct_transport_reports_unsupported_modes_explicitly(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectDaemonRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
            )
            try:
                with self.assertRaises(SDKError) as raised:
                    transport.open_bidi(
                        complete_draft().to_json().encode("utf-8"),
                        b"[]",
                    )
            finally:
                transport.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(raised.exception.stage, "direct_runtime")

    def test_direct_transport_maps_missing_endpoint_to_daemon_offline(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            endpoint = str(Path(tmp) / "missing.sock")
            with self.assertRaises(SDKError) as raised:
                DirectDaemonRuntimeTransport.open(endpoint, dial_timeout_seconds=0.05)

        self.assertTrue(is_code(raised.exception, ErrorCode.DAEMON_OFFLINE))

    def test_direct_transport_rejects_use_after_close(self) -> None:
        servicer = RecordingInvocationServicer()
        with _fake_daemon(servicer) as endpoint:
            transport = DirectDaemonRuntimeTransport.open(
                endpoint,
                dial_timeout_seconds=1,
                invoke_timeout_seconds=1,
            )
            transport.close()
            with self.assertRaises(SDKError) as raised:
                transport.invoke(complete_draft().to_json().encode("utf-8"))

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_HANDLE))


class _fake_daemon:
    def __init__(self, servicer: RecordingInvocationServicer) -> None:
        self._servicer = servicer
        self._tmp = tempfile.TemporaryDirectory()
        self._server = grpc.server(concurrent.futures.ThreadPoolExecutor(max_workers=1))
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
