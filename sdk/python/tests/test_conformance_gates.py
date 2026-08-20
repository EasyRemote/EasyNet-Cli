from __future__ import annotations

import subprocess
from pathlib import Path

from easynet_sdk import (
    BidiSession,
    BidiState,
    ErrorCode,
    SDKError,
    StreamHandle,
    StreamState,
)
from easynet_sdk.errors import is_code
from test_bidi import MemoryBidiTransport
from test_stream import MemoryStreamTransport


ROOT = Path(__file__).resolve().parents[3]


def _run_gate(script: str, *args: str) -> None:
    subprocess.run(
        ["bash", str(ROOT / "tools/scripts" / script), *args],
        cwd=ROOT,
        check=True,
        text=True,
        capture_output=True,
    )


def test_sdk_product_neutrality_gate_scans_all_canonical_roots() -> None:
    _run_gate("check-sdk-product-neutrality.sh")


def test_seven_language_capability_matrix_self_test() -> None:
    _run_gate("check-sdk-parity-matrix.sh", "--self-test")


def test_stream_backpressure_and_bidi_observation_bounds() -> None:
    stream = StreamHandle.from_json(
        MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"data","state":"Open","terminal":false}',
                b'{"sequence":2,"kind":"data","state":"Open","terminal":false}',
            ]
        ),
        b'{"stream_id":"stream-1","state":"Opening","max_buffered_events":1}',
    )
    stream.next()
    try:
        stream.next()
    except SDKError as error:
        assert is_code(error, ErrorCode.INVALID_ARGUMENT)
    else:
        raise AssertionError("stream overflow was accepted")
    assert stream.state is StreamState.FAILED

    bidi = BidiSession.from_json(
        MemoryBidiTransport(
            [
                b'{"sequence":1,"kind":"data","stream_id":1}',
                b'{"sequence":2,"kind":"data","stream_id":1}',
            ]
        ),
        b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":1}',
    )
    bidi.receive()
    bidi.receive()
    assert [frame.sequence for frame in bidi.received_frames] == [2]
    assert bidi.state is BidiState.OPEN
    assert bidi.runtime_state is BidiState.OPEN


def test_stream_cancel_request_is_non_terminal() -> None:
    transport = MemoryStreamTransport()
    stream = StreamHandle.from_json(
        transport,
        b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
    )

    outcome = stream.cancel("client stop")

    assert outcome.terminal is False
    assert outcome.cancelled is False
    assert outcome.state is StreamState.CANCEL_REQUESTED
    assert stream.state is StreamState.CANCEL_REQUESTED
    assert transport.cancel_reason == "client stop"

    terminal_transport = MemoryStreamTransport()
    terminal_transport.cancel_reply = (
        b'{"stream_id":"stream-1","cancelled":true,'
        b'"state":"Cancelled","terminal":true}'
    )
    stream = StreamHandle.from_json(
        terminal_transport,
        b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
    )
    try:
        stream.cancel("client stop")
    except SDKError as error:
        assert is_code(error, ErrorCode.INVALID_ARGUMENT)
    else:
        raise AssertionError("terminal stream cancel ack was accepted")
    assert stream.state is StreamState.FAILED


def test_bidi_cancel_request_is_non_terminal() -> None:
    transport = MemoryBidiTransport()
    bidi = BidiSession.from_json(
        transport,
        b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}',
    )

    outcome = bidi.cancel("client stop")

    assert outcome.terminal is False
    assert outcome.state is BidiState.CANCEL_REQUESTED
    assert bidi.state is BidiState.CANCEL_REQUESTED
    assert transport.cancel_reason == "client stop"
    try:
        bidi.close_send()
    except SDKError as error:
        assert is_code(error, ErrorCode.INVALID_ARGUMENT)
    else:
        raise AssertionError("close_send after bidi cancel was accepted")
    bidi.close()
    assert transport.closed is True
    assert bidi.state is BidiState.CLOSED

    terminal_transport = MemoryBidiTransport()
    terminal_transport.cancel_reply = (
        b'{"session_id":"bidi-1","state":"Cancelled",'
        b'"terminal":true,"reason":"client stop"}'
    )
    bidi = BidiSession.from_json(
        terminal_transport,
        b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}',
    )
    try:
        bidi.cancel("client stop")
    except SDKError as error:
        assert is_code(error, ErrorCode.INVALID_ARGUMENT)
    else:
        raise AssertionError("terminal bidi cancel ack was accepted")
    assert bidi.state is BidiState.FAILED
