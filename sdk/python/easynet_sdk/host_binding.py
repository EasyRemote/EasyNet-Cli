"""Host Binding profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
from .errors import ErrorCode, RetryHint, SDKError


HOST_STREAM_FRAME_SCHEMA = "host-stream-frame.schema.json"
HOST_STREAM_HASH_ALGORITHM = "sha256(prev_hash || seq_be || canonical_json(value))"
HOST_STREAM_EMPTY_OUTPUT_HASH = (
    "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
)


class HostStreamSessionState(StrEnum):
    """Per-call host-stream session states."""

    OPEN = "Open"
    TERMINAL = "Terminal"
    CLOSED = "Closed"


@dataclass(frozen=True)
class HostStreamBindingRequest:
    """Declare a daemon-to-host execution binding."""

    binding_id: str
    descriptor_ref: str
    endpoint: str
    frame_schema: str = HOST_STREAM_FRAME_SCHEMA
    cleanup: Optional[Mapping[str, object]] = None
    timeout_ms: Optional[int] = None
    readiness: Optional[Mapping[str, object]] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        _validate_binding_request(self)
        value: dict[str, object] = {
            "binding_id": self.binding_id,
            "descriptor_ref": self.descriptor_ref,
            "endpoint": self.endpoint,
            "frame_schema": self.frame_schema,
        }
        if self.cleanup is not None:
            value["cleanup"] = dict(self.cleanup)
        if self.timeout_ms is not None:
            value["timeout_ms"] = self.timeout_ms
        if self.readiness is not None:
            value["readiness"] = dict(self.readiness)
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class HostStreamBinding:
    """Schema-backed host-stream binding projection."""

    binding_id: str
    descriptor_ref: str
    endpoint: str
    frame_schema: str
    cleanup: Mapping[str, object]
    timeout_ms: Optional[int]
    readiness: Mapping[str, object]
    lifecycle: Mapping[str, object]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "HostStreamBinding":
        decoded = _json_object(raw, "host stream binding")
        binding = cls(
            binding_id=_required_string(decoded, "binding_id"),
            descriptor_ref=_required_string(decoded, "descriptor_ref"),
            endpoint=_required_string(decoded, "endpoint"),
            frame_schema=_required_string(decoded, "frame_schema"),
            cleanup=_required_mapping(decoded, "cleanup"),
            timeout_ms=_optional_int(decoded.get("timeout_ms"), "timeout_ms"),
            readiness=_required_mapping(decoded, "readiness"),
            lifecycle=_required_mapping(decoded, "lifecycle"),
            metadata=_required_mapping(decoded, "metadata"),
        )
        if binding.frame_schema != HOST_STREAM_FRAME_SCHEMA:
            raise _invalid_host_binding(
                "frame_schema must be host-stream-frame.schema.json"
            )
        if not _is_absolute_host_endpoint(binding.endpoint):
            raise _invalid_host_binding("host stream endpoint must be absolute")
        return binding


@dataclass(frozen=True)
class HostStreamEnvelopeRequest:
    fn: str
    args: Any
    call_id: str
    caller: str

    def to_json_dict(self) -> dict[str, object]:
        if not self.fn or not self.call_id or not self.caller:
            raise _invalid_host_binding("host stream envelope request is incomplete")
        return {
            "fn": self.fn,
            "args": self.args,
            "call_id": self.call_id,
            "caller": self.caller,
        }


@dataclass(frozen=True)
class HostStreamEnvelope:
    request: HostStreamEnvelopeRequest

    def to_json_bytes(self) -> bytes:
        return _json_bytes({"request": self.request.to_json_dict()})


@dataclass(frozen=True)
class HostStreamRequest:
    """Decoded host request projection."""

    function: str
    args: Any
    call_id: str
    caller: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "HostStreamRequest":
        decoded = _json_object(raw, "host stream request")
        return cls(
            function=_required_string(decoded, "function"),
            args=decoded.get("args"),
            call_id=_required_string(decoded, "call_id"),
            caller=_required_string(decoded, "caller"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class HostStreamTerminalSummary:
    output_hash: str
    frames: int
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        if not self.output_hash or self.frames < 0:
            raise _invalid_host_binding("terminal output_hash and frames are required")
        value: dict[str, object] = {
            "output_hash": self.output_hash,
            "frames": self.frames,
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class HostStreamFrame:
    """One item/error/terminal host-stream frame."""

    frame_type: str
    seq: Optional[int]
    value: Any
    error: Optional[SDKError]
    terminal: Optional[HostStreamTerminalSummary]
    output_hash: Optional[str]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "HostStreamFrame":
        decoded = _json_object(raw, "host stream frame")
        error_value = decoded.get("error")
        frame = cls(
            frame_type=_required_string(decoded, "frame_type"),
            seq=_optional_non_negative_int(decoded.get("seq"), "seq"),
            value=decoded.get("value"),
            error=_optional_sdk_error(error_value, "error"),
            terminal=_optional_terminal(decoded.get("terminal"), "terminal"),
            output_hash=_optional_string(decoded.get("output_hash"), "output_hash"),
        )
        _validate_frame(frame)
        return frame


@dataclass(frozen=True)
class HostStreamHashState:
    """Output-hash folding state."""

    algorithm: str
    output_hash: str
    frames: int
    last_seq: Optional[int]
    canonical_json: str = ""

    @classmethod
    def from_json(cls, raw: bytes | str) -> "HostStreamHashState":
        decoded = _json_object(raw, "host stream hash state")
        state = cls(
            algorithm=_required_string(decoded, "algorithm"),
            output_hash=_required_string(decoded, "output_hash"),
            frames=_required_non_negative_int(decoded, "frames"),
            last_seq=_optional_non_negative_int(decoded.get("last_seq"), "last_seq"),
            canonical_json=_optional_string(
                decoded.get("canonical_json"), "canonical_json"
            )
            or "",
        )
        if state.algorithm != HOST_STREAM_HASH_ALGORITHM:
            raise _invalid_host_binding("invalid host stream hash algorithm")
        _validate_hash_state_consistency(state.frames, state.last_seq)
        return state

    @classmethod
    def initial(cls) -> "HostStreamHashState":
        return cls(
            algorithm=HOST_STREAM_HASH_ALGORITHM,
            output_hash=HOST_STREAM_EMPTY_OUTPUT_HASH,
            frames=0,
            last_seq=None,
        )

    def to_json_dict(self) -> dict[str, object]:
        if self.algorithm != HOST_STREAM_HASH_ALGORITHM or not self.output_hash or self.frames < 0:
            raise _invalid_host_binding("valid hash state is required")
        _validate_hash_state_consistency(self.frames, self.last_seq)
        value: dict[str, object] = {
            "algorithm": self.algorithm,
            "output_hash": self.output_hash,
            "frames": self.frames,
            "last_seq": self.last_seq,
        }
        if self.canonical_json:
            value["canonical_json"] = self.canonical_json
        return value


@runtime_checkable
class HostBindingTransport(Protocol):
    """Concrete host binding codec/hash operations supplied by integration."""

    def build_host_stream_binding(self, request_json: bytes) -> bytes:
        ...

    def decode_request(self, envelope_json: bytes) -> bytes:
        ...

    def encode_item(self, request_json: bytes) -> bytes:
        ...

    def encode_error(self, request_json: bytes) -> bytes:
        ...

    def encode_terminal(self, request_json: bytes) -> bytes:
        ...

    def fold_output_hash(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class HostBindingClient:
    """Host Binding profile facade."""

    transport: HostBindingTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_host_binding("host binding transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("host binding"))

    def build_host_stream_binding(
        self, request: HostStreamBindingRequest
    ) -> HostStreamBinding:
        self._require_open()
        try:
            raw = self.transport.build_host_stream_binding(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("host binding build failed", exc) from exc
        return HostStreamBinding.from_json(raw)

    def decode_request(self, envelope: HostStreamEnvelope) -> HostStreamRequest:
        self._require_open()
        try:
            raw = self.transport.decode_request(envelope.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("host binding decode request failed", exc) from exc
        return HostStreamRequest.from_json(raw)

    def encode_item(self, seq: int, value: object) -> HostStreamFrame:
        self._require_open()
        if seq < 0:
            raise _invalid_host_binding("seq must be non-negative")
        try:
            raw = self.transport.encode_item(_json_bytes({"seq": seq, "value": value}))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("host binding encode item failed", exc) from exc
        return HostStreamFrame.from_json(raw)

    def encode_error(self, error: BaseException) -> HostStreamFrame:
        self._require_open()
        if error is None:
            raise _invalid_host_binding("error is required")
        try:
            raw = self.transport.encode_error(
                _json_bytes({"error": _error_json_dict(error)})
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("host binding encode error failed", exc) from exc
        return HostStreamFrame.from_json(raw)

    def encode_terminal(
        self, summary: HostStreamTerminalSummary
    ) -> HostStreamFrame:
        self._require_open()
        try:
            raw = self.transport.encode_terminal(
                _json_bytes({"summary": summary.to_json_dict()})
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("host binding encode terminal failed", exc) from exc
        return HostStreamFrame.from_json(raw)

    def fold_output_hash(
        self, state: HostStreamHashState, seq: int, value: object
    ) -> HostStreamHashState:
        self._require_open()
        if seq < 0:
            raise _invalid_host_binding("seq must be non-negative")
        _validate_hash_fold(state, seq)
        try:
            raw = self.transport.fold_output_hash(
                _json_bytes({"state": state.to_json_dict(), "seq": seq, "value": value})
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("host binding hash fold failed", exc) from exc
        return HostStreamHashState.from_json(raw)

    def open_frame_writer(
        self, initial_state: Optional[HostStreamHashState] = None
    ) -> "HostStreamFrameWriter":
        self._require_open()
        return HostStreamFrameWriter(
            client=self,
            state=initial_state or HostStreamHashState.initial(),
        )

    def open_session(
        self,
        envelope: HostStreamEnvelope,
        initial_state: Optional[HostStreamHashState] = None,
    ) -> "HostStreamSession":
        """Decode one daemon envelope and open a stateful frame session."""

        self._require_open()
        return HostStreamSession(
            request=self.decode_request(envelope),
            writer=self.open_frame_writer(initial_state),
        )

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


@dataclass
class HostStreamFrameWriter:
    """Stateful host-stream frame writer backed by HostBindingClient codecs."""

    client: HostBindingClient
    state: HostStreamHashState = field(default_factory=HostStreamHashState.initial)
    _terminal: bool = field(default=False, init=False, repr=False)

    @property
    def frames(self) -> int:
        return self.state.frames

    @property
    def output_hash(self) -> str:
        return self.state.output_hash

    @property
    def terminal(self) -> bool:
        return self._terminal

    def write_item(self, value: object) -> HostStreamFrame:
        self._require_open()
        seq = self.state.frames
        next_state = self.client.fold_output_hash(self.state, seq, value)
        frame = self.client.encode_item(seq, value)
        self.state = next_state
        return frame

    def finish(
        self, metadata: Optional[Mapping[str, object]] = None
    ) -> HostStreamFrame:
        self._require_open()
        frame = self.client.encode_terminal(
            HostStreamTerminalSummary(
                output_hash=self.state.output_hash,
                frames=self.state.frames,
                metadata=metadata or {},
            )
        )
        self._terminal = True
        return frame

    def fail(self, error: BaseException) -> HostStreamFrame:
        self._require_open()
        frame = self.client.encode_error(error)
        self._terminal = True
        return frame

    def close(self) -> None:
        self._terminal = True

    def _require_open(self) -> None:
        if self._terminal:
            raise _invalid_host_binding("host stream writer is terminal")


@dataclass
class HostStreamSession:
    """One decoded host-stream call plus SDK-owned frame state."""

    request: HostStreamRequest
    writer: HostStreamFrameWriter
    state: HostStreamSessionState = HostStreamSessionState.OPEN
    terminal_frame: Optional[HostStreamFrame] = None

    @property
    def frames(self) -> int:
        return self.writer.frames

    @property
    def output_hash(self) -> str:
        return self.writer.output_hash

    @property
    def terminal(self) -> bool:
        return self.state in {
            HostStreamSessionState.TERMINAL,
            HostStreamSessionState.CLOSED,
        }

    def emit(self, value: object) -> HostStreamFrame:
        self._require_open()
        return self.writer.write_item(value)

    def finish(
        self, metadata: Optional[Mapping[str, object]] = None
    ) -> HostStreamFrame:
        self._require_open()
        frame = self.writer.finish(metadata)
        self.terminal_frame = frame
        self.state = HostStreamSessionState.TERMINAL
        return frame

    def fail(self, error: BaseException) -> HostStreamFrame:
        self._require_open()
        frame = self.writer.fail(error)
        self.terminal_frame = frame
        self.state = HostStreamSessionState.TERMINAL
        return frame

    def close(self) -> None:
        if self.state == HostStreamSessionState.CLOSED:
            return
        self.writer.close()
        self.state = HostStreamSessionState.CLOSED

    def __enter__(self) -> "HostStreamSession":
        self._require_open()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _require_open(self) -> None:
        if self.state != HostStreamSessionState.OPEN:
            raise _invalid_host_binding("host stream session is terminal")


def _validate_binding_request(request: HostStreamBindingRequest) -> None:
    if not request.binding_id or not request.descriptor_ref or not request.endpoint:
        raise _invalid_host_binding("binding_id, descriptor_ref, and endpoint are required")
    if request.frame_schema != HOST_STREAM_FRAME_SCHEMA:
        raise _invalid_host_binding(
            "frame_schema must be host-stream-frame.schema.json"
        )
    if not _is_absolute_host_endpoint(request.endpoint):
        raise _invalid_host_binding("host stream endpoint must be absolute")
    if request.timeout_ms is not None and request.timeout_ms < 0:
        raise _invalid_host_binding("timeout_ms must be non-negative or null")


def _is_absolute_host_endpoint(endpoint: str) -> bool:
    return endpoint.startswith("/") or endpoint.startswith("unix:///")


def _validate_frame(frame: HostStreamFrame) -> None:
    if frame.frame_type == "item":
        if (
            frame.seq is None
            or frame.error is not None
            or frame.terminal is not None
            or frame.output_hash is not None
        ):
            raise _invalid_host_binding("invalid item host stream frame")
        return
    if frame.frame_type == "error":
        if (
            frame.seq is not None
            or frame.value is not None
            or frame.error is None
            or frame.terminal is not None
            or frame.output_hash is not None
        ):
            raise _invalid_host_binding("invalid error host stream frame")
        return
    if frame.frame_type == "terminal":
        if (
            frame.seq is None
            or frame.value is not None
            or frame.error is not None
            or frame.terminal is None
            or frame.output_hash is None
        ):
            raise _invalid_host_binding("invalid terminal host stream frame")
        if (
            frame.terminal.output_hash == ""
            or frame.terminal.frames < 0
            or frame.output_hash != frame.terminal.output_hash
        ):
            raise _invalid_host_binding("invalid terminal host stream summary")
        return
    raise _invalid_host_binding("unknown host stream frame type")


def _validate_hash_fold(state: HostStreamHashState, seq: int) -> None:
    state.to_json_dict()
    if seq != state.frames:
        raise _invalid_host_binding("host stream hash sequence gap")


def _validate_hash_state_consistency(frames: int, last_seq: Optional[int]) -> None:
    if frames == 0:
        if last_seq is not None:
            raise _invalid_host_binding(
                "host stream hash state cannot have last_seq when frames is zero"
            )
        return
    if last_seq != frames - 1:
        raise _invalid_host_binding(
            "host stream hash state last_seq must match frames"
        )


def _optional_terminal(value: object, field_name: str) -> Optional[HostStreamTerminalSummary]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_host_binding(f"{field_name} must be an object or null")
    return HostStreamTerminalSummary(
        output_hash=_required_string(value, "output_hash"),
        frames=_required_non_negative_int(value, "frames"),
        metadata=_optional_mapping(value.get("metadata"), "metadata") or {},
    )


def _optional_sdk_error(value: object, field_name: str) -> Optional[SDKError]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_host_binding(f"{field_name} must be an object or null")
    return SDKError.from_json(json.dumps(value, separators=(",", ":"), sort_keys=True))


def _error_json_dict(error: BaseException) -> dict[str, object]:
    if isinstance(error, SDKError):
        return {
            "code": str(error.code),
            "stage": error.stage,
            "message": error.message,
            "retry": str(error.retry),
            "source": error.source,
            "invocation_id": error.invocation_id,
            "receipt_ura": error.receipt_ura,
            "details": dict(error.details),
        }
    return {
        "code": str(ErrorCode.GENERIC),
        "stage": "host_binding",
        "message": str(error),
        "retry": str(RetryHint.NEVER),
        "details": {},
    }


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_host_binding(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_host_binding(f"{label} JSON must be an object")
    return decoded


def _required_string(
    decoded: Mapping[str, object], field_name: str, *, allow_empty: bool = False
) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or (not allow_empty and value.strip() == ""):
        raise _invalid_host_binding(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_host_binding(f"{field_name} must be a string or null")
    return value


def _optional_int(value: object, field_name: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_host_binding(f"{field_name} must be an integer or null")
    if value < 0:
        raise _invalid_host_binding(f"{field_name} must be non-negative")
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_host_binding(f"{field_name} must be a non-negative integer")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_host_binding(f"{field_name} must be a non-negative integer or null")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_host_binding(f"{field_name} must be an object")
    return dict(value)


def _optional_mapping(value: object, field_name: str) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_host_binding(f"{field_name} must be an object or null")
    return dict(value)


def _invalid_host_binding(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="host_binding",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
