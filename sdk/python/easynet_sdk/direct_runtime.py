"""Direct daemon Runtime Core transport over Axon gRPC UDS.

This module is the Python SDK's concrete daemon Invocation transport. It
translates SDK JSON DTOs into Axon protobuf requests and delegates all runtime
semantics to the daemon endpoint.
"""

from __future__ import annotations

import base64
import binascii
import json
import queue
import secrets
import threading
from dataclasses import dataclass, field
from typing import Any, Mapping

import grpc  # type: ignore[import-untyped]

from ._axon_pb.axon.v1 import (
    invoke_pb2 as _invoke_pb2,
    invoke_pb2_grpc as _invoke_pb2_grpc,
    types_pb2 as _types_pb2,
)
from .control_ipc import ControlDiscovery, read_control_discovery
from .errors import ErrorCode, RetryHint, SDKError, normalize_error_code
from .invocation import InvocationDraft
from .runtime import RuntimeTransport
from .bidi import BidiFrame, BidiTransport
from .stream import StreamTransport

DEFAULT_URA_PROFILE = "easynet-strict-v2"
DEFAULT_DIAL_TIMEOUT_SECONDS = 3.0
DEFAULT_INVOKE_TIMEOUT_SECONDS = 60.0
DEFAULT_DIRECT_STREAM_QUEUE_EVENTS = 1024
DEFAULT_DIRECT_BIDI_QUEUE_FRAMES = 1024
_DIRECT_BIDI_EOF = object()

invoke_pb2: Any = _invoke_pb2
invoke_pb2_grpc: Any = _invoke_pb2_grpc
types_pb2: Any = _types_pb2


@dataclass
class DirectDaemonRuntimeConnector:
    """RuntimeConnector for direct daemon Invocation gRPC over UDS."""

    control_path: str = ""
    discovery_reader: Any = read_control_discovery
    _transports: list["DirectDaemonRuntimeTransport"] = field(default_factory=list)
    _closed: bool = False

    def resolve(self, options_json: bytes) -> bytes:
        self._require_open()
        options = _decode_object(options_json, "connect options")
        endpoint = _optional_string(options.get("endpoint"), "endpoint") or ""
        control_path = (
            _optional_string(options.get("control_path"), "control_path")
            or self.control_path
        )
        facts: dict[str, object] = {
            "endpoint": endpoint,
            "control_path": control_path,
        }
        for option_name in ("dial_timeout_ms", "invoke_timeout_ms", "max_message_bytes"):
            if option_name in options:
                facts[option_name] = _optional_non_negative_int(
                    options.get(option_name),
                    option_name,
                )
        if endpoint:
            return _json_bytes(facts)

        discovery: ControlDiscovery = self.discovery_reader(control_path)
        if not discovery.invocation_endpoint:
            raise SDKError(
                code=ErrorCode.CONTROL_ONLY,
                stage="direct_runtime.resolve",
                retry=RetryHint.SAFE,
                retryable=True,
                message="control discovery did not advertise invocation_endpoint",
                details={"control_path": control_path},
            )
        facts.update(
            {
                "endpoint": discovery.invocation_endpoint,
                "control_endpoint": discovery.socket_path,
                "daemon_version": discovery.daemon_version,
                "capability_flags": list(discovery.capability_flags),
            }
        )
        return _json_bytes(facts)

    def handshake(self, endpoint_json: bytes) -> tuple[RuntimeTransport, bytes]:
        self._require_open()
        endpoint = _decode_object(endpoint_json, "runtime endpoint")
        endpoint_value = _required_string(endpoint, "endpoint")
        dial_timeout = _timeout_seconds(
            endpoint.get("dial_timeout_ms"), DEFAULT_DIAL_TIMEOUT_SECONDS
        )
        invoke_timeout = _timeout_seconds(
            endpoint.get("invoke_timeout_ms"), DEFAULT_INVOKE_TIMEOUT_SECONDS
        )
        max_message_bytes = _optional_non_negative_int(
            endpoint.get("max_message_bytes"), "max_message_bytes"
        )
        transport = DirectDaemonRuntimeTransport.open(
            endpoint_value,
            dial_timeout_seconds=dial_timeout,
            invoke_timeout_seconds=invoke_timeout,
            max_message_bytes=max_message_bytes,
        )
        self._transports.append(transport)
        facts = {
            "transport": "direct-axon-grpc-uds",
            "endpoint": endpoint_value,
            "protocol": "axon.v1.Invocation",
            "unary": True,
            "stream": True,
            "bidi": True,
            "prepare": False,
            "submit_signed": False,
        }
        return transport, _json_bytes(facts)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        while self._transports:
            self._transports.pop().close()

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error("runtime connector is closed", code=ErrorCode.INVALID_HANDLE)


class DirectDaemonRuntimeTransport:
    """Concrete RuntimeTransport using daemon Axon gRPC over UDS."""

    def __init__(
        self,
        channel: grpc.Channel,
        *,
        endpoint: str,
        invoke_timeout_seconds: float,
    ) -> None:
        self._channel = channel
        self._stub = invoke_pb2_grpc.InvocationStub(channel)
        self._endpoint = endpoint
        self._invoke_timeout_seconds = invoke_timeout_seconds
        self._closed = False

    @classmethod
    def open(
        cls,
        endpoint: str,
        *,
        dial_timeout_seconds: float = DEFAULT_DIAL_TIMEOUT_SECONDS,
        invoke_timeout_seconds: float = DEFAULT_INVOKE_TIMEOUT_SECONDS,
        max_message_bytes: int = 0,
    ) -> "DirectDaemonRuntimeTransport":
        target = _grpc_uds_target(endpoint)
        options: list[tuple[str, int]] = []
        if max_message_bytes:
            options.extend(
                [
                    ("grpc.max_send_message_length", max_message_bytes),
                    ("grpc.max_receive_message_length", max_message_bytes),
                ]
            )
        channel = grpc.insecure_channel(target, options=options)
        try:
            grpc.channel_ready_future(channel).result(timeout=dial_timeout_seconds)
        except grpc.FutureTimeoutError as exc:
            _close_channel(channel)
            raise _direct_error(
                "daemon invocation endpoint is not ready",
                code=ErrorCode.DAEMON_OFFLINE,
                retry=RetryHint.SAFE,
                retryable=True,
                details={"endpoint": endpoint},
                cause=exc,
            ) from exc
        except Exception as exc:
            _close_channel(channel)
            raise _direct_error(
                f"open daemon invocation endpoint failed: {exc}",
                code=ErrorCode.TRANSPORT,
                retry=RetryHint.SAFE,
                retryable=True,
                details={"endpoint": endpoint},
                cause=exc,
            ) from exc
        return cls(
            channel,
            endpoint=endpoint,
            invoke_timeout_seconds=invoke_timeout_seconds,
        )

    def invoke(self, draft_json: bytes) -> bytes:
        self._require_open()
        try:
            draft = InvocationDraft.from_json(draft_json)
            request = _draft_to_invoke_request(draft)
            response = self._stub.Invoke(
                request,
                timeout=self._invoke_timeout_seconds,
            )
            return _invoke_response_json(draft, response)
        except SDKError:
            raise
        except grpc.RpcError as exc:
            raise _grpc_error(exc, endpoint=self._endpoint) from exc
        except Exception as exc:
            raise _direct_error(
                f"invoke daemon endpoint failed: {exc}",
                code=ErrorCode.TRANSPORT,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            ) from exc

    def open_stream(self, draft_json: bytes) -> tuple[StreamTransport, bytes]:
        self._require_open()
        try:
            draft = InvocationDraft.from_json(draft_json)
            request = _draft_to_stream_request(draft)
            iterator = self._stub.InvokeStream(
                request,
                timeout=self._invoke_timeout_seconds,
            )
            transport = DirectDaemonStreamTransport(
                iterator,
                endpoint=self._endpoint,
            )
            return transport, _json_bytes(
                {
                    "stream_id": transport.stream_id,
                    "state": "Open",
                    "max_buffered_events": DEFAULT_DIRECT_STREAM_QUEUE_EVENTS,
                }
            )
        except SDKError:
            raise
        except grpc.RpcError as exc:
            raise _grpc_error(exc, endpoint=self._endpoint) from exc
        except Exception as exc:
            raise _direct_error(
                f"open daemon stream endpoint failed: {exc}",
                code=ErrorCode.TRANSPORT,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            ) from exc

    def open_bidi(self, draft_json: bytes, streams_json: bytes) -> tuple[BidiTransport, bytes]:
        self._require_open()
        try:
            draft = InvocationDraft.from_json(draft_json)
            streams = _bidi_stream_descriptors(streams_json)
            open_frame = _draft_to_bidi_open_frame(draft, streams)
            transport = DirectDaemonBidiTransport(endpoint=self._endpoint)
            transport.start(
                self._stub,
                open_frame,
                timeout_seconds=self._invoke_timeout_seconds,
            )
            return transport, _json_bytes(
                {
                    "session_id": transport.session_id,
                    "state": "Open",
                    "max_buffered_frames": DEFAULT_DIRECT_BIDI_QUEUE_FRAMES,
                }
            )
        except SDKError:
            raise
        except grpc.RpcError as exc:
            raise _grpc_error(exc, endpoint=self._endpoint) from exc
        except Exception as exc:
            raise _direct_error(
                f"open daemon bidi endpoint failed: {exc}",
                code=ErrorCode.TRANSPORT,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            ) from exc

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        raise _unsupported("direct daemon prepare transport is not implemented")

    def submit_signed(self, signed_json: bytes) -> bytes:
        raise _unsupported("direct daemon signed submit transport is not implemented")

    def await_handle(self, handle_id: int) -> bytes:
        raise _unsupported("direct daemon handle await transport is not implemented")

    def cancel_handle(self, handle_id: int, reason: str) -> bytes:
        raise _unsupported("direct daemon handle cancel transport is not implemented")

    def handle_events(self, handle_id: int) -> bytes:
        raise _unsupported("direct daemon handle events transport is not implemented")

    def free_handle(self, handle_id: int) -> None:
        raise _unsupported("direct daemon handle free transport is not implemented")

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        _close_channel(self._channel)

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error("runtime transport is closed", code=ErrorCode.INVALID_HANDLE)


class DirectDaemonStreamTransport:
    """Bounded StreamTransport adapter over one Axon InvokeStream iterator."""

    def __init__(
        self,
        iterator: Any,
        *,
        endpoint: str,
        max_buffered_events: int = DEFAULT_DIRECT_STREAM_QUEUE_EVENTS,
    ) -> None:
        if max_buffered_events <= 0:
            raise _direct_error(
                "max_buffered_events must be positive",
                code=ErrorCode.INVALID_ARGUMENT,
            )
        self._iterator = iterator
        self._endpoint = endpoint
        self.stream_id = f"direct-stream-{secrets.token_hex(8)}"
        self._queue: queue.Queue[bytes | SDKError] = queue.Queue(
            maxsize=max_buffered_events
        )
        self._lock = threading.Lock()
        self._closed = False
        self._terminal_seen = False
        self._reader = threading.Thread(
            target=self._read_stream,
            name=f"easynet-direct-stream-{self.stream_id}",
            daemon=True,
        )
        self._reader.start()

    def recv(self, timeout: float | None = None) -> bytes:
        with self._lock:
            self._require_open()
        try:
            item = self._queue.get(timeout=timeout)
        except queue.Empty as exc:
            raise TimeoutError("no direct daemon stream frame available") from exc
        if isinstance(item, SDKError):
            raise item
        return item

    def cancel(self, reason: str) -> bytes:
        self.close()
        return _json_bytes(
            {
                "stream_id": self.stream_id,
                "cancelled": True,
                "state": "Cancelled",
                "terminal": True,
                "reason": reason,
            }
        )

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
            _cancel_stream_iterator(self._iterator)

    def _read_stream(self) -> None:
        try:
            for chunk in self._iterator:
                raw = _stream_chunk_json(chunk)
                if not self._put(raw):
                    return
                if _stream_chunk_terminal(chunk):
                    self._terminal_seen = True
                    return
            if not self._closed and not self._terminal_seen:
                self._put(
                    _direct_error(
                        "daemon stream ended without a terminal frame",
                        code=ErrorCode.PROTOCOL,
                        retry=RetryHint.NEVER,
                        details={"endpoint": self._endpoint, "stream_id": self.stream_id},
                    )
                )
        except SDKError as exc:
            if not self._closed:
                self._put(exc)
        except grpc.RpcError as exc:
            if not self._closed:
                self._put(_grpc_error(exc, endpoint=self._endpoint))
        except Exception as exc:
            if not self._closed:
                self._put(
                    _direct_error(
                        f"daemon stream recv failed: {exc}",
                        code=ErrorCode.TRANSPORT,
                        retry=RetryHint.UNKNOWN,
                        details={"endpoint": self._endpoint, "stream_id": self.stream_id},
                        cause=exc,
                    )
                )

    def _put(self, item: bytes | SDKError) -> bool:
        while not self._closed:
            try:
                self._queue.put(item, timeout=0.1)
                return True
            except queue.Full:
                continue
        return False

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error("stream transport is closed", code=ErrorCode.INVALID_HANDLE)


class DirectDaemonBidiTransport:
    """Bounded BidiTransport adapter over one Axon InvokeBidi call."""

    def __init__(
        self,
        *,
        endpoint: str,
        max_buffered_frames: int = DEFAULT_DIRECT_BIDI_QUEUE_FRAMES,
    ) -> None:
        if max_buffered_frames <= 0:
            raise _direct_error(
                "max_buffered_frames must be positive",
                code=ErrorCode.INVALID_ARGUMENT,
            )
        self._endpoint = endpoint
        self.session_id = f"direct-bidi-{secrets.token_hex(8)}"
        self._outbox: queue.Queue[Any] = queue.Queue(maxsize=max_buffered_frames)
        self._inbox: queue.Queue[bytes | SDKError] = queue.Queue(
            maxsize=max_buffered_frames
        )
        self._lock = threading.Lock()
        self._closed = False
        self._send_closed = False
        self._terminal_seen = False
        self._last_up_sequence = 0
        self._call: Any = None
        self._reader: threading.Thread | None = None

    def start(self, stub: Any, open_frame: Any, *, timeout_seconds: float) -> None:
        with self._lock:
            self._require_open()
            self._call = stub.InvokeBidi(
                self._request_iterator(open_frame),
                timeout=timeout_seconds,
            )
            self._reader = threading.Thread(
                target=self._read_bidi,
                name=f"easynet-direct-bidi-{self.session_id}",
                daemon=True,
            )
            self._reader.start()

    def send(self, frame_json: bytes) -> bytes:
        frame = BidiFrame.from_json(frame_json)
        with self._lock:
            self._require_send_open()
            if frame.sequence != self._last_up_sequence + 1:
                raise _direct_error(
                    "bidi up frames must be contiguous",
                    code=ErrorCode.INVALID_ARGUMENT,
                    details={
                        "session_id": self.session_id,
                        "expected_sequence": self._last_up_sequence + 1,
                        "actual_sequence": frame.sequence,
                    },
                )
            up = _bidi_frame_to_up(frame)
            self._put_outbound(up)
            self._last_up_sequence = frame.sequence
        return frame.to_json()

    def recv(self, timeout: float | None = None) -> bytes:
        with self._lock:
            self._require_open()
        try:
            item = self._inbox.get(timeout=timeout)
        except queue.Empty as exc:
            raise TimeoutError("no direct daemon bidi frame available") from exc
        if isinstance(item, SDKError):
            raise item
        return item

    def close_send(self) -> bytes:
        with self._lock:
            self._require_send_open()
            sequence = self._last_up_sequence + 1
            self._put_outbound(
                invoke_pb2.InvokeBidiUp(
                    sequence=sequence,
                    control=invoke_pb2.BidiControl(eof=True),
                )
            )
            self._last_up_sequence = sequence
            self._send_closed = True
            self._put_outbound(_DIRECT_BIDI_EOF)
        return _json_bytes(
            {
                "session_id": self.session_id,
                "state": "HalfClosedLocal",
                "terminal": False,
            }
        )

    def cancel(self, reason: str) -> bytes:
        self.close()
        return _json_bytes(
            {
                "session_id": self.session_id,
                "state": "Cancelled",
                "terminal": True,
                "reason": reason,
            }
        )

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._put_outbound(_DIRECT_BIDI_EOF)
            self._closed = True
            self._send_closed = True
            _cancel_stream_iterator(self._call)

    def _request_iterator(self, open_frame: Any) -> Any:
        yield open_frame
        while True:
            item = self._outbox.get()
            if item is _DIRECT_BIDI_EOF:
                return
            yield item

    def _read_bidi(self) -> None:
        try:
            for frame in self._call:
                if _bidi_down_is_internal_admission(frame):
                    continue
                raw = _bidi_down_json(frame)
                if not self._put_inbound(raw):
                    return
                if _bidi_down_terminal(frame):
                    self._terminal_seen = True
                    return
            if not self._closed and not self._terminal_seen:
                self._put_inbound(
                    _direct_error(
                        "daemon bidi ended without a terminal frame",
                        code=ErrorCode.PROTOCOL,
                        retry=RetryHint.NEVER,
                        details={"endpoint": self._endpoint, "session_id": self.session_id},
                    )
                )
        except SDKError as exc:
            if not self._closed:
                self._put_inbound(exc)
        except grpc.RpcError as exc:
            if not self._closed:
                self._put_inbound(_grpc_error(exc, endpoint=self._endpoint))
        except Exception as exc:
            if not self._closed:
                self._put_inbound(
                    _direct_error(
                        f"daemon bidi recv failed: {exc}",
                        code=ErrorCode.TRANSPORT,
                        retry=RetryHint.UNKNOWN,
                        details={"endpoint": self._endpoint, "session_id": self.session_id},
                        cause=exc,
                    )
                )

    def _put_outbound(self, item: Any) -> None:
        while not self._closed:
            try:
                self._outbox.put(item, timeout=0.1)
                return
            except queue.Full:
                continue
        if item is not _DIRECT_BIDI_EOF:
            raise _direct_error("bidi transport is closed", code=ErrorCode.INVALID_HANDLE)

    def _put_inbound(self, item: bytes | SDKError) -> bool:
        while not self._closed:
            try:
                self._inbox.put(item, timeout=0.1)
                return True
            except queue.Full:
                continue
        return False

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error("bidi transport is closed", code=ErrorCode.INVALID_HANDLE)

    def _require_send_open(self) -> None:
        self._require_open()
        if self._send_closed:
            raise _direct_error(
                "bidi send path is closed",
                code=ErrorCode.CANCELLED,
                retry=RetryHint.NEVER,
            )


def _cancel_stream_iterator(iterator: Any) -> None:
    cancel = getattr(iterator, "cancel", None)
    if cancel is not None:
        cancel()


def _stream_chunk_terminal(chunk: Any) -> bool:
    return bool(chunk.terminal) or _state_name(chunk.state) in {
        "Completed",
        "Failed",
        "TimedOut",
        "Cancelled",
    }


def _draft_to_invoke_request(draft: InvocationDraft) -> Any:
    fields = _invoke_request_fields(draft)
    return invoke_pb2.InvokeRequest(**fields)


def _draft_to_stream_request(draft: InvocationDraft) -> Any:
    fields = _invoke_request_fields(draft)
    return invoke_pb2.InvokeServerStreamRequest(**fields)


def _draft_to_bidi_open_frame(draft: InvocationDraft, streams: list[Any]) -> Any:
    fields = _invoke_request_fields(draft)
    return invoke_pb2.InvokeBidiUp(
        sequence=0,
        mac=_bidi_open_mac(draft),
        envelope_open=invoke_pb2.EnvelopeOpen(
            envelope=fields["envelope"],
            target=_bidi_target(draft),
            initial_args=fields["arguments"],
            args_content_type=fields["content_type"],
            streams=streams,
            metadata=fields["metadata"],
            content_envelope=fields["content_envelope"],
        ),
    )


def _invoke_request_fields(draft: InvocationDraft) -> dict[str, object]:
    content_type = draft.content_type
    return {
        "envelope": types_pb2.Envelope(
            request_id=f"req-{secrets.token_hex(16)}",
            caller=_agent_identity(draft.caller_ura),
            callee=_agent_identity(draft.callee_ura),
            subject=types_pb2.SubjectIdentity(
                ura=draft.subject_ura,
                profile=DEFAULT_URA_PROFILE,
            ),
            invocation_nonce=_base64_decode(draft.nonce_base64, "nonce_base64"),
            causal_context=_causal_context(draft.causal_context),
            caller_signature=_caller_signature(draft),
        ),
        "function_name": draft.descriptor_ref,
        "arguments": _arguments(draft),
        "content_type": content_type,
        "metadata": _metadata(draft.metadata),
        "content_envelope": types_pb2.ContentEnvelope(
            content_type=content_type,
            encoding="identity",
        ),
    }


def _bidi_stream_descriptors(streams_json: bytes) -> list[Any]:
    try:
        decoded = json.loads(streams_json.decode("utf-8"))
    except Exception as exc:
        raise _direct_error(
            f"decode bidi streams JSON: {exc}",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc
    if not isinstance(decoded, list):
        raise _direct_error(
            "bidi streams JSON must be an array",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    if not decoded:
        raise _direct_error(
            "bidi streams must not be empty",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            details={"field": "bidi_streams"},
        )
    result: list[Any] = []
    seen: set[int] = set()
    for index, item in enumerate(decoded):
        if not isinstance(item, Mapping):
            raise _direct_error(
                "bidi stream descriptor must be an object",
                code=ErrorCode.INVALID_ARGUMENT,
                retry=RetryHint.NEVER,
                details={"index": index},
            )
        stream_id = _required_positive_int(item, "stream_id")
        if stream_id in seen:
            raise _direct_error(
                "bidi stream ids must be unique",
                code=ErrorCode.INVALID_ARGUMENT,
                retry=RetryHint.NEVER,
                details={"stream_id": stream_id},
            )
        seen.add(stream_id)
        result.append(
            invoke_pb2.StreamDescriptor(
                stream_id=stream_id,
                content_type=_optional_string(item.get("content_type"), "content_type")
                or "",
                codec_params=_optional_string(item.get("codec_params"), "codec_params")
                or "",
                ordering=_optional_string(item.get("ordering"), "ordering") or "",
            )
        )
    return result


def _bidi_target(draft: InvocationDraft) -> Any:
    return types_pb2.InvocationTarget(
        ability=types_pb2.AbilityTarget(function_name=draft.descriptor_ref)
    )


def _bidi_open_mac(draft: InvocationDraft) -> bytes:
    signature = draft.caller_signature
    if signature is None:
        return b""
    return _base64_decode(
        signature.signature_base64,
        "caller_signature.signature_base64",
    )


def _invoke_response_json(
    draft: InvocationDraft,
    response: Any,
) -> bytes:
    terminal_state = _state_name(response.state)
    output_content_type = response.result_content_type
    output_base64 = base64.b64encode(response.result).decode("ascii")
    error = _response_failure(response, terminal_state)
    result: dict[str, object] = {
        "ok": error is None,
        "tuple": draft.to_json_dict(),
        "terminal_state": terminal_state,
        "output_content_type": output_content_type,
        "output_base64": output_base64,
        "output_json": _output_json(response.result, output_content_type),
        "selected_node_id": response.selected_node_id,
        "scheduling_reason": response.scheduling_reason,
        "elapsed_ms": response.elapsed_ms,
        "receipt": _receipt(response.terminal_receipt)
        if response.HasField("terminal_receipt")
        else None,
        "error": error,
    }
    return _json_bytes(result)


def _stream_chunk_json(chunk: Any) -> bytes:
    content_type = chunk.content_type
    error = _stream_chunk_error(chunk)
    event: dict[str, object] = {
        "sequence": int(chunk.sequence) + 1,
        "kind": "terminal" if _stream_chunk_terminal(chunk) else "chunk",
        "state": _state_name(chunk.state),
        "terminal": _stream_chunk_terminal(chunk),
        "payload_content_type": content_type,
        "payload_base64": base64.b64encode(chunk.payload).decode("ascii"),
        "payload_json": _output_json(chunk.payload, content_type),
        "error": error,
    }
    if chunk.invocation_id:
        event["invocation_id"] = chunk.invocation_id
    if chunk.selected_node_id:
        event["selected_node_id"] = chunk.selected_node_id
    if chunk.scheduling_reason:
        event["scheduling_reason"] = chunk.scheduling_reason
    if chunk.elapsed_ms:
        event["elapsed_ms"] = chunk.elapsed_ms
    if chunk.HasField("terminal_receipt"):
        event["receipt"] = _receipt(chunk.terminal_receipt)
    return _json_bytes(event)


def _bidi_frame_to_up(frame: BidiFrame) -> Any:
    kind = frame.kind
    if kind in {"data", "binary_chunk", "chunk"}:
        return invoke_pb2.InvokeBidiUp(
            sequence=frame.sequence,
            binary_chunk=invoke_pb2.BinaryChunk(
                stream_id=frame.stream_id,
                data=_bidi_payload_bytes(frame),
            ),
        )
    if kind in {"eof", "close_send"}:
        return invoke_pb2.InvokeBidiUp(
            sequence=frame.sequence,
            control=invoke_pb2.BidiControl(eof=True),
        )
    if kind == "control":
        return invoke_pb2.InvokeBidiUp(
            sequence=frame.sequence,
            control=_bidi_control(frame.payload_json),
        )
    raise _direct_error(
        f"unsupported bidi frame kind: {kind}",
        code=ErrorCode.INVALID_ARGUMENT,
        retry=RetryHint.NEVER,
        details={"kind": kind},
    )


def _bidi_payload_bytes(frame: BidiFrame) -> bytes:
    if frame.payload_base64:
        return _base64_decode(frame.payload_base64, "payload_base64")
    if frame.payload_json is not None:
        return json.dumps(
            frame.payload_json,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
    return b""


def _bidi_control(payload: object) -> Any:
    if not isinstance(payload, Mapping):
        raise _direct_error(
            "bidi control payload_json must be an object",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    if payload.get("eof") is True:
        return invoke_pb2.BidiControl(eof=True)
    resize = payload.get("pty_resize")
    if isinstance(resize, Mapping):
        return invoke_pb2.BidiControl(
            pty_resize=invoke_pb2.PtyResize(
                cols=_required_positive_int(resize, "cols"),
                rows=_required_positive_int(resize, "rows"),
            )
        )
    signal = payload.get("pty_signal")
    if isinstance(signal, Mapping):
        return invoke_pb2.BidiControl(
            pty_signal=invoke_pb2.PtySignal(signal=_required_int(signal, "signal"))
        )
    media = payload.get("media_pts")
    if isinstance(media, Mapping):
        return invoke_pb2.BidiControl(
            media_pts=invoke_pb2.MediaTimestamp(
                stream_id=_required_positive_int(media, "stream_id"),
                pts=_required_non_negative_int(media, "pts"),
            )
        )
    raise _direct_error(
        "unsupported bidi control payload",
        code=ErrorCode.INVALID_ARGUMENT,
        retry=RetryHint.NEVER,
    )


def _bidi_down_json(frame: Any) -> bytes:
    payload = frame.WhichOneof("payload")
    sequence = int(frame.sequence) + 1
    event: dict[str, object] = {
        "sequence": sequence,
        "kind": _bidi_down_kind(frame, payload),
        "stream_id": 0,
        "terminal": _bidi_down_terminal(frame),
    }
    if payload == "binary_chunk":
        chunk = frame.binary_chunk
        event["stream_id"] = int(chunk.stream_id)
        event["payload_base64"] = base64.b64encode(chunk.data).decode("ascii")
    elif payload == "receipt":
        receipt = _receipt(frame.receipt)
        event["payload_json"] = {"receipt": receipt}
        if frame.receipt.HasField("failure"):
            event["error"] = _axon_failure(frame.receipt.failure, "direct_runtime.bidi")
    elif payload == "control":
        event["payload_json"] = _bidi_control_json(frame.control)
    elif payload in {"dispatch_call", "reverse_dispatch_result"}:
        event["error"] = {
            "code": ErrorCode.PROTOCOL_MISMATCH.value,
            "stage": "direct_runtime.bidi",
            "message": "carrier-v1 dispatch frame before SDK dual-read support",
            "retryable": False,
        }
    else:
        raise _direct_error(
            "daemon bidi frame did not include a payload",
            code=ErrorCode.PROTOCOL,
            retry=RetryHint.NEVER,
        )
    return _json_bytes(event)


def _bidi_down_kind(frame: Any, payload: str | None) -> str:
    if payload == "binary_chunk":
        return "data"
    if payload == "control":
        if frame.control.WhichOneof("control") == "eof":
            return "remote_close_send"
        return "control"
    if payload == "receipt":
        return "terminal" if _bidi_receipt_terminal(frame.receipt) else "receipt"
    if payload in {"dispatch_call", "reverse_dispatch_result"}:
        return "unsupported_frame"
    return "unknown"


def _bidi_down_terminal(frame: Any) -> bool:
    return frame.WhichOneof("payload") == "receipt" and _bidi_receipt_terminal(
        frame.receipt
    )


def _bidi_down_is_internal_admission(frame: Any) -> bool:
    return (
        int(frame.sequence) == 0
        and frame.WhichOneof("payload") == "receipt"
        and not _bidi_receipt_terminal(frame.receipt)
    )


def _bidi_receipt_terminal(receipt: Any) -> bool:
    return bool(receipt.cleanup_complete) or _state_name(receipt.state) in {
        "Completed",
        "Failed",
        "TimedOut",
        "Cancelled",
    }


def _bidi_control_json(control: Any) -> dict[str, object]:
    variant = control.WhichOneof("control")
    if variant == "eof":
        return {"eof": True}
    if variant == "pty_resize":
        return {"pty_resize": {"cols": control.pty_resize.cols, "rows": control.pty_resize.rows}}
    if variant == "pty_signal":
        return {"pty_signal": {"signal": control.pty_signal.signal}}
    if variant == "media_pts":
        return {
            "media_pts": {
                "stream_id": control.media_pts.stream_id,
                "pts": control.media_pts.pts,
            }
        }
    return {}


def _stream_chunk_error(chunk: Any) -> dict[str, object] | None:
    if chunk.HasField("error"):
        return _axon_failure(chunk.error, _error_stage(chunk.error.stage))
    state = _state_name(chunk.state)
    if state in {"Failed", "TimedOut", "Cancelled"}:
        code = ErrorCode.TIMEOUT if state == "TimedOut" else ErrorCode.ABILITY_FAILED
        return {
            "code": code.value,
            "stage": "direct_runtime.stream",
            "message": f"daemon stream chunk state is {state}",
            "retryable": code == ErrorCode.TIMEOUT,
        }
    return None


def _axon_failure(error: Any, stage: str) -> dict[str, object]:
    code = _response_error_code(error.code)
    return {
        "code": code.value,
        "stage": stage,
        "message": error.message,
        "retryable": error.retryable,
    }


def _agent_identity(ura: str) -> Any:
    return types_pb2.AgentIdentity(ura=ura, profile=DEFAULT_URA_PROFILE)


def _caller_signature(draft: InvocationDraft) -> Any:
    signature = draft.caller_signature
    if signature is None:
        return None
    return types_pb2.CallerSignature(
        algorithm=signature.algorithm,
        signature=_base64_decode(
            signature.signature_base64,
            "caller_signature.signature_base64",
        ),
        key_id_hint=signature.key_id_hint or "",
    )


def _arguments(draft: InvocationDraft) -> bytes:
    if draft.arguments_base64 is not None:
        return _base64_decode(draft.arguments_base64, "arguments_base64")
    return json.dumps(draft.args, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _metadata(metadata: Mapping[str, object]) -> dict[str, str]:
    result: dict[str, str] = {}
    for key, value in metadata.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise _direct_error(
                "metadata must be a string-to-string map for Axon InvokeRequest",
                code=ErrorCode.INVALID_INVOCATION,
                retry=RetryHint.NEVER,
                details={"field": "metadata"},
            )
        result[key] = value
    return result


def _causal_context(value: Mapping[str, object]) -> Any:
    form = _optional_string(value.get("form"), "causal_context.form") or _optional_string(
        value.get("kind"), "causal_context.kind"
    )
    if form in (None, "", "none", "empty", "null"):
        return types_pb2.CausalContext(none=types_pb2.Empty())
    if form == "scalar":
        return types_pb2.CausalContext(scalar=_receipt_ref(value))
    if form == "list":
        prior = value.get("prior", [])
        if not isinstance(prior, list):
            raise _invalid_causal_context("causal_context.prior must be an array")
        return types_pb2.CausalContext(
            list=types_pb2.ReceiptList(prior=[_receipt_ref(item) for item in prior])
        )
    if form == "merkle":
        root_hex = _required_string(value, "root_hex")
        return types_pb2.CausalContext(
            merkle=types_pb2.MerkleRoot(
                root=_hex_decode(root_hex, "root_hex"),
                proof_ura=_required_string(value, "proof_ura"),
            )
        )
    raise _invalid_causal_context(f"unknown causal_context form: {form}")


def _receipt_ref(value: object) -> Any:
    if not isinstance(value, Mapping):
        raise _invalid_causal_context("causal receipt ref must be an object")
    receipt_hash_hex = _required_string(value, "receipt_hash_hex")
    return types_pb2.ReceiptRef(
        receipt_hash=_hex_decode(receipt_hash_hex, "receipt_hash_hex"),
        receipt_ura=_required_string(value, "receipt_ura"),
    )


def _receipt(receipt: Any) -> dict[str, object]:
    return {
        "index": receipt.index,
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": _state_name(receipt.state),
        "timestamp_unix_ms": receipt.timestamp_unix_ms,
        "prev_receipt_hash_hex": receipt.prev_receipt_hash.hex(),
        "self_hash_hex": receipt.self_hash.hex(),
        "payload_content_type": receipt.payload_content_type,
        "cleanup_complete": receipt.cleanup_complete,
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
    }


def _response_failure(
    response: Any,
    terminal_state: str,
) -> dict[str, object] | None:
    if response.HasField("error"):
        error = response.error
        code = _response_error_code(error.code)
        return {
            "code": code.value,
            "stage": _error_stage(error.stage),
            "message": error.message,
            "retryable": error.retryable,
        }
    if terminal_state in {"Completed", "Accepted", "Admitted", "Dispatched", "Running"}:
        return None
    code = ErrorCode.TIMEOUT if terminal_state == "TimedOut" else ErrorCode.ABILITY_FAILED
    return {
        "code": code.value,
        "stage": "direct_runtime.invoke",
        "message": f"daemon invocation ended in {terminal_state}",
        "retryable": code == ErrorCode.TIMEOUT,
    }


def _response_error_code(code: str) -> ErrorCode:
    if code:
        try:
            return normalize_error_code(code)
        except SDKError:
            return ErrorCode.ABILITY_FAILED
    return ErrorCode.ABILITY_FAILED


def _state_name(value: int) -> str:
    names = {
        types_pb2.INVOCATION_STATE_ACCEPTED: "Accepted",
        types_pb2.INVOCATION_STATE_ADMITTED: "Admitted",
        types_pb2.INVOCATION_STATE_DISPATCHED: "Dispatched",
        types_pb2.INVOCATION_STATE_RUNNING: "Running",
        types_pb2.INVOCATION_STATE_COMPLETED: "Completed",
        types_pb2.INVOCATION_STATE_FAILED: "Failed",
        types_pb2.INVOCATION_STATE_TIMED_OUT: "TimedOut",
        types_pb2.INVOCATION_STATE_CANCELLED: "Cancelled",
    }
    return names.get(value, "Unspecified")


def _error_stage(value: int) -> str:
    try:
        name = types_pb2.ErrorStage.Name(value)
    except ValueError:
        return "direct_runtime.invoke"
    return name.removeprefix("ERROR_STAGE_").lower() or "direct_runtime.invoke"


def _output_json(payload: bytes, content_type: str) -> object:
    if not payload or "json" not in content_type.lower():
        return None
    try:
        return json.loads(payload.decode("utf-8"))
    except Exception:
        return None


def _grpc_uds_target(endpoint: str) -> str:
    if endpoint.startswith("unix:"):
        return endpoint
    return f"unix:{endpoint}"


def _close_channel(channel: grpc.Channel) -> None:
    close = getattr(channel, "close", None)
    if close is not None:
        close()


def _grpc_error(error: grpc.RpcError, *, endpoint: str) -> SDKError:
    code = error.code()
    message = error.details() or str(error)
    mapping = {
        grpc.StatusCode.CANCELLED: (ErrorCode.CANCELLED, RetryHint.UNKNOWN, False),
        grpc.StatusCode.DEADLINE_EXCEEDED: (ErrorCode.TIMEOUT, RetryHint.SAFE, True),
        grpc.StatusCode.UNAVAILABLE: (ErrorCode.DAEMON_OFFLINE, RetryHint.SAFE, True),
        grpc.StatusCode.INVALID_ARGUMENT: (
            ErrorCode.INVALID_INVOCATION,
            RetryHint.NEVER,
            False,
        ),
        grpc.StatusCode.PERMISSION_DENIED: (
            ErrorCode.PERMISSION_DENIED,
            RetryHint.NEVER,
            False,
        ),
        grpc.StatusCode.NOT_FOUND: (ErrorCode.NOT_FOUND, RetryHint.NEVER, False),
        grpc.StatusCode.UNIMPLEMENTED: (
            ErrorCode.PROTOCOL_MISMATCH,
            RetryHint.NEVER,
            False,
        ),
    }
    sdk_code, retry, retryable = mapping.get(
        code,
        (ErrorCode.TRANSPORT, RetryHint.UNKNOWN, False),
    )
    return _direct_error(
        message,
        code=sdk_code,
        retry=retry,
        retryable=retryable,
        details={"endpoint": endpoint, "grpc_status": str(code)},
        cause=error,
    )


def _unsupported(message: str) -> SDKError:
    return _direct_error(
        message,
        code=ErrorCode.NOT_IMPLEMENTED,
        retry=RetryHint.NEVER,
        details={"transport": "direct-axon-grpc-uds"},
    )


def _invalid_causal_context(message: str) -> SDKError:
    return _direct_error(
        message,
        code=ErrorCode.INVALID_INVOCATION,
        retry=RetryHint.NEVER,
        details={"field": "causal_context"},
    )


def _direct_error(
    message: str,
    *,
    code: ErrorCode = ErrorCode.TRANSPORT,
    retry: RetryHint = RetryHint.NEVER,
    retryable: bool = False,
    details: Mapping[str, object] | None = None,
    cause: BaseException | None = None,
) -> SDKError:
    return SDKError(
        code=code,
        stage="direct_runtime",
        retry=retry,
        retryable=retryable,
        message=message,
        details=dict(details or {}),
        cause=cause,
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _decode_object(raw: bytes, name: str) -> dict[str, object]:
    try:
        decoded = json.loads(raw.decode("utf-8"))
    except Exception as exc:
        raise _direct_error(
            f"decode {name} JSON: {exc}",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc
    if not isinstance(decoded, dict):
        raise _direct_error(
            f"{name} JSON must be an object",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return decoded


def _base64_decode(value: str, field_name: str) -> bytes:
    try:
        return base64.b64decode(value.encode("ascii"), validate=True)
    except (binascii.Error, UnicodeEncodeError) as exc:
        raise _direct_error(
            f"{field_name} must be base64: {exc}",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc


def _hex_decode(value: str, field_name: str) -> bytes:
    try:
        return bytes.fromhex(value.removeprefix("sha256:"))
    except ValueError as exc:
        raise _direct_error(
            f"{field_name} must be hex: {exc}",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc


def _timeout_seconds(value: object, default: float) -> float:
    millis = _optional_non_negative_int(value, "timeout_ms")
    if millis <= 0:
        return default
    return millis / 1000.0


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value == "":
        raise _direct_error(
            f"{field_name} is required",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _direct_error(
            f"{field_name} must be a string",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or value < 0:
        raise _direct_error(
            f"{field_name} must be a non-negative integer",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value


def _required_positive_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _direct_error(
            f"{field_name} is required",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _direct_error(
            f"{field_name} must be a non-negative integer",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value


def _required_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise _direct_error(
            f"{field_name} must be an integer",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value
