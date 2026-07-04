"""Mission profile facade."""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass, field, replace
from typing import Any, Callable, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
from .errors import ErrorCode, RetryHint, SDKError, profile_error_details
from .invocation import InvocationDraft


_PROFILE = "mission"
_EASYREMOTE_FAILURE_POLICIES = frozenset({"abort", "skip", "retry", "continue"})
_EASYREMOTE_IDENTIFIER = re.compile(r"[^A-Za-z0-9_]")


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
class MissionEventTailOptions:
    """Bounded live-tail controls for Mission event pages."""

    cursor_sequence: int = 0
    limit: int = 0
    max_empty_pages: int = 0
    poll_interval_seconds: float = 0.0

    def validated(self) -> "MissionEventTailOptions":
        if self.cursor_sequence < 0:
            raise _invalid_mission("mission event cursor_sequence must be non-negative")
        if self.limit < 0:
            raise _invalid_mission("mission event limit must be non-negative")
        if self.limit > 1000:
            raise _invalid_mission("mission event limit exceeds bounds")
        if self.max_empty_pages < 0:
            raise _invalid_mission("mission event max_empty_pages must be non-negative")
        if self.poll_interval_seconds < 0:
            raise _invalid_mission(
                "mission event poll_interval_seconds must be non-negative"
            )
        return self


class MissionEventTailer:
    """SDK-owned state machine for tailing Mission event pages."""

    def __init__(
        self,
        client: "MissionClient",
        request: MissionEventListRequest,
        *,
        options: MissionEventTailOptions | None = None,
        sleep: Callable[[float], None] | None = None,
    ) -> None:
        self._client = client
        self._request = request
        self._options = (options or MissionEventTailOptions()).validated()
        self._sleep = sleep
        self._cursor_sequence = self._options.cursor_sequence
        self._buffer: list[MissionEvent] = []
        self._empty_pages = 0
        self._closed = False
        self._terminal_seen = False

    @property
    def cursor_sequence(self) -> int:
        return self._cursor_sequence

    def close(self) -> None:
        self._closed = True

    def __iter__(self) -> "MissionEventTailer":
        return self

    def __next__(self) -> MissionEvent:
        if not self._buffer:
            self._fill_buffer()
        if not self._buffer:
            raise StopIteration
        event = self._buffer.pop(0)
        if event.terminal:
            self.close()
        return event

    def _fill_buffer(self) -> None:
        while not self._closed and not self._terminal_seen:
            request = replace(
                self._request,
                cursor_sequence=self._cursor_sequence,
                limit=self._options.limit,
            )
            previous_cursor = self._cursor_sequence
            page = self._client.events(request)
            if page.dropped_count:
                raise SDKError(
                    code=ErrorCode.PROTOCOL,
                    stage="mission",
                    retry=RetryHint.SAFE,
                    retryable=True,
                    message="mission event tail dropped daemon events",
                    details={
                        "reason": "mission_events_dropped",
                        "mission_id": page.mission_id,
                        "cursor_sequence": page.cursor_sequence,
                        "dropped_count": page.dropped_count,
                    },
                )
            self._cursor_sequence = page.next_cursor_sequence
            self._buffer.extend(page.events)
            if any(event.terminal for event in page.events):
                self._terminal_seen = True
            if self._buffer:
                return
            if page.has_more and self._cursor_sequence == previous_cursor:
                raise _invalid_mission("mission event tail made no cursor progress")
            if page.has_more:
                continue
            self._empty_pages += 1
            if self._empty_pages > self._options.max_empty_pages:
                self.close()
                return
            self._sleep_once()

    def _sleep_once(self) -> None:
        interval = self._options.poll_interval_seconds
        if interval <= 0:
            return
        if self._sleep is not None:
            self._sleep(interval)
            return
        import time

        time.sleep(interval)


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


class EasyRemoteMissionEventTailer:
    """EasyRemote-facing iterator over SDK Mission tail events."""

    def __init__(self, tailer: MissionEventTailer) -> None:
        self._tailer = tailer

    @property
    def cursor_sequence(self) -> int:
        return self._tailer.cursor_sequence

    def close(self) -> None:
        self._tailer.close()

    def __iter__(self) -> "EasyRemoteMissionEventTailer":
        return self

    def __next__(self) -> Mapping[str, object]:
        return _event_projection(next(self._tailer))

    def __enter__(self) -> "EasyRemoteMissionEventTailer":
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()


@dataclass(frozen=True)
class EasyRemotePipelineStepOutput:
    """A dataflow reference to one Pipeline step result."""

    alias: str

    def render(self) -> str:
        return f"{self.alias}.output"


@dataclass(frozen=True)
class EasyRemotePipelineStep:
    """One SDK-owned EasyRemote Pipeline step plan."""

    alias: str
    ref: str
    args: Mapping[str, object]
    on: str | None = None
    timeout: int | None = None
    retries: int | None = None
    on_failure: str | None = None
    optional: bool = False

    @property
    def output(self) -> EasyRemotePipelineStepOutput:
        return EasyRemotePipelineStepOutput(self.alias)

    def render(self) -> str:
        parts = [f"let {self.alias} = call {_easyremote_eal_string(self.ref)}"]
        if self.on:
            parts.append(f"on {_easyremote_eal_string(self.on)}")
        if self.args:
            fields = ", ".join(
                f"{name} = {_easyremote_eal_field(value)}"
                for name, value in self.args.items()
            )
            parts.append(f"with {{ {fields} }}")
        if self.timeout is not None:
            parts.append(f"timeout {self.timeout}")
        if self.retries is not None:
            parts.append(f"retries {self.retries}")
        if self.on_failure is not None:
            parts.append(f"on_failure {self.on_failure}")
        if self.optional:
            parts.append("optional")
        return " ".join(parts)


@dataclass(frozen=True)
class EasyRemotePipelineChildInvocationIntent:
    """SDK projection of the child Invocation a Pipeline step expects."""

    step_id: str
    ability: str
    on: str | None = None
    optional: bool = False
    on_failure: str | None = None


@dataclass(frozen=True)
class EasyRemotePipelineChildInvocationConformance:
    """Result of matching daemon MissionStatus child facts to a Pipeline plan."""

    mission_id: str
    expected_steps: tuple[str, ...]
    observed_steps: tuple[str, ...]
    missing_steps: tuple[str, ...]
    unexpected_steps: tuple[str, ...]
    ability_mismatched_steps: tuple[str, ...]
    receipt_backed_steps: tuple[str, ...]

    @property
    def passed(self) -> bool:
        return (
            not self.missing_steps
            and not self.unexpected_steps
            and not self.ability_mismatched_steps
        )

    def require_passed(self) -> None:
        if self.passed:
            return
        raise SDKError(
            code=ErrorCode.PROTOCOL,
            stage="mission",
            retry=RetryHint.NEVER,
            retryable=False,
            message="Pipeline child Invocation facts do not match planned steps",
            details={
                "reason": "pipeline_child_invocation_mismatch",
                "mission_id": self.mission_id,
                "missing_steps": list(self.missing_steps),
                "unexpected_steps": list(self.unexpected_steps),
                "ability_mismatched_steps": list(self.ability_mismatched_steps),
            },
        )


class EasyRemotePipelinePlan:
    """SDK-owned EasyRemote Pipeline planning and EAL rendering facade."""

    def __init__(
        self,
        name: str,
        *,
        created_by: str = "",
        version: str = "",
    ) -> None:
        self.name = _required_clean_mission_string(name, "pipeline name")
        self.created_by = created_by.strip()
        self.version = version.strip()
        self._steps: list[EasyRemotePipelineStep] = []
        self._aliases: set[str] = set()

    @property
    def steps(self) -> tuple[EasyRemotePipelineStep, ...]:
        return tuple(self._steps)

    def step(
        self,
        ref: str,
        *,
        on: str | None = None,
        timeout: float | None = None,
        retries: int | None = None,
        on_failure: str | None = None,
        optional: bool = False,
        args: Mapping[str, object] | None = None,
    ) -> EasyRemotePipelineStep:
        target_ref = _required_clean_mission_string(ref, "pipeline step target")
        if on_failure is not None and on_failure not in _EASYREMOTE_FAILURE_POLICIES:
            raise _invalid_pipeline(
                "on_failure must be one of "
                f"{sorted(_EASYREMOTE_FAILURE_POLICIES)}, got {on_failure!r}",
                "invalid_failure_policy",
            )
        step_args = dict(args or {})
        for name, value in step_args.items():
            _easyremote_validate_pipeline_field(name, value, self._aliases)
        step = EasyRemotePipelineStep(
            alias=self._fresh_alias(target_ref),
            ref=target_ref,
            args=step_args,
            on=on.strip() if isinstance(on, str) and on.strip() else None,
            timeout=_easyremote_timeout_seconds(timeout),
            retries=_easyremote_retries_count(retries),
            on_failure=on_failure,
            optional=optional,
        )
        self._steps.append(step)
        self._aliases.add(step.alias)
        return step

    def to_eal(self) -> str:
        if not self._steps:
            raise _invalid_pipeline(f"pipeline '{self.name}' has no steps", "empty_pipeline")
        lines = []
        if self.version:
            lines.append(f"// generated by easyremote {self.version}")
        else:
            lines.append("// generated by easyremote")
        if self.created_by:
            lines.append(f"// created_by: {self.created_by}")
        lines.append(f"mission {_easyremote_eal_string(self.name)} {{")
        lines.extend(f"  {step.render()}" for step in self._steps)
        lines.append("}")
        return "\n".join(lines) + "\n"

    def child_invocation_intents(
        self,
    ) -> tuple[EasyRemotePipelineChildInvocationIntent, ...]:
        return tuple(
            EasyRemotePipelineChildInvocationIntent(
                step_id=step.alias,
                ability=step.ref,
                on=step.on,
                optional=step.optional,
                on_failure=step.on_failure,
            )
            for step in self._steps
        )

    def validate_child_invocations(
        self, status: MissionStatus
    ) -> EasyRemotePipelineChildInvocationConformance:
        intents = self.child_invocation_intents()
        expected_by_step = {intent.step_id: intent for intent in intents}
        expected = set(expected_by_step)
        observed_by_step = {
            child.step_id: child
            for child in status.child_invocations
            if child.step_id is not None
        }
        observed = {
            child.step_id
            for child in status.child_invocations
            if child.step_id is not None
        }
        ability_mismatched = {
            step_id
            for step_id, intent in expected_by_step.items()
            if step_id in observed_by_step
            and observed_by_step[step_id].ability is not None
            and observed_by_step[step_id].ability != intent.ability
        }
        receipt_backed = {
            child.step_id
            for child in status.child_invocations
            if child.step_id is not None and child.receipt is not None
        }
        conformance = EasyRemotePipelineChildInvocationConformance(
            mission_id=status.mission_id,
            expected_steps=tuple(sorted(expected)),
            observed_steps=tuple(sorted(observed)),
            missing_steps=tuple(sorted(expected - observed)),
            unexpected_steps=tuple(sorted(observed - expected)),
            ability_mismatched_steps=tuple(sorted(ability_mismatched)),
            receipt_backed_steps=tuple(sorted(receipt_backed & expected)),
        )
        conformance.require_passed()
        return conformance

    def _fresh_alias(self, ref: str) -> str:
        base = _EASYREMOTE_IDENTIFIER.sub("_", ref.rsplit(".", 1)[-1]) or "step"
        alias = base
        counter = 2
        while alias in self._aliases:
            alias = f"{base}_{counter}"
            counter += 1
        return alias


class EasyRemoteMissionAdapter:
    """SDK-owned Mission cutover adapter for EasyRemote-like callers."""

    def __init__(self, client: "MissionClient", base: MissionCarrierBase) -> None:
        self._client = client
        self._base = base

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

    def events(
        self,
        run_id: str,
        *,
        cursor_sequence: int = 0,
        limit: int = 0,
    ) -> Mapping[str, object]:
        page = self._client.events(
            MissionEventListRequest(
                base=self._base,
                mission_id=_validated_easyremote_run_id(run_id),
                cursor_sequence=cursor_sequence,
                limit=limit,
            )
        )
        return _event_page_projection(page)

    def tail_events(
        self,
        run_id: str,
        *,
        cursor_sequence: int = 0,
        limit: int = 0,
        max_empty_pages: int = 0,
        poll_interval_seconds: float = 0.0,
    ) -> EasyRemoteMissionEventTailer:
        mission_id = _validated_easyremote_run_id(run_id)
        options = MissionEventTailOptions(
            cursor_sequence=cursor_sequence,
            limit=limit,
            max_empty_pages=max_empty_pages,
            poll_interval_seconds=poll_interval_seconds,
        ).validated()
        return EasyRemoteMissionEventTailer(
            MissionEventTailer(
                self._client,
                MissionEventListRequest(
                    base=self._base,
                    mission_id=mission_id,
                    cursor_sequence=options.cursor_sequence,
                    limit=options.limit,
                ),
                options=options,
            )
        )


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


def _required_clean_mission_string(value: str, field_name: str) -> str:
    if not isinstance(value, str):
        raise _invalid_pipeline(
            f"{field_name} must be a string, got {type(value).__name__}",
            f"invalid_{field_name.replace(' ', '_')}",
        )
    trimmed = value.strip()
    if not trimmed:
        raise _invalid_pipeline(
            f"{field_name} must not be empty",
            "empty_name" if field_name == "pipeline name" else "invalid_step_target",
        )
    return trimmed


def _easyremote_validate_pipeline_field(
    name: str,
    value: object,
    aliases: set[str],
) -> None:
    _required_clean_mission_string(name, "pipeline field name")
    if isinstance(value, EasyRemotePipelineStepOutput):
        if value.alias not in aliases:
            raise _invalid_pipeline(
                f"argument '{name}' references step '{value.alias}', which is "
                "not part of this pipeline",
                "foreign_step_output",
            )
        return
    if isinstance(value, bool | str | int):
        return
    if isinstance(value, float):
        if math.isfinite(value):
            return
        raise _invalid_pipeline(
            f"argument '{name}' is non-finite ({value!r}); EAL numbers must be finite",
            "non_finite_field",
        )
    raise _invalid_pipeline(
        f"argument '{name}' is {type(value).__name__}; EAL field values are "
        "scalars or step outputs; pass structured data through an ability that "
        "returns it, then reference its .output",
        "non_scalar_field",
    )


def _easyremote_timeout_seconds(timeout: float | None) -> int | None:
    if timeout is None:
        return None
    if not math.isfinite(timeout) or timeout <= 0:
        raise _invalid_pipeline(
            f"timeout must be a positive finite number of seconds, got {timeout!r}",
            "invalid_timeout",
        )
    return max(1, math.ceil(timeout))


def _easyremote_retries_count(retries: int | None) -> int | None:
    if retries is None:
        return None
    if not isinstance(retries, int) or isinstance(retries, bool) or retries < 0:
        raise _invalid_pipeline(
            f"retries must be a non-negative integer, got {retries!r}",
            "invalid_retries",
        )
    return retries


def _easyremote_eal_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def _easyremote_eal_field(value: object) -> str:
    if isinstance(value, EasyRemotePipelineStepOutput):
        return value.render()
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return repr(value)
    if isinstance(value, float):
        if not math.isfinite(value):
            raise _invalid_pipeline(
                f"EAL number must be finite, got {value!r}", "non_finite_field"
            )
        return repr(value)
    if isinstance(value, str):
        return _easyremote_eal_string(value)
    raise _invalid_pipeline(
        f"EAL field value must be scalar or step output, got {type(value).__name__}",
        "non_scalar_field",
    )


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


def _event_page_projection(page: MissionEventPage) -> dict[str, object]:
    return {
        "mission_id": page.mission_id,
        "cursor_sequence": page.cursor_sequence,
        "next_cursor_sequence": page.next_cursor_sequence,
        "has_more": page.has_more,
        "dropped_count": page.dropped_count,
        "events": [_event_projection(event) for event in page.events],
        "metadata": dict(page.metadata),
    }


def _event_projection(event: MissionEvent) -> dict[str, object]:
    return {
        "event_type": event.event_type,
        "sequence": event.sequence,
        "occurred_unix_ms": event.occurred_unix_ms,
        "terminal": event.terminal,
        "payload": event.payload,
        "receipt": dict(event.receipt),
        "metadata": dict(event.metadata),
    }


def _status_projection(status: MissionStatus) -> dict[str, object]:
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


def _invalid_mission(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="mission",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_PROFILE),
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        details=profile_error_details(_PROFILE),
        cause=cause,
    )


def _invalid_pipeline(message: str, reason: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="mission",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_PROFILE, details={"reason": reason}),
    )
