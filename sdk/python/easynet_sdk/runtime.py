"""Runtime Core prepare and submit facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Mapping, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft
from .signing import PreparedInvocation, SignedInvocation, SigningMaterial


@runtime_checkable
class RuntimeTransport(Protocol):
    """Narrow transport seam owned by the application integration layer."""

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        ...

    def submit_signed(self, signed_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class PrepareOptions:
    """Daemon-owned prepare policy knobs."""

    resolve_descriptor: bool = False
    fill_nonce: bool = False
    require_user_sig: bool = False
    expires_in_ms: int = 0

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {}
        if self.resolve_descriptor:
            value["resolve_descriptor"] = self.resolve_descriptor
        if self.fill_nonce:
            value["fill_nonce"] = self.fill_nonce
        if self.require_user_sig:
            value["require_user_sig"] = self.require_user_sig
        if self.expires_in_ms:
            value["expires_in_ms"] = self.expires_in_ms
        return value

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")


@dataclass(frozen=True)
class InvocationHandleEvent:
    """Submitted invocation event projection."""

    sequence: int
    kind: str
    state: str
    terminal: bool
    reason: Optional[str] = None
    result: Optional[Mapping[str, object]] = None


@dataclass(frozen=True)
class InvocationHandle:
    """Submitted invocation observation handle projection."""

    handle_id: int
    state: str
    terminal: bool
    events: tuple[InvocationHandleEvent, ...] = field(default_factory=tuple)
    result: Optional[Mapping[str, object]] = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationHandle":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_runtime(f"decode invocation handle JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_runtime("invocation handle JSON must be an object")

        handle_id = _required_positive_int(decoded, "handle_id")
        state = _required_string(decoded, "state")
        terminal = _required_bool(decoded, "terminal")
        raw_events = decoded.get("events", [])
        if not isinstance(raw_events, list):
            raise _invalid_runtime("events must be an array")
        events = tuple(_handle_event(item) for item in raw_events)
        result = _optional_mapping(decoded.get("result"), "result")
        return cls(
            handle_id=handle_id,
            state=state,
            terminal=terminal,
            events=events,
            result=result,
        )


class RuntimeClient:
    """Runtime Core invocation facade over an application transport."""

    def __init__(self, transport: RuntimeTransport) -> None:
        if transport is None:
            raise _invalid_runtime_client("runtime transport is required")
        self._transport = transport

    def prepare(
        self,
        draft: InvocationDraft,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        try:
            draft_json = draft.to_json().encode("utf-8")
            options_json = options.to_json_bytes()
            raw = self._transport.prepare(draft_json, options_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("prepare transport failed", exc) from exc
        prepared = PreparedInvocation.from_json(raw)
        return prepared, prepared.signing_material

    def submit_signed(self, signed: SignedInvocation) -> InvocationHandle:
        if not signed.submit_ready():
            raise _invalid_runtime("signed invocation is not submit-ready")
        try:
            raw = self._transport.submit_signed(signed.to_json().encode("utf-8"))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("submit signed transport failed", exc) from exc
        return InvocationHandle.from_json(raw)


def _handle_event(value: object) -> InvocationHandleEvent:
    if not isinstance(value, dict):
        raise _invalid_runtime("event must be an object")
    return InvocationHandleEvent(
        sequence=_required_positive_int(value, "sequence"),
        kind=_required_string(value, "kind"),
        state=_required_string(value, "state"),
        terminal=_required_bool(value, "terminal"),
        reason=_optional_string(value.get("reason"), "reason"),
        result=_optional_mapping(value.get("result"), "result"),
    )


def _required_positive_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_runtime(f"{field_name} is required")
    return value


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_runtime(f"{field_name} is required")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_runtime(f"{field_name} must be a boolean")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_runtime(f"{field_name} must be a string or null")
    return value


def _optional_mapping(
    value: object, field_name: str
) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_runtime(f"{field_name} must be an object or null")
    return dict(value)


def _invalid_runtime_client(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _invalid_runtime(
    message: str, cause: Optional[BaseException] = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime",
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
