"""Convenience wrapper record facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Callable, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft


_PROFILE = "wrappers"
_FILE = "file"
_TERMINAL = "terminal"
_REMOTE_DESKTOP = "remote_desktop"
_BROWSER = "browser"
_MEDIA = "media"


@dataclass(frozen=True)
class WrapperCarrierBase:
    """Complete carrier context shared by wrapper execution helpers."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        _validate_carrier_base(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class WrapperFileRecord:
    profile: str
    kind: str
    file_ref: str
    owner_ura: str
    content_type: str
    size_bytes: Optional[int] = None
    content_hash: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "WrapperFileRecord":
        return _file_record(_json_object(raw, "wrapper file record"))


@dataclass(frozen=True)
class WrapperFileRecordRequest:
    file_ref: str
    owner_ura: str
    content_type: str
    size_bytes: Optional[int] = None
    content_hash: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class WrapperFileTransferRequest:
    base: WrapperCarrierBase
    file: WrapperFileRecordRequest
    operation: str = ""

    def to_json_bytes(self) -> bytes:
        _validate_file_transfer_request(self)
        value = self.base.to_json_dict()
        value["wrapper_kind"] = _FILE
        value["operation"] = self.operation or "transfer"
        _put_file_request(value, self.file)
        return _json_bytes(value)


@dataclass(frozen=True)
class WrapperTerminalSessionRecord:
    profile: str
    kind: str
    session_id: str
    owner_ura: str
    state: str
    terminal_ref: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "WrapperTerminalSessionRecord":
        return _terminal_session(_json_object(raw, "wrapper terminal session"))


@dataclass(frozen=True)
class WrapperTerminalSessionRequest:
    session_id: str
    owner_ura: str
    state: str
    terminal_ref: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class WrapperTerminalStartRequest:
    base: WrapperCarrierBase
    session: WrapperTerminalSessionRequest
    command: tuple[str, ...] = ()
    cwd: str = ""

    def to_json_bytes(self) -> bytes:
        _validate_terminal_start_request(self)
        value = self.base.to_json_dict()
        value["wrapper_kind"] = _TERMINAL
        _put_terminal_request(value, self.session)
        if self.command:
            value["command"] = list(self.command)
        if self.cwd:
            value["cwd"] = self.cwd
        return _json_bytes(value)


@dataclass(frozen=True)
class WrapperRemoteDesktopSessionRecord:
    profile: str
    kind: str
    session_id: str
    owner_ura: str
    state: str
    display_ref: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "WrapperRemoteDesktopSessionRecord":
        return _remote_desktop_session(_json_object(raw, "wrapper remote desktop session"))


@dataclass(frozen=True)
class WrapperRemoteDesktopSessionRequest:
    session_id: str
    owner_ura: str
    state: str
    display_ref: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class WrapperRemoteDesktopStartRequest:
    base: WrapperCarrierBase
    session: WrapperRemoteDesktopSessionRequest
    display: str = ""

    def to_json_bytes(self) -> bytes:
        _validate_remote_desktop_start_request(self)
        value = self.base.to_json_dict()
        value["wrapper_kind"] = _REMOTE_DESKTOP
        _put_remote_desktop_request(value, self.session)
        if self.display:
            value["display"] = self.display
        return _json_bytes(value)


@dataclass(frozen=True)
class WrapperBrowserSessionRecord:
    profile: str
    kind: str
    session_id: str
    owner_ura: str
    state: str
    browser_ref: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "WrapperBrowserSessionRecord":
        return _browser_session(_json_object(raw, "wrapper browser session"))


@dataclass(frozen=True)
class WrapperBrowserSessionRequest:
    session_id: str
    owner_ura: str
    state: str
    browser_ref: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class WrapperBrowserStartRequest:
    base: WrapperCarrierBase
    session: WrapperBrowserSessionRequest
    url: str = ""

    def to_json_bytes(self) -> bytes:
        _validate_browser_start_request(self)
        value = self.base.to_json_dict()
        value["wrapper_kind"] = _BROWSER
        _put_browser_request(value, self.session)
        if self.url:
            value["url"] = self.url
        return _json_bytes(value)


@dataclass(frozen=True)
class WrapperMediaSessionRecord:
    profile: str
    kind: str
    session_id: str
    owner_ura: str
    state: str
    media_kind: str
    stream_ref: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "WrapperMediaSessionRecord":
        return _media_session(_json_object(raw, "wrapper media session"))


@dataclass(frozen=True)
class WrapperMediaSessionRequest:
    session_id: str
    owner_ura: str
    state: str
    media_kind: str
    stream_ref: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class WrapperMediaStartRequest:
    base: WrapperCarrierBase
    session: WrapperMediaSessionRequest
    codec: str = ""

    def to_json_bytes(self) -> bytes:
        _validate_media_start_request(self)
        value = self.base.to_json_dict()
        value["wrapper_kind"] = _MEDIA
        _put_media_request(value, self.session)
        if self.codec:
            value["codec"] = self.codec
        return _json_bytes(value)


FileRecord = WrapperFileRecord
TerminalSessionRecord = WrapperTerminalSessionRecord
RemoteDesktopSessionRecord = WrapperRemoteDesktopSessionRecord
BrowserSessionRecord = WrapperBrowserSessionRecord
MediaSessionRecord = WrapperMediaSessionRecord


@runtime_checkable
class WrapperTransport(Protocol):
    """Concrete wrapper operations supplied by the integration layer."""

    def build_file_transfer_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_terminal_session_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_remote_desktop_session_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_browser_session_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_media_session_invocation(self, request_json: bytes) -> bytes:
        ...

    def transfer_file(self, request_json: bytes) -> bytes:
        ...

    def start_terminal_session(self, request_json: bytes) -> bytes:
        ...

    def start_remote_desktop_session(self, request_json: bytes) -> bytes:
        ...

    def start_browser_session(self, request_json: bytes) -> bytes:
        ...

    def start_media_session(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class WrapperClient:
    """Facade for SDK wrapper DTO records and optional daemon execution helpers."""

    transport: Optional[WrapperTransport] = None
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        object.__setattr__(self, "_lifecycle", ClientLifecycle("wrapper"))

    def build_file_transfer_invocation(self, request: WrapperFileTransferRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self._transport().build_file_transfer_invocation,
            "wrapper file-transfer invocation failed",
        )

    def build_terminal_session_invocation(self, request: WrapperTerminalStartRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self._transport().build_terminal_session_invocation,
            "wrapper terminal-session invocation failed",
        )

    def build_remote_desktop_session_invocation(
        self, request: WrapperRemoteDesktopStartRequest
    ) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self._transport().build_remote_desktop_session_invocation,
            "wrapper remote-desktop-session invocation failed",
        )

    def build_browser_session_invocation(self, request: WrapperBrowserStartRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self._transport().build_browser_session_invocation,
            "wrapper browser-session invocation failed",
        )

    def build_media_session_invocation(self, request: WrapperMediaStartRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self._transport().build_media_session_invocation,
            "wrapper media-session invocation failed",
        )

    def transfer_file(self, request: WrapperFileTransferRequest) -> WrapperFileRecord:
        self._require_open()
        return WrapperFileRecord.from_json(
            self._execute(
                request.to_json_bytes(),
                self._transport().transfer_file,
                "wrapper file transfer failed",
            )
        )

    def start_terminal_session(self, request: WrapperTerminalStartRequest) -> WrapperTerminalSessionRecord:
        self._require_open()
        return WrapperTerminalSessionRecord.from_json(
            self._execute(
                request.to_json_bytes(),
                self._transport().start_terminal_session,
                "wrapper terminal session failed",
            )
        )

    def start_remote_desktop_session(
        self, request: WrapperRemoteDesktopStartRequest
    ) -> WrapperRemoteDesktopSessionRecord:
        self._require_open()
        return WrapperRemoteDesktopSessionRecord.from_json(
            self._execute(
                request.to_json_bytes(),
                self._transport().start_remote_desktop_session,
                "wrapper remote desktop session failed",
            )
        )

    def start_browser_session(self, request: WrapperBrowserStartRequest) -> WrapperBrowserSessionRecord:
        self._require_open()
        return WrapperBrowserSessionRecord.from_json(
            self._execute(
                request.to_json_bytes(),
                self._transport().start_browser_session,
                "wrapper browser session failed",
            )
        )

    def start_media_session(self, request: WrapperMediaStartRequest) -> WrapperMediaSessionRecord:
        self._require_open()
        return WrapperMediaSessionRecord.from_json(
            self._execute(
                request.to_json_bytes(),
                self._transport().start_media_session,
                "wrapper media session failed",
            )
        )

    def project_file_record(self, request: WrapperFileRecordRequest) -> WrapperFileRecord:
        _validate_file_request(request)
        return _file_record(
            {
                "profile": _PROFILE,
                "kind": "file_record",
                "file_ref": request.file_ref,
                "owner_ura": request.owner_ura,
                "content_type": request.content_type,
                "size_bytes": request.size_bytes,
                "content_hash": request.content_hash or None,
                "metadata": _metadata(request.metadata, "wrappers.file_record"),
            }
        )

    def project_terminal_session(
        self, request: WrapperTerminalSessionRequest
    ) -> WrapperTerminalSessionRecord:
        _validate_session_facts(request.session_id, request.owner_ura, request.state)
        return _terminal_session(
            {
                "profile": _PROFILE,
                "kind": "terminal_session",
                "session_id": request.session_id,
                "owner_ura": request.owner_ura,
                "state": request.state,
                "terminal_ref": request.terminal_ref or None,
                "metadata": _metadata(request.metadata, "wrappers.terminal_session"),
            }
        )

    def project_remote_desktop_session(
        self, request: WrapperRemoteDesktopSessionRequest
    ) -> WrapperRemoteDesktopSessionRecord:
        _validate_session_facts(request.session_id, request.owner_ura, request.state)
        return _remote_desktop_session(
            {
                "profile": _PROFILE,
                "kind": "remote_desktop_session",
                "session_id": request.session_id,
                "owner_ura": request.owner_ura,
                "state": request.state,
                "display_ref": request.display_ref or None,
                "metadata": _metadata(request.metadata, "wrappers.remote_desktop_session"),
            }
        )

    def project_browser_session(
        self, request: WrapperBrowserSessionRequest
    ) -> WrapperBrowserSessionRecord:
        _validate_session_facts(request.session_id, request.owner_ura, request.state)
        return _browser_session(
            {
                "profile": _PROFILE,
                "kind": "browser_session",
                "session_id": request.session_id,
                "owner_ura": request.owner_ura,
                "state": request.state,
                "browser_ref": request.browser_ref or None,
                "metadata": _metadata(request.metadata, "wrappers.browser_session"),
            }
        )

    def project_media_session(self, request: WrapperMediaSessionRequest) -> WrapperMediaSessionRecord:
        _validate_session_facts(request.session_id, request.owner_ura, request.state)
        if not request.media_kind:
            raise _invalid_wrappers("wrapper media_kind is required")
        return _media_session(
            {
                "profile": _PROFILE,
                "kind": "media_session",
                "session_id": request.session_id,
                "owner_ura": request.owner_ura,
                "state": request.state,
                "media_kind": request.media_kind,
                "stream_ref": request.stream_ref or None,
                "metadata": _metadata(request.metadata, "wrappers.media_session"),
            }
        )

    def _transport(self) -> WrapperTransport:
        if self.transport is None:
            raise _invalid_wrappers("wrapper transport is required")
        return self.transport

    def _invocation(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> InvocationDraft:
        return InvocationDraft.from_json(self._execute(request_json, fn, label))

    def _execute(self, request_json: bytes, fn: Callable[[bytes], bytes], label: str) -> bytes:
        try:
            return fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc

    def close(self) -> None:
        self._lifecycle.close(self._transport())

    def _require_open(self) -> None:
        self._lifecycle.require_open()


def _file_record(decoded: Mapping[str, object]) -> WrapperFileRecord:
    if decoded.get("profile") != _PROFILE or decoded.get("kind") != "file_record":
        raise _invalid_wrappers("invalid wrapper file record projection")
    size_bytes = _optional_non_negative_int(decoded.get("size_bytes"), "size_bytes")
    return WrapperFileRecord(
        profile=_required_string(decoded, "profile"),
        kind=_required_string(decoded, "kind"),
        file_ref=_required_string(decoded, "file_ref"),
        owner_ura=_required_owner_ura(decoded, "owner_ura"),
        content_type=_required_string(decoded, "content_type"),
        size_bytes=size_bytes,
        content_hash=_optional_string(decoded.get("content_hash"), "content_hash"),
        metadata=_required_mapping(decoded, "metadata"),
    )


def _terminal_session(decoded: Mapping[str, object]) -> WrapperTerminalSessionRecord:
    if decoded.get("profile") != _PROFILE or decoded.get("kind") != "terminal_session":
        raise _invalid_wrappers("invalid wrapper terminal session projection")
    _validate_session_mapping(decoded)
    return WrapperTerminalSessionRecord(
        profile=_required_string(decoded, "profile"),
        kind=_required_string(decoded, "kind"),
        session_id=_required_string(decoded, "session_id"),
        owner_ura=_required_owner_ura(decoded, "owner_ura"),
        state=_required_string(decoded, "state"),
        terminal_ref=_optional_string(decoded.get("terminal_ref"), "terminal_ref"),
        metadata=_required_mapping(decoded, "metadata"),
    )


def _remote_desktop_session(decoded: Mapping[str, object]) -> WrapperRemoteDesktopSessionRecord:
    if decoded.get("profile") != _PROFILE or decoded.get("kind") != "remote_desktop_session":
        raise _invalid_wrappers("invalid wrapper remote desktop session projection")
    _validate_session_mapping(decoded)
    return WrapperRemoteDesktopSessionRecord(
        profile=_required_string(decoded, "profile"),
        kind=_required_string(decoded, "kind"),
        session_id=_required_string(decoded, "session_id"),
        owner_ura=_required_owner_ura(decoded, "owner_ura"),
        state=_required_string(decoded, "state"),
        display_ref=_optional_string(decoded.get("display_ref"), "display_ref"),
        metadata=_required_mapping(decoded, "metadata"),
    )


def _browser_session(decoded: Mapping[str, object]) -> WrapperBrowserSessionRecord:
    if decoded.get("profile") != _PROFILE or decoded.get("kind") != "browser_session":
        raise _invalid_wrappers("invalid wrapper browser session projection")
    _validate_session_mapping(decoded)
    return WrapperBrowserSessionRecord(
        profile=_required_string(decoded, "profile"),
        kind=_required_string(decoded, "kind"),
        session_id=_required_string(decoded, "session_id"),
        owner_ura=_required_owner_ura(decoded, "owner_ura"),
        state=_required_string(decoded, "state"),
        browser_ref=_optional_string(decoded.get("browser_ref"), "browser_ref"),
        metadata=_required_mapping(decoded, "metadata"),
    )


def _media_session(decoded: Mapping[str, object]) -> WrapperMediaSessionRecord:
    if decoded.get("profile") != _PROFILE or decoded.get("kind") != "media_session":
        raise _invalid_wrappers("invalid wrapper media session projection")
    _validate_session_mapping(decoded)
    return WrapperMediaSessionRecord(
        profile=_required_string(decoded, "profile"),
        kind=_required_string(decoded, "kind"),
        session_id=_required_string(decoded, "session_id"),
        owner_ura=_required_owner_ura(decoded, "owner_ura"),
        state=_required_string(decoded, "state"),
        media_kind=_required_string(decoded, "media_kind"),
        stream_ref=_optional_string(decoded.get("stream_ref"), "stream_ref"),
        metadata=_required_mapping(decoded, "metadata"),
    )


def _validate_file_request(request: WrapperFileRecordRequest) -> None:
    if not request.file_ref or not request.content_type:
        raise _invalid_wrappers("wrapper file_ref and content_type are required")
    _validate_owner_ura(request.owner_ura)
    if request.size_bytes is not None and request.size_bytes < 0:
        raise _invalid_wrappers("wrapper size_bytes must be non-negative")


def _validate_file_transfer_request(request: WrapperFileTransferRequest) -> None:
    _validate_carrier_base(request.base)
    if request.operation and request.operation.strip() != request.operation:
        raise _invalid_wrappers("wrapper operation must not contain surrounding whitespace")
    _validate_file_request(request.file)


def _validate_terminal_start_request(request: WrapperTerminalStartRequest) -> None:
    _validate_carrier_base(request.base)
    _validate_session_facts(
        request.session.session_id,
        request.session.owner_ura,
        request.session.state,
    )
    _validate_command(request.command)


def _validate_remote_desktop_start_request(request: WrapperRemoteDesktopStartRequest) -> None:
    _validate_carrier_base(request.base)
    _validate_session_facts(
        request.session.session_id,
        request.session.owner_ura,
        request.session.state,
    )


def _validate_browser_start_request(request: WrapperBrowserStartRequest) -> None:
    _validate_carrier_base(request.base)
    _validate_session_facts(
        request.session.session_id,
        request.session.owner_ura,
        request.session.state,
    )
    if request.url and request.url.strip() != request.url:
        raise _invalid_wrappers("wrapper url must not contain surrounding whitespace")


def _validate_media_start_request(request: WrapperMediaStartRequest) -> None:
    _validate_carrier_base(request.base)
    _validate_session_facts(
        request.session.session_id,
        request.session.owner_ura,
        request.session.state,
    )
    if not request.session.media_kind:
        raise _invalid_wrappers("wrapper media_kind is required")


def _validate_carrier_base(base: WrapperCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_wrappers("complete wrapper invocation carrier is required")


def _validate_command(command: tuple[str, ...]) -> None:
    for part in command:
        if not part or part.strip() != part:
            raise _invalid_wrappers("wrapper command parts must be non-empty without surrounding whitespace")


def _validate_session_facts(session_id: str, owner_ura: str, state: str) -> None:
    if not session_id or not state:
        raise _invalid_wrappers("wrapper session_id and state are required")
    _validate_owner_ura(owner_ura)


def _validate_session_mapping(decoded: Mapping[str, object]) -> None:
    _validate_session_facts(
        _required_string(decoded, "session_id"),
        _required_owner_ura(decoded, "owner_ura"),
        _required_string(decoded, "state"),
    )
    _required_mapping(decoded, "metadata")


def _validate_owner_ura(value: str) -> None:
    if not value or value.strip() != value or not value.startswith("easynet://"):
        raise _invalid_wrappers("wrapper owner_ura must be an EasyNet URA")


def _required_owner_ura(decoded: Mapping[str, object], field: str) -> str:
    value = _required_string(decoded, field)
    _validate_owner_ura(value)
    return value


def _required_mapping(decoded: Mapping[str, object], field: str) -> Mapping[str, object]:
    value = decoded.get(field)
    if not isinstance(value, dict):
        raise _invalid_wrappers(f"{field} must be an object")
    return value


def _required_string(decoded: Mapping[str, object], field: str) -> str:
    value = decoded.get(field)
    if not isinstance(value, str) or not value:
        raise _invalid_wrappers(f"{field} is required")
    return value


def _optional_string(value: object, field: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_wrappers(f"{field} must be a string")
    return value


def _optional_non_negative_int(value: object, field: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_wrappers(f"{field} must be a non-negative integer")
    return value


def _json_object(raw: bytes | str, label: str) -> Mapping[str, object]:
    try:
        decoded = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise _invalid_wrappers(f"decode {label} JSON: {exc}") from exc
    if not isinstance(decoded, dict):
        raise _invalid_wrappers(f"{label} must be a JSON object")
    return decoded


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _put_file_request(value: dict[str, object], request: WrapperFileRecordRequest) -> None:
    value["file_ref"] = request.file_ref
    value["owner_ura"] = request.owner_ura
    value["content_type"] = request.content_type
    if request.size_bytes is not None:
        value["size_bytes"] = request.size_bytes
    if request.content_hash:
        value["content_hash"] = request.content_hash
    _merge_execution_metadata(value, request.metadata)


def _put_terminal_request(value: dict[str, object], request: WrapperTerminalSessionRequest) -> None:
    value["session_id"] = request.session_id
    value["owner_ura"] = request.owner_ura
    value["state"] = request.state
    if request.terminal_ref:
        value["terminal_ref"] = request.terminal_ref
    _merge_execution_metadata(value, request.metadata)


def _put_remote_desktop_request(value: dict[str, object], request: WrapperRemoteDesktopSessionRequest) -> None:
    value["session_id"] = request.session_id
    value["owner_ura"] = request.owner_ura
    value["state"] = request.state
    if request.display_ref:
        value["display_ref"] = request.display_ref
    _merge_execution_metadata(value, request.metadata)


def _put_browser_request(value: dict[str, object], request: WrapperBrowserSessionRequest) -> None:
    value["session_id"] = request.session_id
    value["owner_ura"] = request.owner_ura
    value["state"] = request.state
    if request.browser_ref:
        value["browser_ref"] = request.browser_ref
    _merge_execution_metadata(value, request.metadata)


def _put_media_request(value: dict[str, object], request: WrapperMediaSessionRequest) -> None:
    value["session_id"] = request.session_id
    value["owner_ura"] = request.owner_ura
    value["state"] = request.state
    value["media_kind"] = request.media_kind
    if request.stream_ref:
        value["stream_ref"] = request.stream_ref
    _merge_execution_metadata(value, request.metadata)


def _merge_execution_metadata(value: dict[str, object], metadata: Mapping[str, object]) -> None:
    if not metadata:
        return
    base = value.get("metadata")
    merged: dict[str, object] = {}
    if isinstance(base, dict):
        merged.update(base)
    merged.update(metadata)
    value["metadata"] = merged


def _metadata(metadata: Mapping[str, object], source: str) -> dict[str, object]:
    value = dict(metadata)
    value["profile"] = _PROFILE
    value["source"] = source
    return value


def _invalid_wrappers(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="wrappers",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.UNKNOWN,
        retryable=False,
        message=message,
        cause=cause,
    )
