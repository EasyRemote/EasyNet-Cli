"""Admin + Gateway profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft


_PROFILE = "admin_gateway"


@dataclass(frozen=True)
class AdminCarrierBase:
    """Complete carrier context shared by Admin + Gateway operations."""

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
class AdminGatewayStatusRequest:
    require_public_listener: Optional[bool] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        value: dict[str, object] = {}
        if self.require_public_listener is not None:
            value["require_public_listener"] = self.require_public_listener
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class AdminAgentListRequest:
    base: AdminCarrierBase

    def to_json_bytes(self) -> bytes:
        return _json_bytes(self.base.to_json_dict())


@dataclass(frozen=True)
class AdminAgentStartRequest:
    base: AdminCarrierBase
    name: str
    agent_type: str = ""
    entry: Mapping[str, object] | None = None
    model: str = ""
    label: str = ""
    command: str = ""
    command_args: tuple[str, ...] = ()
    root_path: str = ""
    model_present: Optional[bool] = None
    materialize_directory: Optional[bool] = None
    update_existing_spec: Optional[bool] = None
    project_workspace: Optional[bool] = None

    def to_json_bytes(self) -> bytes:
        _validate_agent_name(self.name, "name")
        if not self.agent_type and self.entry is None:
            raise _invalid_admin("either agent_type or entry is required")
        if self.root_path:
            _validate_absolute_path(self.root_path)
        value = self.base.to_json_dict()
        value["name"] = self.name
        if self.agent_type:
            value["agent_type"] = self.agent_type
        if self.entry is not None:
            value["entry"] = dict(self.entry)
        for key, raw in (
            ("model", self.model),
            ("label", self.label),
            ("command", self.command),
            ("root_path", self.root_path),
        ):
            if raw:
                value[key] = raw
        if self.command_args:
            value["command_args"] = list(self.command_args)
        for key, raw in (
            ("model_present", self.model_present),
            ("materialize_directory", self.materialize_directory),
            ("update_existing_spec", self.update_existing_spec),
            ("project_workspace", self.project_workspace),
        ):
            if raw is not None:
                value[key] = raw
        return _json_bytes(value)


@dataclass(frozen=True)
class AdminAgentStopRequest:
    base: AdminCarrierBase
    name: str = ""
    agent_ura: str = ""

    def to_json_bytes(self) -> bytes:
        if not self.name and not self.agent_ura:
            raise _invalid_admin("either name or agent_ura is required")
        if self.name:
            _validate_agent_name(self.name, "name")
        if self.agent_ura:
            _validate_hosted_agent_ura(self.agent_ura)
            if self.name and not self.agent_ura.endswith("." + self.name):
                raise _invalid_admin("agent_ura must name the same hosted agent as name")
        value = self.base.to_json_dict()
        if self.name:
            value["name"] = self.name
        if self.agent_ura:
            value["agent_ura"] = self.agent_ura
        return _json_bytes(value)


@dataclass(frozen=True)
class AdminAgentRefreshRequest:
    base: AdminCarrierBase
    name: str = ""

    def to_json_bytes(self) -> bytes:
        if self.name:
            _validate_agent_name(self.name, "name")
        value = self.base.to_json_dict()
        if self.name:
            value["name"] = self.name
        return _json_bytes(value)


@dataclass(frozen=True)
class AdminSessionListRequest:
    base: AdminCarrierBase
    include_terminated: Optional[bool] = None

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        if self.include_terminated is not None:
            value["include_terminated"] = self.include_terminated
        return _json_bytes(value)


@dataclass(frozen=True)
class GatewayListener:
    kind: str
    endpoint: str
    ready: bool
    public: bool


@dataclass(frozen=True)
class GatewayStatus:
    """Daemon gateway readiness projection."""

    profile: str
    gateway_id: str
    ready: bool
    state: str
    process_live: bool
    control_ready: bool
    runtime_ready: bool
    directory_ready: bool
    trust_ready: bool
    public_listener_ready: bool
    listeners: tuple[GatewayListener, ...]
    identity: Optional[Mapping[str, object]]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "GatewayStatus":
        decoded = _json_object(raw, "gateway status")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_admin("invalid gateway status projection")
        listeners = decoded.get("listeners")
        if not isinstance(listeners, list):
            raise _invalid_admin("listeners must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            gateway_id=_required_string(decoded, "gateway_id"),
            ready=_required_bool(decoded, "ready"),
            state=_required_string(decoded, "state"),
            process_live=_required_bool(decoded, "process_live"),
            control_ready=_required_bool(decoded, "control_ready"),
            runtime_ready=_required_bool(decoded, "runtime_ready"),
            directory_ready=_required_bool(decoded, "directory_ready"),
            trust_ready=_required_bool(decoded, "trust_ready"),
            public_listener_ready=_required_bool(decoded, "public_listener_ready"),
            listeners=tuple(_gateway_listener(item) for item in listeners),
            identity=_optional_mapping(decoded.get("identity"), "identity"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class AdminAgentRecord:
    name: str
    agent_ura: Optional[str]
    owner_ura: Optional[str]
    device_ura: Optional[str]
    state: str
    runtime: str
    model: Optional[str]
    label: Optional[str]
    abilities: tuple[str, ...]
    metadata: Mapping[str, object]


@dataclass(frozen=True)
class AdminAgentPage:
    profile: str
    kind: str
    state: str
    items: tuple[AdminAgentRecord, ...]
    next_cursor: object
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "AdminAgentPage":
        decoded = _json_object(raw, "admin agent page")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "agent_records":
            raise _invalid_admin("invalid admin agent page projection")
        items = decoded.get("items")
        if not isinstance(items, list):
            raise _invalid_admin("items must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            state=_required_string(decoded, "state"),
            items=tuple(_agent_record(item) for item in items),
            next_cursor=decoded.get("next_cursor"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class AdminGatewayResult:
    profile: str
    kind: str
    state: str
    operation: str = ""
    agent_ura: Optional[str] = None
    device_ura: Optional[str] = None
    ack: Optional[bool] = None
    runtime_not_ready: bool = False
    runtime_catalog_not_ready: bool = False
    items: tuple[Mapping[str, object], ...] = ()
    next_cursor: object = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "AdminGatewayResult":
        decoded = _json_object(raw, "admin result")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_admin("invalid admin result projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            operation=_optional_string(decoded.get("operation"), "operation") or "",
            state=_required_string(decoded, "state"),
            agent_ura=_optional_string(decoded.get("agent_ura"), "agent_ura"),
            device_ura=_optional_string(decoded.get("device_ura"), "device_ura"),
            ack=_optional_bool(decoded.get("ack"), "ack"),
            runtime_not_ready=bool(decoded.get("runtime_not_ready", False)),
            runtime_catalog_not_ready=bool(
                decoded.get("runtime_catalog_not_ready", False)
            ),
            items=_mapping_tuple(decoded.get("items", []), "items"),
            next_cursor=decoded.get("next_cursor"),
            metadata=_required_mapping(decoded, "metadata"),
        )


AgentStartResult = AdminGatewayResult
AgentStopResult = AdminGatewayResult
AgentRefreshResult = AdminGatewayResult
DeviceSessionPage = AdminGatewayResult


@runtime_checkable
class AdminTransport(Protocol):
    """Concrete Admin + Gateway operations supplied by the integration layer."""

    def build_agent_list_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_agent_start_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_agent_stop_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_agent_refresh_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_session_list_invocation(self, request_json: bytes) -> bytes:
        ...

    def gateway_status(self, request_json: bytes) -> bytes:
        ...

    def list_agents(self, request_json: bytes) -> bytes:
        ...

    def agent_start(self, request_json: bytes) -> bytes:
        ...

    def agent_stop(self, request_json: bytes) -> bytes:
        ...

    def agent_refresh(self, request_json: bytes) -> bytes:
        ...

    def list_device_sessions(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class AdminClient:
    """Admin + Gateway profile facade."""

    transport: AdminTransport

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_admin("admin transport is required")

    def build_agent_list_invocation(self, request: AdminAgentListRequest) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_list_invocation,
            "admin agent-list invocation failed",
        )

    def build_agent_start_invocation(self, request: AdminAgentStartRequest) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_start_invocation,
            "admin agent-start invocation failed",
        )

    def build_agent_stop_invocation(self, request: AdminAgentStopRequest) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_stop_invocation,
            "admin agent-stop invocation failed",
        )

    def build_agent_refresh_invocation(
        self, request: AdminAgentRefreshRequest
    ) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_refresh_invocation,
            "admin agent-refresh invocation failed",
        )

    def build_session_list_invocation(
        self, request: AdminSessionListRequest
    ) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_session_list_invocation,
            "admin session-list invocation failed",
        )

    def gateway_status(self, request: AdminGatewayStatusRequest) -> GatewayStatus:
        try:
            raw = self.transport.gateway_status(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin gateway status failed", exc) from exc
        return GatewayStatus.from_json(raw)

    def list_agents(self, request: AdminAgentListRequest) -> AdminAgentPage:
        try:
            raw = self.transport.list_agents(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin list agents failed", exc) from exc
        return AdminAgentPage.from_json(raw)

    def agent_start(self, request: AdminAgentStartRequest) -> AgentStartResult:
        return self._result(
            request.to_json_bytes(),
            self.transport.agent_start,
            "admin agent start failed",
        )

    def agent_stop(self, request: AdminAgentStopRequest) -> AgentStopResult:
        return self._result(
            request.to_json_bytes(),
            self.transport.agent_stop,
            "admin agent stop failed",
        )

    def agent_refresh(self, request: AdminAgentRefreshRequest) -> AgentRefreshResult:
        return self._result(
            request.to_json_bytes(),
            self.transport.agent_refresh,
            "admin agent refresh failed",
        )

    def list_device_sessions(self, request: AdminSessionListRequest) -> DeviceSessionPage:
        return self._result(
            request.to_json_bytes(),
            self.transport.list_device_sessions,
            "admin list device sessions failed",
        )

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

    def _result(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> AdminGatewayResult:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return AdminGatewayResult.from_json(raw)


def _validate_base(base: AdminCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_admin("complete admin invocation carrier is required")


def _validate_agent_name(value: str, field_name: str) -> None:
    if not value or not value.strip():
        raise _invalid_admin(f"{field_name} must not be empty")
    if value == "device" or value.startswith("device."):
        raise _invalid_admin(
            "device system agents are not managed by hosted agent lifecycle"
        )
    if "/" in value or "\\" in value or any(ch.isspace() for ch in value):
        raise _invalid_admin(f"{field_name} must be an owner-local agent id")


def _validate_hosted_agent_ura(value: str) -> None:
    if "/agent/" not in value:
        raise _invalid_admin("agent_ura must be an Agent URA")
    if "/agent/device." in value:
        raise _invalid_admin(
            "device-sponsored System Agents are not managed by hosted agent lifecycle"
        )


def _validate_absolute_path(value: str) -> None:
    if not value.startswith("/"):
        raise _invalid_admin("root_path must be absolute")
    if "/../" in value or value.endswith("/.."):
        raise _invalid_admin("root_path must not contain parent traversal")


def _gateway_listener(value: object) -> GatewayListener:
    if not isinstance(value, dict):
        raise _invalid_admin("gateway listener must be an object")
    return GatewayListener(
        kind=_required_string(value, "kind"),
        endpoint=_required_string(value, "endpoint"),
        ready=_required_bool(value, "ready"),
        public=_required_bool(value, "public"),
    )


def _agent_record(value: object) -> AdminAgentRecord:
    if not isinstance(value, dict):
        raise _invalid_admin("agent record must be an object")
    return AdminAgentRecord(
        name=_required_string(value, "name"),
        agent_ura=_optional_string(value.get("agent_ura"), "agent_ura"),
        owner_ura=_optional_string(value.get("owner_ura"), "owner_ura"),
        device_ura=_optional_string(value.get("device_ura"), "device_ura"),
        state=_required_string(value, "state"),
        runtime=_required_string(value, "runtime"),
        model=_optional_string(value.get("model"), "model"),
        label=_optional_string(value.get("label"), "label"),
        abilities=tuple(_string_array(value.get("abilities"), "abilities")),
        metadata=_required_mapping(value, "metadata"),
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_admin(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_admin(f"{label} JSON must be an object")
    return decoded


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_admin(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_admin(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_admin(f"{field_name} must be a boolean")
    return value


def _optional_bool(value: object, field_name: str) -> Optional[bool]:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise _invalid_admin(f"{field_name} must be a boolean or null")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_admin(f"{field_name} must be an object")
    return dict(value)


def _optional_mapping(value: object, field_name: str) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_admin(f"{field_name} must be an object or null")
    return dict(value)


def _mapping_tuple(value: object, field_name: str) -> tuple[Mapping[str, object], ...]:
    if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
        raise _invalid_admin(f"{field_name} must be an array of objects")
    return tuple(dict(item) for item in value)


def _string_array(value: object, field_name: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise _invalid_admin(f"{field_name} must be an array of strings")
    return tuple(value)


def _invalid_admin(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="admin_gateway",
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
