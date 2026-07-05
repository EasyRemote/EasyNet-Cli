"""EasyRemote profile bridge over SDK-owned Admin and Mission DTOs."""

from __future__ import annotations

import base64
import json
import os
from collections.abc import Mapping
from typing import Callable, Protocol

from .admin import (
    AdminCarrierBase,
    AdminClient,
    EasyRemoteAdminAdapter,
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
_EASYREMOTE_ADMIN_PROFILE = "easyremote_admin_profile"
_EASYREMOTE_MISSION_PROFILE = "easyremote_mission_profile"
_DESCRIPTOR_VERSION = "1.0.0"
_AGENT_START = AdminSystemAbility.AGENT_START
_AGENT_LIST = AdminSystemAbility.AGENT_LIST
_AGENT_STOP = AdminSystemAbility.AGENT_STOP
_AGENT_REFRESH = AdminSystemAbility.AGENT_REFRESH
_MISSION_RUN = MissionSystemAbility.RUN
_MISSION_TRACK = MissionSystemAbility.TRACK
_MISSION_CANCEL = MissionSystemAbility.CANCEL
_MISSION_EVENTS = MissionSystemAbility.EVENTS


class EasyRemoteProfileDispatcher(Protocol):
    """Minimal product dispatcher needed by SDK-owned EasyRemote profile bridges."""

    def device_ura(self) -> str:
        """Return the caller/callee device URA for daemon system abilities."""

    def invoke_system_ability(
        self, ability: str, **kwargs: object
    ) -> Mapping[str, object]:
        """Invoke one daemon system ability and return its product result."""


NonceFactory = Callable[[], bytes]


class EasyRemoteProfileBridge:
    """SDK-owned Admin/Mission bridge for EasyRemote-like product clients."""

    def __init__(
        self,
        dispatcher: EasyRemoteProfileDispatcher,
        *,
        nonce_factory: NonceFactory | None = None,
        descriptor_version: str = _DESCRIPTOR_VERSION,
    ) -> None:
        self._dispatcher = dispatcher
        self._nonce_factory = nonce_factory or (lambda: os.urandom(16))
        self._descriptor_version = descriptor_version

    def admin_facade(self) -> EasyRemoteAdminAdapter:
        return EasyRemoteAdminAdapter(
            AdminClient(_EasyRemoteAdminProfileTransport(self._dispatcher)),
            self.admin_base(),
        )

    def mission_facade(self) -> EasyRemoteMissionAdapter:
        return EasyRemoteMissionAdapter(
            MissionClient(_EasyRemoteMissionProfileTransport(self._dispatcher)),
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
            metadata={"profile": _ADMIN_PROFILE, "source": "easyremote"},
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
            metadata={"profile": _MISSION_PROFILE, "source": "easyremote"},
        )

    def _device_ura(self) -> str:
        device = self._dispatcher.device_ura()
        if not isinstance(device, str) or not device.strip():
            raise _invalid_admin("EasyRemote dispatcher device_ura is required")
        return device

    def _nonce_base64(self) -> str:
        nonce = self._nonce_factory()
        if not isinstance(nonce, bytes) or not nonce:
            raise _invalid_admin("EasyRemote nonce factory must return bytes")
        return base64.b64encode(nonce).decode("ascii")


class _EasyRemoteAdminProfileTransport:
    def __init__(self, dispatcher: EasyRemoteProfileDispatcher) -> None:
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
        return _unsupported_admin_profile("gateway_status")

    def list_agents(self, request_json: bytes) -> bytes:
        _json_object(request_json, "EasyRemote agent list request", _invalid_admin)
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
            request_json, "EasyRemote agent start request", _invalid_admin
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
            request_json, "EasyRemote agent refresh request", _invalid_admin
        )
        payload: dict[str, object] = {}
        if request.get("name"):
            payload["name"] = _required_admin_string(request, "name")
        response = self._invoke(_AGENT_REFRESH, **payload)
        return _admin_result_json(_AGENT_REFRESH, response)

    def agent_stop(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "EasyRemote agent stop request", _invalid_admin
        )
        payload: dict[str, object] = {}
        if request.get("name"):
            payload["name"] = _required_admin_string(request, "name")
        if request.get("agent_ura"):
            payload["agent_ura"] = _required_admin_string(request, "agent_ura")
        response = self._invoke(_AGENT_STOP, **payload)
        return _admin_result_json(_AGENT_STOP, response)

    def list_device_sessions(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("list_device_sessions")

    def join_hub(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("join_hub")

    def leave_hub(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("leave_hub")

    def pairing_preflight(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("pairing_preflight")

    def validate_pairing(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("validate_pairing")

    def verify_device_credential(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("verify_device_credential")

    def create_pairing(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("create_pairing")

    def revoke_device(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("revoke_device")

    def create_device_session(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("create_device_session")

    def delete_device_session(self, request_json: bytes) -> bytes:
        return _unsupported_admin_profile("delete_device_session")

    def close(self) -> None:
        return None

    def _invoke(
        self, ability: AdminSystemAbility, **kwargs: object
    ) -> dict[str, object]:
        return dict(self._dispatcher.invoke_system_ability(ability.value, **kwargs))


class _EasyRemoteMissionProfileTransport:
    def __init__(self, dispatcher: EasyRemoteProfileDispatcher) -> None:
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
            request_json, "EasyRemote mission run request", _invalid_mission
        )
        payload: dict[str, object] = {
            "source": _required_mission_string(request, "source")
        }
        if request.get("label"):
            payload["label"] = _required_mission_string(request, "label")
        response = self._invoke(_MISSION_RUN, **payload)
        return _mission_status_json(_MISSION_RUN, response)

    def run_file(self, request_json: bytes) -> bytes:
        return _unsupported_mission_profile("run_file")

    def track(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "EasyRemote mission track request", _invalid_mission
        )
        run_id = _required_mission_string(request, "mission_id")
        response = self._invoke(_MISSION_TRACK, run_id=run_id)
        return _mission_status_json(_MISSION_TRACK, response, mission_id=run_id)

    def cancel(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "EasyRemote mission cancel request", _invalid_mission
        )
        run_id = _required_mission_string(request, "mission_id")
        response = self._invoke(_MISSION_CANCEL, run_id=run_id)
        return _mission_status_json(_MISSION_CANCEL, response, mission_id=run_id)

    def events(self, request_json: bytes) -> bytes:
        request = _json_object(
            request_json, "EasyRemote mission events request", _invalid_mission
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
    operation: AdminSystemAbility, response: Mapping[str, object]
) -> bytes:
    return _json_bytes(
        {
            "profile": _ADMIN_PROFILE,
            "kind": "agent_lifecycle_result",
            "operation": operation.value,
            "state": str(response.get("state") or "ok"),
            "agent_ura": _optional_string(
                response.get("agent_ura"), "agent_ura", _invalid_admin
            ),
            "ack": _optional_bool(response.get("ack"), "ack"),
            "runtime_not_ready": bool(response.get("runtime_not_ready", False)),
            "runtime_catalog_not_ready": bool(
                response.get("runtime_catalog_not_ready", False)
            ),
            "metadata": {
                "profile": _ADMIN_PROFILE,
                "source": operation.value,
                "raw_result": dict(response),
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
        "state": raw.get("state")
        if isinstance(raw.get("state"), str)
        else "registered",
        "runtime": runtime,
        "model": raw.get("model"),
        "label": raw.get("label"),
        "abilities": raw.get("abilities")
        if isinstance(raw.get("abilities"), list)
        else [],
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
                    "path": _optional_string(
                        item.get("path"), "path", _invalid_mission
                    )
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


def _string_array(value: object, field_name: str) -> tuple[str, ...]:
    if value is None:
        return tuple()
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise _invalid_admin(f"{field_name} must be an array of strings")
    return tuple(value)


def _invalid_admin(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="easyremote_admin_profile",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_EASYREMOTE_ADMIN_PROFILE),
    )


def _invalid_mission(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="easyremote_mission_profile",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_EASYREMOTE_MISSION_PROFILE),
    )


def _unsupported_admin_profile(method_name: str) -> bytes:
    raise SDKError(
        code=ErrorCode.NOT_IMPLEMENTED,
        stage="easyremote_admin_profile",
        retry=RetryHint.NEVER,
        retryable=False,
        message=(
            f"EasyRemote Admin bridge does not support SDK profile method "
            f"{method_name}; use the EasyNet-Cli SDK/Admin backend facade for "
            "Hub, pairing, session, gateway, and invocation-builder operations"
        ),
        details=profile_error_details(
            _EASYREMOTE_ADMIN_PROFILE,
            details={"profile_method": method_name},
        ),
    )


def _unsupported_mission_profile(method_name: str) -> bytes:
    raise SDKError(
        code=ErrorCode.NOT_IMPLEMENTED,
        stage="easyremote_mission_profile",
        retry=RetryHint.NEVER,
        retryable=False,
        message=(
            f"EasyRemote Mission bridge does not support SDK profile method "
            f"{method_name}; use the EasyNet-Cli SDK/Mission backend facade for "
            "file execution and invocation-builder operations"
        ),
        details=profile_error_details(
            _EASYREMOTE_MISSION_PROFILE,
            details={"profile_method": method_name},
        ),
    )
