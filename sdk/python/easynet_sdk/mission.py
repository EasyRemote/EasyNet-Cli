"""Mission profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from typing import Any, Callable, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
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


@dataclass(frozen=True)
class MissionEventListRequest:
    base: MissionCarrierBase
    mission_id: str
    cursor_sequence: int = 0
    limit: int = 0

    def to_json_bytes(self) -> bytes:
        _validate_mission_id(self.mission_id)
        if self.cursor_sequence < 0:
            raise _invalid_mission("mission event cursor_sequence must be non-negative")
        if self.limit < 0:
            raise _invalid_mission("mission event limit must be non-negative")
        if self.limit > 1000:
            raise _invalid_mission("mission event limit exceeds bounds")
        value = self.base.to_json_dict()
        value["mission_id"] = self.mission_id
        value["cursor_sequence"] = self.cursor_sequence
        if self.limit:
            value["limit"] = self.limit
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
class MissionEvent:
    profile: str
    kind: str
    mission_id: str
    sequence: int
    event_type: str
    occurred_unix_ms: int
    terminal: bool
    payload: Any
    receipt: Mapping[str, object]
    metadata: Mapping[str, object]


@dataclass(frozen=True)
class MissionEventPage:
    profile: str
    kind: str
    mission_id: str
    cursor_sequence: int
    next_cursor_sequence: int
    has_more: bool
    dropped_count: int
    events: tuple[MissionEvent, ...]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "MissionEventPage":
        decoded = _json_object(raw, "mission event page")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "mission_event_page":
            raise _invalid_mission("invalid mission event page projection")
        cursor_sequence = _required_non_negative_int(decoded, "cursor_sequence")
        next_cursor_sequence = _required_non_negative_int(decoded, "next_cursor_sequence")
        if next_cursor_sequence < cursor_sequence:
            raise _invalid_mission("next_cursor_sequence must not go backwards")
        raw_events = decoded.get("events")
        if not isinstance(raw_events, list):
            raise _invalid_mission("events must be an array")
        events = tuple(
            _mission_event(item, _required_string(decoded, "mission_id"))
            for item in raw_events
        )
        previous: Optional[int] = None
        for event in events:
            if previous is not None and event.sequence <= previous:
                raise _invalid_mission("mission events must be strictly ordered by sequence")
            previous = event.sequence
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            mission_id=_required_string(decoded, "mission_id"),
            cursor_sequence=cursor_sequence,
            next_cursor_sequence=next_cursor_sequence,
            has_more=_required_bool(decoded, "has_more"),
            dropped_count=_required_non_negative_int(decoded, "dropped_count"),
            events=events,
            metadata=_required_mapping(decoded, "metadata"),
        )


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
        parsed_child_invocations = tuple(
            _child_invocation(item) for item in child_invocations
        )
        parsed_child_receipts = tuple(_child_receipt(item) for item in child_receipts)
        parent_receipt_ura = _optional_string(
            decoded.get("parent_receipt_ura"), "parent_receipt_ura"
        )
        _validate_child_receipt_anchors(
            parent_receipt_ura, parsed_child_invocations, parsed_child_receipts
        )
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
            parent_receipt_ura=parent_receipt_ura,
            parent_invocation=_optional_mapping(
                decoded.get("parent_invocation"), "parent_invocation"
            ),
            child_invocations=parsed_child_invocations,
            child_receipts=parsed_child_receipts,
            output_refs=tuple(_output_ref(item) for item in output_refs),
            error=_optional_sdk_error(decoded.get("error"), "error"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class MissionRun:
    status: MissionStatus


MissionCancelResult = MissionStatus


@dataclass(frozen=True)
class EasyRemoteMissionRunProjection:
    """EasyRemote-facing projection of daemon `mission.run`."""

    run_id: str
    run_dir: str
    outputs: Mapping[str, object]
    raw: Mapping[str, object] = field(default_factory=dict, repr=False)

    @classmethod
    def from_run(cls, run: MissionRun) -> "EasyRemoteMissionRunProjection":
        status = run.status
        return cls(
            run_id=status.mission_id,
            run_dir=_mission_run_dir(status),
            outputs=_mission_outputs(status),
            raw=_status_projection(status),
        )


class EasyRemoteMissionAdapter:
    """SDK-owned Mission cutover adapter for EasyRemote-like callers."""

    def __init__(self, client: "MissionClient", base: MissionCarrierBase) -> None:
        self._client = client
        self._base = base

    @classmethod
    def from_easyremote_client(cls, client: object) -> "EasyRemoteMissionAdapter":
        return cls(
            MissionClient(_EasyRemoteMissionTransport(client)),
            _easyremote_mission_base(client),
        )

    def run_eal(
        self, source: str, *, label: str | None = None
    ) -> EasyRemoteMissionRunProjection:
        source_text = _validated_easyremote_source(source)
        mission_label = _validated_easyremote_label(label)
        run = self._client.run_eal(
            MissionRunRequest(
                base=self._base,
                source=source_text,
                label=mission_label or "",
            )
        )
        return EasyRemoteMissionRunProjection.from_run(run)

    def track(self, run_id: str) -> Mapping[str, object]:
        status = self._client.track(
            MissionTrackRequest(
                base=self._base,
                mission_id=_validated_easyremote_run_id(run_id),
            )
        )
        return _status_projection(status)

    def cancel(self, run_id: str) -> Mapping[str, object]:
        status = self._client.cancel(
            MissionCancelRequest(
                base=self._base,
                mission_id=_validated_easyremote_run_id(run_id),
            )
        )
        return _status_projection(status)


class _EasyRemoteMissionTransport:
    """Narrow Mission transport projection over an EasyRemote invocation client."""

    def __init__(self, client: object) -> None:
        self._client = client

    def run_eal(self, request_json: bytes) -> bytes:
        request = _json_object(request_json, "EasyRemote mission run request")
        payload: dict[str, object] = {"source": _required_string(request, "source")}
        if request.get("label"):
            payload["label"] = _required_string(request, "label")
        response = self._invoke("mission.run", **payload)
        return _easyremote_status_json("mission.run", response)

    def track(self, request_json: bytes) -> bytes:
        request = _json_object(request_json, "EasyRemote mission track request")
        run_id = _required_string(request, "mission_id")
        response = self._invoke("mission.track", run_id=run_id)
        return _easyremote_status_json("mission.track", response, mission_id=run_id)

    def cancel(self, request_json: bytes) -> bytes:
        request = _json_object(request_json, "EasyRemote mission cancel request")
        run_id = _required_string(request, "mission_id")
        response = self._invoke("mission.cancel", run_id=run_id)
        return _easyremote_status_json("mission.cancel", response, mission_id=run_id)

    def close(self) -> None:
        close = getattr(self._client, "close", None)
        if callable(close):
            close()

    def _invoke(self, ability: str, **kwargs: object) -> dict[str, object]:
        invocation = _call_method(self._client, "invoke", ability, **kwargs)
        return _mapping(_call_method(invocation, "result"), "mission response")


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

    def events(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class MissionClient:
    """Mission profile facade."""

    transport: MissionTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_mission("mission transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("mission"))

    def build_run_eal_invocation(self, request: MissionRunRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_run_eal_invocation,
            "mission run invocation failed",
        )

    def build_run_file_invocation(self, request: MissionRunFileRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_run_file_invocation,
            "mission run-file invocation failed",
        )

    def build_track_invocation(self, request: MissionTrackRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_track_invocation,
            "mission track invocation failed",
        )

    def build_cancel_invocation(self, request: MissionCancelRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_cancel_invocation,
            "mission cancel invocation failed",
        )

    def run_eal(self, request: MissionRunRequest) -> MissionRun:
        self._require_open()
        return MissionRun(
            self._status(request.to_json_bytes(), self.transport.run_eal, "mission run failed")
        )

    def run_file(self, path: str, request: MissionRunFileRequest) -> MissionRun:
        self._require_open()
        request = request.with_path(path)
        return MissionRun(
            self._status(
                request.to_json_bytes(),
                self.transport.run_file,
                "mission run-file failed",
            )
        )

    def track(self, request: MissionTrackRequest) -> MissionStatus:
        self._require_open()
        return self._status(request.to_json_bytes(), self.transport.track, "mission track failed")

    def cancel(self, request: MissionCancelRequest) -> MissionStatus:
        self._require_open()
        return self._status(request.to_json_bytes(), self.transport.cancel, "mission cancel failed")

    def events(self, request: MissionEventListRequest) -> MissionEventPage:
        self._require_open()
        try:
            raw = self.transport.events(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("mission events failed", exc) from exc
        return MissionEventPage.from_json(raw)

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

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


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


def _easyremote_mission_base(client: object) -> MissionCarrierBase:
    identity = _call_method(client, "_who")
    device_ura = _required_object_attr(identity, "device_ura")
    return MissionCarrierBase(
        caller_ura=device_ura,
        callee_ura=device_ura,
        subject_ura=device_ura,
        descriptor_version="1.0.0",
        nonce_base64="AAAAAAAAAAAAAAAAAAAAAA==",
        causal_context={"form": "none"},
        metadata={"profile": _PROFILE, "source": "easyremote_adapter"},
    )


def _validate_mission_id(mission_id: str) -> None:
    if not mission_id:
        raise _invalid_mission("mission_id is required")
    if "/" in mission_id or "\\" in mission_id or "://" in mission_id:
        raise _invalid_mission("mission_id must not be path-like")


def _validate_child_receipt_anchors(
    parent_receipt_ura: Optional[str],
    child_invocations: tuple[MissionChildInvocation, ...],
    child_receipts: tuple[MissionChildReceipt, ...],
) -> None:
    if not child_receipts:
        return
    if not parent_receipt_ura:
        raise _invalid_mission("mission child receipts require parent receipt anchor")
    by_invocation_ura = {
        invocation.invocation_ura: invocation
        for invocation in child_invocations
        if invocation.invocation_ura
    }
    for receipt in child_receipts:
        if not receipt.invocation_ura:
            raise _invalid_mission("mission child receipt requires invocation_ura")
        invocation = by_invocation_ura.get(receipt.invocation_ura)
        if invocation is None or invocation.receipt is None:
            raise _invalid_mission(
                "mission child receipt is not anchored to child invocation"
            )
        if (
            invocation.receipt.get("receipt_ura") != receipt.receipt_ura
            or invocation.receipt.get("receipt_hash") != receipt.receipt_hash
        ):
            raise _invalid_mission(
                "mission child receipt does not match child invocation receipt"
            )


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


def _mission_event(value: object, mission_id: str) -> MissionEvent:
    if not isinstance(value, dict):
        raise _invalid_mission("mission event must be an object")
    event_mission_id = _required_string(value, "mission_id")
    if event_mission_id != mission_id:
        raise _invalid_mission("mission event mission_id must match page")
    if value.get("profile") != _PROFILE or value.get("kind") != "mission_event":
        raise _invalid_mission("invalid mission event projection")
    event_type = _required_string(value, "event_type")
    terminal = _required_bool(value, "terminal")
    if terminal and not _mission_event_type_is_terminal(event_type):
        raise _invalid_mission("terminal mission event has non-terminal event_type")
    return MissionEvent(
        profile=_required_string(value, "profile"),
        kind=_required_string(value, "kind"),
        mission_id=event_mission_id,
        sequence=_required_non_negative_int(value, "sequence"),
        event_type=event_type,
        occurred_unix_ms=_required_non_negative_int(value, "occurred_unix_ms"),
        terminal=terminal,
        payload=value.get("payload"),
        receipt=_optional_mapping(value.get("receipt"), "receipt") or {},
        metadata=_required_mapping(value, "metadata"),
    )


def _mission_event_type_is_terminal(event_type: str) -> bool:
    return event_type in {"completed", "failed", "cancelled", "canceled"}


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


def _validated_easyremote_source(source: str) -> str:
    if not isinstance(source, str):
        raise _invalid_mission(
            f"EAL source must be a string, got {type(source).__name__}"
        )
    if not source.strip():
        raise _invalid_mission("EAL source must not be empty")
    return source


def _validated_easyremote_label(label: str | None) -> str | None:
    if label is None:
        return None
    trimmed = label.strip()
    if not trimmed:
        raise _invalid_mission("mission label must not be empty")
    return trimmed


def _validated_easyremote_run_id(value: str) -> str:
    run_id = value.strip()
    if not run_id:
        raise _invalid_mission("mission run_id must not be empty")
    return run_id


def _easyremote_status_json(
    source: str, response: Mapping[str, object], *, mission_id: str | None = None
) -> bytes:
    raw = dict(response)
    run_dir = raw.get("run_dir")
    outputs = raw.get("outputs")
    metadata: dict[str, object] = {
        "profile": _PROFILE,
        "source": source,
        "raw_result": raw,
    }
    if isinstance(run_dir, str):
        metadata["run_dir"] = run_dir
    if isinstance(outputs, Mapping):
        metadata["outputs"] = dict(outputs)
    return _json_bytes(
        {
            "profile": _PROFILE,
            "kind": "mission_status",
            "mission_id": _easyremote_mission_id(source, raw, mission_id),
            "state": _easyremote_state(source, raw),
            "terminal": _easyremote_terminal(source, raw),
            "partial_failures": _easyremote_partial_failures(raw),
            "cancelled": _easyremote_cancelled(raw),
            "parent_invocation_id": _optional_string(
                raw.get("parent_invocation_id"), "parent_invocation_id"
            ),
            "parent_receipt_ura": _optional_string(
                raw.get("parent_receipt_ura"), "parent_receipt_ura"
            ),
            "parent_invocation": _optional_mapping(
                raw.get("parent_invocation"), "parent_invocation"
            ),
            "child_invocations": [],
            "child_receipts": [],
            "output_refs": _easyremote_output_refs(raw),
            "metadata": metadata,
        }
    )


def _easyremote_mission_id(
    source: str, raw: Mapping[str, object], mission_id: str | None
) -> str:
    if mission_id:
        return mission_id
    for field_name in ("mission_id", "run_id"):
        value = raw.get(field_name)
        if isinstance(value, str) and value.strip():
            return value
    raise _invalid_mission(f"{source} response is missing mission run_id")


def _easyremote_state(source: str, raw: Mapping[str, object]) -> str:
    value = raw.get("state")
    if isinstance(value, str) and value.strip():
        return value
    if _easyremote_cancelled(raw):
        return "cancelled"
    if source == "mission.run":
        return "running"
    return "ok"


def _easyremote_terminal(source: str, raw: Mapping[str, object]) -> bool:
    value = raw.get("terminal")
    if isinstance(value, bool):
        return value
    if _easyremote_cancelled(raw):
        return True
    state = raw.get("state")
    if isinstance(state, str) and state.lower() in {
        "completed",
        "failed",
        "cancelled",
        "canceled",
    }:
        return True
    return source == "mission.cancel" and bool(raw.get("ok", True))


def _easyremote_cancelled(raw: Mapping[str, object]) -> bool:
    value = raw.get("cancelled")
    return value if isinstance(value, bool) else False


def _easyremote_partial_failures(raw: Mapping[str, object]) -> int:
    value = raw.get("partial_failures")
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_mission("partial_failures must be a non-negative integer")
    return value


def _easyremote_output_refs(raw: Mapping[str, object]) -> list[dict[str, object]]:
    refs: list[dict[str, object]] = []
    run_dir = raw.get("run_dir")
    if isinstance(run_dir, str) and run_dir:
        refs.append({"kind": "run_dir", "path": run_dir, "metadata": {}})
    output_refs = raw.get("output_refs")
    if isinstance(output_refs, list):
        for item in output_refs:
            if not isinstance(item, Mapping):
                raise _invalid_mission("output_refs items must be objects")
            refs.append(
                {
                    "kind": _required_string(item, "kind"),
                    "path": _optional_string(item.get("path"), "path") or "",
                    "metadata": _optional_mapping(item.get("metadata"), "metadata")
                    or {},
                }
            )
    return refs


def _mission_run_dir(status: MissionStatus) -> str:
    for output in status.output_refs:
        if output.kind == "run_dir" and output.path:
            return output.path
    run_dir = status.metadata.get("run_dir")
    if isinstance(run_dir, str):
        return run_dir
    return ""


def _mission_outputs(status: MissionStatus) -> Mapping[str, object]:
    outputs = status.metadata.get("outputs")
    if isinstance(outputs, Mapping):
        return dict(outputs)
    projected: dict[str, object] = {}
    for output in status.output_refs:
        if output.path:
            projected[output.kind] = output.path
        elif output.metadata:
            projected[output.kind] = dict(output.metadata)
    return projected


def _status_projection(status: MissionStatus) -> dict[str, object]:
    raw = status.metadata.get("raw_result")
    if isinstance(raw, Mapping):
        return dict(raw)
    return {
        "run_id": status.mission_id,
        "mission_id": status.mission_id,
        "state": status.state,
        "terminal": status.terminal,
        "partial_failures": status.partial_failures,
        "cancelled": status.cancelled,
        "parent_invocation_id": status.parent_invocation_id,
        "parent_receipt_ura": status.parent_receipt_ura,
        "run_dir": _mission_run_dir(status),
        "outputs": dict(_mission_outputs(status)),
        "child_invocations": [
            {
                "step_id": child.step_id,
                "request_id": child.request_id,
                "trace_id": child.trace_id,
                "ability": child.ability,
                "invocation_ura": child.invocation_ura,
                "caller_ura": child.caller_ura,
                "callee_ura": child.callee_ura,
                "subject_ura": child.subject_ura,
                "metadata_state": child.metadata_state,
                "ledger_state": child.ledger_state,
                "receipt": child.receipt,
            }
            for child in status.child_invocations
        ],
        "child_receipts": [
            {
                "step_id": receipt.step_id,
                "invocation_ura": receipt.invocation_ura,
                "receipt_ura": receipt.receipt_ura,
                "receipt_hash": receipt.receipt_hash,
            }
            for receipt in status.child_receipts
        ],
        "output_refs": [
            {
                "kind": output.kind,
                "path": output.path,
                "metadata": dict(output.metadata),
            }
            for output in status.output_refs
        ],
        "metadata": dict(status.metadata),
    }


def _required_object_attr(value: object, attr: str) -> str:
    candidate = getattr(value, attr, None)
    if not isinstance(candidate, str) or not candidate.strip():
        raise _invalid_mission(f"EasyRemote client identity field {attr!r} is required")
    return candidate


def _call_method(
    target: object, method_name: str, *args: object, **kwargs: object
) -> object:
    method = getattr(target, method_name, None)
    if not callable(method):
        raise _invalid_mission(f"EasyRemote client does not expose {method_name}()")
    try:
        return method(*args, **kwargs)
    except SDKError:
        raise
    except Exception as exc:
        raise _transport_error(f"EasyRemote client {method_name}() failed", exc) from exc


def _mapping(value: object, field_name: str) -> dict[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_mission(f"{field_name} must be an object")
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
