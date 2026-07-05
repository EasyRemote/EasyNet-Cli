"""Daemon profile bridge over SDK-owned Admin and Mission DTOs."""

from __future__ import annotations

import base64
import json
import os
from collections.abc import Mapping
from typing import Callable, Protocol

from .admin import (
    AdminCarrierBase,
    AdminClient,
    AgentLifecycleAdapter,
)
from .errors import ErrorCode, RetryHint, SDKError, profile_error_details
from .mission import (
    EasyRemoteMissionAdapter,
    MissionCarrierBase,
    MissionClient,
)
from .system_abilities import AdminSystemAbility, MissionSystemAbility

_ADMIN_PROFILE = "admin_gateway"
_MISSION_PROFILE = "mission"
_DESCRIPTOR_VERSION = "1.0.0"
_AGENT_START = AdminSystemAbility.AGENT_START
_AGENT_LIST = AdminSystemAbility.AGENT_LIST
_AGENT_STOP = AdminSystemAbility.AGENT_STOP
_AGENT_REFRESH = AdminSystemAbility.AGENT_REFRESH
_GATEWAY_STATUS = AdminSystemAbility.GATEWAY_STATUS
_SESSION_LIST = AdminSystemAbility.SESSION_LIST
_SESSION_CREATE = AdminSystemAbility.SESSION_CREATE
_SESSION_DELETE = AdminSystemAbility.SESSION_DELETE
_HUB_JOIN = AdminSystemAbility.HUB_JOIN
_HUB_LEAVE = AdminSystemAbility.HUB_LEAVE
_PAIRING_PREFLIGHT = AdminSystemAbility.PAIRING_PREFLIGHT
_PAIRING_VALIDATE = AdminSystemAbility.PAIRING_VALIDATE
_CREDENTIAL_VERIFY = AdminSystemAbility.CREDENTIAL_VERIFY
_PAIRING_CREATE = AdminSystemAbility.PAIRING_CREATE
_FEDERATION_REVOKE = AdminSystemAbility.FEDERATION_REVOKE
_MISSION_RUN = MissionSystemAbility.RUN
_MISSION_TRACK = MissionSystemAbility.TRACK
_MISSION_CANCEL = MissionSystemAbility.CANCEL
_MISSION_EVENTS = MissionSystemAbility.EVENTS


class ProfileBridgeDispatcher(Protocol):
    """Minimal dispatcher needed by SDK-owned daemon profile bridges."""

    def device_ura(self) -> str:
        """Return the caller/callee device URA for daemon system abilities."""

    def invoke_system_ability(
        self, ability: str, **kwargs: object
    ) -> Mapping[str, object]:
        """Invoke one daemon system ability and return its profile result."""


NonceFactory = Callable[[], bytes]


class DaemonProfileBridge:
    """SDK-owned Admin/Mission bridge for host applications."""

    def __init__(
        self,
        dispatcher: ProfileBridgeDispatcher,
        *,
        nonce_factory: NonceFactory | None = None,
        descriptor_version: str = _DESCRIPTOR_VERSION,
    ) -> None:
        self._dispatcher = dispatcher
        self._nonce_factory = nonce_factory or (lambda: os.urandom(16))
        self._descriptor_version = descriptor_version

    def admin_facade(self) -> AgentLifecycleAdapter:
        return AgentLifecycleAdapter(
            AdminClient(_AdminBridgeTransport(self._dispatcher)),
            self.admin_base(),
        )

    def mission_facade(self) -> EasyRemoteMissionAdapter:
        return EasyRemoteMissionAdapter(
            MissionClient(_MissionBridgeTransport(self._dispatcher)),
            self.mission_base(),
        )

    def admin_base(self) -> AdminCarrierBase:
        device = self._device_ura()
        return AdminCarrierBase(
            caller_ura=device,
            callee_ura=device,
            subject_ura=device,
            descriptor_version=self._descriptor_version,
            nonce_base64=self._nonce_base64(),
            causal_context={"form": "none"},
            metadata={"profile": _ADMIN_PROFILE, "source": "profile_bridge"},
        )

    def mission_base(self) -> MissionCarrierBase:
        device = self._device_ura()
        return MissionCarrierBase(
            caller_ura=device,
            callee_ura=device,
            subject_ura=device,
            descriptor_version=self._descriptor_version,
            nonce_base64=self._nonce_base64(),
            causal_context={"form": "none"},
            metadata={"profile": _MISSION_PROFILE, "source": "profile_bridge"},
        )

    def _device_ura(self) -> str:
        device = self._dispatcher.device_ura()
        if not isinstance(device, str) or not device.strip():
            raise _invalid_admin("profile bridge dispatcher device_ura is required")
        return device

    def _nonce_base64(self) -> str:
        nonce = self._nonce_factory()
        if not isinstance(nonce, bytes) or not nonce:
            raise _invalid_admin("profile bridge nonce factory must return bytes")
        return base64.b64encode(nonce).decode("ascii")


class _AdminBridgeTransport:
    def __init__(self, dispatcher: ProfileBridgeDispatcher) -> None:
        self._dispatcher = dispatcher

    def build_agent_list_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("build_agent_list_invocation")

    def build_agent_start_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("build_agent_start_invocation")

    def build_agent_stop_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("build_agent_stop_invocation")

    def build_agent_refresh_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("build_agent_refresh_invocation")

    def build_session_list_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("build_session_list_invocation")

    def build_revoke_device_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("build_revoke_device_invocation")

    def gateway_status(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge gateway status request", _invalid_admin
        )
        payload: dict[str, object] = {}
        if "require_public_listener" in request:
            payload["require_public_listener"] = _optional_bool(
                request.get("require_public_listener"), "require_public_listener"
            )
        response = self._invoke(_GATEWAY_STATUS, **payload)
        return _gateway_status_json(
            response,
            require_public_listener=payload.get("require_public_listener"),
        )

    def list_agents(self, request_json: bytes) -> bytes:
        _json_object(request_json, "profile bridge agent list request", _invalid_admin)
        response = self._invoke(_AGENT_LIST)
        agents = response.get("agents") or []
        if not isinstance(agents, list):
            raise _invalid_admin("agent.list response field 'agents' must be an array")
        return _json_bytes(
            {
                "profile": _ADMIN_PROFILE,
                "kind": "agent_records",
                "state": "ok",
                "items": [_agent_record(row) for row in agents],
                "next_cursor": None,
                "metadata": {
                    "profile": _ADMIN_PROFILE,
                    "source": _AGENT_LIST.value,
                    "count": len(agents),
                    "raw_result": dict(response),
                },
            }
        )

    def agent_start(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge agent start request", _invalid_admin
        )
        response = self._invoke(
            _AGENT_START,
            name=_required_admin_string(request, "name"),
            agent_type=_required_admin_string(request, "agent_type"),
            model=request.get("model"),
            model_present=request.get("model_present", True),
            label=request.get("label"),
            command=request.get("command"),
            command_args=list(
                _string_array(request.get("command_args", []), "command_args")
            ),
            materialize_directory=request.get("materialize_directory", True),
            update_existing_spec=request.get("update_existing_spec", False),
            project_workspace=request.get("project_workspace", True),
        )
        return _admin_result_json(_AGENT_START, response)

    def agent_refresh(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge agent refresh request", _invalid_admin
        )
        payload: dict[str, object] = {}
        if request.get("name"):
            payload["name"] = _required_admin_string(request, "name")
        response = self._invoke(_AGENT_REFRESH, **payload)
        return _admin_result_json(_AGENT_REFRESH, response)

    def agent_stop(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge agent stop request", _invalid_admin
        )
        payload: dict[str, object] = {}
        if request.get("name"):
            payload["name"] = _required_admin_string(request, "name")
        if request.get("agent_ura"):
            payload["agent_ura"] = _required_admin_string(request, "agent_ura")
        response = self._invoke(_AGENT_STOP, **payload)
        return _admin_result_json(_AGENT_STOP, response)

    def list_device_sessions(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge session list request", _invalid_admin
        )
        payload: dict[str, object] = {}
        if "include_terminated" in request:
            payload["include_terminated"] = _optional_bool(
                request.get("include_terminated"), "include_terminated"
            )
        response = self._invoke(_SESSION_LIST, **payload)
        return _device_session_page_json(response)

    def join_hub(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge hub join request", _invalid_admin
        )
        device_ura = _required_admin_string(request, "device_ura")
        response = self._invoke(
            _HUB_JOIN,
            hub_ura=_required_admin_string(request, "hub_ura"),
            device_ura=device_ura,
        )
        return _admin_result_json(_HUB_JOIN, response, device_ura=device_ura)

    def leave_hub(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge hub leave request", _invalid_admin
        )
        payload: dict[str, object] = {
            "hub_ura": _required_admin_string(request, "hub_ura")
        }
        if request.get("reason"):
            payload["reason"] = _required_admin_string(request, "reason")
        response = self._invoke(_HUB_LEAVE, **payload)
        return _admin_result_json(_HUB_LEAVE, response)

    def pairing_preflight(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge pairing preflight request", _invalid_admin
        )
        payload: dict[str, object] = {
            "hub_ura": _required_admin_string(request, "hub_ura"),
            "device_ura": _required_admin_string(request, "device_ura"),
        }
        requested_scopes = _string_array(
            request.get("requested_scopes"), "requested_scopes"
        )
        if requested_scopes:
            payload["requested_scopes"] = list(requested_scopes)
        response = self._invoke(_PAIRING_PREFLIGHT, **payload)
        return _pairing_preflight_json(response, request)

    def validate_pairing(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge pairing validation request", _invalid_admin
        )
        response = self._invoke(
            _PAIRING_VALIDATE,
            token=_required_admin_string(request, "token"),
            device_ura=_required_admin_string(request, "device_ura"),
        )
        return _device_credential_json(response, request)

    def verify_device_credential(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json,
            "profile bridge device credential verification request",
            _invalid_admin,
        )
        response = self._invoke(
            _CREDENTIAL_VERIFY,
            credential_id=_required_admin_string(request, "credential_id"),
            device_ura=_required_admin_string(request, "device_ura"),
            hub_ura=_required_admin_string(request, "hub_ura"),
        )
        return _credential_verification_json(response, request)

    def create_pairing(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge pairing create request", _invalid_admin
        )
        payload: dict[str, object] = {
            "hub_ura": _required_admin_string(request, "hub_ura"),
            "device_ura": _required_admin_string(request, "device_ura"),
            "expires_unix_ms": _positive_admin_int(
                request.get("expires_unix_ms"), "expires_unix_ms"
            ),
        }
        scopes = _string_array(request.get("scopes"), "scopes")
        if scopes:
            payload["scopes"] = list(scopes)
        response = self._invoke(_PAIRING_CREATE, **payload)
        return _pairing_token_json(response, request)

    def revoke_device(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge device revoke request", _invalid_admin
        )
        device_ura = _required_admin_string(request, "device_ura")
        response = self._invoke(
            _FEDERATION_REVOKE,
            agent_ura=device_ura,
            reason=_required_admin_string(request, "reason"),
        )
        return _admin_result_json(
            _FEDERATION_REVOKE,
            response,
            device_ura=device_ura,
        )

    def create_device_session(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge session create request", _invalid_admin
        )
        payload: dict[str, object] = {
            "device_ura": _required_admin_string(request, "device_ura"),
            "hub_ura": _required_admin_string(request, "hub_ura"),
            "session_kind": _required_admin_string(request, "session_kind"),
        }
        if request.get("expires_unix_ms"):
            payload["expires_unix_ms"] = _positive_admin_int(
                request.get("expires_unix_ms"), "expires_unix_ms"
            )
        response = self._invoke(_SESSION_CREATE, **payload)
        return _device_session_json(response, request)

    def delete_device_session(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge session delete request", _invalid_admin
        )
        payload: dict[str, object] = {
            "session_id": _required_admin_string(request, "session_id")
        }
        if request.get("reason"):
            payload["reason"] = _required_admin_string(request, "reason")
        response = self._invoke(_SESSION_DELETE, **payload)
        return _admin_result_json(_SESSION_DELETE, response)

    def close(self) -> None:
        return None

    def _invoke(
        self, ability: AdminSystemAbility, **kwargs: object
    ) -> dict[str, object]:
        return dict(self._dispatcher.invoke_system_ability(ability.value, **kwargs))


class _MissionBridgeTransport:
    def __init__(self, dispatcher: ProfileBridgeDispatcher) -> None:
        self._dispatcher = dispatcher

    def build_run_eal_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_mission_profile("build_run_eal_invocation")

    def build_run_file_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_mission_profile("build_run_file_invocation")

    def build_track_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_mission_profile("build_track_invocation")

    def build_cancel_invocation(self, request_json: bytes) -> bytes:
        return _unsupported_mission_profile("build_cancel_invocation")

    def run_eal(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge mission run request", _invalid_mission
        )
        payload: dict[str, object] = {
            "source": _required_mission_string(request, "source")
        }
        if request.get("label"):
            payload["label"] = _required_mission_string(request, "label")
        response = self._invoke(_MISSION_RUN, **payload)
        return _mission_status_json(_MISSION_RUN, response)

    def run_file(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge mission run-file request", _invalid_mission
        )
        payload: dict[str, object] = {"path": _required_mission_string(request, "path")}
        if request.get("label"):
            payload["label"] = _required_mission_string(request, "label")
        response = self._invoke(_MISSION_RUN, **payload)
        return _mission_status_json(_MISSION_RUN, response)

    def track(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge mission track request", _invalid_mission
        )
        run_id = _required_mission_string(request, "mission_id")
        response = self._invoke(_MISSION_TRACK, run_id=run_id)
        return _mission_status_json(_MISSION_TRACK, response, mission_id=run_id)

    def cancel(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge mission cancel request", _invalid_mission
        )
        run_id = _required_mission_string(request, "mission_id")
        response = self._invoke(_MISSION_CANCEL, run_id=run_id)
        return _mission_status_json(_MISSION_CANCEL, response, mission_id=run_id)

    def events(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "profile bridge mission events request", _invalid_mission
        )
        run_id = _required_mission_string(request, "mission_id")
        payload: dict[str, object] = {
            "run_id": run_id,
            "cursor_sequence": _non_negative_int(
                request.get("cursor_sequence", 0), "cursor_sequence"
            ),
        }
        if "limit" in request:
            payload["limit"] = _non_negative_int(request.get("limit"), "limit")
        response = self._invoke(_MISSION_EVENTS, **payload)
        return _mission_event_page_json(response, mission_id=run_id)

    def close(self) -> None:
        return None

    def _invoke(
        self, ability: MissionSystemAbility, **kwargs: object
    ) -> dict[str, object]:
        return dict(self._dispatcher.invoke_system_ability(ability.value, **kwargs))


def _admin_result_json(
    operation: AdminSystemAbility,
    response: Mapping[str, object],
    *,
    device_ura: str | None = None,
) -> bytes:
    raw = dict(response)
    ack = raw.get("ack")
    if ack is None:
        ack = raw.get("ok", True)
    kind = raw.get("kind")
    if not isinstance(kind, str):
        kind = (
            "agent_lifecycle_result"
            if operation in {_AGENT_START, _AGENT_STOP, _AGENT_REFRESH}
            else "admin_result"
        )
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "kind": kind,
            "operation": operation.value,
            "state": str(raw.get("state") or raw.get("status") or "ok"),
            "agent_ura": _optional_string(
                raw.get("agent_ura"), "agent_ura", _invalid_admin
            ),
            "device_ura": _optional_string(
                raw.get("device_ura") or device_ura, "device_ura", _invalid_admin
            ),
            "ack": _optional_bool(ack, "ack"),
            "runtime_not_ready": bool(raw.get("runtime_not_ready", False)),
            "runtime_catalog_not_ready": bool(
                raw.get("runtime_catalog_not_ready", False)
            ),
            "metadata": {
                "profile": _ADMIN_PROFILE,
                "source": operation.value,
                "raw_result": raw,
            },
        }
    )


def _gateway_status_json(
    response: Mapping[str, object], *, require_public_listener: object = None
) -> bytes:
    raw = _admin_output_object(response)
    if raw.get("profile") == _ADMIN_PROFILE and raw.get("kind") == "gateway_status":
        return _json_bytes(raw)
    listeners = raw.get("listeners") if isinstance(raw.get("listeners"), list) else []
    process_live = _admin_bool(raw.get("process_live"), fallback=raw.get("running"))
    control_ready = _admin_bool(raw.get("control_ready"), fallback=raw.get("ready"))
    runtime_ready = _admin_bool(raw.get("runtime_ready"), fallback=raw.get("ready"))
    directory_ready = _admin_bool(raw.get("directory_ready"), fallback=raw.get("ready"))
    trust_ready = _admin_bool(raw.get("trust_ready"), fallback=raw.get("ready"))
    public_ready = _admin_bool(raw.get("public_listener_ready"), fallback=False)
    require_public = (
        require_public_listener if isinstance(require_public_listener, bool) else False
    )
    ready = _admin_bool(
        raw.get("ready"),
        fallback=(
            process_live
            and control_ready
            and runtime_ready
            and directory_ready
            and trust_ready
            and (public_ready or not require_public)
        ),
    )
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "gateway_id": _first_admin_string(raw, "gateway_id", "id", "device_ura"),
            "ready": ready,
            "state": str(
                raw.get("state")
                or raw.get("status")
                or ("ready" if ready else "not_ready")
            ),
            "process_live": process_live,
            "control_ready": control_ready,
            "runtime_ready": runtime_ready,
            "directory_ready": directory_ready,
            "trust_ready": trust_ready,
            "public_listener_ready": public_ready,
            "listeners": [_gateway_listener(row) for row in listeners],
            "identity": _optional_mapping(
                raw.get("identity"), "identity", _invalid_admin
            ),
            "metadata": {
                **_mapping_or_empty(raw.get("metadata")),
                "profile": _ADMIN_PROFILE,
                "source": _GATEWAY_STATUS.value,
                "raw_result": raw,
            },
        }
    )


def _device_session_page_json(response: Mapping[str, object]) -> bytes:
    raw = _admin_output_object(response)
    if raw.get("profile") == _ADMIN_PROFILE and raw.get("kind") == "device_sessions":
        return _json_bytes(raw)
    rows = raw.get("sessions") or raw.get("items") or []
    if not isinstance(rows, list):
        raise _invalid_admin("session.list response field 'sessions' must be an array")
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "kind": "device_sessions",
            "state": str(raw.get("state") or raw.get("status") or "ok"),
            "items": [
                _device_session(row, source=_SESSION_LIST, request={}) for row in rows
            ],
            "next_cursor": raw.get("next_cursor") or raw.get("nextCursor"),
            "metadata": {
                **_mapping_or_empty(raw.get("metadata")),
                "profile": _ADMIN_PROFILE,
                "source": _SESSION_LIST.value,
                "raw_result": raw,
            },
        }
    )


def _device_session_json(
    response: Mapping[str, object], request: Mapping[str, object]
) -> bytes:
    raw = _admin_output_object(response)
    if raw.get("profile") == _ADMIN_PROFILE and raw.get("kind") == "device_session":
        return _json_bytes(raw)
    return _json_bytes(_device_session(raw, source=_SESSION_CREATE, request=request))


def _device_session(
    value: object, *, source: AdminSystemAbility, request: Mapping[str, object]
) -> dict[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_admin("device session item must be an object")
    raw = dict(value)
    return {
        "profile": _ADMIN_PROFILE,
        "kind": "device_session",
        "session_id": _first_admin_string(
            raw, "session_id", "sessionId", "id", "session"
        ),
        "device_ura": _first_admin_string_with_default(
            raw,
            (
                _required_admin_string(request, "device_ura")
                if "device_ura" in request
                else ""
            ),
            "device_ura",
            "deviceUra",
        ),
        "hub_ura": _first_admin_string_with_default(
            raw,
            _required_admin_string(request, "hub_ura") if "hub_ura" in request else "",
            "hub_ura",
            "hubUra",
        ),
        "state": str(raw.get("state") or raw.get("status") or "active"),
        "session_kind": _first_admin_string_with_default(
            raw,
            (
                _required_admin_string(request, "session_kind")
                if "session_kind" in request
                else ""
            ),
            "session_kind",
            "sessionKind",
            "kind",
        ),
        "created_unix_ms": _positive_admin_int(
            raw.get("created_unix_ms")
            or raw.get("createdUnixMs")
            or raw.get("created_at_ms"),
            "created_unix_ms",
        ),
        "expires_unix_ms": _non_negative_admin_int(
            raw.get("expires_unix_ms")
            or raw.get("expiresUnixMs")
            or raw.get("expires_at_ms")
            or request.get("expires_unix_ms")
            or 0,
            "expires_unix_ms",
        ),
        "metadata": {
            **_mapping_or_empty(raw.get("metadata")),
            "profile": _ADMIN_PROFILE,
            "source": source.value,
            "raw_result": raw,
        },
    }


def _pairing_preflight_json(
    response: Mapping[str, object], request: Mapping[str, object]
) -> bytes:
    raw = _admin_output_object(response)
    if raw.get("profile") == _ADMIN_PROFILE and raw.get("kind") == "pairing_preflight":
        return _json_bytes(raw)
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "kind": "pairing_preflight",
            "state": str(raw.get("state") or raw.get("status") or "unknown"),
            "hub_ura": _first_admin_string_with_default(
                raw, _required_admin_string(request, "hub_ura"), "hub_ura", "hubUra"
            ),
            "device_ura": _first_admin_string_with_default(
                raw,
                _required_admin_string(request, "device_ura"),
                "device_ura",
                "deviceUra",
            ),
            "pairing_required": _admin_bool(raw.get("pairing_required"), fallback=True),
            "trust_ready": _admin_bool(raw.get("trust_ready"), fallback=False),
            "scopes": list(
                _string_array(
                    raw.get("scopes") or raw.get("requested_scopes"), "scopes"
                )
            ),
            "metadata": {
                **_mapping_or_empty(raw.get("metadata")),
                "profile": _ADMIN_PROFILE,
                "source": _PAIRING_PREFLIGHT.value,
                "raw_result": raw,
            },
        }
    )


def _pairing_token_json(
    response: Mapping[str, object], request: Mapping[str, object]
) -> bytes:
    raw = _admin_output_object(response)
    if raw.get("profile") == _ADMIN_PROFILE and raw.get("kind") == "pairing_token":
        return _json_bytes(raw)
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "kind": "pairing_token",
            "token_id": _first_admin_string(raw, "token_id", "tokenId", "id"),
            "token": _first_admin_string(raw, "token", "pairing_token", "pairingToken"),
            "hub_ura": _first_admin_string_with_default(
                raw, _required_admin_string(request, "hub_ura"), "hub_ura", "hubUra"
            ),
            "device_ura": _first_admin_string_with_default(
                raw,
                _required_admin_string(request, "device_ura"),
                "device_ura",
                "deviceUra",
            ),
            "state": str(raw.get("state") or raw.get("status") or "issued"),
            "expires_unix_ms": _positive_admin_int(
                raw.get("expires_unix_ms")
                or raw.get("expiresUnixMs")
                or raw.get("expires_at_ms")
                or request.get("expires_unix_ms"),
                "expires_unix_ms",
            ),
            "scopes": list(
                _string_array(raw.get("scopes") or raw.get("granted_scopes"), "scopes")
            ),
            "metadata": {
                **_mapping_or_empty(raw.get("metadata")),
                "profile": _ADMIN_PROFILE,
                "source": _PAIRING_CREATE.value,
                "raw_result": raw,
            },
        }
    )


def _device_credential_json(
    response: Mapping[str, object], request: Mapping[str, object]
) -> bytes:
    raw = _admin_output_object(response)
    if raw.get("profile") == _ADMIN_PROFILE and raw.get("kind") == "device_credential":
        return _json_bytes(raw)
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "kind": "device_credential",
            "credential_id": _first_admin_string(
                raw, "credential_id", "credentialId", "id"
            ),
            "device_ura": _first_admin_string_with_default(
                raw,
                _required_admin_string(request, "device_ura"),
                "device_ura",
                "deviceUra",
            ),
            "hub_ura": _first_admin_string(raw, "hub_ura", "hubUra"),
            "state": str(raw.get("state") or raw.get("status") or "active"),
            "issued_unix_ms": _positive_admin_int(
                raw.get("issued_unix_ms")
                or raw.get("issuedUnixMs")
                or raw.get("created_unix_ms"),
                "issued_unix_ms",
            ),
            "expires_unix_ms": _positive_admin_int(
                raw.get("expires_unix_ms")
                or raw.get("expiresUnixMs")
                or raw.get("expires_at_ms"),
                "expires_unix_ms",
            ),
            "scopes": list(
                _string_array(raw.get("scopes") or raw.get("granted_scopes"), "scopes")
            ),
            "metadata": {
                **_mapping_or_empty(raw.get("metadata")),
                "profile": _ADMIN_PROFILE,
                "source": _PAIRING_VALIDATE.value,
                "raw_result": raw,
            },
        }
    )


def _credential_verification_json(
    response: Mapping[str, object], request: Mapping[str, object]
) -> bytes:
    raw = _admin_output_object(response)
    if (
        raw.get("profile") == _ADMIN_PROFILE
        and raw.get("kind") == "device_credential_verification"
    ):
        return _json_bytes(raw)
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "kind": "device_credential_verification",
            "verified": _admin_bool(raw.get("verified"), fallback=False),
            "credential_id": _first_admin_string_with_default(
                raw,
                _required_admin_string(request, "credential_id"),
                "credential_id",
                "credentialId",
                "id",
            ),
            "device_ura": _first_admin_string_with_default(
                raw,
                _required_admin_string(request, "device_ura"),
                "device_ura",
                "deviceUra",
            ),
            "hub_ura": _first_admin_string_with_default(
                raw, _required_admin_string(request, "hub_ura"), "hub_ura", "hubUra"
            ),
            "method": (
                _first_admin_string(
                    raw, "method", "verification_method", "verificationMethod"
                )
                if _admin_bool(raw.get("verified"), fallback=False)
                else _first_admin_optional_string(
                    raw, "method", "verification_method", "verificationMethod"
                )
                or ""
            ),
            "metadata": {
                **_mapping_or_empty(raw.get("metadata")),
                "profile": _ADMIN_PROFILE,
                "source": _CREDENTIAL_VERIFY.value,
                "raw_result": raw,
            },
        }
    )


def _agent_record(value: object) -> dict[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_admin("agent.list item must be an object")
    raw = dict(value)
    name = raw.get("name")
    runtime = raw.get("runtime") or raw.get("kind")
    if not isinstance(name, str) or not name.strip():
        raise _invalid_admin("agent.list item field 'name' is required")
    if not isinstance(runtime, str) or not runtime.strip():
        raise _invalid_admin("agent.list item field 'runtime' is required")
    metadata = _mapping_or_empty(raw.get("metadata"))
    for key in ("root_path", "root_exists", "timeout_secs"):
        if key in raw and key not in metadata:
            metadata[key] = raw[key]
    metadata.setdefault("profile", _ADMIN_PROFILE)
    metadata.setdefault("source", _AGENT_LIST.value)
    return {
        "name": name,
        "agent_ura": raw.get("agent_ura"),
        "owner_ura": raw.get("owner_ura"),
        "device_ura": raw.get("device_ura"),
        "state": (
            raw.get("state") if isinstance(raw.get("state"), str) else "registered"
        ),
        "runtime": runtime,
        "model": raw.get("model"),
        "label": raw.get("label"),
        "abilities": (
            raw.get("abilities") if isinstance(raw.get("abilities"), list) else []
        ),
        "metadata": metadata,
    }


def _mission_status_json(
    source: MissionSystemAbility,
    response: Mapping[str, object],
    *,
    mission_id: str | None = None,
) -> bytes:
    raw = dict(response)
    metadata: dict[str, object] = {
        "profile": _MISSION_PROFILE,
        "source": source.value,
        "raw_result": raw,
    }
    run_dir = raw.get("run_dir")
    outputs = raw.get("outputs")
    if isinstance(run_dir, str):
        metadata["run_dir"] = run_dir
    if isinstance(outputs, Mapping):
        metadata["outputs"] = dict(outputs)
    return _json_bytes(
        {
            "profile": _MISSION_PROFILE,
            "kind": "mission_status",
            "mission_id": _mission_id(source, raw, mission_id),
            "state": _mission_state(source, raw),
            "terminal": _mission_terminal(source, raw),
            "partial_failures": _partial_failures(raw),
            "cancelled": _cancelled(raw),
            "parent_invocation_id": _optional_string(
                raw.get("parent_invocation_id"),
                "parent_invocation_id",
                _invalid_mission,
            ),
            "parent_receipt_ura": _optional_string(
                raw.get("parent_receipt_ura"), "parent_receipt_ura", _invalid_mission
            ),
            "parent_invocation": _optional_mapping(
                raw.get("parent_invocation"), "parent_invocation", _invalid_mission
            ),
            "child_invocations": [],
            "child_receipts": [],
            "output_refs": _output_refs(raw),
            "metadata": metadata,
        }
    )


def _mission_event_page_json(
    response: Mapping[str, object], *, mission_id: str
) -> bytes:
    raw = dict(response)
    events = raw.get("events")
    if not isinstance(events, list):
        raise _invalid_mission(
            "mission events response field 'events' must be an array"
        )
    cursor_sequence = _non_negative_int(
        raw.get("cursor_sequence", 0), "cursor_sequence"
    )
    next_cursor_sequence = _non_negative_int(
        raw.get("next_cursor_sequence", cursor_sequence + len(events)),
        "next_cursor_sequence",
    )
    return _json_bytes(
        {
            "profile": _MISSION_PROFILE,
            "kind": "mission_event_page",
            "mission_id": mission_id,
            "cursor_sequence": cursor_sequence,
            "next_cursor_sequence": next_cursor_sequence,
            "has_more": _bool_value(raw.get("has_more", False), "has_more"),
            "dropped_count": _non_negative_int(
                raw.get("dropped_count", 0), "dropped_count"
            ),
            "events": [_mission_event(row, mission_id) for row in events],
            "metadata": {
                **_mapping_or_empty(raw.get("metadata")),
                "profile": _MISSION_PROFILE,
                "source": _MISSION_EVENTS.value,
                "raw_result": raw,
            },
        }
    )


def _mission_event(value: object, mission_id: str) -> dict[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_mission("mission event item must be an object")
    raw = dict(value)
    return {
        "profile": _MISSION_PROFILE,
        "kind": "mission_event",
        "mission_id": mission_id,
        "sequence": _non_negative_int(raw.get("sequence"), "sequence"),
        "event_type": _required_mission_string(raw, "event_type"),
        "occurred_unix_ms": _non_negative_int(
            raw.get("occurred_unix_ms"), "occurred_unix_ms"
        ),
        "terminal": _bool_value(raw.get("terminal"), "terminal"),
        "payload": raw.get("payload"),
        "receipt": _mapping_or_empty(raw.get("receipt")),
        "metadata": _mapping_or_empty(raw.get("metadata")),
    }


def _mission_id(
    source: MissionSystemAbility,
    raw: Mapping[str, object],
    mission_id: str | None,
) -> str:
    if mission_id:
        return mission_id
    for field_name in ("mission_id", "run_id"):
        value = raw.get(field_name)
        if isinstance(value, str) and value.strip():
            return value
    raise _invalid_mission(f"{source.value} response is missing mission run_id")


def _mission_state(source: MissionSystemAbility, raw: Mapping[str, object]) -> str:
    value = raw.get("state")
    if isinstance(value, str) and value.strip():
        return value
    if _cancelled(raw):
        return "cancelled"
    if source is _MISSION_RUN:
        return "running"
    return "ok"


def _mission_terminal(source: MissionSystemAbility, raw: Mapping[str, object]) -> bool:
    value = raw.get("terminal")
    if isinstance(value, bool):
        return value
    if _cancelled(raw):
        return True
    state = raw.get("state")
    if isinstance(state, str) and state.lower() in {
        "completed",
        "failed",
        "cancelled",
        "canceled",
    }:
        return True
    return source is _MISSION_CANCEL and bool(raw.get("ok", True))


def _cancelled(raw: Mapping[str, object]) -> bool:
    value = raw.get("cancelled")
    return value if isinstance(value, bool) else False


def _partial_failures(raw: Mapping[str, object]) -> int:
    value = raw.get("partial_failures")
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_mission("partial_failures must be a non-negative integer")
    return value


def _output_refs(raw: Mapping[str, object]) -> list[dict[str, object]]:
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
                    "kind": _required_mission_string(item, "kind"),
                    "path": _optional_string(item.get("path"), "path", _invalid_mission)
                    or "",
                    "metadata": _optional_mapping(
                        item.get("metadata"), "metadata", _invalid_mission
                    )
                    or {},
                }
            )
    return refs


def _json_object(
    raw: bytes, label: str, invalid: Callable[[str], SDKError]
) -> dict[str, object]:
    try:
        decoded = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise invalid(f"{label} is not valid JSON") from exc
    if not isinstance(decoded, dict):
        raise invalid(f"{label} must be a JSON object")
    return dict(decoded)


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _required_admin_string(value: Mapping[str, object], field_name: str) -> str:
    raw = value.get(field_name)
    if not isinstance(raw, str) or not raw.strip():
        raise _invalid_admin(f"{field_name} is required")
    return raw


def _required_mission_string(value: Mapping[str, object], field_name: str) -> str:
    raw = value.get(field_name)
    if not isinstance(raw, str) or not raw.strip():
        raise _invalid_mission(f"{field_name} is required")
    return raw


def _optional_string(
    value: object, field_name: str, invalid: Callable[[str], SDKError]
) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise invalid(f"{field_name} must be a string")
    return value


def _optional_bool(value: object, field_name: str) -> bool | None:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise _invalid_admin(f"{field_name} must be a boolean")
    return value


def _bool_value(value: object, field_name: str) -> bool:
    if not isinstance(value, bool):
        raise _invalid_mission(f"{field_name} must be a boolean")
    return value


def _non_negative_int(value: object, field_name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_mission(f"{field_name} must be a non-negative integer")
    return value


def _optional_mapping(
    value: object, field_name: str, invalid: Callable[[str], SDKError]
) -> Mapping[str, object] | None:
    if value is None:
        return None
    if not isinstance(value, Mapping):
        raise invalid(f"{field_name} must be an object")
    return dict(value)


def _mapping_or_empty(value: object) -> dict[str, object]:
    return dict(value) if isinstance(value, Mapping) else {}


def _admin_output_object(value: Mapping[str, object]) -> dict[str, object]:
    raw = dict(value)
    nested = raw.get("result")
    if isinstance(nested, Mapping):
        return dict(nested)
    return raw


def _string_array(value: object, field_name: str) -> tuple[str, ...]:
    if value is None:
        return tuple()
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise _invalid_admin(f"{field_name} must be an array of strings")
    return tuple(value)


def _admin_bool(value: object, *, fallback: object) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(fallback, bool):
        return fallback
    return False


def _positive_admin_int(value: object, field_name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_admin(f"{field_name} must be a positive integer")
    return value


def _non_negative_admin_int(value: object, field_name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_admin(f"{field_name} must be a non-negative integer")
    return value


def _first_admin_string(value: Mapping[str, object], *field_names: str) -> str:
    for field_name in field_names:
        raw = value.get(field_name)
        if isinstance(raw, str) and raw.strip():
            return raw
    raise _invalid_admin(f"{field_names[0]} is required")


def _first_admin_optional_string(
    value: Mapping[str, object], *field_names: str
) -> str | None:
    for field_name in field_names:
        raw = value.get(field_name)
        if raw is None:
            continue
        if not isinstance(raw, str):
            raise _invalid_admin(f"{field_name} must be a string")
        if raw.strip():
            return raw
    return None


def _first_admin_string_with_default(
    value: Mapping[str, object], default: str, *field_names: str
) -> str:
    found = _first_admin_optional_string(value, *field_names)
    if found:
        return found
    if default:
        return default
    raise _invalid_admin(f"{field_names[0]} is required")


def _gateway_listener(value: object) -> dict[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_admin("gateway listener must be an object")
    raw = dict(value)
    return {
        "kind": _first_admin_string(raw, "kind"),
        "endpoint": _first_admin_string(raw, "endpoint", "address"),
        "ready": _admin_bool(raw.get("ready"), fallback=False),
        "public": _admin_bool(raw.get("public"), fallback=False),
    }


def _invalid_admin(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="admin_gateway",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_ADMIN_PROFILE),
    )


def _invalid_mission(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="mission",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_MISSION_PROFILE),
    )


def _unsupported_admin_profile(method_name: str) -> bytes:
    raise SDKError(
        code=ErrorCode.NOT_IMPLEMENTED,
        stage="admin_gateway",
        retry=RetryHint.NEVER,
        retryable=False,
        message=(
            f"Admin profile bridge does not support SDK profile method "
            f"{method_name}; use the EasyNet-Cli SDK/Admin backend facade for "
            "Hub, pairing, session, gateway, and invocation-builder operations"
        ),
        details=profile_error_details(
            _ADMIN_PROFILE,
            details={"profile_method": method_name},
        ),
    )


def _unsupported_mission_profile(method_name: str) -> bytes:
    raise SDKError(
        code=ErrorCode.NOT_IMPLEMENTED,
        stage="mission",
        retry=RetryHint.NEVER,
        retryable=False,
        message=(
            f"Mission profile bridge does not support SDK profile method "
            f"{method_name}; use the EasyNet-Cli SDK/Mission backend facade for "
            "file execution and invocation-builder operations"
        ),
        details=profile_error_details(
            _MISSION_PROFILE,
            details={"profile_method": method_name},
        ),
    )
