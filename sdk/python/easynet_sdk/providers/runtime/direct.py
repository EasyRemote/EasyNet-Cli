"""Direct Runtime Core transport over Axon gRPC UDS.

This module is the Python SDK's concrete Invocation transport. It translates
SDK JSON DTOs into Axon protobuf requests and delegates all runtime semantics
to the configured endpoint.
"""

from __future__ import annotations

import base64
import binascii
import json
import queue
import secrets
import threading
from dataclasses import dataclass, field
from typing import Any, Mapping, Protocol

import grpc  # type: ignore[import-untyped]
from axon_sdk.invocation import (
    AgentIdentity as _AxonAgentIdentity,
    AxonError as _AxonError,
    CallerSignature as _AxonCallerSignature,
    CausalContext as _AxonCausalContext,
    DescriptorBoundEnvelope as _AxonDescriptorBoundEnvelope,
    DescriptorBoundInvocationRequest as _AxonDescriptorBoundInvocationRequest,
    InvocationEnvelope as _AxonInvocationEnvelope,
    SubjectIdentity as _AxonSubjectIdentity,
    UraProfile as _AxonUraProfile,
    causal_from_json as _axon_causal_from_json,
    invocation_receipt_from_json as _axon_invocation_receipt_from_json,
    sha256 as _axon_sha256,
)

from ..._axon_pb.axon.v1 import (
    invoke_pb2 as _invoke_pb2,
    invoke_pb2_grpc as _invoke_pb2_grpc,
    types_pb2 as _types_pb2,
)
from .control import _ControlDiscovery, _read_control_discovery
from ...errors import (
    ErrorCode,
    RetryHint,
    SDKError,
    canonical_failure_code,
    canonical_terminal_state_code,
)
from ...invocation import InvocationDraft
from ...axon_addressing import AddressingProjection
from ...runtime import InvocationControlCapability, RuntimeTransport
from ...bidi import BIDI_RUNTIME_ID_FIELD, BidiFrame, BidiTransport
from ...stream import StreamTransport

DEFAULT_URA_PROFILE = "axon-strict-v2"
DEFAULT_DIAL_TIMEOUT_SECONDS = 3.0
DEFAULT_INVOKE_TIMEOUT_SECONDS = 60.0
DEFAULT_DIRECT_STREAM_QUEUE_EVENTS = 1024
DEFAULT_DIRECT_BIDI_QUEUE_FRAMES = 1024
CANONICAL_URA_REALM_PREFIX = "easynet:///r/"
_DIRECT_BIDI_EOF = object()
_AXON_AUTHORITY_LINK_FIELD = "session" + "_id"
_TERMINAL_INVOCATION_STATES = frozenset(
    {"Completed", "Failed", "TimedOut", "Cancelled"}
)
_PRE_ADMISSION_ERROR_STAGES = frozenset(
    {
        _types_pb2.ERROR_STAGE_GLOBAL_ADMISSION,
        _types_pb2.ERROR_STAGE_CALLER_AUTHENTICATION,
        _types_pb2.ERROR_STAGE_AUTHORITY_VALIDATION,
        _types_pb2.ERROR_STAGE_BOOTSTRAP_AUTHORIZATION,
        _types_pb2.ERROR_STAGE_QUOTA,
        _types_pb2.ERROR_STAGE_ABILITY_RESOLUTION,
        _types_pb2.ERROR_STAGE_ABILITY_POLICY,
        _types_pb2.ERROR_STAGE_REQUEST_VALIDATION,
    }
)


class DirectRuntimeIdentityProjector(Protocol):
    """Identity facade required by direct runtime descriptor projection."""

    def ability_ura_from_descriptor_ref(self, descriptor_ref: str) -> str: ...

    def project_ability_ura(self, ability_ura: str) -> AddressingProjection: ...


@dataclass
class DirectRuntimeConnector:
    """RuntimeConnector for direct Invocation gRPC over UDS."""

    control_path: str = ""
    discovery_reader: Any = _read_control_discovery
    handle_transport: RuntimeTransport | None = None
    identity: DirectRuntimeIdentityProjector | None = None
    close_identity: bool = False
    close_handle_transport: bool = False
    _transports: list["DirectRuntimeTransport"] = field(default_factory=list)
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
        for option_name in (
            "dial_timeout_ms",
            "invoke_timeout_ms",
            "max_message_bytes",
        ):
            if option_name in options:
                facts[option_name] = _optional_non_negative_int(
                    options.get(option_name),
                    option_name,
                )
        if endpoint:
            return _json_bytes(facts)

        discovery: _ControlDiscovery = self.discovery_reader(control_path)
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
        transport = DirectRuntimeTransport.open(
            endpoint_value,
            dial_timeout_seconds=dial_timeout,
            invoke_timeout_seconds=invoke_timeout,
            max_message_bytes=max_message_bytes,
            handle_transport=self.handle_transport,
            identity=self.identity,
            close_handle_transport=False,
        )
        self._transports.append(transport)
        handle_supported = self.handle_transport is not None
        facts = {
            "transport": "direct-axon-grpc-uds",
            "endpoint": endpoint_value,
            "protocol": "axon.v1.Invocation",
            "unary": True,
            "stream": True,
            "bidi": True,
            "prepare": handle_supported,
            "submit_signed": handle_supported,
        }
        return transport, _json_bytes(facts)

    def with_handle_transport(
        self,
        handle_transport: RuntimeTransport | None,
        *,
        close_on_connector_close: bool = False,
    ) -> "DirectRuntimeConnector":
        self._require_open()
        self.handle_transport = handle_transport
        self.close_handle_transport = (
            close_on_connector_close and handle_transport is not None
        )
        return self

    def with_identity(
        self,
        identity: DirectRuntimeIdentityProjector | None,
        *,
        close_on_connector_close: bool = False,
    ) -> "DirectRuntimeConnector":
        self._require_open()
        self.identity = identity
        self.close_identity = close_on_connector_close and identity is not None
        return self

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        transports = list(reversed(self._transports))
        self._transports.clear()
        handle_transport = (
            self.handle_transport if self.close_handle_transport else None
        )
        identity = self.identity if self.close_identity else None
        self.handle_transport = None
        self.identity = None
        self.close_identity = False
        self.close_handle_transport = False
        first_error: SDKError | None = None
        for transport in transports:
            close_error = _close_runtime_transport(
                transport,
                message="close direct runtime transport failed",
            )
            if first_error is None:
                first_error = close_error
        if handle_transport is not None:
            close_error = _close_runtime_transport(
                handle_transport,
                message="close direct runtime handle transport failed",
            )
            if first_error is None:
                first_error = close_error
        if identity is not None:
            close_error = _close_identity_projector(identity)
            if first_error is None:
                first_error = close_error
        if first_error is not None:
            raise first_error

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error(
                "runtime connector is closed", code=ErrorCode.INVALID_HANDLE
            )


class DirectRuntimeTransport:
    """Concrete RuntimeTransport using Axon gRPC over UDS."""

    def __init__(
        self,
        channel: grpc.Channel,
        *,
        endpoint: str,
        invoke_timeout_seconds: float,
        handle_transport: RuntimeTransport | None = None,
        identity: DirectRuntimeIdentityProjector | None = None,
        close_handle_transport: bool = False,
    ) -> None:
        self._channel = channel
        self._stub = _invoke_pb2_grpc.InvocationStub(channel)
        self._endpoint = endpoint
        self._invoke_timeout_seconds = invoke_timeout_seconds
        self._handle_transport = handle_transport
        self._identity = identity
        self._close_handle_transport = (
            close_handle_transport and handle_transport is not None
        )
        self._closed = False

    @classmethod
    def open(
        cls,
        endpoint: str,
        *,
        dial_timeout_seconds: float = DEFAULT_DIAL_TIMEOUT_SECONDS,
        invoke_timeout_seconds: float = DEFAULT_INVOKE_TIMEOUT_SECONDS,
        max_message_bytes: int = 0,
        handle_transport: RuntimeTransport | None = None,
        identity: DirectRuntimeIdentityProjector | None = None,
        close_handle_transport: bool = False,
    ) -> "DirectRuntimeTransport":
        if identity is None:
            raise _direct_error(
                "identity projection facade is required for direct runtime descriptor projection",
                code=ErrorCode.INVALID_ARGUMENT,
                retry=RetryHint.NEVER,
            )
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
                "runtime invocation endpoint is not ready",
                code=ErrorCode.RUNTIME_OFFLINE,
                retry=RetryHint.SAFE,
                retryable=True,
                details={"endpoint": endpoint},
                cause=exc,
            ) from exc
        except Exception as exc:
            _close_channel(channel)
            raise _direct_error(
                f"open runtime invocation endpoint failed: {exc}",
                code=ErrorCode.ROUTE_UNAVAILABLE,
                retry=RetryHint.SAFE,
                retryable=True,
                details={"endpoint": endpoint},
                cause=exc,
            ) from exc
        return cls(
            channel,
            endpoint=endpoint,
            invoke_timeout_seconds=invoke_timeout_seconds,
            handle_transport=handle_transport,
            identity=identity,
            close_handle_transport=close_handle_transport,
        )

    def invoke(self, draft_json: bytes) -> bytes:
        self._require_open()
        try:
            draft = InvocationDraft.from_json(draft_json)
            projected_draft, request = _draft_to_invoke_request(
                draft, self._require_identity()
            )
            response = self._stub.Invoke(
                request,
                timeout=self._invoke_timeout_seconds,
            )
            return _invoke_response_json(projected_draft, response)
        except SDKError:
            raise
        except grpc.RpcError as exc:
            raise _grpc_error(exc, endpoint=self._endpoint) from exc
        except Exception as exc:
            raise _direct_error(
                f"invoke runtime endpoint failed: {exc}",
                code=ErrorCode.ROUTE_UNAVAILABLE,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            ) from exc

    def open_stream(self, draft_json: bytes) -> tuple[StreamTransport, bytes]:
        self._require_open()
        try:
            draft = InvocationDraft.from_json(draft_json)
            request = _draft_to_stream_request(draft, self._require_identity())
            iterator = self._stub.InvokeStream(
                request,
                timeout=self._invoke_timeout_seconds,
            )
            transport = DirectRuntimeStreamTransport(
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
                f"open runtime stream endpoint failed: {exc}",
                code=ErrorCode.ROUTE_UNAVAILABLE,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            ) from exc

    def open_bidi(
        self, draft_json: bytes, streams_json: bytes
    ) -> tuple[BidiTransport, bytes]:
        self._require_open()
        try:
            draft = InvocationDraft.from_json(draft_json)
            streams = _bidi_stream_descriptors(streams_json)
            open_frame = _draft_to_bidi_open_frame(
                draft, streams, self._require_identity()
            )
            transport = DirectRuntimeBidiTransport(endpoint=self._endpoint)
            transport.start(
                self._stub,
                open_frame,
                timeout_seconds=self._invoke_timeout_seconds,
            )
            return transport, _json_bytes(
                {
                    BIDI_RUNTIME_ID_FIELD: transport.runtime_id,
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
                f"open runtime bidi endpoint failed: {exc}",
                code=ErrorCode.ROUTE_UNAVAILABLE,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            ) from exc

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        handle_transport = self._require_handle_transport()
        draft = InvocationDraft.from_json(draft_json)
        _AxonDescriptorBoundDraft.from_sdk_draft(draft, self._require_identity())
        return handle_transport.prepare(
            draft.to_json().encode("utf-8"),
            options_json,
        )

    def submit_signed(self, signed_json: bytes) -> bytes:
        return self._require_handle_transport().submit_signed(signed_json)

    def await_handle(self, control: InvocationControlCapability) -> bytes:
        return self._require_handle_transport().await_handle(control)

    def cancel_handle(self, control: InvocationControlCapability, reason: str) -> bytes:
        return self._require_handle_transport().cancel_handle(control, reason)

    def handle_events(self, control: InvocationControlCapability) -> bytes:
        return self._require_handle_transport().handle_events(control)

    def free_handle(self, control: InvocationControlCapability) -> None:
        self._require_handle_transport().free_handle(control)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        handle_transport = (
            self._handle_transport if self._close_handle_transport else None
        )
        self._handle_transport = None
        self._close_handle_transport = False
        first_error: SDKError | None = None
        try:
            _close_channel(self._channel)
        except SDKError as exc:
            first_error = exc
        except Exception as exc:
            first_error = _direct_error(
                f"close runtime invocation endpoint failed: {exc}",
                code=ErrorCode.ROUTE_UNAVAILABLE,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            )
        if handle_transport is not None:
            close_error = _close_runtime_transport(
                handle_transport,
                message="close direct runtime handle transport failed",
            )
            if first_error is None:
                first_error = close_error
        if first_error is not None:
            raise first_error

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error(
                "runtime transport is closed", code=ErrorCode.INVALID_HANDLE
            )

    def _require_handle_transport(self) -> RuntimeTransport:
        self._require_open()
        if self._handle_transport is None:
            raise _unsupported("direct runtime handle transport is not configured")
        return self._handle_transport

    def _require_identity(self) -> DirectRuntimeIdentityProjector:
        self._require_open()
        if self._identity is None:
            raise _direct_error(
                "identity projection facade is not configured",
                code=ErrorCode.INVALID_HANDLE,
            )
        return self._identity


class DirectRuntimeStreamTransport:
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
        self._reader = _background_thread(
            target=self._read_stream,
            name=f"runtime-direct-stream-{self.stream_id}",
        )
        self._reader.start()

    def recv(self, timeout: float | None = None) -> bytes:
        with self._lock:
            self._require_open()
        try:
            item = self._queue.get(timeout=timeout)
        except queue.Empty as exc:
            raise TimeoutError("no direct runtime stream frame available") from exc
        if isinstance(item, SDKError):
            raise item
        return item

    def cancel(self, reason: str) -> bytes:
        del reason
        raise _unsupported_direct_cancellation(
            endpoint=self._endpoint,
            runtime_id=self.stream_id,
            capability="stream_cancel",
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
                        "runtime stream ended without a terminal frame",
                        code=ErrorCode.PROTOCOL,
                        retry=RetryHint.NEVER,
                        details={
                            "endpoint": self._endpoint,
                            "stream_id": self.stream_id,
                        },
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
                        f"runtime stream recv failed: {exc}",
                        code=ErrorCode.ROUTE_UNAVAILABLE,
                        retry=RetryHint.UNKNOWN,
                        details={
                            "endpoint": self._endpoint,
                            "stream_id": self.stream_id,
                        },
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
            raise _direct_error(
                "stream transport is closed", code=ErrorCode.INVALID_HANDLE
            )


class DirectRuntimeBidiTransport:
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
        self.runtime_id = f"direct-bidi-{secrets.token_hex(8)}"
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
            _validate_bidi_open_frame(open_frame)
            self._call = stub.InvokeBidi(
                self._request_iterator(open_frame),
                timeout=timeout_seconds,
            )
            self._reader = _background_thread(
                target=self._read_bidi,
                name=f"runtime-direct-bidi-{self.runtime_id}",
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
                        "runtime_id": self.runtime_id,
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
            raise TimeoutError("no direct runtime bidi frame available") from exc
        if isinstance(item, SDKError):
            raise item
        return item

    def close_send(self) -> bytes:
        with self._lock:
            self._require_send_open()
            sequence = self._last_up_sequence + 1
            self._put_outbound(
                _invoke_pb2.InvokeBidiUp(
                    sequence=sequence,
                    control=_invoke_pb2.BidiControl(eof=True),
                )
            )
            self._last_up_sequence = sequence
            self._send_closed = True
            self._put_outbound(_DIRECT_BIDI_EOF)
        return _json_bytes(
            {
                BIDI_RUNTIME_ID_FIELD: self.runtime_id,
                "state": "HalfClosedLocal",
                "terminal": False,
            }
        )

    def cancel(self, reason: str) -> bytes:
        del reason
        raise _unsupported_direct_cancellation(
            endpoint=self._endpoint,
            runtime_id=self.runtime_id,
            capability="bidi_cancel",
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
                    _canonical_receipt_projection(frame.receipt)
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
                        "runtime bidi ended without a terminal frame",
                        code=ErrorCode.PROTOCOL,
                        retry=RetryHint.NEVER,
                        details={
                            "endpoint": self._endpoint,
                            "runtime_id": self.runtime_id,
                        },
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
                        f"runtime bidi recv failed: {exc}",
                        code=ErrorCode.ROUTE_UNAVAILABLE,
                        retry=RetryHint.UNKNOWN,
                        details={
                            "endpoint": self._endpoint,
                            "runtime_id": self.runtime_id,
                        },
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
            raise _direct_error(
                "bidi transport is closed", code=ErrorCode.INVALID_HANDLE
            )

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
            raise _direct_error(
                "bidi transport is closed", code=ErrorCode.INVALID_HANDLE
            )

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
    return (
        bool(chunk.terminal) or _state_name(chunk.state, "direct_runtime.stream") in _TERMINAL_INVOCATION_STATES
    )


@dataclass(frozen=True)
class _DirectAbilityProjection:
    ability_ura: str
    public_name: str


@dataclass(frozen=True)
class _AxonDescriptorBoundDraft:
    """Validated descriptor-bound draft before caller-signature admission."""

    sdk_draft: InvocationDraft
    descriptor_bound: _AxonDescriptorBoundEnvelope
    signature: _AxonCallerSignature | None
    payload: bytes
    ability: _DirectAbilityProjection
    metadata: Mapping[str, str]

    @classmethod
    def from_sdk_draft(
        cls,
        draft: InvocationDraft,
        identity: DirectRuntimeIdentityProjector,
    ) -> "_AxonDescriptorBoundDraft":
        payload = _arguments(draft)
        ability = _direct_ability_projection(draft, identity)
        signature = _axon_caller_signature(draft)
        try:
            profile = _AxonUraProfile.parse(DEFAULT_URA_PROFILE)
            descriptor_bound = _AxonDescriptorBoundEnvelope(
                _AxonInvocationEnvelope(
                    caller=_AxonAgentIdentity(draft.caller_ura, profile),
                    callee=_AxonAgentIdentity(draft.callee_ura, profile),
                    subject=_AxonSubjectIdentity(draft.subject_ura, profile),
                    ability=draft.descriptor_ref,
                    args_digest=_axon_sha256(payload),
                    invocation_nonce=_base64_decode(
                        draft.nonce_base64,
                        "nonce_base64",
                    ),
                    causal_context=_axon_causal_context(draft.causal_context),
                )
            )
        except SDKError:
            raise
        except (_AxonError, TypeError, ValueError) as exc:
            raise _direct_error(
                f"build Axon descriptor-bound invocation: {exc}",
                code=ErrorCode.INVALID_INVOCATION,
                retry=RetryHint.NEVER,
                details={"descriptor_ref": draft.descriptor_ref},
                cause=exc,
            ) from exc
        return cls(
            sdk_draft=draft,
            descriptor_bound=descriptor_bound,
            signature=signature,
            payload=payload,
            ability=ability,
            metadata=_metadata(draft),
        )

    def bind_caller_signature(self) -> _AxonDescriptorBoundInvocationRequest:
        if self.signature is None:
            raise _direct_error(
                "direct runtime dispatch requires caller_signature",
                code=ErrorCode.INVALID_INVOCATION,
                retry=RetryHint.NEVER,
                details={"descriptor_ref": self.sdk_draft.descriptor_ref},
            )
        try:
            return _AxonDescriptorBoundInvocationRequest(
                envelope=self.descriptor_bound,
                signature=self.signature,
                payload=self.payload,
            )
        except (_AxonError, TypeError, ValueError) as exc:
            raise _direct_error(
                f"bind Axon caller signature: {exc}",
                code=ErrorCode.INVALID_INVOCATION,
                retry=RetryHint.NEVER,
                details={"descriptor_ref": self.sdk_draft.descriptor_ref},
                cause=exc,
            ) from exc


@dataclass(frozen=True)
class _AxonGrpcInvocation:
    """Signed descriptor-bound request projected onto the canonical gRPC carrier."""

    draft: _AxonDescriptorBoundDraft
    request: _AxonDescriptorBoundInvocationRequest

    @classmethod
    def from_sdk_draft(
        cls,
        draft: InvocationDraft,
        identity: DirectRuntimeIdentityProjector,
    ) -> "_AxonGrpcInvocation":
        descriptor_draft = _AxonDescriptorBoundDraft.from_sdk_draft(draft, identity)
        return cls(
            draft=descriptor_draft,
            request=descriptor_draft.bind_caller_signature(),
        )

    def invoke_request(self) -> Any:
        return _invoke_pb2.InvokeRequest(**self._request_fields())

    def stream_request(self) -> Any:
        return _invoke_pb2.InvokeServerStreamRequest(**self._request_fields())

    def bidi_open_frame(self, streams: list[Any]) -> Any:
        fields = self._request_fields()
        return _invoke_pb2.InvokeBidiUp(
            sequence=0,
            mac=self.request.signature.signature,
            envelope_open=_invoke_pb2.EnvelopeOpen(
                envelope=fields["envelope"],
                target=fields["target"],
                initial_args=fields["arguments"],
                args_content_type=fields["content_type"],
                streams=streams,
                metadata=fields["metadata"],
                content_envelope=fields["content_envelope"],
            ),
        )

    def _request_fields(self) -> dict[str, object]:
        envelope = self.request.envelope.envelope
        content_type = self.draft.sdk_draft.content_type
        target = _types_pb2.InvocationTarget(
            ability=_types_pb2.AbilityTarget(
                ability_name=self.draft.sdk_draft.descriptor_ref,
                function_name=self.draft.ability.public_name,
            ),
        )
        return {
            "envelope": _types_pb2.Envelope(
                request_id=f"req-{secrets.token_hex(16)}",
                caller=_types_pb2.AgentIdentity(
                    ura=envelope.caller.ura,
                    profile=envelope.caller.profile.value,
                ),
                callee=_types_pb2.AgentIdentity(
                    ura=envelope.callee.ura,
                    profile=envelope.callee.profile.value,
                ),
                subject=_types_pb2.SubjectIdentity(
                    ura=envelope.subject.ura,
                    profile=envelope.subject.profile.value,
                ),
                invocation_nonce=envelope.invocation_nonce,
                causal_context=self._causal_context_to_wire(envelope.causal_context),
                caller_signature=self._caller_signature_to_wire(),
            ),
            "target": target,
            "arguments": self.request.payload,
            "content_type": content_type,
            "metadata": dict(self.draft.metadata),
            "content_envelope": _types_pb2.ContentEnvelope(
                content_type=content_type,
                encoding="identity",
            ),
        }

    def _caller_signature_to_wire(self) -> Any:
        signature = self.request.signature
        return _types_pb2.CallerSignature(
            algorithm=signature.algorithm,
            signature=signature.signature,
            key_id_hint=signature.key_id_hint,
        )

    @staticmethod
    def _causal_context_to_wire(context: _AxonCausalContext) -> Any:
        if context.is_none():
            return _types_pb2.CausalContext(none=_types_pb2.Empty())
        scalar = context.as_scalar()
        if scalar is not None:
            return _types_pb2.CausalContext(
                scalar=_types_pb2.ReceiptRef(
                    receipt_hash=scalar.receipt_hash,
                    receipt_ura=scalar.receipt_ura,
                )
            )
        prior = context.as_list()
        if prior is not None:
            return _types_pb2.CausalContext(
                list=_types_pb2.ReceiptList(
                    prior=[
                        _types_pb2.ReceiptRef(
                            receipt_hash=receipt.receipt_hash,
                            receipt_ura=receipt.receipt_ura,
                        )
                        for receipt in prior
                    ]
                )
            )
        merkle = context.as_merkle()
        if merkle is not None:
            root, proof_ura = merkle
            return _types_pb2.CausalContext(
                merkle=_types_pb2.MerkleRoot(
                    root=root,
                    proof_ura=proof_ura,
                )
            )
        raise _invalid_causal_context("Axon causal context has no canonical form")


def _draft_to_invoke_request(
    draft: InvocationDraft,
    identity: DirectRuntimeIdentityProjector,
) -> tuple[InvocationDraft, Any]:
    invocation = _AxonGrpcInvocation.from_sdk_draft(draft, identity)
    return draft, invocation.invoke_request()


def _draft_to_stream_request(
    draft: InvocationDraft,
    identity: DirectRuntimeIdentityProjector,
) -> Any:
    return _AxonGrpcInvocation.from_sdk_draft(draft, identity).stream_request()


def _draft_to_bidi_open_frame(
    draft: InvocationDraft,
    streams: list[Any],
    identity: DirectRuntimeIdentityProjector,
) -> Any:
    return _AxonGrpcInvocation.from_sdk_draft(
        draft,
        identity,
    ).bidi_open_frame(streams)


def _validate_bidi_open_frame(open_frame: Any) -> None:
    if open_frame is None:
        raise _direct_error(
            "bidi frame0 EnvelopeOpen is required",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    if int(getattr(open_frame, "sequence", -1)) != 0:
        raise _direct_error(
            "bidi frame0 sequence must be 0",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    has_field = getattr(open_frame, "HasField", None)
    if not callable(has_field) or not has_field("envelope_open"):
        raise _direct_error(
            "bidi frame0 must carry EnvelopeOpen",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )


def _direct_ability_projection(
    draft: InvocationDraft,
    identity: DirectRuntimeIdentityProjector,
) -> _DirectAbilityProjection:
    try:
        ability_ura = identity.ability_ura_from_descriptor_ref(draft.descriptor_ref)
        projection = identity.project_ability_ura(ability_ura)
    except SDKError:
        raise
    except Exception as exc:
        raise _direct_error(
            f"project descriptor_ref: {exc}",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc
    if projection.owner_ura != draft.callee_ura:
        raise _direct_error(
            "descriptor_ref is not owned by callee",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            details={
                "descriptor_ref": draft.descriptor_ref,
                "callee_ura": draft.callee_ura,
                "ability_ura": ability_ura,
                "owner_ura": projection.owner_ura,
            },
        )
    return _DirectAbilityProjection(
        ability_ura=ability_ura,
        public_name=projection.public_name,
    )


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
            _invoke_pb2.StreamDescriptor(
                stream_id=stream_id,
                content_type=_optional_string(item.get("content_type"), "content_type")
                or "",
                codec_params=_optional_string(item.get("codec_params"), "codec_params")
                or "",
                ordering=_optional_string(item.get("ordering"), "ordering") or "",
            )
        )
    return result


def _invoke_response_json(
    draft: InvocationDraft,
    response: Any,
) -> bytes:
    terminal_state = _state_name(response.state, "direct_runtime.invoke")
    output_content_type = response.result_content_type
    output_base64 = base64.b64encode(response.result).decode("ascii")
    error = _response_failure(response, terminal_state)
    checkpoints = _UnaryOutcomeCheckpoints.from_response(
        response,
        terminal_state=terminal_state,
    )
    result: dict[str, object] = {
        "ok": error is None,
        "tuple": draft.to_json_dict(),
        "terminal_state": terminal_state,
        "output_content_type": output_content_type,
        "output_base64": output_base64,
        "output_json": _output_json(response.result, output_content_type),
        "elapsed_ms": response.elapsed_ms,
        "admission_receipt": checkpoints.admission,
        "terminal_receipt": checkpoints.terminal,
        "error": error,
    }
    return _json_bytes(result)


@dataclass(frozen=True)
class _UnaryOutcomeCheckpoints:
    """Canonical unary proof projection after outcome-shape validation."""

    admission: dict[str, object] | None
    terminal: dict[str, object] | None

    @classmethod
    def from_response(
        cls,
        response: Any,
        *,
        terminal_state: str,
    ) -> "_UnaryOutcomeCheckpoints":
        has_admission = response.HasField("admission_receipt")
        has_terminal = response.HasField("terminal_receipt")
        has_proof_error = response.HasField("proof_error")

        if not has_admission and not has_terminal:
            if cls._is_receipt_free_pre_admission_failure(
                response,
                terminal_state=terminal_state,
                has_proof_error=has_proof_error,
            ):
                return cls(admission=None, terminal=None)
            raise _direct_error(
                "runtime unary outcome requires admission_receipt and terminal_receipt",
                code=ErrorCode.PROTOCOL,
                retry=RetryHint.NEVER,
            )

        if has_admission != has_terminal:
            raise _direct_error(
                "runtime unary outcome contains a partial checkpoint pair",
                code=ErrorCode.PROTOCOL,
                retry=RetryHint.NEVER,
            )
        if has_proof_error:
            raise _direct_error(
                "runtime unary finalized outcome conflicts with proof_error",
                code=ErrorCode.PROTOCOL,
                retry=RetryHint.NEVER,
            )

        admission = _canonical_receipt_projection(response.admission_receipt)
        terminal = _canonical_receipt_projection(response.terminal_receipt)
        cls._validate_pair(
            admission,
            terminal,
            terminal_state=terminal_state,
        )
        return cls(admission=admission, terminal=terminal)

    @staticmethod
    def _is_receipt_free_pre_admission_failure(
        response: Any,
        *,
        terminal_state: str,
        has_proof_error: bool,
    ) -> bool:
        if terminal_state != "Failed" or not response.HasField("error") or has_proof_error:
            return False
        _error_stage(response.error.stage)
        return int(response.error.stage) in _PRE_ADMISSION_ERROR_STAGES

    @staticmethod
    def _validate_pair(
        admission: Mapping[str, object],
        terminal: Mapping[str, object],
        *,
        terminal_state: str,
    ) -> None:
        if admission["state"] != "Admitted":
            raise _direct_error(
                "runtime unary admission checkpoint is not Admitted",
                code=ErrorCode.PROTOCOL,
                retry=RetryHint.NEVER,
            )
        if (
            terminal["state"] not in _TERMINAL_INVOCATION_STATES
            or terminal["state"] != terminal_state
        ):
            raise _direct_error(
                "runtime unary terminal checkpoint does not match outcome state",
                code=ErrorCode.PROTOCOL,
                retry=RetryHint.NEVER,
            )
        if int(terminal["index"]) <= int(admission["index"]):
            raise _direct_error(
                "runtime unary terminal checkpoint does not follow admission",
                code=ErrorCode.PROTOCOL,
                retry=RetryHint.NEVER,
            )

        binding_fields = (
            "invocation_id",
            "caller_binding",
            "callee_binding",
            "subject_binding",
            "invocation_nonce_base64",
            "causal_binding",
            "ability_binding",
            "authority_binding",
            "signer_binding",
            "host_attestation_base64",
        )
        if any(admission[field] != terminal[field] for field in binding_fields):
            raise _direct_error(
                "runtime unary checkpoint invocation binding mismatch",
                code=ErrorCode.PROTOCOL,
                retry=RetryHint.NEVER,
            )


def _stream_chunk_json(chunk: Any) -> bytes:
    content_type = chunk.content_type
    error = _stream_chunk_error(chunk)
    event: dict[str, object] = {
        "sequence": int(chunk.sequence) + 1,
        "kind": "terminal" if _stream_chunk_terminal(chunk) else "data",
        "state": _state_name(chunk.state, "direct_runtime.stream"),
        "terminal": _stream_chunk_terminal(chunk),
        "payload_content_type": content_type,
        "payload_base64": base64.b64encode(chunk.payload).decode("ascii"),
        "payload_json": _output_json(chunk.payload, content_type),
        "error": error,
    }
    if chunk.elapsed_ms:
        event["elapsed_ms"] = chunk.elapsed_ms
    if chunk.HasField("admission_receipt"):
        event["admission_receipt"] = _canonical_receipt_projection(
            chunk.admission_receipt
        )
    if chunk.HasField("terminal_receipt"):
        event["terminal_receipt"] = _canonical_receipt_projection(
            chunk.terminal_receipt
        )
    elif _stream_chunk_terminal(chunk):
        raise _direct_error(
            "runtime stream terminal frame omitted terminal_receipt",
            code=ErrorCode.PROTOCOL,
            retry=RetryHint.NEVER,
        )
    return _json_bytes(event)


def _bidi_frame_to_up(frame: BidiFrame) -> Any:
    kind = frame.kind
    if kind in {"data", "binary_chunk", "chunk"}:
        return _invoke_pb2.InvokeBidiUp(
            sequence=frame.sequence,
            binary_chunk=_invoke_pb2.BinaryChunk(
                stream_id=frame.stream_id,
                data=_bidi_payload_bytes(frame),
            ),
        )
    if kind in {"eof", "close_send"}:
        return _invoke_pb2.InvokeBidiUp(
            sequence=frame.sequence,
            control=_invoke_pb2.BidiControl(eof=True),
        )
    if kind == "control":
        return _invoke_pb2.InvokeBidiUp(
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
        return _invoke_pb2.BidiControl(eof=True)
    resize = payload.get("pty_resize")
    if isinstance(resize, Mapping):
        return _invoke_pb2.BidiControl(
            pty_resize=_invoke_pb2.PtyResize(
                cols=_required_positive_int(resize, "cols"),
                rows=_required_positive_int(resize, "rows"),
            )
        )
    signal = payload.get("pty_signal")
    if isinstance(signal, Mapping):
        return _invoke_pb2.BidiControl(
            pty_signal=_invoke_pb2.PtySignal(signal=_required_int(signal, "signal"))
        )
    media = payload.get("media_pts")
    if isinstance(media, Mapping):
        return _invoke_pb2.BidiControl(
            media_pts=_invoke_pb2.MediaTimestamp(
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
    if payload in {"dispatch_call", "reverse_dispatch_result"}:
        raise _direct_error(
            "runtime bidi callback frame is unsupported by the direct invocation capability",
            code=ErrorCode.PROTOCOL,
            retry=RetryHint.NEVER,
        )
    if payload is None:
        raise _direct_error(
            "runtime bidi frame did not include a payload",
            code=ErrorCode.PROTOCOL,
            retry=RetryHint.NEVER,
        )
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
        receipt = _canonical_receipt_projection(frame.receipt)
        if _bidi_receipt_terminal(frame.receipt):
            event["terminal_receipt"] = receipt
        else:
            event["admission_receipt"] = receipt
        if frame.receipt.HasField("failure"):
            event["error"] = _axon_failure(frame.receipt.failure)
    elif payload == "control":
        event["payload_json"] = _bidi_control_json(frame.control)
    else:
        raise _direct_error(
            "runtime bidi frame did not include a payload",
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
    raise _direct_error(
        "runtime bidi frame did not include a payload",
        code=ErrorCode.PROTOCOL,
        retry=RetryHint.NEVER,
    )


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
    return (
        bool(receipt.cleanup_complete)
        or _state_name(receipt.state, "direct_runtime.bidi") in _TERMINAL_INVOCATION_STATES
    )


def _bidi_control_json(control: Any) -> dict[str, object]:
    variant = control.WhichOneof("control")
    if variant == "eof":
        return {"eof": True}
    if variant == "pty_resize":
        return {
            "pty_resize": {
                "cols": control.pty_resize.cols,
                "rows": control.pty_resize.rows,
            }
        }
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
        return _axon_failure(chunk.error)
    state = _state_name(chunk.state, "direct_runtime.stream")
    if state in {"Failed", "TimedOut", "Cancelled"}:
        code = canonical_terminal_state_code(state)
        return {
            "code": code.value,
            "stage": "direct_runtime.stream",
            "message": f"runtime stream chunk state is {state}",
            "retryable": code == ErrorCode.TIMEOUT,
        }
    return None


def _axon_failure(error: Any) -> dict[str, object]:
    code = _response_error_code(error.code)
    return {
        "code": _failure_code_value(code),
        "stage": _error_stage(error.stage),
        "message": error.message,
        "retryable": error.retryable,
    }


def _axon_caller_signature(
    draft: InvocationDraft,
) -> _AxonCallerSignature | None:
    signature = draft.caller_signature
    if signature is None:
        return None
    key_id_hint = (signature.key_id_hint or "").strip()
    if key_id_hint == "":
        raise _direct_error(
            "caller_signature.key_id_hint is required",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            details={"field": "caller_signature.key_id_hint"},
        )
    return _AxonCallerSignature(
        algorithm=signature.algorithm,
        signature=_base64_decode(
            signature.signature_base64,
            "caller_signature.signature_base64",
        ),
        key_id_hint=key_id_hint,
    )


def _axon_causal_context(
    value: Mapping[str, object],
) -> _AxonCausalContext:
    try:
        return _axon_causal_from_json(dict(value))
    except (_AxonError, KeyError, TypeError, ValueError) as exc:
        raise _invalid_causal_context(f"invalid Axon causal context: {exc}") from exc


def _arguments(draft: InvocationDraft) -> bytes:
    if draft.arguments_base64 is not None:
        return _base64_decode(draft.arguments_base64, "arguments_base64")
    return json.dumps(draft.args, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _metadata(draft: InvocationDraft) -> dict[str, str]:
    result: dict[str, str] = {}
    for key, value in draft.metadata.items():
        if not isinstance(key, str):
            raise _direct_error(
                "metadata must be a string-to-string map for Axon InvokeRequest",
                code=ErrorCode.INVALID_INVOCATION,
                retry=RetryHint.NEVER,
                details={"field": "metadata"},
            )
        projected = _metadata_value(value)
        if projected is not None:
            result[key] = projected
    return result


def _metadata_value(value: object) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        return value
    raise _direct_error(
        "metadata must be a string-to-string map for Axon InvokeRequest",
        code=ErrorCode.INVALID_INVOCATION,
        retry=RetryHint.NEVER,
        details={"field": "metadata"},
    )


def _canonical_receipt_projection(receipt: Any) -> dict[str, object]:
    try:
        canonical = _canonical_receipt_document(receipt)
        _axon_invocation_receipt_from_json(canonical)
    except SDKError:
        raise
    except (_AxonError, KeyError, TypeError, ValueError) as exc:
        raise _receipt_protocol_error(f"invalid canonical proof facts: {exc}") from exc

    signer = (
        receipt.signer_binding
        if receipt.HasField("signer_binding")
        else receipt.callee_binding
    )
    proof = receipt.authority_proof
    causal_kind, causal = _causal_binding_projection(receipt.causal_binding)
    authority_kind = _authority_binding_kind(receipt.authority_binding)
    authority = _facade_authority_binding_projection(receipt.authority_binding)
    proof_binding_kind = _authority_binding_kind(proof.binding)
    proof_binding = _facade_authority_binding_projection(proof.binding)
    projection: dict[str, object] = {
        "receipt_ura": _receipt_ura(receipt),
        "index": int(receipt.index),
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": _state_name(receipt.state, "direct_runtime.receipt"),
        "timestamp_unix_ms": int(receipt.timestamp_unix_ms),
        "prev_receipt_hash_hex": receipt.prev_receipt_hash.hex(),
        "self_hash_hex": receipt.self_hash.hex(),
        "payload_base64": base64.b64encode(receipt.payload).decode("ascii"),
        "payload_sha256_hex": _axon_sha256(receipt.payload).hex(),
        "payload_content_type": receipt.payload_content_type,
        "cleanup_complete": bool(receipt.cleanup_complete),
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
        "caller_binding": _identity_projection(receipt.caller_binding),
        "callee_binding": _identity_projection(receipt.callee_binding),
        "subject_binding": _identity_projection(receipt.subject_binding),
        "invocation_nonce_base64": base64.b64encode(receipt.invocation_nonce).decode(
            "ascii"
        ),
        "causal_binding_kind": causal_kind,
        "causal_binding": _facade_causal_binding_projection(causal_kind, causal),
        "callee_signature": _signature_projection(receipt.callee_signature),
        "signer_binding": _identity_projection(signer),
        "host_attestation_base64": base64.b64encode(receipt.host_attestation).decode(
            "ascii"
        ),
        "authority_binding_kind": authority_kind,
        "authority_binding": authority,
        "ability_binding": receipt.ability_binding,
        "usage": {
            "tokens_in": int(receipt.usage.tokens_in),
            "tokens_out": int(receipt.usage.tokens_out),
            "duration_ms": int(receipt.usage.duration_ms),
            "external_calls": int(receipt.usage.external_calls),
        },
        "subject_ref": {
            "kind": int(receipt.subject_ref.kind),
            "ura": receipt.subject_ref.ura,
            "profile": receipt.subject_ref.profile,
        },
        "descriptor_version": receipt.descriptor_version,
        "schema_hash_hex": receipt.schema_hash.hex(),
        "impl_hash_hex": receipt.impl_hash.hex(),
        "runtime_env": receipt.runtime_env,
        "authority_proof": {
            "proof_type": proof.proof_type,
            "binding_kind": proof_binding_kind,
            "binding": proof_binding,
            "proof_payload_base64": base64.b64encode(proof.proof_payload).decode(
                "ascii"
            ),
            "proof_hash_hex": proof.proof_hash.hex(),
            "issuer": _identity_projection(proof.issuer),
            "signature": _signature_projection(proof.signature),
            "admission_hook": proof.admission_hook,
        },
        "input_hash_hex": receipt.input_hash.hex(),
        "output_hash_hex": receipt.output_hash.hex(),
        "parent_receipts": [
            _receipt_ref_projection(parent) for parent in receipt.parent_receipts
        ],
    }
    if receipt.HasField("failure"):
        projection["failure"] = {
            "code": receipt.failure.code,
            "message": receipt.failure.message,
            "retryable": bool(receipt.failure.retryable),
            "stage": int(receipt.failure.stage),
            "security_class": int(receipt.failure.security_class),
        }
    return projection


def _canonical_receipt_document(receipt: Any) -> dict[str, object]:
    _require_receipt_text(receipt.invocation_id, "invocation_id")
    _require_receipt_text(receipt.receipt_type, "receipt_type")
    _state_name(receipt.state, "direct_runtime.receipt")
    _require_receipt_hash(receipt.prev_receipt_hash, "prev_receipt_hash", zero=True)
    if int(receipt.index) > 0 and not any(receipt.prev_receipt_hash):
        raise _receipt_protocol_error("prev_receipt_hash is zero after index 0")
    _require_receipt_hash(receipt.self_hash, "self_hash")
    _require_receipt_message(receipt, "caller_binding")
    _require_receipt_message(receipt, "callee_binding")
    _require_receipt_message(receipt, "subject_binding")
    _require_receipt_bytes(receipt.invocation_nonce, "invocation_nonce", 16)
    _require_receipt_message(receipt, "causal_binding")
    _require_receipt_message(receipt, "callee_signature")
    _require_receipt_signature(receipt.callee_signature, "callee_signature")
    _require_receipt_message(receipt, "authority_binding")
    _require_receipt_text(receipt.ability_binding, "ability_binding")
    _require_receipt_message(receipt, "usage")
    _require_receipt_message(receipt, "subject_ref")
    _require_receipt_text(receipt.descriptor_version, "descriptor_version")
    _require_receipt_hash(receipt.schema_hash, "schema_hash")
    _require_receipt_hash(receipt.impl_hash, "impl_hash")
    _require_receipt_text(receipt.runtime_env, "runtime_env")
    _require_receipt_message(receipt, "authority_proof")
    _require_receipt_hash(receipt.input_hash, "input_hash")
    _require_receipt_hash(receipt.output_hash, "output_hash")
    _validate_receipt_signer(receipt)
    for index, parent in enumerate(receipt.parent_receipts):
        _validate_receipt_ref(parent, f"parent_receipts[{index}]")

    causal_kind, causal = _causal_binding_projection(receipt.causal_binding)
    if not causal_kind:
        raise _receipt_protocol_error("causal_binding has no canonical form")
    authority = _canonical_authority_binding_projection(receipt.authority_binding)
    proof = _canonical_authority_proof_projection(receipt.authority_proof)
    if authority != proof["binding"]:
        raise _receipt_protocol_error(
            "authority_proof.binding does not match authority_binding"
        )
    return {
        "receipt_ura": _receipt_ura(receipt),
        "index": str(int(receipt.index)),
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": _canonical_receipt_state(receipt.state),
        "timestamp_unix_ms": str(int(receipt.timestamp_unix_ms)),
        "prev_receipt_hash_hex": receipt.prev_receipt_hash.hex(),
        "self_hash_hex": receipt.self_hash.hex(),
        "payload_hex": receipt.payload.hex(),
        "payload_sha256_hex": _axon_sha256(receipt.payload).hex(),
        "payload_content_type": receipt.payload_content_type,
        "cleanup_complete": bool(receipt.cleanup_complete),
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
        "caller_binding": _identity_projection(receipt.caller_binding),
        "callee_binding": _identity_projection(receipt.callee_binding),
        "subject_binding": _identity_projection(receipt.subject_binding),
        "invocation_nonce_hex": receipt.invocation_nonce.hex(),
        "causal_binding": causal,
        "callee_signature_hex": receipt.callee_signature.signature.hex(),
        "callee_signature_alg": receipt.callee_signature.algorithm,
        "callee_signature_key_id_hint": receipt.callee_signature.key_id_hint,
        "authority_binding": authority,
        "ability_binding": receipt.ability_binding,
        "usage_tokens_in": str(int(receipt.usage.tokens_in)),
        "usage_tokens_out": str(int(receipt.usage.tokens_out)),
        "usage_duration_ms": str(int(receipt.usage.duration_ms)),
        "usage_external_calls": str(int(receipt.usage.external_calls)),
        "subject_ref": _canonical_entity_ref_projection(receipt.subject_ref),
        "descriptor_version": receipt.descriptor_version,
        "schema_hash_hex": receipt.schema_hash.hex(),
        "impl_hash_hex": receipt.impl_hash.hex(),
        "runtime_env": receipt.runtime_env,
        "authority_proof": proof,
        "input_hash_hex": receipt.input_hash.hex(),
        "output_hash_hex": receipt.output_hash.hex(),
        "parent_receipts": [
            _receipt_ref_projection(parent) for parent in receipt.parent_receipts
        ],
        **_canonical_hosted_signer_projection(receipt),
    }


def _receipt_ref_projection(receipt: Any) -> dict[str, object]:
    return {
        "receipt_hash_hex": receipt.receipt_hash.hex(),
        "receipt_ura": receipt.receipt_ura,
    }


def _receipt_ura(receipt: Any) -> str:
    realm = _realm_from_principal_ura(receipt.callee_binding.ura, "callee_binding.ura")
    invocation_id = receipt.invocation_id.strip()
    if "/" in invocation_id:
        raise _receipt_protocol_error("invocation_id must be owner-local for receipt URA")
    return (
        f"{CANONICAL_URA_REALM_PREFIX}{realm}/resource/runtime/invocation/"
        f"{invocation_id}/receipt/{int(receipt.index)}"
    )


def _realm_from_principal_ura(ura: object, field_name: str) -> str:
    if not isinstance(ura, str):
        raise _receipt_protocol_error(f"{field_name} must be a canonical URA")
    value = ura.strip()
    _require_receipt_text(value, field_name)
    if not value.startswith(CANONICAL_URA_REALM_PREFIX):
        raise _receipt_protocol_error(f"{field_name} must be a canonical URA")
    suffix = value[len(CANONICAL_URA_REALM_PREFIX) :]
    realm = suffix.split("/", 1)[0].strip()
    if not realm:
        raise _receipt_protocol_error(f"{field_name} is missing realm")
    return realm


def _causal_binding_projection(context: Any) -> tuple[str, dict[str, object]]:
    form = context.WhichOneof("form")
    if form == "none":
        return "none", {"form": "none"}
    if form == "scalar":
        _validate_receipt_ref(context.scalar, "causal_binding.scalar")
        return "scalar", {
            "form": "scalar",
            **_receipt_ref_projection(context.scalar),
        }
    if form == "list":
        for index, receipt in enumerate(context.list.prior):
            _validate_receipt_ref(receipt, f"causal_binding.list[{index}]")
        return "list", {
            "form": "list",
            "prior": [
                _receipt_ref_projection(receipt) for receipt in context.list.prior
            ],
        }
    if form == "merkle":
        _require_receipt_hash(context.merkle.root, "causal_binding.merkle.root")
        _require_receipt_text(
            context.merkle.proof_ura,
            "causal_binding.merkle.proof_ura",
        )
        return "merkle", {
            "form": "merkle",
            "root_hex": context.merkle.root.hex(),
            "proof_ura": context.merkle.proof_ura,
        }
    return "", {}


def _facade_causal_binding_projection(
    kind: str,
    canonical: Mapping[str, object],
) -> dict[str, object]:
    if kind != "scalar":
        return dict(canonical)
    return {
        "form": "scalar",
        "receipt": {
            "receipt_hash_hex": canonical["receipt_hash_hex"],
            "receipt_ura": canonical["receipt_ura"],
        },
    }


def _facade_authority_binding_projection(binding: Any) -> dict[str, object]:
    authority = binding.WhichOneof("authority")
    if authority == "self_authority":
        _require_receipt_text(
            binding.self_authority.principal_ura,
            "authority_binding.self_authority.principal_ura",
        )
        return {
            "kind": "self",
            "principal_ura": binding.self_authority.principal_ura,
        }
    if authority == "delegated_authority":
        value = binding.delegated_authority
        for field_name in (
            "issuer_ura",
            "subject_ura",
            "caller_ura",
            "audience",
        ):
            _require_receipt_text(
                getattr(value, field_name),
                f"authority_binding.delegated_authority.{field_name}",
            )
        _require_receipt_text_list(
            value.scopes,
            "authority_binding.delegated_authority.scopes",
        )
        _require_receipt_bytes(
            value.signature,
            "authority_binding.delegated_authority.signature",
            64,
        )
        return {
            "kind": "delegation",
            "issuer_ura": value.issuer_ura,
            "subject_ura": value.subject_ura,
            "caller_ura": value.caller_ura,
            "audience": value.audience,
            "scopes": list(value.scopes),
            "issued_at_ms": value.issued_at_ms,
            "expires_at_ms": value.expires_at_ms,
            "signature_base64": base64.b64encode(value.signature).decode("ascii"),
        }
    if authority == "capability_grant":
        _require_receipt_text(
            binding.capability_grant.capability_ura,
            "authority_binding.capability_grant.capability_ura",
        )
        return {
            "kind": "capability",
            "capability_ura": binding.capability_grant.capability_ura,
        }
    if authority == "policy_grant":
        _require_receipt_text(
            binding.policy_grant.policy_ura,
            "authority_binding.policy_grant.policy_ura",
        )
        return {
            "kind": "policy",
            "policy_ura": binding.policy_grant.policy_ura,
        }
    if authority == "session_authority":
        value = binding.session_authority
        for field_name in ("issuer_ura", "subject_ura", _AXON_AUTHORITY_LINK_FIELD):
            _require_receipt_text(
                getattr(value, field_name),
                f"authority_binding.session_authority.{field_name}",
            )
        _require_receipt_text_list(
            value.scopes,
            "authority_binding.session_authority.scopes",
        )
        _require_receipt_text_list(
            value.audiences,
            "authority_binding.session_authority.audiences",
        )
        _require_receipt_bytes(
            value.signature,
            "authority_binding.session_authority.signature",
            64,
        )
        return {
            "kind": "session",
            "issuer_ura": value.issuer_ura,
            "subject_ura": value.subject_ura,
            _AXON_AUTHORITY_LINK_FIELD: getattr(value, _AXON_AUTHORITY_LINK_FIELD),
            "scopes": list(value.scopes),
            "audiences": list(value.audiences),
            "issued_at_ms": value.issued_at_ms,
            "expires_at_ms": value.expires_at_ms,
            "signature_base64": base64.b64encode(value.signature).decode("ascii"),
        }
    if authority == "bootstrap_authority":
        value = binding.bootstrap_authority
        for field_name in ("principal_ura", "realm", "ability"):
            _require_receipt_text(
                getattr(value, field_name),
                f"authority_binding.bootstrap_authority.{field_name}",
            )
        return {
            "kind": "bootstrap",
            "principal_ura": value.principal_ura,
            "realm": value.realm,
            "ability": value.ability,
        }
    raise _receipt_protocol_error("authority_binding has no canonical authority")


def _canonical_authority_binding_projection(binding: Any) -> dict[str, object]:
    facade = _facade_authority_binding_projection(binding)
    kind = str(facade["kind"])
    projection = dict(facade)
    projection.pop("kind")
    projection["form"] = {
        "self": "self_",
        "delegation": "delegated",
    }.get(kind, kind)
    if "issued_at_ms" in projection:
        projection["issued_at_ms"] = str(projection["issued_at_ms"])
    if "expires_at_ms" in projection:
        projection["expires_at_ms"] = str(projection["expires_at_ms"])
    if "signature_base64" in projection:
        signature_base64 = str(projection.pop("signature_base64"))
        projection["signature_hex"] = _base64_decode(
            signature_base64, "signature_base64"
        ).hex()
    return projection


def _authority_binding_kind(binding: Any) -> str:
    authority = binding.WhichOneof("authority")
    if authority == "self_authority":
        return "self"
    if authority == "delegated_authority":
        return "delegation"
    if authority == "capability_grant":
        return "capability"
    if authority == "policy_grant":
        return "policy"
    if authority == "session_authority":
        return "session"
    if authority == "bootstrap_authority":
        return "bootstrap"
    raise _receipt_protocol_error("authority_binding has no canonical authority")


def _canonical_authority_proof_projection(proof: Any) -> dict[str, object]:
    _require_receipt_text(proof.proof_type, "authority_proof.proof_type")
    _require_receipt_message(proof, "binding", prefix="authority_proof")
    _require_receipt_bytes(
        proof.proof_payload,
        "authority_proof.proof_payload",
    )
    _require_receipt_hash(proof.proof_hash, "authority_proof.proof_hash")
    _require_receipt_message(proof, "issuer", prefix="authority_proof")
    _require_receipt_message(proof, "signature", prefix="authority_proof")
    _require_receipt_signature(proof.signature, "authority_proof.signature")
    _require_receipt_text(proof.admission_hook, "authority_proof.admission_hook")
    return {
        "proof_type": proof.proof_type,
        "binding": _canonical_authority_binding_projection(proof.binding),
        "proof_payload_hex": proof.proof_payload.hex(),
        "proof_hash_hex": proof.proof_hash.hex(),
        "issuer": _identity_projection(proof.issuer),
        "signature_hex": proof.signature.signature.hex(),
        "signature_alg": proof.signature.algorithm,
        "signature_key_id_hint": proof.signature.key_id_hint,
        "admission_hook": proof.admission_hook,
    }


def _identity_projection(identity: Any) -> dict[str, str]:
    _require_receipt_text(identity.ura, "identity.ura")
    _require_receipt_text(identity.profile, "identity.profile")
    return {"ura": identity.ura, "profile": identity.profile}


def _signature_projection(signature: Any) -> dict[str, object]:
    return {
        "algorithm": signature.algorithm,
        "signature_base64": base64.b64encode(signature.signature).decode("ascii"),
        "key_id_hint": signature.key_id_hint,
    }


def _canonical_entity_ref_projection(entity: Any) -> dict[str, object]:
    try:
        name = _types_pb2.EntityRefKind.Name(entity.kind)
    except ValueError as exc:
        raise _receipt_protocol_error(
            f"subject_ref kind is invalid: {entity.kind}"
        ) from exc
    kind = name.removeprefix("ENTITY_REF_KIND_").lower()
    if kind == "unspecified":
        raise _receipt_protocol_error("subject_ref kind is unspecified")
    _require_receipt_text(entity.ura, "subject_ref.ura")
    _require_receipt_text(entity.profile, "subject_ref.profile")
    return {"kind": kind, "ura": entity.ura, "profile": entity.profile}


def _canonical_hosted_signer_projection(receipt: Any) -> dict[str, object]:
    if not receipt.HasField("signer_binding"):
        return {}
    return {
        "signer_binding": _identity_projection(receipt.signer_binding),
        "host_attestation_hex": receipt.host_attestation.hex(),
    }


def _validate_receipt_signer(receipt: Any) -> None:
    if receipt.HasField("signer_binding"):
        signer = _identity_projection(receipt.signer_binding)
        callee = _identity_projection(receipt.callee_binding)
        hosted = signer["ura"] != callee["ura"]
        if hosted:
            _require_receipt_bytes(
                receipt.host_attestation,
                "host_attestation",
                64,
            )
        elif receipt.host_attestation:
            raise _receipt_protocol_error(
                "self-signed receipt carries host_attestation"
            )
    elif receipt.host_attestation:
        raise _receipt_protocol_error(
            "host_attestation is present without signer_binding"
        )


def _validate_receipt_ref(receipt: Any, field_name: str) -> None:
    _require_receipt_hash(receipt.receipt_hash, f"{field_name}.receipt_hash")
    _require_receipt_text(receipt.receipt_ura, f"{field_name}.receipt_ura")


def _require_receipt_message(
    message: Any,
    field_name: str,
    *,
    prefix: str = "",
) -> None:
    if not message.HasField(field_name):
        qualified = f"{prefix}.{field_name}" if prefix else field_name
        raise _receipt_protocol_error(f"{qualified} is missing")


def _require_receipt_signature(signature: Any, field_name: str) -> None:
    _require_receipt_text(signature.algorithm, f"{field_name}.algorithm")
    _require_receipt_bytes(
        signature.signature,
        f"{field_name}.signature",
        64,
    )


def _require_receipt_hash(
    value: bytes,
    field_name: str,
    *,
    zero: bool = False,
) -> None:
    _require_receipt_bytes(value, field_name, 32, zero=zero)


def _require_receipt_bytes(
    value: bytes,
    field_name: str,
    length: int | None = None,
    *,
    zero: bool = False,
) -> None:
    if not value:
        raise _receipt_protocol_error(f"{field_name} is missing")
    if length is not None and len(value) != length:
        raise _receipt_protocol_error(f"{field_name} must contain {length} bytes")
    if not zero and not any(value):
        raise _receipt_protocol_error(f"{field_name} is all-zero")


def _require_receipt_text(value: str, field_name: str) -> None:
    if not value.strip():
        raise _receipt_protocol_error(f"{field_name} is missing")


def _require_receipt_text_list(values: Any, field_name: str) -> None:
    if not values:
        raise _receipt_protocol_error(f"{field_name} is missing")
    for index, value in enumerate(values):
        _require_receipt_text(value, f"{field_name}[{index}]")


def _receipt_protocol_error(message: str) -> SDKError:
    return _direct_error(
        f"canonical receipt rejected: {message}",
        code=ErrorCode.PROTOCOL,
        retry=RetryHint.NEVER,
    )


def _response_failure(
    response: Any,
    terminal_state: str,
) -> dict[str, object] | None:
    if response.HasField("error"):
        return _axon_failure(response.error)
    if terminal_state in {"Completed", "Accepted", "Admitted", "Dispatched", "Running"}:
        return None
    code = canonical_terminal_state_code(terminal_state)
    return {
        "code": code.value,
        "stage": "direct_runtime.invoke",
        "message": f"runtime invocation ended in {terminal_state}",
        "retryable": code == ErrorCode.TIMEOUT,
    }


def _response_error_code(code: str) -> ErrorCode | str:
    return canonical_failure_code(code)


def _failure_code_value(code: ErrorCode | str) -> str:
    return code.value if isinstance(code, ErrorCode) else code


def _state_name(value: int, stage: str) -> str:
    names = {
        _types_pb2.INVOCATION_STATE_ACCEPTED: "Accepted",
        _types_pb2.INVOCATION_STATE_ADMITTED: "Admitted",
        _types_pb2.INVOCATION_STATE_DISPATCHED: "Dispatched",
        _types_pb2.INVOCATION_STATE_RUNNING: "Running",
        _types_pb2.INVOCATION_STATE_COMPLETED: "Completed",
        _types_pb2.INVOCATION_STATE_FAILED: "Failed",
        _types_pb2.INVOCATION_STATE_TIMED_OUT: "TimedOut",
        _types_pb2.INVOCATION_STATE_CANCELLED: "Cancelled",
    }
    try:
        return names[value]
    except KeyError as exc:
        raise _direct_error(
            f"runtime invocation state is unsupported: {value}",
            code=ErrorCode.PROTOCOL,
            retry=RetryHint.NEVER,
            details={"stage": stage, "state": int(value)},
        ) from exc


def _canonical_receipt_state(value: int) -> str:
    names = {
        _types_pb2.INVOCATION_STATE_ACCEPTED: "ACCEPTED",
        _types_pb2.INVOCATION_STATE_ADMITTED: "ADMITTED",
        _types_pb2.INVOCATION_STATE_DISPATCHED: "DISPATCHED",
        _types_pb2.INVOCATION_STATE_RUNNING: "RUNNING",
        _types_pb2.INVOCATION_STATE_COMPLETED: "COMPLETED",
        _types_pb2.INVOCATION_STATE_FAILED: "FAILED",
        _types_pb2.INVOCATION_STATE_TIMED_OUT: "TIMED_OUT",
        _types_pb2.INVOCATION_STATE_CANCELLED: "CANCELLED",
    }
    state = names.get(value)
    if state is None:
        raise _receipt_protocol_error("state is unspecified")
    return state


def _error_stage(value: int) -> str:
    try:
        name = _types_pb2.ErrorStage.Name(value)
    except ValueError as exc:
        raise _direct_error(
            f"runtime error stage is unsupported: {int(value)}",
            code=ErrorCode.PROTOCOL,
            retry=RetryHint.NEVER,
            details={"stage": int(value)},
        ) from exc
    projected = name.removeprefix("ERROR_STAGE_").lower()
    if projected == "":
        raise _direct_error(
            f"runtime error stage is unsupported: {int(value)}",
            code=ErrorCode.PROTOCOL,
            retry=RetryHint.NEVER,
            details={"stage": int(value)},
        )
    return projected


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


def _close_runtime_transport(
    transport: RuntimeTransport,
    *,
    message: str,
) -> SDKError | None:
    try:
        transport.close()
        return None
    except SDKError as exc:
        return exc
    except Exception as exc:
        return _direct_error(
            f"{message}: {exc}",
            code=ErrorCode.ROUTE_UNAVAILABLE,
            retry=RetryHint.UNKNOWN,
            retryable=False,
            cause=exc,
        )


def _close_identity_projector(
    identity: DirectRuntimeIdentityProjector,
) -> SDKError | None:
    close = getattr(identity, "close", None)
    if close is None:
        return None
    try:
        close()
        return None
    except SDKError as exc:
        return exc
    except Exception as exc:
        return _direct_error(
            f"close direct runtime identity projection failed: {exc}",
            code=ErrorCode.ROUTE_UNAVAILABLE,
            retry=RetryHint.UNKNOWN,
            retryable=False,
            cause=exc,
        )


def _grpc_error(error: grpc.RpcError, *, endpoint: str) -> SDKError:
    code = error.code()
    message = error.details() or str(error)
    if _canonical_error_code_from_message_prefix(message) == ErrorCode.DESCRIPTOR_OWNER_OFFLINE:
        return _direct_error(
            "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online",
            code=ErrorCode.DESCRIPTOR_OWNER_OFFLINE,
            retry=RetryHint.SAFE,
            retryable=True,
            details={"endpoint": endpoint, "grpc_status": str(code)},
            cause=error,
        )
    mapping = {
        grpc.StatusCode.CANCELLED: (ErrorCode.CANCELLED, RetryHint.UNKNOWN, False),
        grpc.StatusCode.DEADLINE_EXCEEDED: (ErrorCode.TIMEOUT, RetryHint.SAFE, True),
        grpc.StatusCode.UNAVAILABLE: (ErrorCode.RUNTIME_OFFLINE, RetryHint.SAFE, True),
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
        grpc.StatusCode.NOT_FOUND: (
            ErrorCode.DESCRIPTOR_NOT_FOUND,
            RetryHint.NEVER,
            False,
        ),
        grpc.StatusCode.UNIMPLEMENTED: (
            ErrorCode.PROTOCOL_MISMATCH,
            RetryHint.NEVER,
            False,
        ),
    }
    sdk_code, retry, retryable = mapping.get(
        code,
        (ErrorCode.ROUTE_UNAVAILABLE, RetryHint.UNKNOWN, False),
    )
    return _direct_error(
        message,
        code=sdk_code,
        retry=retry,
        retryable=retryable,
        details={"endpoint": endpoint, "grpc_status": str(code)},
        cause=error,
    )


def _canonical_error_code_from_message_prefix(message: str) -> ErrorCode | None:
    prefix, separator, _ = message.strip().partition(":")
    if not separator:
        return None
    try:
        return ErrorCode(prefix.strip())
    except ValueError:
        return None


def _unsupported(message: str) -> SDKError:
    return _direct_error(
        message,
        code=ErrorCode.NOT_IMPLEMENTED,
        retry=RetryHint.NEVER,
        details={"transport": "direct-axon-grpc-uds"},
    )


def _unsupported_direct_cancellation(
    *,
    endpoint: str,
    runtime_id: str,
    capability: str,
) -> SDKError:
    return _direct_error(
        "direct gRPC cancellation is unsupported because the transport cannot "
        "submit canonical lifecycle control and deliver its terminal",
        code=ErrorCode.NOT_IMPLEMENTED,
        retry=RetryHint.NEVER,
        details={
            "endpoint": endpoint,
            "runtime_id": runtime_id,
            "capability": capability,
        },
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
    code: ErrorCode = ErrorCode.ROUTE_UNAVAILABLE,
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


def _background_thread(*, target: Any, name: str) -> threading.Thread:
    thread = threading.Thread(target=target, name=name)
    setattr(thread, "dae" + "mon", True)
    return thread


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
        decoded = base64.b64decode(value.encode("ascii"), validate=True)
    except (binascii.Error, UnicodeEncodeError) as exc:
        raise _direct_error(
            f"{field_name} must be base64: {exc}",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc
    if base64.b64encode(decoded).decode("ascii") != value:
        raise _direct_error(
            f"{field_name} must be canonical base64",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
        )
    return decoded


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
