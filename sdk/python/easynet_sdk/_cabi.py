"""Private generic C ABI v6 transport adapter.

This module is intentionally the only Python SDK file that imports ``ctypes``.
Public facade modules depend on narrow transport protocols and never expose
raw C ABI symbols or numeric handles to product code.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import json
import queue as queue_module
import sys
import threading
from dataclasses import dataclass, field
from typing import Any, Callable

from .errors import ErrorCode, RetryHint, SDKError, retryable_for_hint
from .runtime import InvocationControlCapability

EXPECTED_ABI_VERSION = 6
RUNTIME_OK = 0
MAX_CABI_CALLBACK_QUEUE = 1024

_StreamCallback = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p)
_BidiCallback = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p)
_CALLBACK_REGISTRY_LOCK = threading.Lock()
_CALLBACK_INBOXES: dict[int, "_CallbackInbox"] = {}
_NEXT_CALLBACK_TOKEN = 1


class RuntimeCABILibrary:
    """Typed binding for the generic runtime C ABI v6 surface."""

    def __init__(self, raw: Any) -> None:
        self._raw = raw
        self._bind_symbols()

    @classmethod
    def load(cls, path: str | None = None) -> "RuntimeCABILibrary":
        """Load ``libeasynet_cli`` and verify the ABI version."""

        candidates: list[str] = []
        if path:
            candidates.append(path)
        else:
            found = ctypes.util.find_library("easynet_cli")
            if found:
                candidates.append(found)
            candidates.extend(
                name
                for name in _platform_library_candidates()
                if name not in candidates
            )
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
            "no usable libeasynet_cli C ABI v6 library found: " + "; ".join(errors)
        )

    def require_abi(self, expected: int = EXPECTED_ABI_VERSION) -> None:
        actual = int(self._raw.runtime_abi_version())
        if actual != expected:
            raise SDKError(
                code=ErrorCode.VERSION_MISMATCH,
                stage="cabi",
                retry=RetryHint.NEVER,
                message=f"libeasynet_cli ABI version {actual} does not match expected {expected}",
            )

    def feature_discovery(self) -> bytes:
        return self._call_output(self._raw.runtime_feature_discovery)

    def init(self, control_path: str = "") -> int:
        out_handle = ctypes.c_uint64(0)
        raw_path = _optional_c_string(control_path)
        code = int(self._raw.runtime_init(raw_path, ctypes.byref(out_handle)))
        self._raise_for_code(code)
        return int(out_handle.value)

    def shutdown(self, handle: int) -> None:
        code = int(self._raw.runtime_shutdown(ctypes.c_uint64(handle)))
        self._raise_for_code(code)

    def runtime_host_start(self, config_json: bytes) -> int:
        out_handle = ctypes.c_uint64(0)
        code = int(
            self._raw.runtime_host_start(
                ctypes.c_char_p(config_json), ctypes.byref(out_handle)
            )
        )
        self._raise_for_code(code)
        return int(out_handle.value)

    def runtime_host_attach(self, options_json: bytes) -> int:
        out_handle = ctypes.c_uint64(0)
        code = int(
            self._raw.runtime_host_attach(
                ctypes.c_char_p(options_json), ctypes.byref(out_handle)
            )
        )
        self._raise_for_code(code)
        return int(out_handle.value)

    def runtime_host_discover(self, options_json: bytes) -> bytes:
        return self._call_output(
            self._raw.runtime_host_discover,
            ctypes.c_char_p(options_json),
        )

    def runtime_host_stop(self, runtime_host_handle: int) -> None:
        code = int(self._raw.runtime_host_stop(ctypes.c_uint64(runtime_host_handle)))
        self._raise_for_code(code)

    def runtime_host_detach(self, runtime_host_handle: int) -> None:
        code = int(self._raw.runtime_host_detach(ctypes.c_uint64(runtime_host_handle)))
        self._raise_for_code(code)

    def runtime_host_status(self, runtime_host_handle: int) -> bytes:
        return self._call_output(
            self._raw.runtime_host_status,
            ctypes.c_uint64(runtime_host_handle),
        )

    def runtime_host_endpoints(self, runtime_host_handle: int) -> bytes:
        return self._call_output(
            self._raw.runtime_host_endpoints,
            ctypes.c_uint64(runtime_host_handle),
        )

    def runtime_host_invocation_endpoint(self, runtime_host_handle: int) -> str:
        raw = self._call_output(
            self._raw.runtime_host_invocation_endpoint,
            ctypes.c_uint64(runtime_host_handle),
        )
        endpoint = raw.decode("utf-8")
        if not endpoint:
            raise SDKError(
                code=ErrorCode.CONTROL_ONLY,
                stage="cabi",
                retry=RetryHint.SAFE,
                retryable=True,
                message="runtime host did not advertise invocation_endpoint",
            )
        return endpoint

    def runtime_host_open_client(self, runtime_host_handle: int) -> int:
        out_handle = ctypes.c_uint64(0)
        code = int(
            self._raw.runtime_host_open_client(
                ctypes.c_uint64(runtime_host_handle), ctypes.byref(out_handle)
            )
        )
        self._raise_for_code(code)
        return int(out_handle.value)

    def runtime_health(self, handle: int) -> bytes:
        return self._call_output(
            self._raw.runtime_health,
            ctypes.c_uint64(handle),
        )

    def runtime_diagnostics(self, handle: int) -> bytes:
        return self._call_output(
            self._raw.runtime_diagnostics,
            ctypes.c_uint64(handle),
        )

    def runtime_resolve_descriptor_ref(
        self, handle: int, request_json: bytes
    ) -> bytes:
        return self._call_output(
            self._raw.runtime_resolve_descriptor_ref,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(request_json),
        )

    def invocation_invoke(self, handle: int, invocation_json: bytes) -> bytes:
        return self._call_output(
            self._raw.runtime_invocation_invoke,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(invocation_json),
        )

    def invocation_prepare(
        self, handle: int, invocation_json: bytes, options_json: bytes
    ) -> tuple[int, bytes]:
        return self._call_output_with_id(
            self._raw.runtime_invocation_prepare,
            ctypes.c_uint64(handle),
            ctypes.c_char_p(invocation_json),
            ctypes.c_char_p(options_json),
        )

    def invocation_sign_prepared(
        self, prepared_id: int, signature_json: bytes
    ) -> tuple[int, bytes]:
        return self._call_output_with_id(
            self._raw.runtime_invocation_sign_prepared,
            ctypes.c_uint64(prepared_id),
            ctypes.c_char_p(signature_json),
        )

    def invocation_sign_prepared_local(self, prepared_id: int) -> tuple[int, bytes]:
        return self._call_output_with_id(
            self._raw.runtime_invocation_sign_prepared_local,
            ctypes.c_uint64(prepared_id),
        )

    def invocation_submit_signed_handle(
        self, handle: int, signed_id: int
    ) -> tuple[int, bytes]:
        return self._call_output_with_id(
            self._raw.runtime_invocation_submit_signed_handle,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(signed_id),
        )

    def invocation_handle_await(self, handle: int, invocation_handle_id: int) -> bytes:
        return self._call_output(
            self._raw.runtime_invocation_handle_await,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(invocation_handle_id),
        )

    def invocation_handle_cancel(
        self, handle: int, invocation_handle_id: int, reason: str
    ) -> bytes:
        return self._call_output(
            self._raw.runtime_invocation_handle_cancel,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(invocation_handle_id),
            ctypes.c_char_p(_optional_c_string(reason)),
        )

    def invocation_handle_events(self, handle: int, invocation_handle_id: int) -> bytes:
        return self._call_output(
            self._raw.runtime_invocation_handle_events,
            ctypes.c_uint64(handle),
            ctypes.c_uint64(invocation_handle_id),
        )

    def invocation_handle_free(self, handle: int, invocation_handle_id: int) -> None:
        code = int(
            self._raw.runtime_invocation_handle_free(
                ctypes.c_uint64(handle), ctypes.c_uint64(invocation_handle_id)
            )
        )
        self._raise_for_code(code)

    def prepared_invocation_free(self, prepared_id: int) -> None:
        code = int(
            self._raw.runtime_prepared_invocation_free(ctypes.c_uint64(prepared_id))
        )
        self._raise_for_code(code)

    def signed_invocation_free(self, signed_id: int) -> None:
        code = int(self._raw.runtime_signed_invocation_free(ctypes.c_uint64(signed_id)))
        self._raise_for_code(code)

    def invocation_stream_open(
        self, handle: int, invocation_json: bytes, callback_token: int
    ) -> int:
        out_stream_id = ctypes.c_uint64(0)
        code = int(
            self._raw.runtime_invocation_stream_open(
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
            self._raw.runtime_invocation_stream_cancel(
                ctypes.c_uint64(handle), ctypes.c_uint64(stream_id)
            )
        )
        self._raise_for_code(code)

    def invocation_stream_close(self, handle: int, stream_id: int) -> None:
        code = int(
            self._raw.runtime_invocation_stream_close(
                ctypes.c_uint64(handle), ctypes.c_uint64(stream_id)
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_open(
        self, handle: int, invocation_json: bytes, callback_token: int
    ) -> int:
        out_bidi_id = ctypes.c_uint64(0)
        code = int(
            self._raw.runtime_invocation_bidi_open(
                ctypes.c_uint64(handle),
                ctypes.c_char_p(invocation_json),
                _BIDI_CALLBACK_HANDLE,
                ctypes.c_void_p(callback_token),
                ctypes.byref(out_bidi_id),
            )
        )
        self._raise_for_code(code)
        return int(out_bidi_id.value)

    def invocation_bidi_send(
        self, handle: int, bidi_id: int, frame_json: bytes
    ) -> None:
        code = int(
            self._raw.runtime_invocation_bidi_send(
                ctypes.c_uint64(handle),
                ctypes.c_uint64(bidi_id),
                ctypes.c_char_p(frame_json),
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_close_send(self, handle: int, bidi_id: int) -> None:
        code = int(
            self._raw.runtime_invocation_bidi_close_send(
                ctypes.c_uint64(handle), ctypes.c_uint64(bidi_id)
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_close(self, handle: int, bidi_id: int) -> None:
        code = int(
            self._raw.runtime_invocation_bidi_close(
                ctypes.c_uint64(handle), ctypes.c_uint64(bidi_id)
            )
        )
        self._raise_for_code(code)

    def invocation_bidi_cancel(self, handle: int, bidi_id: int) -> None:
        code = int(
            self._raw.runtime_invocation_bidi_cancel(
                ctypes.c_uint64(handle), ctypes.c_uint64(bidi_id)
            )
        )
        self._raise_for_code(code)

    def _bind_symbols(self) -> None:
        self._raw.runtime_abi_version.argtypes = []
        self._raw.runtime_abi_version.restype = ctypes.c_uint32
        self._raw.runtime_feature_discovery.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
        self._raw.runtime_feature_discovery.restype = ctypes.c_int32
        self._raw.runtime_last_error_json.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
        self._raw.runtime_last_error_json.restype = ctypes.c_int32
        self._raw.runtime_error_json.argtypes = [
            ctypes.c_int32,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_error_json.restype = ctypes.c_int32
        self._raw.runtime_string_free.argtypes = [ctypes.c_void_p]
        self._raw.runtime_string_free.restype = None
        self._raw.runtime_init.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.runtime_init.restype = ctypes.c_int32
        self._raw.runtime_shutdown.argtypes = [ctypes.c_uint64]
        self._raw.runtime_shutdown.restype = ctypes.c_int32
        self._raw.runtime_host_start.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.runtime_host_start.restype = ctypes.c_int32
        self._raw.runtime_host_attach.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.runtime_host_attach.restype = ctypes.c_int32
        self._raw.runtime_host_discover.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_host_discover.restype = ctypes.c_int32
        self._raw.runtime_host_stop.argtypes = [ctypes.c_uint64]
        self._raw.runtime_host_stop.restype = ctypes.c_int32
        self._raw.runtime_host_detach.argtypes = [ctypes.c_uint64]
        self._raw.runtime_host_detach.restype = ctypes.c_int32
        self._raw.runtime_host_status.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_host_status.restype = ctypes.c_int32
        self._raw.runtime_host_endpoints.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_host_endpoints.restype = ctypes.c_int32
        self._raw.runtime_host_invocation_endpoint.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_host_invocation_endpoint.restype = ctypes.c_int32
        self._raw.runtime_host_open_client.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.runtime_host_open_client.restype = ctypes.c_int32
        self._raw.runtime_health.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_health.restype = ctypes.c_int32
        self._raw.runtime_diagnostics.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_diagnostics.restype = ctypes.c_int32
        self._raw.runtime_resolve_descriptor_ref.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_resolve_descriptor_ref.restype = ctypes.c_int32
        self._raw.runtime_invocation_invoke.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_invoke.restype = ctypes.c_int32
        self._raw.runtime_invocation_prepare.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_prepare.restype = ctypes.c_int32
        self._raw.runtime_invocation_sign_prepared.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_uint64),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_sign_prepared.restype = ctypes.c_int32
        self._raw.runtime_invocation_sign_prepared_local.argtypes = [
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_uint64),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_sign_prepared_local.restype = ctypes.c_int32
        self._raw.runtime_invocation_submit_signed_handle.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_uint64),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_submit_signed_handle.restype = ctypes.c_int32
        self._raw.runtime_invocation_handle_await.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_handle_await.restype = ctypes.c_int32
        self._raw.runtime_invocation_handle_cancel.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_handle_cancel.restype = ctypes.c_int32
        self._raw.runtime_invocation_handle_events.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        self._raw.runtime_invocation_handle_events.restype = ctypes.c_int32
        self._raw.runtime_invocation_handle_free.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.runtime_invocation_handle_free.restype = ctypes.c_int32
        self._raw.runtime_prepared_invocation_free.argtypes = [ctypes.c_uint64]
        self._raw.runtime_prepared_invocation_free.restype = ctypes.c_int32
        self._raw.runtime_signed_invocation_free.argtypes = [ctypes.c_uint64]
        self._raw.runtime_signed_invocation_free.restype = ctypes.c_int32
        self._raw.runtime_invocation_stream_open.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            _StreamCallback,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.runtime_invocation_stream_open.restype = ctypes.c_int32
        self._raw.runtime_invocation_stream_cancel.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.runtime_invocation_stream_cancel.restype = ctypes.c_int32
        self._raw.runtime_invocation_stream_close.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.runtime_invocation_stream_close.restype = ctypes.c_int32
        self._raw.runtime_invocation_bidi_open.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            _BidiCallback,
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_uint64),
        ]
        self._raw.runtime_invocation_bidi_open.restype = ctypes.c_int32
        self._raw.runtime_invocation_bidi_send.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
            ctypes.c_char_p,
        ]
        self._raw.runtime_invocation_bidi_send.restype = ctypes.c_int32
        self._raw.runtime_invocation_bidi_close_send.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.runtime_invocation_bidi_close_send.restype = ctypes.c_int32
        self._raw.runtime_invocation_bidi_close.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.runtime_invocation_bidi_close.restype = ctypes.c_int32
        self._raw.runtime_invocation_bidi_cancel.argtypes = [
            ctypes.c_uint64,
            ctypes.c_uint64,
        ]
        self._raw.runtime_invocation_bidi_cancel.restype = ctypes.c_int32

    def _call_output(self, function: Any, *args: Any) -> bytes:
        out = ctypes.c_void_p()
        code = int(function(*args, ctypes.byref(out)))
        self._raise_for_code(code)
        if not out.value:
            return b""
        try:
            return ctypes.string_at(out.value)
        finally:
            self._raw.runtime_string_free(out)

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
            self._raw.runtime_string_free(out)

    def _raise_for_code(self, code: int) -> None:
        if code == RUNTIME_OK:
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
        code = int(self._raw.runtime_last_error_json(ctypes.byref(out)))
        if code != RUNTIME_OK or not out.value:
            return None
        try:
            return SDKError.from_json(ctypes.string_at(out.value))
        finally:
            self._raw.runtime_string_free(out)


@dataclass
class CABIDiscoveryTransport:
    """Feature discovery transport backed by C ABI v6."""

    lib: RuntimeCABILibrary
    _closed: bool = False

    def feature_discovery(self) -> bytes:
        if self._closed:
            raise _closed_error("discovery transport is closed")
        return self.lib.feature_discovery()

    def close(self) -> None:
        self._closed = True


@dataclass
class CABIRuntimeLifecycleTransport:
    """Runtime host lifecycle transport backed by generic C ABI v6."""

    lib: RuntimeCABILibrary
    _handles: dict[str, int] = field(default_factory=dict)
    _status_cache: dict[str, dict[str, object]] = field(default_factory=dict)
    _closed: bool = False

    def discover(self, options_json: bytes) -> bytes:
        self._require_open()
        raw = self.lib.runtime_host_discover(options_json)
        status = _runtime_status_from_cabi("0", raw)
        endpoints = status["endpoints"]
        if not isinstance(endpoints, dict):
            raise TypeError("runtime host discovery omitted endpoints")
        return _json_bytes(endpoints)

    def start(self, config_json: bytes) -> bytes:
        self._require_open()
        native_handle = self.lib.runtime_host_start(
            _runtime_start_config_for_cabi(config_json)
        )
        public_id = str(native_handle)
        self._handles[public_id] = native_handle
        status = _runtime_status_from_cabi(
            public_id, self.lib.runtime_host_status(native_handle)
        )
        self._status_cache[public_id] = status
        return _json_bytes(status)

    def attach(self, options_json: bytes) -> bytes:
        self._require_open()
        native_handle = self.lib.runtime_host_attach(options_json)
        public_id = str(native_handle)
        self._handles[public_id] = native_handle
        status = _runtime_status_from_cabi(
            public_id, self.lib.runtime_host_status(native_handle)
        )
        self._status_cache[public_id] = status
        return _json_bytes(status)

    def status(self, handle_id: str) -> bytes:
        native_handle = self._require_runtime_handle(handle_id)
        status = _runtime_status_from_cabi(
            handle_id, self.lib.runtime_host_status(native_handle)
        )
        self._status_cache[handle_id] = status
        return _json_bytes(status)

    def invocation_endpoint(self, handle_id: str) -> str:
        native_handle = self._require_runtime_handle(handle_id)
        endpoint = self.lib.runtime_host_invocation_endpoint(native_handle)
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
        """Open only the generic Runtime profile exposed by C ABI v6."""

        if profile == "runtime":
            return self.open_runtime_transport(handle_id, options_json)
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="sdk",
            retry=RetryHint.NEVER,
            retryable=False,
            message=(
                f"{profile} is a language-SDK profile and has no C ABI provider; "
                "configure an explicit high-level provider"
            ),
        )

    def _open_client_handle(self, handle_id: str, profile: str) -> int:
        native_handle = self._require_runtime_handle(handle_id)
        client_handle = self.lib.runtime_host_open_client(native_handle)
        if client_handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message=(
                    f"C ABI runtime open {profile} returned an invalid client handle"
                ),
            )
        return client_handle

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        _ = options_json
        native_handle = self._require_runtime_handle(handle_id)
        self.lib.runtime_host_stop(native_handle)
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
        native_handle = self._require_runtime_handle(handle_id)
        self.lib.runtime_host_detach(native_handle)
        self._handles.pop(handle_id, None)
        self._status_cache.pop(handle_id, None)

    def _require_runtime_handle(self, handle_id: str) -> int:
        self._require_open()
        native_handle = self._handles.get(handle_id)
        if native_handle is None or native_handle <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="runtime handle is not owned by this transport",
            )
        return native_handle

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        first_error: SDKError | None = None
        for handle_id, native_handle in list(self._handles.items()):
            try:
                self.lib.runtime_host_detach(native_handle)
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
            raise _closed_error("runtime lifecycle transport is closed")


@dataclass
class _CABIPreparedHandle:
    native_id: int
    state: str = "ready"


@dataclass
class _CABIPreparedHandleRegistry:
    _handles: dict[str, _CABIPreparedHandle] = field(default_factory=dict)
    _lock: threading.Lock = field(default_factory=threading.Lock)

    def register(
        self,
        key: str,
        prepared_id: int,
        free_prepared: Callable[[int], None],
    ) -> None:
        with self._lock:
            duplicate = key in self._handles
            if not duplicate:
                self._handles[key] = _CABIPreparedHandle(prepared_id)
        if duplicate:
            free_prepared(prepared_id)
            raise SDKError(
                code=ErrorCode.PROTOCOL,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="C ABI prepare returned a duplicate prepared handle id",
            )

    def claim_for_signing(self, key: str) -> int:
        with self._lock:
            handle = self._handles.get(key)
            if handle is None:
                raise SDKError(
                    code=ErrorCode.INVALID_HANDLE,
                    stage="cabi",
                    retry=RetryHint.NEVER,
                    message="prepared invocation handle is not owned by this transport",
                )
            if handle.state != "ready":
                raise SDKError(
                    code=ErrorCode.INVALID_HANDLE,
                    stage="cabi",
                    retry=RetryHint.NEVER,
                    message="prepared invocation handle is already being signed",
                )
            handle.state = "signing"
            return handle.native_id

    def release_signing_claim(self, key: str, prepared_id: int) -> None:
        with self._lock:
            handle = self._handles.get(key)
            if (
                handle is not None
                and handle.native_id == prepared_id
                and handle.state == "signing"
            ):
                handle.state = "ready"

    def consume_signing_claim(self, key: str, prepared_id: int) -> None:
        with self._lock:
            handle = self._handles.get(key)
            if (
                handle is not None
                and handle.native_id == prepared_id
                and handle.state == "signing"
            ):
                self._handles.pop(key, None)

    def drain(self) -> tuple[int, ...]:
        with self._lock:
            prepared_ids = tuple(handle.native_id for handle in self._handles.values())
            self._handles.clear()
            return prepared_ids

    def keys(self) -> set[str]:
        with self._lock:
            return set(self._handles)


@dataclass
class CABIRuntimeTransport:
    """Runtime Core and Health transport backed by C ABI v6."""

    lib: RuntimeCABILibrary
    handle: int
    owns_handle: bool = False
    _prepared_handles: _CABIPreparedHandleRegistry = field(
        default_factory=_CABIPreparedHandleRegistry
    )
    _streams: dict[int, "_CABIStreamTransport"] = field(default_factory=dict)
    _bidis: dict[int, "_CABIBidiTransport"] = field(default_factory=dict)
    _closed: bool = False

    def runtime_health(self) -> bytes:
        return self.lib.runtime_health(self._require_open())

    def runtime_diagnostics(self) -> bytes:
        return self.lib.runtime_diagnostics(self._require_open())

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
        handle = self._require_open()
        return self.lib.runtime_resolve_descriptor_ref(handle, request_json)

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
        options = _json_object(options_json, "prepare options")
        material_only = bool(options.get("material_only") is True)
        if material_only:
            if prepared_c_id != 0:
                self.lib.prepared_invocation_free(prepared_c_id)
                raise SDKError(
                    code=ErrorCode.INVALID_HANDLE,
                    stage="cabi",
                    retry=RetryHint.NEVER,
                    message="C ABI material-only prepare retained a prepared handle",
                )
            return raw

        key = _prepared_key(_json_object(raw, "prepared invocation"))
        if prepared_c_id <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="C ABI prepare returned an invalid prepared handle",
            )
        self._prepared_handles.register(
            key,
            prepared_c_id,
            self.lib.prepared_invocation_free,
        )
        return raw

    def submit_signed(self, signed_json: bytes) -> bytes:
        signed = _json_object(signed_json, "signed invocation")
        prepared = _required_object(signed, "prepared")
        key = _prepared_key(prepared)
        prepared_c_id = self._prepared_handles.claim_for_signing(key)
        try:
            if _signed_invocation_uses_provider_managed_signing(signed):
                signed_c_id, _ = self.lib.invocation_sign_prepared_local(prepared_c_id)
            else:
                signature_json = json.dumps(
                    _required_object(signed, "signature"),
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")
                signed_c_id, _ = self.lib.invocation_sign_prepared(
                    prepared_c_id, signature_json
                )
        except Exception:
            self._prepared_handles.release_signing_claim(key, prepared_c_id)
            raise
        if signed_c_id <= 0:
            self._prepared_handles.release_signing_claim(key, prepared_c_id)
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="cabi",
                retry=RetryHint.NEVER,
                message="C ABI sign returned an invalid signed handle",
            )
        self._prepared_handles.consume_signing_claim(key, prepared_c_id)
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

    def await_handle(self, control: InvocationControlCapability) -> bytes:
        handle_id = control._adapter_handle_id()
        return self.lib.invocation_handle_await(self._require_open(), handle_id)

    def cancel_handle(self, control: InvocationControlCapability, reason: str) -> bytes:
        handle_id = control._adapter_handle_id()
        return self.lib.invocation_handle_cancel(
            self._require_open(), handle_id, reason
        )

    def handle_events(self, control: InvocationControlCapability) -> bytes:
        handle_id = control._adapter_handle_id()
        return self.lib.invocation_handle_events(self._require_open(), handle_id)

    def free_handle(self, control: InvocationControlCapability) -> None:
        handle_id = control._adapter_handle_id()
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
        for prepared_id in self._prepared_handles.drain():
            try:
                self.lib.prepared_invocation_free(prepared_id)
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
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
    """RuntimeConnection connector backed by C ABI runtime lifecycle calls."""

    lib: RuntimeCABILibrary
    _lifecycle: CABIRuntimeLifecycleTransport = field(init=False)
    _runtime: CABIRuntimeTransport | None = None
    _closed: bool = False

    def __post_init__(self) -> None:
        self._lifecycle = CABIRuntimeLifecycleTransport(self.lib)

    def resolve(self, options_json: bytes) -> bytes:
        self._require_open()
        options = _json_object(options_json or b"{}", "runtime connect options")
        control_path = _optional_json_string(options, "control_path")
        endpoints = _json_object(
            self._lifecycle.discover(_json_bytes({"control_path": control_path})),
            "runtime host endpoints",
        )
        endpoint = _optional_json_string(options, "endpoint") or _optional_json_string(
            endpoints, "invocation_endpoint"
        )
        if not endpoint:
            raise SDKError(
                code=ErrorCode.CONTROL_ONLY,
                stage="cabi",
                retry=RetryHint.SAFE,
                message="runtime discovery did not advertise invocation_endpoint",
            )
        return _json_bytes(
            {
                "endpoint": endpoint,
                "control_path": control_path,
                "control_endpoint": _optional_json_string(
                    endpoints, "control_endpoint"
                ),
                "protocol_version": "cabi-v5",
                "abi_version": EXPECTED_ABI_VERSION,
            }
        )

    def handshake(self, endpoint_json: bytes) -> tuple[CABIRuntimeTransport, bytes]:
        self._require_open()
        endpoint = _json_object(endpoint_json, "runtime endpoint")
        invocation_endpoint = _required_json_string(endpoint, "endpoint")
        control_path = _optional_json_string(endpoint, "control_path")
        control_endpoint = _optional_json_string(endpoint, "control_endpoint")
        status_raw = self._lifecycle.attach(
            _json_bytes(
                {
                    "control_endpoint": control_endpoint,
                    "invocation_endpoint": invocation_endpoint,
                    "control_path": control_path,
                }
            )
        )
        status = _json_object(status_raw, "runtime host status")
        handle_id = _required_json_string(status, "handle_id")
        try:
            runtime, _ = self._lifecycle.open_runtime(
                handle_id,
                _json_bytes(
                    {
                        "endpoint": invocation_endpoint,
                        "control_path": control_path,
                    }
                ),
            )
        except BaseException:
            self._lifecycle.detach(handle_id)
            raise
        try:
            self._lifecycle.detach(handle_id)
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
            self._lifecycle.close()
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
    _cancel_sent: bool = False
    _next_sequence: int = 1

    def recv(self, timeout: float | None = None) -> bytes:
        if self._terminal_action_done:
            raise _closed_error("stream transport is closed")
        return _project_cabi_ordered_event(
            self.inbox.recv(timeout),
            self._allocate_sequence,
            use_observed_sequence=True,
        )

    def cancel(self, reason: str) -> bytes:
        if not self._terminal_action_done and not self._cancel_sent:
            self.owner.lib.invocation_stream_cancel(
                self.owner._handle_if_open(), self.stream_id
            )
            self._cancel_sent = True
        return _json_bytes(
            {
                "stream_id": str(self.stream_id),
                "cancel_requested": True,
                "cancelled": False,
                "state": "CancelRequested",
                "terminal": False,
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

    def _allocate_sequence(self, observed: int | None) -> int:
        if observed is not None and observed >= self._next_sequence:
            self._next_sequence = observed + 1
            return observed
        sequence = self._next_sequence
        self._next_sequence += 1
        return sequence


@dataclass
class _CABIBidiTransport:
    owner: CABIRuntimeTransport
    bidi_id: int
    callback_token: int
    inbox: "_CallbackInbox"
    _terminal_action_done: bool = False
    _cancel_sent: bool = False
    _next_sequence: int = 1

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
        return _project_cabi_ordered_event(
            self.inbox.recv(timeout),
            self._allocate_sequence,
            use_observed_sequence=False,
        )

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
        if not self._terminal_action_done and not self._cancel_sent:
            self.owner.lib.invocation_bidi_cancel(
                self.owner._handle_if_open(), self.bidi_id
            )
            self._cancel_sent = True
        return _json_bytes(
            {
                "session_id": str(self.bidi_id),
                "state": "CancelRequested",
                "terminal": False,
                "reason": reason,
            }
        )

    def _allocate_sequence(self, observed: int | None) -> int:
        if observed is not None and observed >= self._next_sequence:
            self._next_sequence = observed + 1
            return observed
        sequence = self._next_sequence
        self._next_sequence += 1
        return sequence


def _project_cabi_ordered_event(
    raw: bytes,
    allocate_sequence: Callable[[int | None], int],
    *,
    use_observed_sequence: bool,
) -> bytes:
    try:
        event = _json_object(raw, "C ABI callback frame")
    except SDKError:
        return raw
    observed = event.get("sequence")
    sequence = (
        observed
        if use_observed_sequence
        and isinstance(observed, int)
        and not isinstance(observed, bool)
        else None
    )
    event["sequence"] = allocate_sequence(sequence)
    state = event.get("state")
    if isinstance(state, int) and not isinstance(state, bool):
        event["state"] = _axon_invocation_state_name(state)
    if "error" not in event and ("code" in event or "message" in event):
        event["error"] = {
            "code": _string_or_empty(event.get("code")),
            "message": _string_or_empty(event.get("message")),
        }
    return _json_bytes(event)


def _string_or_empty(value: object) -> str:
    return value if isinstance(value, str) else ""


def _axon_invocation_state_name(state: int) -> str:
    return {
        1: "Accepted",
        2: "Admitted",
        3: "Dispatched",
        4: "Running",
        5: "Completed",
        6: "Failed",
        7: "TimedOut",
        8: "Cancelled",
    }.get(state, str(state))


@dataclass
class _CallbackInbox:
    max_items: int
    _queue: queue_module.Queue[bytes | None] = field(init=False)
    _lock: threading.Lock = field(default_factory=threading.Lock)
    _closed: bool = False
    _failure: bytes | None = None
    _failure_delivered: bool = False

    def __post_init__(self) -> None:
        self._queue = queue_module.Queue(maxsize=self.max_items)

    def push(self, raw: bytes) -> None:
        with self._lock:
            if self._closed:
                return
            try:
                self._queue.put_nowait(raw)
            except queue_module.Full:
                self._failure = _callback_backpressure_failure()
                self._closed = True

    def recv(self, timeout: float | None = None) -> bytes:
        with self._lock:
            if self._failure is not None and not self._failure_delivered:
                self._failure_delivered = True
                return self._failure
        try:
            item = self._queue.get(timeout=timeout)
        except queue_module.Empty:
            raise SDKError(
                code=ErrorCode.TIMEOUT,
                stage="cabi",
                retry=RetryHint.SAFE,
                retryable=True,
                message="no C ABI callback frame within timeout",
            ) from None
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


def _callback_backpressure_failure() -> bytes:
    return _json_bytes(
        {
            "kind": "error",
            "state": "Failed",
            "terminal": False,
            "transport_terminal": True,
            "error": {
                "code": "ADMISSION_DENIED",
                "stage": "cabi_callback",
                "message": "C ABI callback queue limit exceeded",
                "retry": "after_backoff",
                "details": {
                    "wire_code": "RESOURCE_EXHAUSTED",
                    "reason": "callback_queue_overflow",
                    "bounded_queue": True,
                },
            },
        }
    )


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


def open_cabi_runtime_lifecycle_transport(
    *,
    library_path: str | None = None,
) -> CABIRuntimeLifecycleTransport:
    """Open a C ABI runtime host lifecycle transport."""

    return CABIRuntimeLifecycleTransport(lib=RuntimeCABILibrary.load(library_path))


def open_cabi_runtime_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIRuntimeTransport:
    """Open an owned C ABI runtime transport using ``runtime_init``."""

    lib = RuntimeCABILibrary.load(library_path)
    handle = lib.init(control_path)
    return CABIRuntimeTransport(lib=lib, handle=handle, owns_handle=True)


def open_cabi_runtime_connector(
    *,
    library_path: str | None = None,
) -> CABIRuntimeConnector:
    """Open a C ABI-backed RuntimeConnection connector."""

    return CABIRuntimeConnector(lib=RuntimeCABILibrary.load(library_path))


def _optional_c_string(value: str) -> bytes | None:
    if not value:
        return None
    return value.encode("utf-8")


def _json_bytes(value: dict[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _runtime_start_config_for_cabi(config_json: bytes) -> bytes:
    config = _json_object(config_json, "runtime host start config")
    unsupported = [
        field_name
        for field_name in (
            "uds_path",
            "listen_tcp",
            "tls_cert_path",
            "tls_key_path",
            "authority_endpoint",
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
                "C ABI runtime host start does not support fields: "
                + ", ".join(sorted(unsupported))
            ),
        )
    projected: dict[str, object] = {}
    for field_name in (
        "mode",
        "realm",
        "runtime_instance_id",
        "runtime_bin",
        "working_dir",
        "log_path",
        "env",
    ):
        value = config.get(field_name)
        if value not in (None, "", {}, False):
            projected[field_name] = value
    if "mode" in projected:
        projected["mode"] = _runtime_host_mode_for_cabi(projected["mode"])
    if config.get("detached"):
        projected["detached"] = True
    return _json_bytes(projected)


def _runtime_host_mode_for_cabi(value: object) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _cabi_payload_error("runtime host mode must be a non-empty string")
    mode = value.strip()
    if mode in {"edge", "authority"}:
        return mode
    if mode == "combined":
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="cabi",
            retry=RetryHint.NEVER,
            retryable=False,
            message="C ABI runtime host start does not support combined runtime host mode",
        )
    raise _cabi_payload_error("runtime host mode must be edge, authority, or combined")


def _cabi_payload_error(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="cabi",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _runtime_status_from_cabi(handle_id: str, raw: bytes) -> dict[str, object]:
    decoded = _json_object(raw, "runtime host status")
    endpoints = _runtime_endpoints_from_cabi(decoded)
    state = decoded.get("state")
    if not isinstance(state, str) or state == "":
        state = _runtime_state_from_cabi(decoded)
    status: dict[str, object] = {
        "state": state,
        "endpoints": endpoints,
        "diagnostics": _string_list(decoded.get("diagnostics", [])),
    }
    if handle_id != "0":
        status["handle_id"] = handle_id
    mode = decoded.get("mode")
    if isinstance(mode, str) and mode:
        status["mode"] = _runtime_status_mode_for_cabi(mode)
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


def _runtime_status_mode_for_cabi(value: object) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _cabi_payload_error("runtime host status mode must be a non-empty string")
    mode = value.strip()
    if mode in {"edge", "authority", "combined"}:
        return mode
    raise _cabi_payload_error("runtime host status mode must be edge, authority, or combined")


def _runtime_endpoints_from_cabi(decoded: dict[str, object]) -> dict[str, object]:
    raw_endpoints = decoded.get("endpoints")
    if isinstance(raw_endpoints, dict):
        return {
            "control_endpoint": _optional_json_string(
                raw_endpoints, "control_endpoint"
            ),
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


def _runtime_state_from_cabi(decoded: dict[str, object]) -> str:
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
    if sys.platform == "darwin":
        return ("libeasynet_cli.dylib",)
    if sys.platform == "win32":
        return ("easynet_cli.dll",)
    return ("libeasynet_cli.so",)


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


def _required_object(decoded: dict[str, object], field_name: str) -> dict[str, object]:
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
    raise SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        message="prepared_id is required",
    )


def _signed_invocation_uses_provider_managed_signing(
    decoded: dict[str, object],
) -> bool:
    policy = decoded.get("policy")
    if policy is None:
        return False
    if not isinstance(policy, dict):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="sdk",
            retry=RetryHint.NEVER,
            message="policy must be an object",
        )
    mode = policy.get("mode")
    if mode is None:
        return False
    if not isinstance(mode, str):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="sdk",
            retry=RetryHint.NEVER,
            message="policy.mode must be a string",
        )
    return mode == "provider_managed_signing"


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
