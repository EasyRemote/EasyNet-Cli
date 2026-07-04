import unittest

from easynet_sdk import (
    BidiState,
    DaemonInvocationTransport,
    ErrorCode,
    RuntimeClient,
    SDKError,
    StreamState,
    is_code,
)

from test_runtime import MemoryRuntimeTransport, complete_draft


class DaemonInvocationTransportTests(unittest.TestCase):
    def test_invoke_accepts_complete_invocation_mapping(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        result = transport.invoke(complete_draft().to_json_dict())

        self.assertTrue(result["ok"])
        self.assertEqual(result["output_json"], {"ready": True})
        self.assertEqual(result["terminal_state"], "Completed")
        assert runtime.seen_draft is not None
        self.assertEqual(
            runtime.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

    def test_stream_projects_sdk_events_to_dicts(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        stream = transport.stream(complete_draft())
        event = stream.recv()
        stream.close()

        self.assertTrue(event["terminal"])
        self.assertEqual(event["kind"], "terminal")

    def test_stream_timeout_is_forwarded_without_state_mutation(self) -> None:
        class TimeoutRuntimeTransport(MemoryRuntimeTransport):
            def open_stream(self, draft_json: bytes):
                from test_stream import MemoryStreamTransport

                class TimeoutStreamTransport(MemoryStreamTransport):
                    def recv(self, timeout: float | None = None) -> bytes:
                        self.timeout = timeout
                        raise TimeoutError("no frame")

                self.stream_transport = TimeoutStreamTransport()
                return (
                    self.stream_transport,
                    b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
                )

        runtime = TimeoutRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        stream = transport.stream(complete_draft())
        with self.assertRaises(TimeoutError):
            stream.recv(timeout=0.01)

        self.assertEqual(runtime.stream_transport.timeout, 0.01)
        self.assertEqual(stream.handle.state, StreamState.OPEN)

    def test_bidi_keeps_half_close_cancel_and_close_distinct(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))
        channel = transport.bidi(
            complete_draft().to_json_dict(),
            [{"stream_id": 1, "content_type": "application/json"}],
        )

        ack = channel.send({"sequence": 1, "kind": "data", "stream_id": 1})
        half_closed = channel.close_send()
        with self.assertRaises(SDKError) as caught:
            channel.close()
        cancelled = channel.cancel("client stop")
        channel.close()

        self.assertEqual(ack["sequence"], 1)
        self.assertEqual(half_closed["state"], "HalfClosedLocal")
        self.assertFalse(half_closed["terminal"])
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(cancelled["state"], "Cancelled")
        self.assertEqual(channel.session.state, BidiState.CLOSED)

    def test_bidi_timeout_is_forwarded_without_state_mutation(self) -> None:
        class TimeoutRuntimeTransport(MemoryRuntimeTransport):
            def open_bidi(self, draft_json: bytes, streams_json: bytes):
                from test_bidi import MemoryBidiTransport

                class TimeoutBidiTransport(MemoryBidiTransport):
                    def recv(self, timeout: float | None = None) -> bytes:
                        self.timeout = timeout
                        raise TimeoutError("no frame")

                self.bidi_transport = TimeoutBidiTransport()
                return (
                    self.bidi_transport,
                    b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}',
                )

        runtime = TimeoutRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        channel = transport.bidi(
            complete_draft().to_json_dict(),
            [{"stream_id": 1, "content_type": "application/json"}],
        )
        with self.assertRaises(TimeoutError):
            channel.recv(timeout=0.01)

        self.assertEqual(runtime.bidi_transport.timeout, 0.01)
        self.assertEqual(channel.session.state, BidiState.OPEN)

    def test_rejects_incomplete_invocation_mapping_before_dispatch(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        with self.assertRaises(SDKError) as caught:
            transport.invoke({"caller_ura": "easynet:///r/example/agent/alice.sdk"})

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(runtime.seen_draft)


if __name__ == "__main__":
    unittest.main()
