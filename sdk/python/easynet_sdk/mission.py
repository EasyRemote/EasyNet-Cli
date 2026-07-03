"""Mission profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from typing import Any, Callable, Mapping, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft


_PROFILE = "mission"


@dataclass(frozen=True)
class MissionCarrierBase:
    """Complete carrier context shared by Mission operations."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        _validate_base(self)
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
class MissionRunRequest:
    base: MissionCarrierBase
    source: str
    label: str = ""

    def to_json_bytes(self) -> bytes:
        if not self.source:
            raise _invalid_mission("mission source is required")
        value = self.base.to_json_dict()
        value["source"] = self.source
        if self.label:
            value["label"] = self.label
        return _json_bytes(value)


@dataclass(frozen=True)
class MissionRunFileRequest:
    base: MissionCarrierBase
    path: str
    label: str = ""

    def with_path(self, path: str) -> "MissionRunFileRequest":
        return replace(self, path=path)

    def to_json_bytes(self) -> bytes:
        if not self.path or not self.path.startswith("/"):
            raise _invalid_mission("absolute mission file path is required")
        value = self.base.to_json_dict()
        value["path"] = self.path
        if self.label:
            value["label"] = self.label
        return _json_bytes(value)


@dataclass(frozen=True)
class MissionTrackRequest:
    base: MissionCarrierBase
    mission_id: str

    def to_json_bytes(self) -> bytes:
        _validate_mission_id(self.mission_id)
        value = self.base.to_json_dict()
        value["mission_id"] = self.mission_id
        return _json_bytes(value)


@dataclass(frozen=True)
class MissionCancelRequest:
    base: MissionCarrierBase
    mission_id: str

    def to_json_bytes(self) -> bytes:
        _validate_mission_id(self.mission_id)
        value = self.base.to_json_dict()
        value["mission_id"] = self.mission_id
        return _json_bytes(value)


MissionID = str


@dataclass(frozen=True)
class MissionChildInvocation:
    step_id: Optional[str]
    request_id: Optional[str]
    trace_id: Optional[str]
    ability: Optional[str]
    invocation_ura: Optional[str]
    caller_ura: Optional[str]
    callee_ura: Optional[str]
    subject_ura: Optional[str]
    metadata_state: Optional[str]
    ledger_state: Any
    receipt: Optional[Mapping[str, object]]


@dataclass(frozen=True)
class MissionChildReceipt:
    step_id: Optional[str]
    invocation_ura: Optional[str]
    receipt_ura: str
    receipt_hash: str


@dataclass(frozen=True)
class MissionOutputRef:
    kind: str
    path: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class MissionStatus:
    """SDK mission-status.schema.json projection."""

    profile: str
    kind: str
    mission_id: str
    state: str
    terminal: bool
    partial_failures: int
    cancelled: bool
    child_invocations: tuple[MissionChildInvocation, ...]
    child_receipts: tuple[MissionChildReceipt, ...]
    output_refs: tuple[MissionOutputRef, ...]
    metadata: Mapping[str, object]
    parent_invocation_id: Optional[str] = None
    parent_receipt_ura: Optional[str] = None
    parent_invocation: Optional[Mapping[str, object]] = None
    error: Optional[SDKError] = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "MissionStatus":
        decoded = _json_object(raw, "mission status")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "mission_status":
            raise _invalid_mission("invalid mission status projection")
        partial_failures = _required_non_negative_int(decoded, "partial_failures")
        child_invocations = decoded.get("child_invocations")
        child_receipts = decoded.get("child_receipts")
        output_refs = decoded.get("output_refs")
        if not isinstance(child_invocations, list):
            raise _invalid_mission("child_invocations must be an array")
        if not isinstance(child_receipts, list):
            raise _invalid_mission("child_receipts must be an array")
        if not isinstance(output_refs, list):
            raise _invalid_mission("output_refs must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            mission_id=_required_string(decoded, "mission_id"),
            state=_required_string(decoded, "state"),
            terminal=_required_bool(decoded, "terminal"),
            partial_failures=partial_failures,
            cancelled=_required_bool(decoded, "cancelled"),
            parent_invocation_id=_optional_string(
                decoded.get("parent_invocation_id"), "parent_invocation_id"
            ),
            parent_receipt_ura=_optional_string(
                decoded.get("parent_receipt_ura"), "parent_receipt_ura"
            ),
            parent_invocation=_optional_mapping(
                decoded.get("parent_invocation"), "parent_invocation"
            ),
            child_invocations=tuple(_child_invocation(item) for item in child_invocations),
            child_receipts=tuple(_child_receipt(item) for item in child_receipts),
            output_refs=tuple(_output_ref(item) for item in output_refs),
            error=_optional_sdk_error(decoded.get("error"), "error"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class MissionRun:
    status: MissionStatus


MissionCancelResult = MissionStatus


@runtime_checkable
class MissionTransport(Protocol):
    """Concrete Mission operations supplied by the integration layer."""

    def build_run_eal_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_run_file_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_track_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_cancel_invocation(self, request_json: bytes) -> bytes:
        ...

    def run_eal(self, request_json: bytes) -> bytes:
        ...

    def run_file(self, request_json: bytes) -> bytes:
        ...

    def track(self, request_json: bytes) -> bytes:
        ...

    def cancel(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class MissionClient:
    """Mission profile facade."""

    transport: MissionTransport

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_mission("mission transport is required")

    def build_run_eal_invocation(self, request: MissionRunRequest) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_run_eal_invocation,
            "mission run invocation failed",
        )

    def build_run_file_invocation(self, request: MissionRunFileRequest) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_run_file_invocation,
            "mission run-file invocation failed",
        )

    def build_track_invocation(self, request: MissionTrackRequest) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_track_invocation,
            "mission track invocation failed",
        )

    def build_cancel_invocation(self, request: MissionCancelRequest) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_cancel_invocation,
            "mission cancel invocation failed",
        )

    def run_eal(self, request: MissionRunRequest) -> MissionRun:
        return MissionRun(
            self._status(request.to_json_bytes(), self.transport.run_eal, "mission run failed")
        )

    def run_file(self, path: str, request: MissionRunFileRequest) -> MissionRun:
        request = request.with_path(path)
        return MissionRun(
            self._status(
                request.to_json_bytes(),
                self.transport.run_file,
                "mission run-file failed",
            )
        )

    def track(self, request: MissionTrackRequest) -> MissionStatus:
        return self._status(request.to_json_bytes(), self.transport.track, "mission track failed")

    def cancel(self, request: MissionCancelRequest) -> MissionStatus:
        return self._status(request.to_json_bytes(), self.transport.cancel, "mission cancel failed")

    def _invocation(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> InvocationDraft:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return InvocationDraft.from_json(raw)

    def _status(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> MissionStatus:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return MissionStatus.from_json(raw)


def _validate_base(base: MissionCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_mission("complete mission invocation carrier is required")


def _validate_mission_id(mission_id: str) -> None:
    if not mission_id:
        raise _invalid_mission("mission_id is required")
    if "/" in mission_id or "\\" in mission_id or "://" in mission_id:
        raise _invalid_mission("mission_id must not be path-like")


def _child_invocation(value: object) -> MissionChildInvocation:
    if not isinstance(value, dict):
        raise _invalid_mission("child invocation must be an object")
    return MissionChildInvocation(
        step_id=_optional_string(value.get("step_id"), "step_id"),
        request_id=_optional_string(value.get("request_id"), "request_id"),
        trace_id=_optional_string(value.get("trace_id"), "trace_id"),
        ability=_optional_string(value.get("ability"), "ability"),
        invocation_ura=_optional_string(value.get("invocation_ura"), "invocation_ura"),
        caller_ura=_optional_string(value.get("caller_ura"), "caller_ura"),
        callee_ura=_optional_string(value.get("callee_ura"), "callee_ura"),
        subject_ura=_optional_string(value.get("subject_ura"), "subject_ura"),
        metadata_state=_optional_string(value.get("metadata_state"), "metadata_state"),
        ledger_state=value.get("ledger_state"),
        receipt=_optional_mapping(value.get("receipt"), "receipt"),
    )


def _child_receipt(value: object) -> MissionChildReceipt:
    if not isinstance(value, dict):
        raise _invalid_mission("child receipt must be an object")
    return MissionChildReceipt(
        step_id=_optional_string(value.get("step_id"), "step_id"),
        invocation_ura=_optional_string(value.get("invocation_ura"), "invocation_ura"),
        receipt_ura=_required_string(value, "receipt_ura"),
        receipt_hash=_required_string(value, "receipt_hash"),
    )


def _output_ref(value: object) -> MissionOutputRef:
    if not isinstance(value, dict):
        raise _invalid_mission("output ref must be an object")
    return MissionOutputRef(
        kind=_required_string(value, "kind"),
        path=_optional_string(value.get("path"), "path") or "",
        metadata=_optional_mapping(value.get("metadata"), "metadata") or {},
    )


def _optional_sdk_error(value: object, field_name: str) -> Optional[SDKError]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_mission(f"{field_name} must be an object or null")
    return SDKError.from_json(json.dumps(value, separators=(",", ":"), sort_keys=True))


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_mission(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_mission(f"{label} JSON must be an object")
    return decoded


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_mission(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_mission(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_mission(f"{field_name} must be a boolean")
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_mission(f"{field_name} must be a non-negative integer")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_mission(f"{field_name} must be an object")
    return dict(value)


def _optional_mapping(value: object, field_name: str) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_mission(f"{field_name} must be an object or null")
    return dict(value)


def _invalid_mission(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="mission",
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
