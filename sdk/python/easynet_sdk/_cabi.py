"""Private C ABI v4 transport adapter.

This module is intentionally the only Python SDK file that imports ``ctypes``.
Public facade modules depend on narrow transport protocols and never expose
raw C ABI symbols or numeric handles to product code.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import json
import queue as queue_module
import threading
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .directory import DirectorySubscription, DirectorySubscriptionCursor
from .errors import ErrorCode, RetryHint, SDKError, retryable_for_hint
from .events import EventStream
from .stream import StreamHandle

EXPECTED_ABI_VERSION = 4
EASYNET_OK = 0
MAX_CABI_CALLBACK_QUEUE = 1024

_StreamCallback = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p)
_BidiCallback = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p)
_CALLBACK_REGISTRY_LOCK = threading.Lock()
_CALLBACK_INBOXES: dict[int, "_CallbackInbox"] = {}
_NEXT_CALLBACK_TOKEN = 1

_JSON_HANDLE_OUTPUT_SYMBOLS = (
    "easynet_identity_build_register_signing_key_invocation",
    "easynet_identity_build_list_signing_keys_invocation",
    "easynet_identity_build_revoke_signing_key_invocation",
    "easynet_identity_project_signing_key_record",
    "easynet_identity_project_signing_key_page",
    "easynet_identity_project_signing_key_revoke_result",
    "easynet_identity_project_signer_handle",
    "easynet_directory_build_list_devices_invocation",
    "easynet_directory_build_list_agents_invocation",
    "easynet_directory_build_list_abilities_invocation",
    "easynet_directory_build_resolve_invocation",
    "easynet_directory_build_subscription_invocation",
    "easynet_directory_project_device_page",
    "easynet_directory_project_agent_page",
    "easynet_directory_project_ability_page",
    "easynet_directory_project_resolved_ref",
    "easynet_directory_project_subscription",
    "easynet_receipt_build_fetch_invocation",
    "easynet_receipt_build_list_history_invocation",
    "easynet_receipt_build_get_history_invocation",
    "easynet_receipt_build_trace_invocation",
    "easynet_receipt_project",
    "easynet_receipt_verify",
    "easynet_receipt_verify_chain",
    "easynet_receipt_causal_ref",
    "easynet_host_binding_build",
    "easynet_host_binding_decode_request",
    "easynet_host_binding_encode_item",
    "easynet_host_binding_encode_error",
    "easynet_host_binding_encode_terminal",
    "easynet_host_binding_fold_output_hash",
    "easynet_publication_build_resource_ref",
    "easynet_publication_validate_package",
    "easynet_publication_install_plugin",
    "easynet_publication_build_deploy_invocation",
    "easynet_publication_project_deploy_result",
    "easynet_publication_build_list_abilities_invocation",
    "easynet_publication_project_ability_page",
    "easynet_publication_build_show_ability_invocation",
    "easynet_publication_project_ability_record",
    "easynet_publication_build_unpublish_invocation",
    "easynet_publication_project_unpublish_result",
    "easynet_publication_build_enable_ability_impl_invocation",
    "easynet_publication_project_enable_ability_impl_result",
    "easynet_publication_build_disable_ability_impl_invocation",
    "easynet_publication_project_disable_ability_impl_result",
    "easynet_mission_build_run_eal_invocation",
    "easynet_mission_build_run_file_invocation",
    "easynet_mission_build_track_invocation",
    "easynet_mission_build_cancel_invocation",
    "easynet_mission_build_events_invocation",
    "easynet_mission_project_status",
    "easynet_mission_project_events",
    "easynet_events_build_directory_subscription_invocation",
    "easynet_events_build_device_subscription_invocation",
    "easynet_events_build_session_subscription_invocation",
    "easynet_events_build_invocation_subscription_invocation",
    "easynet_events_build_device_event_history_invocation",
    "easynet_events_project_device_event_page",
    "easynet_events_project_directory_event",
    "easynet_events_project_terminal",
    "easynet_events_project_drop_report",
    "easynet_admin_build_agent_list_invocation",
    "easynet_admin_build_agent_start_invocation",
    "easynet_admin_build_agent_stop_invocation",
    "easynet_admin_build_agent_refresh_invocation",
    "easynet_admin_build_session_list_invocation",
    "easynet_admin_build_session_create_invocation",
    "easynet_admin_build_session_delete_invocation",
    "easynet_admin_build_hub_join_invocation",
    "easynet_admin_build_hub_leave_invocation",
    "easynet_admin_build_pairing_preflight_invocation",
    "easynet_admin_build_pairing_create_invocation",
    "easynet_admin_build_pairing_validate_invocation",
    "easynet_admin_build_credential_verify_invocation",
    "easynet_admin_build_revoke_device_invocation",
    "easynet_admin_project_gateway_status",
    "easynet_admin_project_agent_records",
    "easynet_admin_project_agent_lifecycle_result",
    "easynet_admin_project_hub_lifecycle_result",
    "easynet_admin_project_pairing_preflight",
    "easynet_admin_project_pairing_token",
    "easynet_admin_project_device_credential",
    "easynet_admin_project_device_credential_verification",
    "easynet_admin_project_device_session_page",
    "easynet_admin_project_device_session_result",
    "easynet_admin_project_device_admin_result",
    "easynet_surface_build_list_pages_invocation",
    "easynet_surface_build_create_page_invocation",
    "easynet_surface_build_delete_page_invocation",
    "easynet_surface_build_manifest_invocation",
    "easynet_surface_build_health_invocation",
    "easynet_surface_project_page_record",
    "easynet_surface_project_page_page",
    "easynet_surface_project_manifest",
    "easynet_surface_project_public_page_ref",
    "easynet_surface_project_mutation_result",
    "easynet_surface_project_health",
    "easynet_compatibility_build_list_models_invocation",
    "easynet_compatibility_build_chat_completion_invocation",
    "easynet_compatibility_build_stream_chat_completion_invocation",
    "easynet_compatibility_build_file_upload_invocation",
    "easynet_compatibility_build_file_retrieve_invocation",
    "easynet_compatibility_build_file_delete_invocation",
    "easynet_compatibility_project_model_page",
    "easynet_compatibility_project_chat_completion",
    "easynet_compatibility_project_chat_stream",
    "easynet_compatibility_project_file_upload",
    "easynet_compatibility_project_file",
    "easynet_compatibility_project_file_delete_result",
    "easynet_wrappers_build_file_transfer_invocation",
    "easynet_wrappers_build_terminal_session_invocation",
    "easynet_wrappers_build_remote_desktop_session_invocation",
    "easynet_wrappers_build_browser_session_invocation",
    "easynet_wrappers_build_media_session_invocation",
    "easynet_wrappers_project_file_record",
    "easynet_wrappers_project_terminal_session",
    "easynet_wrappers_project_remote_desktop_session",
    "easynet_wrappers_project_browser_session",
    "easynet_wrappers_project_media_session",
)


class CLILibrary:
    """Typed binding for the EasyNet-Cli C ABI v4 surface."""

    def __init__(self, raw: Any) -> None:
        self._raw = raw
        self._bind_symbols()

    @classmethod
    def load(cls, path: str | None = None) -> "CLILibrary":
        """Load ``libeasynet_cli`` and verify the ABI version."""

        candidates: list[str] = []
        if path:
            candidates.append(path)
        else:
            found = ctypes.util.find_library("easynet_cli")
            if found:
                candidates.append(found)
            candidates.extend(name for name in _platform_library_candidates() if Path(name).exists())
        if not candidates:
            raise _transport_error("libeasynet_cli was not found")

        errors: list[str] = []
        for candidate in candidates:
            try:
                raw = ctypes.CDLL(candidate)
                lib = cls(raw)
                lib.require_abi(EXPECTED_ABI_VERSION)
                return lib
            except (AttributeError, OSError, SDKError) as exc:
                if path:
                    raise _transport_error(
                        f"load libeasynet_cli failed for {candidate}: {exc}", exc
                    ) from exc
                errors.append(f"{candidate}: {exc}")
        raise _transport_error(
            "no usable libeasynet_cli C ABI v4 library found: " + "; ".join(errors)
        )

    def require_abi(self, expected: int = EXPECTED_ABI_VERSION) -> None:
        actual = int(self._raw.easynet_abi_version())
        if actual != expected:
            raise SDKError(
                code=ErrorCode.VERSION_MISMATCH,
                stage="cabi",
                retry=RetryHint.NEVER,
                message=f"libeasynet_cli ABI version {actual} does not match expected {expected}",
            )

    def feature_discovery(self) -> bytes:
        return self._call_output(self._raw.easynet_feature_discovery)

    def init(self, control_path: str = "") -> int:
        out_handle = ctypes.c_uint64(0)
        raw_path = _optional_c_string(control_path)
        code = int(self._raw.easynet_init(raw_path, ctypes.byref(out_handle)))
        self._raise_for_code(code)
        return int(out_handle.value)

    def shutdown(self, handle: int) -> None:
        code = int(self._raw.easynet_shutdown(ctypes.c_uint64(handle)))
        self._raise_for_code(code)

    def daemon_start(self, config_json: bytes) -> int:
        out_handle = ctypes.c_uint64(0)
        code = int(
            self._raw.easynet_daemon_start(
                ctypes.c_char_p(config_json), ctypes.byref(out_handle)
            )
        )
        self._raise_for_code(code)
        return int(out_handle.value)

    def daemon_attach(self, options_json: bytes) -> int:
        out_handle = ctypes.c_uint64(0)
        code = int(
            self._raw.easynet_daemon_attach(
                ctypes.c_char_p(options_json), ctypes.byref(out_handle)
            )
        )
        self._raise_for_code(code)
        return int(out_handle.value)

    def daemon_discover(self, options_json: bytes) -> bytes:
        return self._call_output(
            self._raw.easynet_daemon_discover,
            ctypes.c_char_p(options_json),
        )

    def daemon_stop(self, daemon_handle: int) -> None:
        code = int(self._raw.easynet_daemon_stop(ctypes.c_uint64(daemon_handle)))
        self._raise_for_code(code)

    def daemon_detach(self, daemon_handle: int) -> None:
        code = int(self._raw.easynet_daemon_detach(ctypes.c_uint64(daemon_handle)))
        self._raise_for_code(code)

    def daemon_status(self, daemon_handle: int) -> bytes:
        return self._call_output(
            self._raw.easynet_daemon_status,
            ctypes.c_uint64(daemon_handle),
        )

    def daemon_endpoints(self, daemon_handle: int) -> bytes:
        return self._call_output(
            self._raw.easynet_daemon_endpoints,
            ctypes.c_uint64(daemon_handle),
        )

    def daemon_invocation_endpoint(self, daemon_handle: int) -> str:
        raw = self._call_output(
            self._raw.easynet_daemon_invocation_endpoint,
            ctypes.c_uint64(daemon_handle),
        )
        endpoint = raw.decode("utf-8")
        if not endpoint:
            raise SDKError(
                code=ErrorCode.CONTROL_ONLY,
                stage="cabi",
                retry=RetryHint.SAFE,
                retryable=True,
                message="daemon did not advertise invocation_endpoint",
            )
        return endpoint

    def daemon_open_client(self, daemon_handle: int) -> int:
        out_handle = ctypes.c_uint64(0)
        code = int(
            self._raw.easynet_daemon_open_client(
                ctypes.c_uint64(daemon_handle), ctypes.byref(out_handle)
            )
        )
        self._raise_for_code(code)
        return int(out_handle.value)

    def identity_project_ura(self, handle: int, ura: bytes) -> bytes:
        return self._call_output(
            self._raw.easynet_identity_project_ura,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(ura),
        )

    def identity_build_ura(self, handle: int, request_json: bytes) -> bytes:
        return self._call_output(
            self._raw.easynet_identity_build_ura,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(request_json),
        )

    def identity_project_descriptor_ref(self, handle: int, descriptor_ref: bytes) -> bytes:
        return self._call_output(
            self._raw.easynet_identity_project_descriptor_ref,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(descriptor_ref),
        )

    def identity_build_descriptor_ref(self, handle: int, request_json: bytes) -> bytes:
        return self._call_output(
            self._raw.easynet_identity_build_descriptor_ref,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(request_json),
        )

    def runtime_health(self, handle: int) -> bytes:
        return self._call_output(
            self._raw.easynet_runtime_health,
            ctypes.c_uint64(handle),
        )

    def runtime_diagnostics(self, handle: int) -> bytes:
        return self._call_output(
            self._raw.easynet_runtime_diagnostics,
            ctypes.c_uint64(handle),
        )

    def invocation_invoke(self, handle: int, invocation_json: bytes) -> bytes:
        return self._call_output(
            self._raw.easynet_invocation_invoke,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(invocation_json),
        )

    def invocation_prepare(
        self, handle: int, invocation_json: bytes, options_json: bytes
    ) -> tuple[int, bytes]:
        return self._call_output_with_id(
            self._raw.easynet_invocation_prepare,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(invocation_json),
            ctypes.c_char_p(options_json),
        )

    def invocation_sign_prepared(
        self, prepared_id: int, signature_json: bytes
    ) -> tuple[int, bytes]:
        return self._call_output_with_id(
            self._raw.easynet_invocation_sign_prepared,
            ctypes.c_uint64(prepared_id),
            ctypes.c_char_p(signature_json),
        )

    def invocation_submit_signed_handle(
        self, handle: int, signed_id: int
    ) -> tuple[int, bytes]:
        return self._call_output_with_id(
            self._raw.easynet_invocation_submit_signed_handle,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(signed_id),
        )

    def invocation_handle_await(self, handle: int, invocation_handle_id: int) -> bytes:
        return self._call_output(
            self._raw.easynet_invocation_handle_await,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(invocation_handle_id),
        )

    def invocation_handle_cancel(
        self, handle: int, invocation_handle_id: int, reason: str
    ) -> bytes:
        return self._call_output(
            self._raw.easynet_invocation_handle_cancel,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(invocation_handle_id),
            ctypes.c_char_p(_optional_c_string(reason)),
        )

    def invocation_handle_events(self, handle: int, invocation_handle_id: int) -> bytes:
        return self._call_output(
            self._raw.easynet_invocation_handle_events,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(invocation_handle_id),
        )

    def invocation_handle_free(self, handle: int, invocation_handle_id: int) -> None:
        code = int(
            self._raw.easynet_invocation_handle_free(
                ctypes.c_uint64(handle), ctypes.c_uint64(invocation_handle_id)
            )
        )
        self._raise_for_code(code)

    def prepared_invocation_free(self, prepared_id: int) -> None:
        code = int(
            self._raw.easynet_prepared_invocation_free(ctypes.c_uint64(prepared_id))
        )
        self._raise_for_code(code)

    def signed_invocation_free(self, signed_id: int) -> None:
        code = int(self._raw.easynet_signed_invocation_free(ctypes.c_uint64(signed_id)))
        self._raise_for_code(code)

    def invocation_stream_open(
        self, handle: int, invocation_json: bytes, callback_token: int
    ) -> int:
        out_stream_id = ctypes.c_uint64(0)
        code = int(
            self._raw.easynet_invocation_stream_open(
                ctypes.c_uint64(handle),
                ctypes.c_char_p(invocation_json),
                _STREAM_CALLBACK_HANDLE,
                ctypes.c_void_p(callback_token),
                ctypes.byref(out_stream_id),
            )
        )
        self._raise_for_code(code)
        return int(out_stream_id.value)

    def invocation_stream_cancel(self, handle: int, stream_id: int) -> None:
        code = int(
            self._raw.easynet_invocation_stream_cancel(
                ctypes.c_uint64(handle), ctypes.c_uint64(stream_id)
            )
        )
        self._raise_for_code(code)

    def invocation_stream_close(self, handle: int, stream_id: int) -> None:
        code = int(
            self._raw.easynet_invocation_stream_close(
                ctypes.c_uint64(handle), ctypes.c_uint64(stream_id)
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_open(
        self, handle: int, invocation_json: bytes, callback_token: int
    ) -> int:
        out_bidi_id = ctypes.c_uint64(0)
        code = int(
            self._raw.easynet_invocation_bidi_open(
                ctypes.c_uint64(handle),
                ctypes.c_char_p(invocation_json),
                _BIDI_CALLBACK_HANDLE,
                ctypes.c_void_p(callback_token),
                ctypes.byref(out_bidi_id),
            )
        )
        self._raise_for_code(code)
        return int(out_bidi_id.value)

    def invocation_bidi_send(self, handle: int, bidi_id: int, frame_json: bytes) -> None:
        code = int(
            self._raw.easynet_invocation_bidi_send(
                ctypes.c_uint64(handle),
                ctypes.c_uint64(bidi_id),
                ctypes.c_char_p(frame_json),
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_close_send(self, handle: int, bidi_id: int) -> None:
        code = int(
            self._raw.easynet_invocation_bidi_close_send(
                ctypes.c_uint64(handle), ctypes.c_uint64(bidi_id)
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_close(self, handle: int, bidi_id: int) -> None:
        code = int(
            self._raw.easynet_invocation_bidi_close(
                ctypes.c_uint64(handle), ctypes.c_uint64(bidi_id)
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_cancel(self, handle: int, bidi_id: int) -> None:
        code = int(
            self._raw.easynet_invocation_bidi_cancel(
                ctypes.c_uint64(handle), ctypes.c_uint64(bidi_id)
            )
        )
        self._raise_for_code(code)

    def json_handle_output(self, symbol: str, handle: int, payload_json: bytes) -> bytes:
        return self._call_output(
            getattr(self._raw, symbol),
            ctypes.c_uint64(handle),
            ctypes.c_char_p(payload_json),
        )

    def _bind_symbols(self) -> None:
        self._raw.easynet_abi_version.argtypes = []
        self._raw.easynet_abi_version.restype = ctypes.c_uint32
        self._raw.easynet_feature_discovery.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
        self._raw.easynet_feature_discovery.restype = ctypes.c_int32
        self._raw.easynet_last_error_json.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
        self._raw.easynet_last_error_json.restype = ctypes.c_int32
        self._raw.easynet_error_json.argtypes = [
            ctypes.c_int32,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_error_json.restype = ctypes.c_int32
        self._raw.easynet_string_free.argtypes = [ctypes.c_void_p]
        self._raw.easynet_string_free.restype = None
        self._raw.easynet_init.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_uint64)]
        self._raw.easynet_init.restype = ctypes.c_int32
        self._raw.easynet_shutdown.argtypes = [ctypes.c_uint64]
        self._raw.easynet_shutdown.restype = ctypes.c_int32
        self._raw.easynet_daemon_start.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.easynet_daemon_start.restype = ctypes.c_int32
        self._raw.easynet_daemon_attach.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.easynet_daemon_attach.restype = ctypes.c_int32
        self._raw.easynet_daemon_discover.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_daemon_discover.restype = ctypes.c_int32
        self._raw.easynet_daemon_stop.argtypes = [ctypes.c_uint64]
        self._raw.easynet_daemon_stop.restype = ctypes.c_int32
        self._raw.easynet_daemon_detach.argtypes = [ctypes.c_uint64]
        self._raw.easynet_daemon_detach.restype = ctypes.c_int32
        self._raw.easynet_daemon_status.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_daemon_status.restype = ctypes.c_int32
        self._raw.easynet_daemon_endpoints.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_daemon_endpoints.restype = ctypes.c_int32
        self._raw.easynet_daemon_invocation_endpoint.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_daemon_invocation_endpoint.restype = ctypes.c_int32
        self._raw.easynet_daemon_open_client.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.easynet_daemon_open_client.restype = ctypes.c_int32
        self._raw.easynet_identity_project_ura.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_identity_project_ura.restype = ctypes.c_int32
        self._raw.easynet_identity_build_ura.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_identity_build_ura.restype = ctypes.c_int32
        self._raw.easynet_identity_project_descriptor_ref.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_identity_project_descriptor_ref.restype = ctypes.c_int32
        self._raw.easynet_identity_build_descriptor_ref.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_identity_build_descriptor_ref.restype = ctypes.c_int32
        self._raw.easynet_runtime_health.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_runtime_health.restype = ctypes.c_int32
        self._raw.easynet_runtime_diagnostics.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_runtime_diagnostics.restype = ctypes.c_int32
        self._raw.easynet_invocation_invoke.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_invocation_invoke.restype = ctypes.c_int32
        self._raw.easynet_invocation_prepare.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_invocation_prepare.restype = ctypes.c_int32
        self._raw.easynet_invocation_sign_prepared.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_invocation_sign_prepared.restype = ctypes.c_int32
        self._raw.easynet_invocation_submit_signed_handle.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_uint64),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_invocation_submit_signed_handle.restype = ctypes.c_int32
        self._raw.easynet_invocation_handle_await.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_invocation_handle_await.restype = ctypes.c_int32
        self._raw.easynet_invocation_handle_cancel.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_invocation_handle_cancel.restype = ctypes.c_int32
        self._raw.easynet_invocation_handle_events.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.easynet_invocation_handle_events.restype = ctypes.c_int32
        self._raw.easynet_invocation_handle_free.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.easynet_invocation_handle_free.restype = ctypes.c_int32
        self._raw.easynet_prepared_invocation_free.argtypes = [ctypes.c_uint64]
        self._raw.easynet_prepared_invocation_free.restype = ctypes.c_int32
        self._raw.easynet_signed_invocation_free.argtypes = [ctypes.c_uint64]
        self._raw.easynet_signed_invocation_free.restype = ctypes.c_int32
        self._raw.easynet_invocation_stream_open.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            _StreamCallback,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.easynet_invocation_stream_open.restype = ctypes.c_int32
        self._raw.easynet_invocation_stream_cancel.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.easynet_invocation_stream_cancel.restype = ctypes.c_int32
        self._raw.easynet_invocation_stream_close.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.easynet_invocation_stream_close.restype = ctypes.c_int32
        self._raw.easynet_invocation_bidi_open.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            _BidiCallback,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.easynet_invocation_bidi_open.restype = ctypes.c_int32
        self._raw.easynet_invocation_bidi_send.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.c_char_p,
        ]
        self._raw.easynet_invocation_bidi_send.restype = ctypes.c_int32
        self._raw.easynet_invocation_bidi_close_send.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.easynet_invocation_bidi_close_send.restype = ctypes.c_int32
        self._raw.easynet_invocation_bidi_close.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.easynet_invocation_bidi_close.restype = ctypes.c_int32
        self._raw.easynet_invocation_bidi_cancel.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.easynet_invocation_bidi_cancel.restype = ctypes.c_int32
        for symbol in _JSON_HANDLE_OUTPUT_SYMBOLS:
            function = getattr(self._raw, symbol)
            function.argtypes = [
                ctypes.c_uint64,
                ctypes.c_char_p,
                ctypes.POINTER(ctypes.c_void_p),
            ]
            function.restype = ctypes.c_int32

    def _call_output(self, function: Any, *args: Any) -> bytes:
        out = ctypes.c_void_p()
        code = int(function(*args, ctypes.byref(out)))
        self._raise_for_code(code)
        if not out.value:
            return b""
        try:
            return ctypes.string_at(out.value)
        finally:
            self._raw.easynet_string_free(out)

    def _call_output_with_id(self, function: Any, *args: Any) -> tuple[int, bytes]:
        out_id = ctypes.c_uint64(0)
        out = ctypes.c_void_p()
        code = int(function(*args, ctypes.byref(out_id), ctypes.byref(out)))
        self._raise_for_code(code)
        if not out.value:
            return int(out_id.value), b""
        try:
            return int(out_id.value), ctypes.string_at(out.value)
        finally:
            self._raw.easynet_string_free(out)

    def _raise_for_code(self, code: int) -> None:
        if code == EASYNET_OK:
            return
        error = self._last_error_json()
        if error is not None:
            raise error
        raise SDKError(
            code=ErrorCode.GENERIC,
            stage="cabi",
            retry=RetryHint.UNKNOWN,
            retryable=retryable_for_hint(RetryHint.UNKNOWN),
            message=f"C ABI call failed with code {code}",
        )

    def _last_error_json(self) -> SDKError | None:
        out = ctypes.c_void_p()
        code = int(self._raw.easynet_last_error_json(ctypes.byref(out)))
        if code != EASYNET_OK or not out.value:
            return None
        try:
            return SDKError.from_json(ctypes.string_at(out.value))
        finally:
            self._raw.easynet_string_free(out)


@dataclass
class CABIDiscoveryTransport:
    """Feature discovery transport backed by C ABI v4."""

    lib: CLILibrary
    _closed: bool = False

    def feature_discovery(self) -> bytes:
        if self._closed:
            raise _closed_error("discovery transport is closed")
        return self.lib.feature_discovery()

    def close(self) -> None:
        self._closed = True


@dataclass
class CABIIdentityTransport:
    """Directory + Identity transport backed by C ABI v4."""

    lib: CLILibrary
    handle: int
    owns_handle: bool = False
    _closed: bool = False

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        request = _json_object(request_json, "descriptor-ref request")
        return self.lib.identity_project_descriptor_ref(
            self._require_open(), _required_string(request, "descriptor_ref").encode("utf-8")
        )

    def build_descriptor_ref(self, request_json: bytes) -> bytes:
        return self.lib.identity_build_descriptor_ref(self._require_open(), request_json)

    def project_identity(self, request_json: bytes) -> bytes:
        request = _json_object(request_json, "identity projection request")
        return self.lib.identity_project_ura(
            self._require_open(), _required_string(request, "ura").encode("utf-8")
        )

    def build_ura(self, request_json: bytes) -> bytes:
        return self.lib.identity_build_ura(self._require_open(), request_json)

    def build_resource_ref(self, request_json: bytes) -> bytes:
        return self.lib.json_handle_output(
            "easynet_publication_build_resource_ref",
            self._require_open(),
            request_json,
        )

    def build_register_signing_key_invocation(self, request_json: bytes) -> bytes:
        return self.lib.json_handle_output(
            "easynet_identity_build_register_signing_key_invocation",
            self._require_open(),
            request_json,
        )

    def build_list_signing_keys_invocation(self, request_json: bytes) -> bytes:
        return self.lib.json_handle_output(
            "easynet_identity_build_list_signing_keys_invocation",
            self._require_open(),
            request_json,
        )

    def build_revoke_signing_key_invocation(self, request_json: bytes) -> bytes:
        return self.lib.json_handle_output(
            "easynet_identity_build_revoke_signing_key_invocation",
            self._require_open(),
            request_json,
        )

    def register_signing_key(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_identity_build_register_signing_key_invocation",
            project_symbol="easynet_identity_project_signing_key_record",
            projection_keys=(
                "owner_ura",
                "key_id",
                "algorithm",
                "public_key_base64",
                "usage",
                "role",
            ),
        )

    def list_signing_keys(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_identity_build_list_signing_keys_invocation",
            project_symbol="easynet_identity_project_signing_key_page",
            projection_keys=("owner_ura", "limit", "cursor"),
        )

    def revoke_signing_key(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_identity_build_revoke_signing_key_invocation",
            project_symbol="easynet_identity_project_signing_key_revoke_result",
            projection_keys=("owner_ura", "key_id", "public_key_base64", "reason"),
        )

    def signer(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_identity_build_list_signing_keys_invocation",
            project_symbol="easynet_identity_project_signer_handle",
            projection_keys=("owner_ura", "key_id", "usage"),
        )

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self.owns_handle:
            self.lib.shutdown(self.handle)

    def _missing(self, method: str) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="cabi",
            retry=RetryHint.NEVER,
            retryable=False,
            message=f"{method} is not exposed by the C ABI identity bridge",
        )

    def _invoke_output_projected_with_request(
        self,
        request_json: bytes,
        *,
        build_symbol: str,
        project_symbol: str,
        projection_keys: tuple[str, ...],
    ) -> bytes:
        handle = self._require_open()
        draft_json = self.lib.json_handle_output(build_symbol, handle, request_json)
        output = self.lib.invocation_invoke(handle, draft_json)
        output_json = _json_object(output, "identity invocation result")
        result = output_json.get("output_json")
        if not isinstance(result, dict):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="cabi",
                retry=RetryHint.NEVER,
                retryable=False,
                message="identity invocation result is missing output_json",
            )
        projection_json = _projection_request_json(
            request_json,
            result,
            passthrough_keys=projection_keys,
        )
        return self.lib.json_handle_output(project_symbol, handle, projection_json)

    def _require_open(self) -> int:
        if self._closed:
            raise _closed_error("identity transport is closed")
        if self.handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="identity transport handle is invalid",
            )
        return self.handle


@dataclass
class _CABIProfileTransport:
    """Base for schema-backed profile carrier/projection C ABI transports."""

    lib: CLILibrary
    handle: int
    owns_handle: bool = False
    _closed: bool = False

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self.owns_handle:
            self.lib.shutdown(self.handle)

    def _call(self, symbol: str, request_json: bytes) -> bytes:
        return self.lib.json_handle_output(symbol, self._require_open(), request_json)

    def _invoke_projected(
        self,
        request_json: bytes,
        *,
        build_symbol: str,
        project_symbol: str,
    ) -> bytes:
        handle = self._require_open()
        draft_json = self.lib.json_handle_output(build_symbol, handle, request_json)
        result_json = self.lib.invocation_invoke(handle, draft_json)
        return self.lib.json_handle_output(project_symbol, handle, result_json)

    def _invoke_projected_with_controls(
        self,
        request_json: bytes,
        *,
        build_symbol: str,
        project_symbol: str,
    ) -> bytes:
        handle = self._require_open()
        output = self._invoke_output_json(handle, build_symbol, request_json)
        projection_json = _projection_request_json(request_json, output)
        return self.lib.json_handle_output(project_symbol, handle, projection_json)

    def _invoke_output_projected(
        self,
        request_json: bytes,
        *,
        build_symbol: str,
        project_symbol: str,
    ) -> bytes:
        handle = self._require_open()
        output = self._invoke_output_json(handle, build_symbol, request_json)
        return self.lib.json_handle_output(project_symbol, handle, _json_bytes(output))

    def _invoke_output_projected_with_request(
        self,
        request_json: bytes,
        *,
        build_symbol: str,
        project_symbol: str,
        projection_keys: tuple[str, ...],
    ) -> bytes:
        handle = self._require_open()
        output = self._invoke_output_json(handle, build_symbol, request_json)
        projection_json = _projection_request_json(
            request_json,
            output,
            passthrough_keys=projection_keys,
        )
        return self.lib.json_handle_output(project_symbol, handle, projection_json)

    def _invoke_output_json(
        self,
        handle: int,
        build_symbol: str,
        request_json: bytes,
    ) -> dict[str, object]:
        draft_json = self.lib.json_handle_output(build_symbol, handle, request_json)
        result_json = self.lib.invocation_invoke(handle, draft_json)
        result = _json_object(result_json, "profile invocation result")
        output = result.get("output_json")
        if not isinstance(output, dict):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="cabi",
                retry=RetryHint.NEVER,
                retryable=False,
                message="profile invocation output_json must be an object",
            )
        return output

    def _open_runtime_stream(
        self,
        request_json: bytes,
        *,
        build_symbol: str,
    ) -> StreamHandle:
        handle = self._require_open()
        draft_json = self.lib.json_handle_output(build_symbol, handle, request_json)
        runtime = CABIRuntimeTransport(self.lib, handle)
        stream_transport, open_json = runtime.open_stream(draft_json)
        return StreamHandle.from_json(stream_transport, open_json)

    def _missing(self, method: str) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="cabi",
            retry=RetryHint.NEVER,
            retryable=False,
            message=f"{method} is not exposed by the C ABI profile bridge",
        )

    def _require_open(self) -> int:
        if self._closed:
            raise _closed_error("profile transport is closed")
        if self.handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="profile transport handle is invalid",
            )
        return self.handle


@dataclass
class CABIReceiptTransport(_CABIProfileTransport):
    """Receipt carrier/projection transport backed by C ABI v4."""

    def fetch(self, request_json: bytes) -> bytes:
        return self._invoke_projected(
            request_json,
            build_symbol="easynet_receipt_build_fetch_invocation",
            project_symbol="easynet_receipt_project",
        )

    def build_fetch_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_receipt_build_fetch_invocation", request_json)

    def build_list_history_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_receipt_build_list_history_invocation", request_json
        )

    def build_get_history_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_receipt_build_get_history_invocation", request_json)

    def build_trace_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_receipt_build_trace_invocation", request_json)

    def list_history(self, request_json: bytes) -> bytes:
        return _json_bytes(
            self._invoke_output_json(
                self._require_open(),
                "easynet_receipt_build_list_history_invocation",
                request_json,
            )
        )

    def get_history(self, request_json: bytes) -> bytes:
        return _json_bytes(
            self._invoke_output_json(
                self._require_open(),
                "easynet_receipt_build_get_history_invocation",
                request_json,
            )
        )

    def get_trace(self, request_json: bytes) -> bytes:
        return _json_bytes(
            self._invoke_output_json(
                self._require_open(),
                "easynet_receipt_build_trace_invocation",
                request_json,
            )
        )

    def project(self, receipt_json: bytes) -> bytes:
        return self._call("easynet_receipt_project", receipt_json)

    def verify(self, receipt_json: bytes) -> bytes:
        return self._call("easynet_receipt_verify", receipt_json)

    def verify_chain(self, request_json: bytes) -> bytes:
        return self._call("easynet_receipt_verify_chain", request_json)

    def causal_ref(self, receipt_json: bytes) -> bytes:
        return self._call("easynet_receipt_causal_ref", receipt_json)


@dataclass
class CABIDirectoryTransport(_CABIProfileTransport):
    """Directory carrier/projection transport backed by C ABI v4."""

    _subscriptions: list[DirectorySubscription] = field(default_factory=list)

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_directory_build_subscription_invocation", request_json
        )

    def resolve(self, request_json: bytes) -> bytes:
        return self._invoke_projected(
            request_json,
            build_symbol="easynet_directory_build_resolve_invocation",
            project_symbol="easynet_directory_project_resolved_ref",
        )

    def list_devices(self, request_json: bytes) -> bytes:
        return self._invoke_projected(
            request_json,
            build_symbol="easynet_directory_build_list_devices_invocation",
            project_symbol="easynet_directory_project_device_page",
        )

    def list_agents(self, request_json: bytes) -> bytes:
        return self._invoke_projected(
            request_json,
            build_symbol="easynet_directory_build_list_agents_invocation",
            project_symbol="easynet_directory_project_agent_page",
        )

    def list_abilities(self, request_json: bytes) -> bytes:
        return self._invoke_projected(
            request_json,
            build_symbol="easynet_directory_build_list_abilities_invocation",
            project_symbol="easynet_directory_project_ability_page",
        )

    def subscribe_directory(self, request_json: bytes) -> DirectorySubscription:
        runtime_stream = self._open_runtime_stream(
            request_json,
            build_symbol="easynet_directory_build_subscription_invocation",
        )
        projection_json = self._call(
            "easynet_directory_project_subscription",
            _projection_request_json(
                request_json,
                {
                    "stream_id": runtime_stream.stream_id,
                    "state": getattr(runtime_stream.state, "value", str(runtime_stream.state)),
                    "max_buffered_events": runtime_stream.max_buffered_events,
                },
                passthrough_keys=("resume_cursor",),
            ),
        )
        subscription = DirectorySubscription.from_json(projection_json)
        object.__setattr__(subscription, "_runtime_stream", runtime_stream)
        self._subscriptions.append(subscription)
        return subscription

    def build_list_devices_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_directory_build_list_devices_invocation", request_json
        )

    def build_list_agents_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_directory_build_list_agents_invocation", request_json
        )

    def build_list_abilities_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_directory_build_list_abilities_invocation", request_json
        )

    def build_resolve_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_directory_build_resolve_invocation", request_json)

    def project_device_page(self, page_json: bytes) -> bytes:
        return self._call("easynet_directory_project_device_page", page_json)

    def project_agent_page(self, page_json: bytes) -> bytes:
        return self._call("easynet_directory_project_agent_page", page_json)

    def project_ability_page(self, page_json: bytes) -> bytes:
        return self._call("easynet_directory_project_ability_page", page_json)

    def project_resolved_ref(self, answer_json: bytes) -> bytes:
        return self._call("easynet_directory_project_resolved_ref", answer_json)

    def project_subscription(self, subscription_json: bytes) -> bytes:
        return self._call("easynet_directory_project_subscription", subscription_json)

    def close(self) -> None:
        if self._closed:
            return
        for subscription in tuple(self._subscriptions):
            subscription.close()
        self._subscriptions.clear()
        super().close()


@dataclass
class CABIPublicationTransport(_CABIProfileTransport):
    """Publication carrier/projection transport backed by C ABI v4."""

    def build_resource_ref(self, request_json: bytes) -> bytes:
        return self._call("easynet_publication_build_resource_ref", request_json)

    def validate_package(self, request_json: bytes) -> bytes:
        return self._call("easynet_publication_validate_package", request_json)

    def deploy_ability(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_publication_build_deploy_invocation",
            project_symbol="easynet_publication_project_deploy_result",
        )

    def build_deploy_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_build_deploy_invocation", request_json
        )

    def project_deploy_result(self, result_json: bytes) -> bytes:
        return self._call("easynet_publication_project_deploy_result", result_json)

    def install_plugin(self, request_json: bytes) -> bytes:
        return self._call("easynet_publication_install_plugin", request_json)

    def list_abilities(self, request_json: bytes) -> bytes:
        return self._invoke_projected_with_controls(
            request_json,
            build_symbol="easynet_publication_build_list_abilities_invocation",
            project_symbol="easynet_publication_project_ability_page",
        )

    def build_list_abilities_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_build_list_abilities_invocation", request_json
        )

    def project_ability_page(self, page_json: bytes) -> bytes:
        return self._call("easynet_publication_project_ability_page", page_json)

    def build_show_ability_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_build_show_ability_invocation", request_json
        )

    def project_ability_record(self, record_json: bytes) -> bytes:
        return self._call("easynet_publication_project_ability_record", record_json)

    def project_unpublish_result(self, result_json: bytes) -> bytes:
        return self._call("easynet_publication_project_unpublish_result", result_json)

    def build_enable_ability_impl_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_build_enable_ability_impl_invocation",
            request_json,
        )

    def project_enable_ability_impl_result(self, result_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_project_enable_ability_impl_result",
            result_json,
        )

    def build_disable_ability_impl_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_build_disable_ability_impl_invocation",
            request_json,
        )

    def project_disable_ability_impl_result(self, result_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_project_disable_ability_impl_result",
            result_json,
        )

    def show_ability(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_publication_build_show_ability_invocation",
            project_symbol="easynet_publication_project_ability_record",
            projection_keys=("descriptor_ref",),
        )

    def enable_ability_impl(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_publication_build_enable_ability_impl_invocation",
            project_symbol="easynet_publication_project_enable_ability_impl_result",
            projection_keys=("impl_id", "ability_ura", "metadata"),
        )

    def disable_ability_impl(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_publication_build_disable_ability_impl_invocation",
            project_symbol="easynet_publication_project_disable_ability_impl_result",
            projection_keys=("impl_id", "ability_ura", "metadata"),
        )

    def build_unpublish_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_publication_build_unpublish_invocation", request_json
        )

    def unpublish_ability(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_publication_build_unpublish_invocation",
            project_symbol="easynet_publication_project_unpublish_result",
            projection_keys=("descriptor_version", "ability_ura"),
        )

@dataclass
class CABIHostBindingTransport(_CABIProfileTransport):
    """Host Binding codec/hash transport backed by C ABI v4."""

    def build_host_stream_binding(self, request_json: bytes) -> bytes:
        return self._call("easynet_host_binding_build", request_json)

    def decode_request(self, envelope_json: bytes) -> bytes:
        return self._call("easynet_host_binding_decode_request", envelope_json)

    def encode_item(self, request_json: bytes) -> bytes:
        return self._call("easynet_host_binding_encode_item", request_json)

    def encode_error(self, request_json: bytes) -> bytes:
        return self._call("easynet_host_binding_encode_error", request_json)

    def encode_terminal(self, request_json: bytes) -> bytes:
        return self._call("easynet_host_binding_encode_terminal", request_json)

    def fold_output_hash(self, request_json: bytes) -> bytes:
        return self._call("easynet_host_binding_fold_output_hash", request_json)


@dataclass
class CABIMissionTransport(_CABIProfileTransport):
    """Mission carrier/projection transport backed by C ABI v4."""

    def build_run_eal_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_mission_build_run_eal_invocation", request_json)

    def build_run_file_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_mission_build_run_file_invocation", request_json)

    def build_track_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_mission_build_track_invocation", request_json)

    def build_cancel_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_mission_build_cancel_invocation", request_json)

    def build_events_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_mission_build_events_invocation", request_json)

    def run_eal(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_mission_build_run_eal_invocation",
            project_symbol="easynet_mission_project_status",
        )

    def run_file(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_mission_build_run_file_invocation",
            project_symbol="easynet_mission_project_status",
        )

    def track(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_mission_build_track_invocation",
            project_symbol="easynet_mission_project_status",
        )

    def cancel(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_mission_build_cancel_invocation",
            project_symbol="easynet_mission_project_status",
        )

    def events(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_mission_build_events_invocation",
            project_symbol="easynet_mission_project_events",
            projection_keys=("mission_id", "cursor_sequence"),
        )

    def open_event_stream(self, request_json: bytes) -> StreamHandle:
        return self._open_runtime_stream(
            request_json,
            build_symbol="easynet_mission_build_events_invocation",
        )

    def project_status(self, status_json: bytes) -> bytes:
        return self._call("easynet_mission_project_status", status_json)

    def project_events(self, events_json: bytes) -> bytes:
        return self._call("easynet_mission_project_events", events_json)


@dataclass
class CABIAdminTransport(_CABIProfileTransport):
    """Admin + Gateway carrier/projection transport backed by C ABI v4."""

    daemon_handle: int = 0
    owns_daemon_handle: bool = False

    def close(self) -> None:
        if self._closed:
            return
        daemon_handle = self.daemon_handle
        owns_daemon_handle = self.owns_daemon_handle
        super().close()
        if owns_daemon_handle and daemon_handle > 0:
            self.lib.daemon_detach(daemon_handle)

    def build_agent_list_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_admin_build_agent_list_invocation", request_json)

    def build_agent_start_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_admin_build_agent_start_invocation", request_json)

    def build_agent_stop_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_admin_build_agent_stop_invocation", request_json)

    def build_agent_refresh_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_admin_build_agent_refresh_invocation", request_json
        )

    def build_session_list_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_admin_build_session_list_invocation", request_json)

    def build_session_create_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_admin_build_session_create_invocation", request_json)

    def build_session_delete_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_admin_build_session_delete_invocation", request_json)

    def build_revoke_device_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_admin_build_revoke_device_invocation", request_json)

    def gateway_status(self, request_json: bytes) -> bytes:
        if self.daemon_handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                retryable=False,
                message="admin gateway status requires a daemon lifecycle handle",
            )
        status = _json_object(
            self.lib.daemon_status(self.daemon_handle), "daemon status"
        )
        request = _json_object(request_json, "admin gateway status request")
        return self._call(
            "easynet_admin_project_gateway_status",
            _admin_gateway_status_projection_input(status, request),
        )

    def list_agents(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_admin_build_agent_list_invocation",
            project_symbol="easynet_admin_project_agent_records",
        )

    def agent_start(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_admin_build_agent_start_invocation",
            project_symbol="easynet_admin_project_agent_lifecycle_result",
        )

    def agent_stop(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_admin_build_agent_stop_invocation",
            project_symbol="easynet_admin_project_agent_lifecycle_result",
        )

    def agent_refresh(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_admin_build_agent_refresh_invocation",
            project_symbol="easynet_admin_project_agent_lifecycle_result",
        )

    def list_device_sessions(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_admin_build_session_list_invocation",
            project_symbol="easynet_admin_project_device_session_page",
        )

    def join_hub(self, request_json: bytes) -> bytes:
        return self._invoke_hub_lifecycle(
            request_json,
            build_symbol="easynet_admin_build_hub_join_invocation",
            operation="hub.join",
            projection_keys=("hub_ura", "device_ura"),
        )

    def leave_hub(self, request_json: bytes) -> bytes:
        return self._invoke_hub_lifecycle(
            request_json,
            build_symbol="easynet_admin_build_hub_leave_invocation",
            operation="hub.leave",
            projection_keys=("hub_ura",),
        )

    def pairing_preflight(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_admin_build_pairing_preflight_invocation",
            project_symbol="easynet_admin_project_pairing_preflight",
            projection_keys=("hub_ura", "device_ura"),
        )

    def validate_pairing(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_admin_build_pairing_validate_invocation",
            project_symbol="easynet_admin_project_device_credential",
            projection_keys=("device_ura",),
        )

    def verify_device_credential(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_admin_build_credential_verify_invocation",
            project_symbol="easynet_admin_project_device_credential_verification",
            projection_keys=("credential_id", "device_ura", "hub_ura"),
        )

    def create_pairing(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_admin_build_pairing_create_invocation",
            project_symbol="easynet_admin_project_pairing_token",
            projection_keys=("hub_ura", "device_ura", "expires_unix_ms"),
        )

    def revoke_device(self, request_json: bytes) -> bytes:
        request = _json_object(request_json, "admin revoke-device request")
        output = self._invoke_output_json(
            self._require_open(),
            "easynet_admin_build_revoke_device_invocation",
            request_json,
        )
        return self._call(
            "easynet_admin_project_device_admin_result",
            _json_bytes(
                {
                    "operation": "federation.revoke",
                    "device_ura": request.get("device_ura"),
                    "result": output,
                }
            ),
        )

    def create_device_session(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_admin_build_session_create_invocation",
            project_symbol="easynet_admin_project_device_session_result",
            projection_keys=(
                "device_ura",
                "hub_ura",
                "session_kind",
                "expires_unix_ms",
            ),
        )

    def delete_device_session(self, request_json: bytes) -> bytes:
        output = self._invoke_output_json(
            self._require_open(),
            "easynet_admin_build_session_delete_invocation",
            request_json,
        )
        return self._call(
            "easynet_admin_project_device_admin_result",
            _json_bytes({"operation": "session.delete", "result": output}),
        )

    def project_gateway_status(self, status_json: bytes) -> bytes:
        return self._call("easynet_admin_project_gateway_status", status_json)

    def project_agent_records(self, agents_json: bytes) -> bytes:
        return self._call("easynet_admin_project_agent_records", agents_json)

    def project_agent_lifecycle_result(self, result_json: bytes) -> bytes:
        return self._call(
            "easynet_admin_project_agent_lifecycle_result", result_json
        )

    def project_hub_lifecycle_result(self, result_json: bytes) -> bytes:
        return self._call("easynet_admin_project_hub_lifecycle_result", result_json)

    def project_pairing_preflight(self, result_json: bytes) -> bytes:
        return self._call("easynet_admin_project_pairing_preflight", result_json)

    def project_pairing_token(self, result_json: bytes) -> bytes:
        return self._call("easynet_admin_project_pairing_token", result_json)

    def project_device_credential(self, result_json: bytes) -> bytes:
        return self._call("easynet_admin_project_device_credential", result_json)

    def project_device_credential_verification(self, result_json: bytes) -> bytes:
        return self._call(
            "easynet_admin_project_device_credential_verification", result_json
        )

    def project_device_session_page(self, sessions_json: bytes) -> bytes:
        return self._call("easynet_admin_project_device_session_page", sessions_json)

    def project_device_session_result(self, session_json: bytes) -> bytes:
        return self._call("easynet_admin_project_device_session_result", session_json)

    def project_device_admin_result(self, result_json: bytes) -> bytes:
        return self._call("easynet_admin_project_device_admin_result", result_json)

    def _invoke_hub_lifecycle(
        self,
        request_json: bytes,
        *,
        build_symbol: str,
        operation: str,
        projection_keys: tuple[str, ...],
    ) -> bytes:
        handle = self._require_open()
        output = self._invoke_output_json(handle, build_symbol, request_json)
        projection = _json_object(request_json, "admin hub lifecycle request")
        selected = {
            key: projection[key]
            for key in projection_keys
            if key in projection and projection[key] is not None
        }
        selected["operation"] = operation
        selected["result"] = output
        return self.lib.json_handle_output(
            "easynet_admin_project_hub_lifecycle_result",
            handle,
            _json_bytes(selected),
        )

@dataclass
class CABIEventTransport(_CABIProfileTransport):
    """Events carrier/projection transport backed by C ABI v4."""

    _event_streams: list[EventStream] = field(default_factory=list)

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_events_build_directory_subscription_invocation", request_json
        )

    def build_device_subscription_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_events_build_device_subscription_invocation", request_json
        )

    def build_session_subscription_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_events_build_session_subscription_invocation", request_json
        )

    def build_invocation_subscription_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_events_build_invocation_subscription_invocation", request_json
        )

    def subscribe_directory(self, request_json: bytes) -> EventStream:
        runtime_stream = self._open_runtime_stream(
            request_json,
            build_symbol="easynet_events_build_directory_subscription_invocation",
        )
        stream = EventStream.from_runtime_stream(
            "directory",
            runtime_stream,
            resume_token=_resume_token_from_event_subscription(request_json),
            metadata={
                "profile": "events",
                "source": "runtime_stream",
                "stream_ability": "federation.subscribe_directory_v2",
                "carrier_owner": "daemon_sdk",
            },
        )
        self._event_streams.append(stream)
        return stream

    def subscribe_devices(self, request_json: bytes) -> EventStream:
        runtime_stream = self._open_runtime_stream(
            request_json,
            build_symbol="easynet_events_build_device_subscription_invocation",
        )
        stream = EventStream.from_runtime_stream(
            "device",
            runtime_stream,
            resume_token=_resume_token_from_event_subscription(request_json),
            metadata={
                "profile": "events",
                "source": "runtime_stream",
                "stream_ability": "events.device.subscribe",
                "carrier_owner": "daemon_sdk",
            },
        )
        self._event_streams.append(stream)
        return stream

    def subscribe_sessions(self, request_json: bytes) -> bytes:
        runtime_stream = self._open_runtime_stream(
            request_json,
            build_symbol="easynet_events_build_session_subscription_invocation",
        )
        stream = EventStream.from_runtime_stream(
            "session",
            runtime_stream,
            resume_token=_resume_token_from_event_subscription(request_json),
            metadata={
                "profile": "events",
                "source": "runtime_stream",
                "stream_ability": "session.attach",
                "carrier_owner": "daemon_sdk",
            },
        )
        self._event_streams.append(stream)
        return stream

    def subscribe_invocations(self, request_json: bytes) -> EventStream:
        runtime_stream = self._open_runtime_stream(
            request_json,
            build_symbol="easynet_events_build_invocation_subscription_invocation",
        )
        stream = EventStream.from_runtime_stream(
            "invocation",
            runtime_stream,
            resume_token=_resume_token_from_event_subscription(request_json),
            metadata={
                "profile": "events",
                "source": "runtime_stream",
                "stream_ability": "events.invocation.subscribe",
                "carrier_owner": "daemon_sdk",
            },
        )
        self._event_streams.append(stream)
        return stream

    def list_device_events(self, request_json: bytes) -> bytes:
        return self._invoke_projected_with_controls(
            request_json,
            build_symbol="easynet_events_build_device_event_history_invocation",
            project_symbol="easynet_events_project_device_event_page",
        )

    def project_directory_event(self, event_json: bytes) -> bytes:
        return self._call("easynet_events_project_directory_event", event_json)

    def project_drop_report(self, drop_json: bytes) -> bytes:
        return self._call("easynet_events_project_drop_report", drop_json)

    def project_terminal(self, terminal_json: bytes) -> bytes:
        return self._call("easynet_events_project_terminal", terminal_json)

    def close(self) -> None:
        if self._closed:
            return
        for stream in tuple(self._event_streams):
            stream.close()
        self._event_streams.clear()
        super().close()


@dataclass
class CABISurfaceTransport(_CABIProfileTransport):
    """Surface carrier/projection transport backed by C ABI v4."""

    def build_list_pages_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_surface_build_list_pages_invocation", request_json)

    def build_create_page_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_surface_build_create_page_invocation", request_json)

    def build_delete_page_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_surface_build_delete_page_invocation", request_json)

    def build_manifest_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_surface_build_manifest_invocation", request_json)

    def build_health_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_surface_build_health_invocation", request_json)

    def list_pages(self, request_json: bytes) -> bytes:
        return self._invoke_projected_with_controls(
            request_json,
            build_symbol="easynet_surface_build_list_pages_invocation",
            project_symbol="easynet_surface_project_page_page",
        )

    def create_page(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_surface_build_create_page_invocation",
            project_symbol="easynet_surface_project_page_record",
        )

    def delete_page(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_surface_build_delete_page_invocation",
            project_symbol="easynet_surface_project_mutation_result",
        )

    def surface_manifest(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_surface_build_manifest_invocation",
            project_symbol="easynet_surface_project_manifest",
        )

    def public_page_ref(self, request_json: bytes) -> bytes:
        return self._call("easynet_surface_project_public_page_ref", request_json)

    def surface_health(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected_with_request(
            request_json,
            build_symbol="easynet_surface_build_health_invocation",
            project_symbol="easynet_surface_project_health",
            projection_keys=(
                "callee_ura",
                "descriptor_version",
                "project_id",
                "surface_ref",
            ),
        )

    def project_page_record(self, page_json: bytes) -> bytes:
        return self._call("easynet_surface_project_page_record", page_json)

    def project_page_page(self, pages_json: bytes) -> bytes:
        return self._call("easynet_surface_project_page_page", pages_json)

    def project_manifest(self, page_json: bytes) -> bytes:
        return self._call("easynet_surface_project_manifest", page_json)

    def project_public_page_ref(self, page_json: bytes) -> bytes:
        return self._call("easynet_surface_project_public_page_ref", page_json)

    def project_mutation_result(self, result_json: bytes) -> bytes:
        return self._call("easynet_surface_project_mutation_result", result_json)


@dataclass
class CABICompatibilityTransport(_CABIProfileTransport):
    """Compatibility carrier/projection transport backed by C ABI v4."""

    def build_list_models_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_build_list_models_invocation", request_json
        )

    def build_chat_completion_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_build_chat_completion_invocation", request_json
        )

    def build_stream_chat_completion_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_build_stream_chat_completion_invocation",
            request_json,
        )

    def build_file_upload_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_build_file_upload_invocation", request_json
        )

    def build_file_retrieve_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_build_file_retrieve_invocation", request_json
        )

    def build_file_delete_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_build_file_delete_invocation", request_json
        )

    def list_models(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_compatibility_build_list_models_invocation",
            project_symbol="easynet_compatibility_project_model_page",
        )

    def create_chat_completion(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_compatibility_build_chat_completion_invocation",
            project_symbol="easynet_compatibility_project_chat_completion",
        )

    def stream_chat_completion(self, request_json: bytes) -> bytes:
        stream = self._open_runtime_stream(
            request_json,
            build_symbol="easynet_compatibility_build_stream_chat_completion_invocation",
        )
        failed = False
        try:
            chunks: list[object] = []
            while True:
                event = stream.next(getattr(self, "stream_recv_timeout", None))
                if event.error is not None:
                    raise _profile_stream_protocol_error(
                        "compatibility stream event carried an error", event.error
                    )
                if event.terminal:
                    break
                if event.payload_json is None:
                    raise _profile_stream_protocol_error(
                        "compatibility stream chunk is missing payload_json",
                        {"sequence": event.sequence, "kind": event.kind},
                    )
                chunks.append(event.payload_json)
            return self.project_chat_stream(
                _json_bytes(
                    {
                        "stream": True,
                        "chunks": chunks,
                        "done_sentinel": "[DONE]",
                    }
                )
            )
        except BaseException:
            failed = True
            raise
        finally:
            try:
                stream.close()
            except SDKError:
                if not failed:
                    raise

    def upload_file(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_compatibility_build_file_upload_invocation",
            project_symbol="easynet_compatibility_project_file_upload",
        )

    def retrieve_file(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_compatibility_build_file_retrieve_invocation",
            project_symbol="easynet_compatibility_project_file",
        )

    def delete_file(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_compatibility_build_file_delete_invocation",
            project_symbol="easynet_compatibility_project_file_delete_result",
        )

    def project_model_page(self, models_json: bytes) -> bytes:
        return self._call("easynet_compatibility_project_model_page", models_json)

    def project_chat_completion(self, completion_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_project_chat_completion", completion_json
        )

    def project_chat_stream(self, stream_json: bytes) -> bytes:
        return self._call("easynet_compatibility_project_chat_stream", stream_json)

    def project_file_upload(self, file_json: bytes) -> bytes:
        return self._call("easynet_compatibility_project_file_upload", file_json)

    def project_file(self, file_json: bytes) -> bytes:
        return self._call("easynet_compatibility_project_file", file_json)

    def project_file_delete_result(self, result_json: bytes) -> bytes:
        return self._call(
            "easynet_compatibility_project_file_delete_result", result_json
        )


@dataclass
class CABIWrapperTransport(_CABIProfileTransport):
    """Convenience wrapper carrier/projection transport backed by C ABI v4."""

    def build_file_transfer_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_wrappers_build_file_transfer_invocation", request_json)

    def build_terminal_session_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_wrappers_build_terminal_session_invocation", request_json
        )

    def build_remote_desktop_session_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_wrappers_build_remote_desktop_session_invocation", request_json
        )

    def build_browser_session_invocation(self, request_json: bytes) -> bytes:
        return self._call(
            "easynet_wrappers_build_browser_session_invocation", request_json
        )

    def build_media_session_invocation(self, request_json: bytes) -> bytes:
        return self._call("easynet_wrappers_build_media_session_invocation", request_json)

    def transfer_file(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_wrappers_build_file_transfer_invocation",
            project_symbol="easynet_wrappers_project_file_record",
        )

    def start_terminal_session(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_wrappers_build_terminal_session_invocation",
            project_symbol="easynet_wrappers_project_terminal_session",
        )

    def start_remote_desktop_session(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_wrappers_build_remote_desktop_session_invocation",
            project_symbol="easynet_wrappers_project_remote_desktop_session",
        )

    def start_browser_session(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_wrappers_build_browser_session_invocation",
            project_symbol="easynet_wrappers_project_browser_session",
        )

    def start_media_session(self, request_json: bytes) -> bytes:
        return self._invoke_output_projected(
            request_json,
            build_symbol="easynet_wrappers_build_media_session_invocation",
            project_symbol="easynet_wrappers_project_media_session",
        )

    def project_file_record(self, file_json: bytes) -> bytes:
        return self._call("easynet_wrappers_project_file_record", file_json)

    def project_terminal_session(self, session_json: bytes) -> bytes:
        return self._call("easynet_wrappers_project_terminal_session", session_json)

    def project_remote_desktop_session(self, session_json: bytes) -> bytes:
        return self._call(
            "easynet_wrappers_project_remote_desktop_session", session_json
        )

    def project_browser_session(self, session_json: bytes) -> bytes:
        return self._call("easynet_wrappers_project_browser_session", session_json)

    def project_media_session(self, session_json: bytes) -> bytes:
        return self._call("easynet_wrappers_project_media_session", session_json)


@dataclass
class CABIDaemonTransport:
    """Daemon lifecycle transport backed by C ABI v4."""

    lib: CLILibrary
    _handles: dict[str, int] = field(default_factory=dict)
    _status_cache: dict[str, dict[str, object]] = field(default_factory=dict)
    _closed: bool = False

    def discover(self, options_json: bytes) -> bytes:
        self._require_open()
        raw = self.lib.daemon_discover(options_json)
        status = _daemon_status_from_cabi("0", raw)
        return _json_bytes(status["endpoints"])

    def start(self, config_json: bytes) -> bytes:
        self._require_open()
        daemon_handle = self.lib.daemon_start(_daemon_start_config_for_cabi(config_json))
        public_id = str(daemon_handle)
        self._handles[public_id] = daemon_handle
        status = _daemon_status_from_cabi(public_id, self.lib.daemon_status(daemon_handle))
        self._status_cache[public_id] = status
        return _json_bytes(status)

    def attach(self, options_json: bytes) -> bytes:
        self._require_open()
        daemon_handle = self.lib.daemon_attach(options_json)
        public_id = str(daemon_handle)
        self._handles[public_id] = daemon_handle
        status = _daemon_status_from_cabi(public_id, self.lib.daemon_status(daemon_handle))
        self._status_cache[public_id] = status
        return _json_bytes(status)

    def status(self, handle_id: str) -> bytes:
        daemon_handle = self._require_daemon_handle(handle_id)
        status = _daemon_status_from_cabi(handle_id, self.lib.daemon_status(daemon_handle))
        self._status_cache[handle_id] = status
        return _json_bytes(status)

    def invocation_endpoint(self, handle_id: str) -> str:
        daemon_handle = self._require_daemon_handle(handle_id)
        endpoint = self.lib.daemon_invocation_endpoint(daemon_handle)
        status = dict(self._status_cache.get(handle_id, {}))
        cached_endpoints = status.get("endpoints", {})
        endpoints = dict(cached_endpoints) if isinstance(cached_endpoints, dict) else {}
        endpoints["invocation_endpoint"] = endpoint
        status["endpoints"] = endpoints
        self._status_cache[handle_id] = status
        return endpoint

    def open_runtime(
        self, handle_id: str, options_json: bytes
    ) -> tuple["CABIRuntimeTransport", bytes]:
        _ = options_json
        runtime_handle = self._open_client_handle(handle_id, "runtime")
        return (
            CABIRuntimeTransport(
                lib=self.lib,
                handle=runtime_handle,
                owns_handle=True,
            ),
            _json_bytes({"ready": True, "abi_version": EXPECTED_ABI_VERSION}),
        )

    def open_runtime_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return CABIRuntimeTransport(
            self.lib,
            self._open_client_handle(handle_id, "runtime"),
            owns_handle=True,
        )

    def open_profile(self, handle_id: str, profile: str, options_json: bytes) -> object:
        openers = {
            "runtime": self.open_runtime_transport,
            "directory": self.open_directory_transport,
            "identity": self.open_identity_transport,
            "receipt": self.open_receipt_transport,
            "publication": self.open_publication_transport,
            "host_binding": self.open_host_binding_transport,
            "mission": self.open_mission_transport,
            "admin": self.open_admin_transport,
            "events": self.open_events_transport,
            "surface": self.open_surface_transport,
            "compatibility": self.open_compatibility_transport,
            "wrapper": self.open_wrapper_transport,
        }
        opener = openers.get(profile)
        if opener is None:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="cabi",
                retry=RetryHint.NEVER,
                retryable=False,
                message=f"unsupported daemon profile: {profile}",
            )
        return opener(handle_id, options_json)

    def open_directory_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(handle_id, "directory", CABIDirectoryTransport)

    def open_identity_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return CABIIdentityTransport(
            self.lib,
            self._open_client_handle(handle_id, "identity"),
            owns_handle=True,
        )

    def open_receipt_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(handle_id, "receipt", CABIReceiptTransport)

    def open_publication_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(
            handle_id, "publication", CABIPublicationTransport
        )

    def open_host_binding_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(
            handle_id, "host_binding", CABIHostBindingTransport
        )

    def open_mission_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(handle_id, "mission", CABIMissionTransport)

    def open_admin_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        daemon_handle = self._require_daemon_handle(handle_id)
        return CABIAdminTransport(
            self.lib,
            self._open_client_handle(handle_id, "admin"),
            owns_handle=True,
            daemon_handle=daemon_handle,
            owns_daemon_handle=False,
        )

    def open_events_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(handle_id, "events", CABIEventTransport)

    def open_surface_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(handle_id, "surface", CABISurfaceTransport)

    def open_compatibility_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(
            handle_id, "compatibility", CABICompatibilityTransport
        )

    def open_wrapper_transport(self, handle_id: str, options_json: bytes) -> object:
        _ = options_json
        return self._open_profile_transport(handle_id, "wrapper", CABIWrapperTransport)

    def _open_profile_transport(
        self,
        handle_id: str,
        profile: str,
        transport_type: type[_CABIProfileTransport],
    ) -> _CABIProfileTransport:
        return transport_type(
            self.lib,
            self._open_client_handle(handle_id, profile),
            owns_handle=True,
        )

    def _open_client_handle(self, handle_id: str, profile: str) -> int:
        daemon_handle = self._require_daemon_handle(handle_id)
        client_handle = self.lib.daemon_open_client(daemon_handle)
        if client_handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message=f"C ABI daemon open {profile} returned an invalid client handle",
            )
        return client_handle

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        _ = options_json
        daemon_handle = self._require_daemon_handle(handle_id)
        self.lib.daemon_stop(daemon_handle)
        self._handles.pop(handle_id, None)
        prior = self._status_cache.pop(handle_id, {})
        stopped = {
            "handle_id": handle_id,
            "state": "Stopped",
            "mode": prior.get("mode", ""),
            "endpoints": prior.get("endpoints", {}),
            "diagnostics": [],
        }
        return _json_bytes(stopped)

    def detach(self, handle_id: str) -> None:
        daemon_handle = self._require_daemon_handle(handle_id)
        self.lib.daemon_detach(daemon_handle)
        self._handles.pop(handle_id, None)
        self._status_cache.pop(handle_id, None)

    def _require_daemon_handle(self, handle_id: str) -> int:
        self._require_open()
        daemon_handle = self._handles.get(handle_id)
        if daemon_handle is None or daemon_handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="daemon handle is not owned by this transport",
            )
        return daemon_handle

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        first_error: SDKError | None = None
        for handle_id, daemon_handle in list(self._handles.items()):
            try:
                self.lib.daemon_detach(daemon_handle)
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
            finally:
                self._handles.pop(handle_id, None)
                self._status_cache.pop(handle_id, None)
        if first_error is not None:
            raise first_error

    def _require_open(self) -> None:
        if self._closed:
            raise _closed_error("daemon transport is closed")


@dataclass
class CABIRuntimeTransport:
    """Runtime Core and Health transport backed by C ABI v4."""

    lib: CLILibrary
    handle: int
    owns_handle: bool = False
    _prepared_ids: dict[str, int] = field(default_factory=dict)
    _streams: dict[int, "_CABIStreamTransport"] = field(default_factory=dict)
    _bidis: dict[int, "_CABIBidiTransport"] = field(default_factory=dict)
    _closed: bool = False

    def runtime_health(self) -> bytes:
        return self.lib.runtime_health(self._require_open())

    def runtime_diagnostics(self) -> bytes:
        return self.lib.runtime_diagnostics(self._require_open())

    def invoke(self, draft_json: bytes) -> bytes:
        return self.lib.invocation_invoke(self._require_open(), draft_json)

    def open_stream(self, draft_json: bytes) -> tuple[Any, bytes]:
        inbox = _CallbackInbox(MAX_CABI_CALLBACK_QUEUE)
        token = _register_callback_inbox(inbox)
        try:
            stream_id = self.lib.invocation_stream_open(
                self._require_open(), draft_json, token
            )
            if stream_id <= 0:
                raise SDKError(
                    code=ErrorCode.INVALID_HANDLE,
                    stage="cabi",
                    retry=RetryHint.NEVER,
                    message="C ABI stream open returned an invalid stream id",
                )
            transport = _CABIStreamTransport(
                owner=self, stream_id=stream_id, callback_token=token, inbox=inbox
            )
            self._streams[stream_id] = transport
            return transport, _json_bytes(
                {
                    "stream_id": str(stream_id),
                    "state": "Open",
                    "max_buffered_events": MAX_CABI_CALLBACK_QUEUE,
                }
            )
        except Exception:
            _release_callback_inbox(token)
            raise

    def open_bidi(self, draft_json: bytes, streams_json: bytes) -> tuple[Any, bytes]:
        invocation_json = _merge_bidi_streams(draft_json, streams_json)
        inbox = _CallbackInbox(MAX_CABI_CALLBACK_QUEUE)
        token = _register_callback_inbox(inbox)
        try:
            bidi_id = self.lib.invocation_bidi_open(
                self._require_open(), invocation_json, token
            )
            if bidi_id <= 0:
                raise SDKError(
                    code=ErrorCode.INVALID_HANDLE,
                    stage="cabi",
                    retry=RetryHint.NEVER,
                    message="C ABI bidi open returned an invalid session id",
                )
            transport = _CABIBidiTransport(
                owner=self, bidi_id=bidi_id, callback_token=token, inbox=inbox
            )
            self._bidis[bidi_id] = transport
            return transport, _json_bytes(
                {
                    "session_id": str(bidi_id),
                    "state": "Open",
                    "max_buffered_frames": MAX_CABI_CALLBACK_QUEUE,
                }
            )
        except Exception:
            _release_callback_inbox(token)
            raise

    def _remove_stream(self, stream_id: int, callback_token: int) -> None:
        self._streams.pop(stream_id, None)
        _release_callback_inbox(callback_token)

    def _remove_bidi(self, bidi_id: int, callback_token: int) -> None:
        self._bidis.pop(bidi_id, None)
        _release_callback_inbox(callback_token)

    def _handle_if_open(self) -> int:
        return self._require_open()

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        prepared_c_id, raw = self.lib.invocation_prepare(
            self._require_open(), draft_json, options_json
        )
        key = _prepared_key(_json_object(raw, "prepared invocation"))
        if prepared_c_id <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="C ABI prepare returned an invalid prepared handle",
            )
        if key in self._prepared_ids:
            self.lib.prepared_invocation_free(prepared_c_id)
            raise SDKError(
                code=ErrorCode.PROTOCOL,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="C ABI prepare returned a duplicate prepared request id",
            )
        self._prepared_ids[key] = prepared_c_id
        return raw

    def submit_signed(self, signed_json: bytes) -> bytes:
        signed = _json_object(signed_json, "signed invocation")
        prepared = _required_object(signed, "prepared")
        key = _prepared_key(prepared)
        prepared_c_id = self._prepared_ids.pop(key, None)
        if prepared_c_id is None:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="prepared invocation handle is not owned by this transport",
            )
        signature_json = json.dumps(
            _required_object(signed, "signature"),
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
        signed_c_id, _ = self.lib.invocation_sign_prepared(prepared_c_id, signature_json)
        if signed_c_id <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="C ABI sign returned an invalid signed handle",
            )
        try:
            _, submitted_json = self.lib.invocation_submit_signed_handle(
                self._require_open(), signed_c_id
            )
        except Exception:
            try:
                self.lib.signed_invocation_free(signed_c_id)
            except SDKError:
                pass
            raise
        return submitted_json

    def await_handle(self, handle_id: int) -> bytes:
        return self.lib.invocation_handle_await(self._require_open(), handle_id)

    def cancel_handle(self, handle_id: int, reason: str) -> bytes:
        return self.lib.invocation_handle_cancel(self._require_open(), handle_id, reason)

    def handle_events(self, handle_id: int) -> bytes:
        return self.lib.invocation_handle_events(self._require_open(), handle_id)

    def free_handle(self, handle_id: int) -> None:
        self.lib.invocation_handle_free(self._require_open(), handle_id)

    def close(self) -> None:
        if self._closed:
            return
        first_error: SDKError | None = None
        for stream in tuple(self._streams.values()):
            try:
                stream.close()
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
        for bidi in tuple(self._bidis.values()):
            try:
                bidi.close()
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
        for prepared_id in tuple(self._prepared_ids.values()):
            try:
                self.lib.prepared_invocation_free(prepared_id)
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
        self._prepared_ids.clear()
        self._closed = True
        if self.owns_handle:
            try:
                self.lib.shutdown(self.handle)
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
        if first_error is not None:
            raise first_error

    def _require_open(self) -> int:
        if self._closed:
            raise _closed_error("runtime transport is closed")
        if self.handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="runtime transport handle is invalid",
            )
        return self.handle


@dataclass
class CABIRuntimeConnector:
    """RuntimeConnection connector backed by C ABI daemon lifecycle calls."""

    lib: CLILibrary
    _daemon: CABIDaemonTransport = field(init=False)
    _runtime: CABIRuntimeTransport | None = None
    _closed: bool = False

    def __post_init__(self) -> None:
        self._daemon = CABIDaemonTransport(self.lib)

    def resolve(self, options_json: bytes) -> bytes:
        self._require_open()
        options = _json_object(options_json or b"{}", "runtime connect options")
        control_path = _optional_json_string(options, "control_path")
        endpoints = _json_object(
            self._daemon.discover(_json_bytes({"control_path": control_path})),
            "daemon endpoints",
        )
        endpoint = _optional_json_string(options, "endpoint") or _optional_json_string(
            endpoints, "invocation_endpoint"
        )
        if not endpoint:
            raise SDKError(
                code=ErrorCode.CONTROL_ONLY,
                stage="cabi",
                retry=RetryHint.SAFE,
                message="daemon discovery did not advertise invocation_endpoint",
            )
        return _json_bytes(
            {
                "endpoint": endpoint,
                "control_path": control_path,
                "control_endpoint": _optional_json_string(
                    endpoints, "control_endpoint"
                ),
                "protocol_version": "cabi-v4",
                "abi_version": EXPECTED_ABI_VERSION,
            }
        )

    def handshake(self, endpoint_json: bytes) -> tuple[CABIRuntimeTransport, bytes]:
        self._require_open()
        endpoint = _json_object(endpoint_json, "runtime endpoint")
        invocation_endpoint = _required_json_string(endpoint, "endpoint")
        control_path = _optional_json_string(endpoint, "control_path")
        control_endpoint = _optional_json_string(endpoint, "control_endpoint")
        status_raw = self._daemon.attach(
            _json_bytes(
                {
                    "control_endpoint": control_endpoint,
                    "invocation_endpoint": invocation_endpoint,
                    "control_path": control_path,
                }
            )
        )
        status = _json_object(status_raw, "daemon status")
        handle_id = _required_json_string(status, "handle_id")
        try:
            runtime, _ = self._daemon.open_runtime(
                handle_id,
                _json_bytes(
                    {
                        "endpoint": invocation_endpoint,
                        "control_path": control_path,
                    }
                ),
            )
        except BaseException:
            self._daemon.detach(handle_id)
            raise
        try:
            self._daemon.detach(handle_id)
        except BaseException:
            runtime.close()
            raise
        self._runtime = runtime
        return runtime, _json_bytes(
            {
                "ready": True,
                "transport": "c_abi",
                "endpoint": invocation_endpoint,
                "abi_version": EXPECTED_ABI_VERSION,
            }
        )

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        first_error: SDKError | None = None
        if self._runtime is not None:
            try:
                self._runtime.close()
            except SDKError as exc:
                first_error = exc
            finally:
                self._runtime = None
        try:
            self._daemon.close()
        except SDKError as exc:
            if first_error is None:
                first_error = exc
        if first_error is not None:
            raise first_error

    def _require_open(self) -> None:
        if self._closed:
            raise _closed_error("runtime connector is closed")


@dataclass
class _CABIStreamTransport:
    owner: CABIRuntimeTransport
    stream_id: int
    callback_token: int
    inbox: "_CallbackInbox"
    _terminal_action_done: bool = False

    def recv(self, timeout: float | None = None) -> bytes:
        if self._terminal_action_done:
            raise _closed_error("stream transport is closed")
        return self.inbox.recv(timeout)

    def cancel(self, reason: str) -> bytes:
        if not self._terminal_action_done:
            self.owner.lib.invocation_stream_cancel(
                self.owner._handle_if_open(), self.stream_id
            )
            self._terminal_action_done = True
            self.owner._remove_stream(self.stream_id, self.callback_token)
        return _json_bytes(
            {
                "stream_id": str(self.stream_id),
                "cancelled": True,
                "state": "Cancelled",
                "terminal": True,
            }
        )

    def close(self) -> None:
        if self._terminal_action_done:
            return
        self.owner.lib.invocation_stream_close(
            self.owner._handle_if_open(), self.stream_id
        )
        self._terminal_action_done = True
        self.owner._remove_stream(self.stream_id, self.callback_token)


@dataclass
class _CABIBidiTransport:
    owner: CABIRuntimeTransport
    bidi_id: int
    callback_token: int
    inbox: "_CallbackInbox"
    _terminal_action_done: bool = False

    def send(self, frame_json: bytes) -> bytes:
        if self._terminal_action_done:
            raise _closed_error("bidi transport is closed")
        self.owner.lib.invocation_bidi_send(
            self.owner._handle_if_open(), self.bidi_id, frame_json
        )
        return frame_json

    def recv(self, timeout: float | None = None) -> bytes:
        if self._terminal_action_done:
            raise _closed_error("bidi transport is closed")
        return self.inbox.recv(timeout)

    def close_send(self) -> bytes:
        if self._terminal_action_done:
            raise _closed_error("bidi transport is closed")
        self.owner.lib.invocation_bidi_close_send(
            self.owner._handle_if_open(), self.bidi_id
        )
        return _json_bytes(
            {
                "session_id": str(self.bidi_id),
                "state": "HalfClosedLocal",
                "terminal": False,
            }
        )

    def close(self) -> None:
        if self._terminal_action_done:
            return
        self.owner.lib.invocation_bidi_close(self.owner._handle_if_open(), self.bidi_id)
        self._terminal_action_done = True
        self.owner._remove_bidi(self.bidi_id, self.callback_token)

    def cancel(self, reason: str) -> bytes:
        if not self._terminal_action_done:
            self.owner.lib.invocation_bidi_cancel(
                self.owner._handle_if_open(), self.bidi_id
            )
            self._terminal_action_done = True
            self.owner._remove_bidi(self.bidi_id, self.callback_token)
        return _json_bytes(
            {
                "session_id": str(self.bidi_id),
                "state": "Cancelled",
                "terminal": True,
                "reason": reason,
            }
        )


@dataclass
class _CallbackInbox:
    max_items: int
    _queue: queue_module.Queue[bytes | None] = field(init=False)
    _lock: threading.Lock = field(default_factory=threading.Lock)
    _closed: bool = False
    _failure: SDKError | None = None

    def __post_init__(self) -> None:
        self._queue = queue_module.Queue(maxsize=self.max_items)

    def push(self, raw: bytes) -> None:
        with self._lock:
            if self._closed or self._failure is not None:
                return
            try:
                self._queue.put_nowait(raw)
            except queue_module.Full:
                self._failure = SDKError(
                    code=ErrorCode.PROTOCOL,
                    stage="cabi",
                    retry=RetryHint.NEVER,
                    message="C ABI callback queue limit exceeded",
                )

    def recv(self, timeout: float | None = None) -> bytes:
        with self._lock:
            failure = self._failure
        if failure is not None:
            raise failure
        try:
            item = self._queue.get(timeout=timeout)
        except queue_module.Empty:
            raise TimeoutError("no C ABI callback frame within timeout") from None
        with self._lock:
            failure = self._failure
        if failure is not None:
            raise failure
        if item is None:
            raise _closed_error("C ABI callback inbox is closed")
        return item

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
            try:
                self._queue.put_nowait(None)
            except queue_module.Full:
                pass


def _register_callback_inbox(inbox: _CallbackInbox) -> int:
    global _NEXT_CALLBACK_TOKEN
    with _CALLBACK_REGISTRY_LOCK:
        token = _NEXT_CALLBACK_TOKEN
        _NEXT_CALLBACK_TOKEN += 1
        _CALLBACK_INBOXES[token] = inbox
        return token


def _release_callback_inbox(token: int) -> None:
    with _CALLBACK_REGISTRY_LOCK:
        inbox = _CALLBACK_INBOXES.pop(token, None)
    if inbox is not None:
        inbox.close()


def _callback_inbox(token: int) -> _CallbackInbox | None:
    with _CALLBACK_REGISTRY_LOCK:
        return _CALLBACK_INBOXES.get(token)


def _stream_callback(user_data: int | None, chunk_json: int | None) -> None:
    _push_callback_payload(user_data, chunk_json)


def _bidi_callback(user_data: int | None, frame_json: int | None) -> None:
    _push_callback_payload(user_data, frame_json)


def _push_callback_payload(user_data: int | None, raw_ptr: int | None) -> None:
    try:
        if not user_data or not raw_ptr:
            return
        inbox = _callback_inbox(int(user_data))
        if inbox is None:
            return
        inbox.push(ctypes.string_at(raw_ptr))
    except BaseException:
        return


_STREAM_CALLBACK_HANDLE = _StreamCallback(_stream_callback)
_BIDI_CALLBACK_HANDLE = _BidiCallback(_bidi_callback)


def open_cabi_identity_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIIdentityTransport:
    """Open an owned C ABI identity transport using ``easynet_init``."""

    lib = CLILibrary.load(library_path)
    handle = lib.init(control_path)
    return CABIIdentityTransport(lib=lib, handle=handle, owns_handle=True)


def open_cabi_receipt_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIReceiptTransport:
    """Open an owned C ABI Receipt profile transport."""

    return _open_cabi_profile_transport(
        CABIReceiptTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_directory_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIDirectoryTransport:
    """Open an owned C ABI Directory profile carrier/projection transport."""

    return _open_cabi_profile_transport(
        CABIDirectoryTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_publication_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIPublicationTransport:
    """Open an owned C ABI Publication profile transport."""

    return _open_cabi_profile_transport(
        CABIPublicationTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_host_binding_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIHostBindingTransport:
    """Open an owned C ABI Host Binding profile transport."""

    return _open_cabi_profile_transport(
        CABIHostBindingTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_mission_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIMissionTransport:
    """Open an owned C ABI Mission profile transport."""

    return _open_cabi_profile_transport(
        CABIMissionTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_admin_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIAdminTransport:
    """Open an owned C ABI Admin + Gateway profile transport."""

    lib = CLILibrary.load(library_path)
    handle = lib.init(control_path)
    try:
        daemon_handle = lib.daemon_attach(
            _json_bytes({"control_path": control_path} if control_path else {})
        )
    except BaseException:
        lib.shutdown(handle)
        raise
    return CABIAdminTransport(
        lib=lib,
        handle=handle,
        owns_handle=True,
        daemon_handle=daemon_handle,
        owns_daemon_handle=True,
    )


def open_cabi_events_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIEventTransport:
    """Open an owned C ABI Events profile transport."""

    return _open_cabi_profile_transport(
        CABIEventTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_surface_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABISurfaceTransport:
    """Open an owned C ABI Surface profile transport."""

    return _open_cabi_profile_transport(
        CABISurfaceTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_compatibility_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABICompatibilityTransport:
    """Open an owned C ABI Compatibility profile transport."""

    return _open_cabi_profile_transport(
        CABICompatibilityTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_wrapper_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIWrapperTransport:
    """Open an owned C ABI Convenience Wrapper profile transport."""

    return _open_cabi_profile_transport(
        CABIWrapperTransport,
        control_path=control_path,
        library_path=library_path,
    )


def open_cabi_daemon_transport(
    *,
    library_path: str | None = None,
) -> CABIDaemonTransport:
    """Open a C ABI daemon lifecycle transport."""

    return CABIDaemonTransport(lib=CLILibrary.load(library_path))


def open_cabi_runtime_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIRuntimeTransport:
    """Open an owned C ABI runtime transport using ``easynet_init``."""

    lib = CLILibrary.load(library_path)
    handle = lib.init(control_path)
    return CABIRuntimeTransport(lib=lib, handle=handle, owns_handle=True)


def open_cabi_runtime_connector(
    *,
    library_path: str | None = None,
) -> CABIRuntimeConnector:
    """Open a C ABI-backed RuntimeConnection connector."""

    return CABIRuntimeConnector(lib=CLILibrary.load(library_path))


def _open_cabi_profile_transport(
    transport_type: type[_CABIProfileTransport],
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> _CABIProfileTransport:
    lib = CLILibrary.load(library_path)
    handle = lib.init(control_path)
    return transport_type(lib=lib, handle=handle, owns_handle=True)


def _optional_c_string(value: str) -> bytes | None:
    if not value:
        return None
    return value.encode("utf-8")


def _json_bytes(value: dict[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _projection_request_json(
    request_json: bytes,
    result: dict[str, object],
    *,
    passthrough_keys: tuple[str, ...] = ("limit", "cursor"),
) -> bytes:
    request = _json_object(request_json, "profile projection request")
    projection: dict[str, object] = {"result": result}
    for key in passthrough_keys:
        value = request.get(key)
        if value is not None:
            projection[key] = value
    return _json_bytes(projection)


def _resume_token_from_event_subscription(request_json: bytes) -> str:
    request = _json_object(request_json, "events subscription request")
    cursor = request.get("resume_cursor")
    if not isinstance(cursor, dict):
        return ""
    token = cursor.get("token")
    if isinstance(token, str) and token:
        return token
    stream = cursor.get("stream")
    sequence = cursor.get("sequence")
    if isinstance(stream, str) and isinstance(sequence, int) and not isinstance(sequence, bool):
        return f"{stream}:{sequence}"
    return ""


def _directory_subscription_cursor_from_request(
    request_json: bytes,
) -> DirectorySubscriptionCursor:
    request = _json_object(request_json, "directory subscription request")
    cursor = request.get("resume_cursor")
    if not isinstance(cursor, dict):
        return DirectorySubscriptionCursor("directory", 0)
    stream = cursor.get("stream")
    sequence = cursor.get("sequence")
    token = cursor.get("token")
    if stream != "directory" or not isinstance(sequence, int) or isinstance(sequence, bool):
        return DirectorySubscriptionCursor("directory", 0)
    return DirectorySubscriptionCursor(
        stream,
        sequence,
        token if isinstance(token, str) else "",
    )


def _admin_gateway_status_projection_input(
    daemon_status: dict[str, object], request: dict[str, object]
) -> bytes:
    projection: dict[str, object] = {
        "runtime_status": _daemon_state_from_cabi(daemon_status),
        "daemon": dict(daemon_status),
    }
    require_public_listener = request.get("require_public_listener")
    if isinstance(require_public_listener, bool):
        projection["require_public_listener"] = require_public_listener
    metadata = request.get("metadata")
    if isinstance(metadata, dict):
        projection["metadata"] = dict(metadata)
    return _json_bytes(projection)


def _daemon_start_config_for_cabi(config_json: bytes) -> bytes:
    config = _json_object(config_json, "daemon start config")
    unsupported = [
        field_name
        for field_name in (
            "uds_path",
            "listen_tcp",
            "tls_cert_path",
            "tls_key_path",
            "hub_endpoint",
            "trust_path",
        )
        if config.get(field_name)
    ]
    if unsupported:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="cabi",
            retry=RetryHint.NEVER,
            message=(
                "C ABI daemon start does not support fields: "
                + ", ".join(sorted(unsupported))
            ),
        )
    projected: dict[str, object] = {}
    for field_name in (
        "mode",
        "realm",
        "device_id",
        "daemon_bin",
        "log_path",
        "env",
    ):
        value = config.get(field_name)
        if value not in (None, "", {}, False):
            projected[field_name] = value
    if config.get("detached"):
        projected["detached"] = True
    return _json_bytes(projected)


def _daemon_status_from_cabi(handle_id: str, raw: bytes) -> dict[str, object]:
    decoded = _json_object(raw, "daemon status")
    endpoints = _daemon_endpoints_from_cabi(decoded)
    state = decoded.get("state")
    if not isinstance(state, str) or state == "":
        state = _daemon_state_from_cabi(decoded)
    status: dict[str, object] = {
        "state": state,
        "endpoints": endpoints,
        "diagnostics": _string_list(decoded.get("diagnostics", [])),
    }
    if handle_id != "0":
        status["handle_id"] = handle_id
    mode = decoded.get("mode")
    if isinstance(mode, str) and mode:
        status["mode"] = mode
    pid = decoded.get("pid")
    if isinstance(pid, int) and not isinstance(pid, bool) and pid >= 0:
        status["pid"] = pid
    version = decoded.get("version")
    if isinstance(version, str) and version:
        status["version"] = version
    message = decoded.get("message")
    if isinstance(message, str) and message:
        status["message"] = message
    return status


def _daemon_endpoints_from_cabi(decoded: dict[str, object]) -> dict[str, object]:
    raw_endpoints = decoded.get("endpoints")
    if isinstance(raw_endpoints, dict):
        return {
            "control_endpoint": _optional_json_string(raw_endpoints, "control_endpoint"),
            "invocation_endpoint": _optional_json_string(
                raw_endpoints, "invocation_endpoint"
            ),
            "public_endpoint": _optional_json_string(raw_endpoints, "public_endpoint"),
        }
    return {
        "control_endpoint": _optional_json_string(decoded, "control_endpoint"),
        "invocation_endpoint": _optional_json_string(decoded, "invocation_endpoint"),
        "public_endpoint": _optional_json_string(decoded, "public_endpoint"),
    }


def _daemon_state_from_cabi(decoded: dict[str, object]) -> str:
    invocation_ready = decoded.get("invocation_accepting") is True
    control_ready = decoded.get("control_accepting") is True
    pid_alive = decoded.get("pid_alive") is True
    if invocation_ready:
        return "Running"
    if control_ready:
        return "ControlOnly"
    if pid_alive:
        return "ControlReady"
    return "Stopped"


def _optional_json_string(decoded: dict[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    return value if isinstance(value, str) else ""


def _required_json_string(decoded: dict[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"{field_name} is required",
        )
    return value


def _string_list(value: object) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item for item in value if isinstance(item, str)]


def _merge_bidi_streams(draft_json: bytes, streams_json: bytes) -> bytes:
    draft = _json_object(draft_json, "bidi invocation")
    try:
        streams = json.loads(streams_json.decode("utf-8"))
    except Exception as exc:
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"decode bidi streams: {exc}",
            cause=exc,
        ) from exc
    if not isinstance(streams, list) or not streams:
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="cabi",
            retry=RetryHint.NEVER,
            message="bidi_streams must be a non-empty array",
        )
    if any(not isinstance(stream, dict) for stream in streams):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="cabi",
            retry=RetryHint.NEVER,
            message="bidi_streams entries must be objects",
        )
    draft["bidi_streams"] = streams
    return _json_bytes(draft)


def _platform_library_candidates() -> tuple[str, ...]:
    repo_root = Path(__file__).resolve().parents[3]
    return tuple(
        str(repo_root / path)
        for path in (
            "target/release/libeasynet_cli.dylib",
            "target/release/libeasynet_cli.so",
            "target/release/deps/libeasynet_cli.dylib",
            "target/release/deps/libeasynet_cli.so",
            "target/debug/libeasynet_cli.dylib",
            "target/debug/libeasynet_cli.so",
            "target/debug/deps/libeasynet_cli.dylib",
            "target/debug/deps/libeasynet_cli.so",
        )
    )


def _json_object(raw: bytes, label: str) -> dict[str, object]:
    try:
        decoded = json.loads(raw.decode("utf-8"))
    except Exception as exc:
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"decode {label}: {exc}",
            cause=exc,
        ) from exc
    if not isinstance(decoded, dict):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"{label} must be an object",
        )
    return decoded


def _required_string(decoded: dict[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value == "":
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="sdk",
            retry=RetryHint.NEVER,
            message=f"{field_name} is required",
        )
    return value


def _required_object(
    decoded: dict[str, object], field_name: str
) -> dict[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="sdk",
            retry=RetryHint.NEVER,
            message=f"{field_name} must be an object",
        )
    return value


def _prepared_key(decoded: dict[str, object]) -> str:
    prepared_id = decoded.get("prepared_id")
    if isinstance(prepared_id, str) and prepared_id.strip() != "":
        return prepared_id
    request_id = decoded.get("request_id")
    if isinstance(request_id, str) and request_id.strip() != "":
        return request_id
    raise SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        message="prepared_id or request_id is required",
    )


def _profile_stream_protocol_error(message: str, details: object) -> SDKError:
    detail_value = details if isinstance(details, dict) else {"error": details}
    return SDKError(
        code=ErrorCode.PROTOCOL,
        stage="cabi",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=detail_value,
    )


def _closed_error(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        message=message,
    )


def _transport_error(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.ROUTE_UNAVAILABLE,
        stage="transport",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )
