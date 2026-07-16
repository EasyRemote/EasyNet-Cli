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


def test_stream_and_bidi_backpressure_bounds() -> None:
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
    try:
        bidi.receive()
    except SDKError as error:
        assert is_code(error, ErrorCode.INVALID_ARGUMENT)
    else:
        raise AssertionError("bidi overflow was accepted")
    assert bidi.state is BidiState.FAILED
