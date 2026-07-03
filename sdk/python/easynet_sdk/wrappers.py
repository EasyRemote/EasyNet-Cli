"""Convenience wrapper record facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Mapping, Optional

from .errors import ErrorCode, RetryHint, SDKError


_PROFILE = "wrappers"


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


FileRecord = WrapperFileRecord
TerminalSessionRecord = WrapperTerminalSessionRecord
RemoteDesktopSessionRecord = WrapperRemoteDesktopSessionRecord
BrowserSessionRecord = WrapperBrowserSessionRecord
MediaSessionRecord = WrapperMediaSessionRecord


@dataclass(frozen=True)
class WrapperClient:
    """Projection-only facade for SDK wrapper DTO records."""

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

