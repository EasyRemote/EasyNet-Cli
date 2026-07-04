"""Admin + Gateway profile facade."""

from __future__ import annotations

import json
from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import Callable, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
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
class AdminJoinHubRequest:
    base: AdminCarrierBase
    hub_ura: str
    device_ura: str

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        value["hub_ura"] = _validate_hub_ura(self.hub_ura)
        value["device_ura"] = _validate_device_ura(self.device_ura)
        return _json_bytes(value)


@dataclass(frozen=True)
class AdminLeaveHubRequest:
    base: AdminCarrierBase
    hub_ura: str
    reason: str = ""

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        value["hub_ura"] = _validate_hub_ura(self.hub_ura)
        if self.reason:
            value["reason"] = _validate_admin_identifier(self.reason, "reason")
        return _json_bytes(value)


@dataclass(frozen=True)
class PairingPreflightRequest:
    base: AdminCarrierBase
    hub_ura: str
    device_ura: str
    requested_scopes: tuple[str, ...] = ()

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        value["hub_ura"] = _validate_hub_ura(self.hub_ura)
        value["device_ura"] = _validate_device_ura(self.device_ura)
        if self.requested_scopes:
            value["requested_scopes"] = list(_validate_admin_scopes(self.requested_scopes))
        return _json_bytes(value)


@dataclass(frozen=True)
class ValidatePairingRequest:
    base: AdminCarrierBase
    token: str
    device_ura: str

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        value["token"] = _validate_admin_identifier(self.token, "token")
        value["device_ura"] = _validate_device_ura(self.device_ura)
        return _json_bytes(value)


@dataclass(frozen=True)
class VerifyDeviceCredentialRequest:
    base: AdminCarrierBase
    credential_id: str
    device_ura: str
    hub_ura: str

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        value["credential_id"] = _validate_admin_identifier(
            self.credential_id, "credential_id"
        )
        value["device_ura"] = _validate_device_ura(self.device_ura)
        value["hub_ura"] = _validate_hub_ura(self.hub_ura)
        return _json_bytes(value)


@dataclass(frozen=True)
class CreatePairingRequest:
    base: AdminCarrierBase
    hub_ura: str
    device_ura: str
    expires_unix_ms: int
    scopes: tuple[str, ...] = ()

    def to_json_bytes(self) -> bytes:
        if self.expires_unix_ms <= 0:
            raise _invalid_admin("expires_unix_ms is required")
        value = self.base.to_json_dict()
        value["hub_ura"] = _validate_hub_ura(self.hub_ura)
        value["device_ura"] = _validate_device_ura(self.device_ura)
        value["expires_unix_ms"] = self.expires_unix_ms
        if self.scopes:
            value["scopes"] = list(_validate_admin_scopes(self.scopes))
        return _json_bytes(value)


@dataclass(frozen=True)
class RevokeDeviceRequest:
    base: AdminCarrierBase
    device_ura: str
    reason: str

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        value["device_ura"] = _validate_device_ura(self.device_ura)
        value["reason"] = _validate_admin_identifier(self.reason, "reason")
        return _json_bytes(value)


@dataclass(frozen=True)
class CreateDeviceSessionRequest:
    base: AdminCarrierBase
    device_ura: str
    hub_ura: str
    session_kind: str
    expires_unix_ms: int = 0

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        value["device_ura"] = _validate_device_ura(self.device_ura)
        value["hub_ura"] = _validate_hub_ura(self.hub_ura)
        value["session_kind"] = _validate_admin_identifier(
            self.session_kind, "session_kind"
        )
        if self.expires_unix_ms:
            value["expires_unix_ms"] = self.expires_unix_ms
        return _json_bytes(value)


@dataclass(frozen=True)
class DeleteDeviceSessionRequest:
    base: AdminCarrierBase
    session_id: str
    reason: str = ""

    def to_json_bytes(self) -> bytes:
        session_id = _validate_admin_identifier(self.session_id, "session_id")
        if "browser" in session_id.lower():
            raise _invalid_admin("session_id must be a daemon device-session id")
        value = self.base.to_json_dict()
        value["session_id"] = session_id
        if self.reason:
            value["reason"] = _validate_admin_identifier(self.reason, "reason")
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


@dataclass(frozen=True)
class PairingPreflight:
    profile: str
    kind: str
    state: str
    hub_ura: str
    device_ura: str
    pairing_required: bool
    trust_ready: bool
    scopes: tuple[str, ...]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PairingPreflight":
        decoded = _json_object(raw, "pairing preflight")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_admin("invalid pairing preflight projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            state=_required_string(decoded, "state"),
            hub_ura=_required_string(decoded, "hub_ura"),
            device_ura=_required_string(decoded, "device_ura"),
            pairing_required=_required_bool(decoded, "pairing_required"),
            trust_ready=_required_bool(decoded, "trust_ready"),
            scopes=_string_array(decoded.get("scopes"), "scopes"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class PairingToken:
    profile: str
    kind: str
    token_id: str
    token: str
    hub_ura: str
    device_ura: str
    state: str
    expires_unix_ms: int
    scopes: tuple[str, ...]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PairingToken":
        decoded = _json_object(raw, "pairing token")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_admin("invalid pairing token projection")
        expires = _required_int(decoded, "expires_unix_ms")
        if expires <= 0:
            raise _invalid_admin("invalid pairing token projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            token_id=_required_string(decoded, "token_id"),
            token=_required_string(decoded, "token"),
            hub_ura=_required_string(decoded, "hub_ura"),
            device_ura=_required_string(decoded, "device_ura"),
            state=_required_string(decoded, "state"),
            expires_unix_ms=expires,
            scopes=_string_array(decoded.get("scopes"), "scopes"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class DeviceCredential:
    profile: str
    kind: str
    credential_id: str
    device_ura: str
    hub_ura: str
    state: str
    issued_unix_ms: int
    expires_unix_ms: int
    scopes: tuple[str, ...]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DeviceCredential":
        decoded = _json_object(raw, "device credential")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_admin("invalid device credential projection")
        issued = _required_int(decoded, "issued_unix_ms")
        expires = _required_int(decoded, "expires_unix_ms")
        if issued <= 0 or expires <= 0:
            raise _invalid_admin("invalid device credential projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            credential_id=_required_string(decoded, "credential_id"),
            device_ura=_required_string(decoded, "device_ura"),
            hub_ura=_required_string(decoded, "hub_ura"),
            state=_required_string(decoded, "state"),
            issued_unix_ms=issued,
            expires_unix_ms=expires,
            scopes=_string_array(decoded.get("scopes"), "scopes"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class DeviceCredentialVerification:
    profile: str
    kind: str
    verified: bool
    credential_id: str
    device_ura: str
    hub_ura: str
    method: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DeviceCredentialVerification":
        decoded = _json_object(raw, "device credential verification")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_admin("invalid device credential verification projection")
        verification = cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            verified=_required_bool(decoded, "verified"),
            credential_id=_required_string(decoded, "credential_id"),
            device_ura=_required_string(decoded, "device_ura"),
            hub_ura=_required_string(decoded, "hub_ura"),
            method=_optional_string(decoded.get("method"), "method") or "",
            metadata=_required_mapping(decoded, "metadata"),
        )
        if verification.verified and not verification.method:
            raise _invalid_admin("verified device credential must include method")
        return verification


@dataclass(frozen=True)
class DeviceSession:
    profile: str
    kind: str
    session_id: str
    device_ura: str
    hub_ura: str
    state: str
    session_kind: str
    created_unix_ms: int
    expires_unix_ms: int
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DeviceSession":
        return _device_session(_json_object(raw, "device session"))


@dataclass(frozen=True)
class DeviceSessionPage:
    profile: str
    kind: str
    state: str
    items: tuple[DeviceSession, ...]
    next_cursor: object
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DeviceSessionPage":
        decoded = _json_object(raw, "device session page")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "device_sessions":
            raise _invalid_admin("invalid device session page projection")
        items = decoded.get("items")
        if not isinstance(items, list):
            raise _invalid_admin("items must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            state=_required_string(decoded, "state"),
            items=tuple(_device_session(item) for item in items),
            next_cursor=decoded.get("next_cursor"),
            metadata=_required_mapping(decoded, "metadata"),
        )


AgentStartResult = AdminGatewayResult
AgentStopResult = AdminGatewayResult
AgentRefreshResult = AdminGatewayResult
JoinResult = AdminGatewayResult
LeaveResult = AdminGatewayResult
DeviceAdminResult = AdminGatewayResult
VerificationResult = DeviceCredentialVerification


@dataclass(frozen=True)
class EasyRemoteAgentRecord:
    """EasyRemote-facing projection of one hosted daemon agent."""

    name: str
    runtime: str
    model: Optional[str] = None
    root_path: Optional[str] = None
    timeout_secs: Optional[int] = None
    root_exists: Optional[bool] = None
    raw: Mapping[str, object] = field(default_factory=dict, repr=False)

    @classmethod
    def from_wire(cls, value: Mapping[str, object]) -> "EasyRemoteAgentRecord":
        return cls(
            name=str(value.get("name") or ""),
            runtime=str(value.get("runtime") or value.get("agent_type") or ""),
            model=_optional_string(value.get("model"), "model"),
            root_path=_optional_string(value.get("root_path"), "root_path"),
            timeout_secs=_optional_int(value.get("timeout_secs"), "timeout_secs"),
            root_exists=_optional_bool(value.get("root_exists"), "root_exists"),
            raw=dict(value),
        )


@dataclass(frozen=True)
class EasyRemoteAgentStartProjection:
    """EasyRemote-facing projection of daemon `agent.start`."""

    name: str
    runtime: str
    model: Optional[str]
    root_path: Optional[str]
    replaced_prior: bool
    raw: Mapping[str, object] = field(default_factory=dict, repr=False)

    @classmethod
    def from_wire(
        cls, value: Mapping[str, object], *, name: str, runtime: str
    ) -> "EasyRemoteAgentStartProjection":
        return cls(
            name=name,
            runtime=runtime,
            model=_optional_string(value.get("model"), "model"),
            root_path=_optional_string(value.get("root_path"), "root_path"),
            replaced_prior=bool(value.get("replaced_prior", False)),
            raw=dict(value),
        )


class EasyRemoteAdminAdapter:
    """SDK-owned Admin/Gateway cutover adapter for EasyRemote-like clients."""

    def __init__(self, client: object) -> None:
        self._client = client

    def start_agent(
        self,
        name: str,
        *,
        kind: str,
        model: str | None = None,
        label: str | None = None,
        command: str | None = None,
        args: Sequence[str] = (),
    ) -> EasyRemoteAgentStartProjection:
        agent_name = _required_clean_string(name, "agent name")
        _validate_agent_name(agent_name, "name")
        runtime = _required_clean_string(kind, "agent type")
        response = self._invoke(
            "agent.start",
            name=agent_name,
            agent_type=runtime,
            model=model,
            model_present=True,
            label=label,
            command=command,
            command_args=list(args),
            materialize_directory=True,
            update_existing_spec=False,
            project_workspace=True,
        )
        return EasyRemoteAgentStartProjection.from_wire(
            response,
            name=agent_name,
            runtime=runtime,
        )

    def list_agents(self) -> tuple[EasyRemoteAgentRecord, ...]:
        response = self._invoke("agent.list")
        agents = response.get("agents") or []
        if not isinstance(agents, list):
            raise _invalid_admin("agent.list response field 'agents' is not a list")
        return tuple(EasyRemoteAgentRecord.from_wire(_mapping(row, "agent row")) for row in agents)

    def refresh_agents(self, name: str | None = None) -> Mapping[str, object]:
        agent_name = name.strip() if name is not None else None
        if name is not None and not agent_name:
            raise _invalid_admin("agent name must not be empty")
        payload = {"name": agent_name} if agent_name else {}
        return self._invoke("agent.refresh", **payload)

    def _invoke(self, ability: str, **kwargs: object) -> dict[str, object]:
        invocation = _call_method(self._client, "invoke", ability, **kwargs)
        return _mapping(_call_method(invocation, "result"), "admin response")


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

    def join_hub(self, request_json: bytes) -> bytes:
        ...

    def leave_hub(self, request_json: bytes) -> bytes:
        ...

    def pairing_preflight(self, request_json: bytes) -> bytes:
        ...

    def validate_pairing(self, request_json: bytes) -> bytes:
        ...

    def verify_device_credential(self, request_json: bytes) -> bytes:
        ...

    def create_pairing(self, request_json: bytes) -> bytes:
        ...

    def revoke_device(self, request_json: bytes) -> bytes:
        ...

    def create_device_session(self, request_json: bytes) -> bytes:
        ...

    def delete_device_session(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class AdminClient:
    """Admin + Gateway profile facade."""

    transport: AdminTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_admin("admin transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("admin"))

    def build_agent_list_invocation(self, request: AdminAgentListRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_list_invocation,
            "admin agent-list invocation failed",
        )

    def build_agent_start_invocation(self, request: AdminAgentStartRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_start_invocation,
            "admin agent-start invocation failed",
        )

    def build_agent_stop_invocation(self, request: AdminAgentStopRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_stop_invocation,
            "admin agent-stop invocation failed",
        )

    def build_agent_refresh_invocation(
        self, request: AdminAgentRefreshRequest
    ) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_agent_refresh_invocation,
            "admin agent-refresh invocation failed",
        )

    def build_session_list_invocation(
        self, request: AdminSessionListRequest
    ) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_session_list_invocation,
            "admin session-list invocation failed",
        )

    def gateway_status(self, request: AdminGatewayStatusRequest) -> GatewayStatus:
        self._require_open()
        try:
            raw = self.transport.gateway_status(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin gateway status failed", exc) from exc
        return GatewayStatus.from_json(raw)

    def list_agents(self, request: AdminAgentListRequest) -> AdminAgentPage:
        self._require_open()
        try:
            raw = self.transport.list_agents(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin list agents failed", exc) from exc
        return AdminAgentPage.from_json(raw)

    def agent_start(self, request: AdminAgentStartRequest) -> AgentStartResult:
        self._require_open()
        return self._result(
            request.to_json_bytes(),
            self.transport.agent_start,
            "admin agent start failed",
        )

    def agent_stop(self, request: AdminAgentStopRequest) -> AgentStopResult:
        self._require_open()
        return self._result(
            request.to_json_bytes(),
            self.transport.agent_stop,
            "admin agent stop failed",
        )

    def agent_refresh(self, request: AdminAgentRefreshRequest) -> AgentRefreshResult:
        self._require_open()
        return self._result(
            request.to_json_bytes(),
            self.transport.agent_refresh,
            "admin agent refresh failed",
        )

    def list_device_sessions(self, request: AdminSessionListRequest) -> DeviceSessionPage:
        self._require_open()
        try:
            raw = self.transport.list_device_sessions(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin list device sessions failed", exc) from exc
        return DeviceSessionPage.from_json(raw)

    def join_hub(self, request: AdminJoinHubRequest) -> JoinResult:
        self._require_open()
        return self._result(
            request.to_json_bytes(),
            self.transport.join_hub,
            "admin join hub failed",
        )

    def leave_hub(self, request: AdminLeaveHubRequest) -> LeaveResult:
        self._require_open()
        return self._result(
            request.to_json_bytes(),
            self.transport.leave_hub,
            "admin leave hub failed",
        )

    def pairing_preflight(self, request: PairingPreflightRequest) -> PairingPreflight:
        self._require_open()
        try:
            raw = self.transport.pairing_preflight(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin pairing preflight failed", exc) from exc
        return PairingPreflight.from_json(raw)

    def validate_pairing(self, request: ValidatePairingRequest) -> DeviceCredential:
        self._require_open()
        try:
            raw = self.transport.validate_pairing(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin validate pairing failed", exc) from exc
        return DeviceCredential.from_json(raw)

    def verify_device_credential(
        self, request: VerifyDeviceCredentialRequest
    ) -> VerificationResult:
        self._require_open()
        try:
            raw = self.transport.verify_device_credential(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin verify device credential failed", exc) from exc
        return DeviceCredentialVerification.from_json(raw)

    def create_pairing(self, request: CreatePairingRequest) -> PairingToken:
        self._require_open()
        try:
            raw = self.transport.create_pairing(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin create pairing failed", exc) from exc
        return PairingToken.from_json(raw)

    def revoke_device(self, request: RevokeDeviceRequest) -> DeviceAdminResult:
        self._require_open()
        return self._result(
            request.to_json_bytes(),
            self.transport.revoke_device,
            "admin revoke device failed",
        )

    def create_device_session(
        self, request: CreateDeviceSessionRequest
    ) -> DeviceSession:
        self._require_open()
        try:
            raw = self.transport.create_device_session(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("admin create device session failed", exc) from exc
        return DeviceSession.from_json(raw)

    def delete_device_session(
        self, request: DeleteDeviceSessionRequest
    ) -> DeviceAdminResult:
        self._require_open()
        return self._result(
            request.to_json_bytes(),
            self.transport.delete_device_session,
            "admin delete device session failed",
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

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


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


def _validate_admin_identifier(value: str, field_name: str) -> str:
    if not value or not value.strip():
        raise _invalid_admin(f"{field_name} is required")
    if value.strip() != value or any(ch in value for ch in ("/", "\\", "\t", "\r", "\n")):
        raise _invalid_admin(f"{field_name} must be an opaque daemon identifier")
    return value


def _required_clean_string(value: str, field_name: str) -> str:
    trimmed = value.strip()
    if not trimmed:
        raise _invalid_admin(f"{field_name} must not be empty")
    return trimmed


def _validate_admin_scopes(value: tuple[str, ...]) -> tuple[str, ...]:
    for item in value:
        _validate_admin_identifier(item, "scope")
    return value


def _validate_hub_ura(value: str) -> str:
    if not value or not value.strip():
        raise _invalid_admin("hub_ura is required")
    if "/hub/" not in value:
        raise _invalid_admin("hub_ura must be a Hub URA")
    return value


def _validate_device_ura(value: str) -> str:
    if not value or not value.strip():
        raise _invalid_admin("device_ura is required")
    if "/device/" not in value:
        raise _invalid_admin("device_ura must be a Device URA")
    return value


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


def _device_session(value: object) -> DeviceSession:
    if not isinstance(value, dict):
        raise _invalid_admin("device session must be an object")
    created = _required_int(value, "created_unix_ms")
    if created <= 0:
        raise _invalid_admin("invalid device session projection")
    return DeviceSession(
        profile=_required_string(value, "profile"),
        kind=_required_string(value, "kind"),
        session_id=_required_string(value, "session_id"),
        device_ura=_required_string(value, "device_ura"),
        hub_ura=_required_string(value, "hub_ura"),
        state=_required_string(value, "state"),
        session_kind=_required_string(value, "session_kind"),
        created_unix_ms=created,
        expires_unix_ms=_optional_int(value.get("expires_unix_ms"), "expires_unix_ms") or 0,
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


def _required_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_admin(f"{field_name} must be an integer")
    return value


def _optional_int(value: object, field_name: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_admin(f"{field_name} must be an integer or null")
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


def _mapping(value: object, field_name: str) -> dict[str, object]:
    if isinstance(value, Mapping):
        return dict(value)
    raise _invalid_admin(f"{field_name} must be an object")


def _string_array(value: object, field_name: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise _invalid_admin(f"{field_name} must be an array of strings")
    return tuple(value)


def _call_method(
    target: object, method_name: str, *args: object, **kwargs: object
) -> object:
    method = getattr(target, method_name, None)
    if not callable(method):
        raise _invalid_admin(f"EasyRemote client is missing {method_name}()")
    try:
        return method(*args, **kwargs)
    except SDKError:
        raise
    except Exception as exc:
        raise _transport_error(f"EasyRemote admin {method_name} failed", exc) from exc


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
