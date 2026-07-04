"""Private C ABI v4 transport adapter.

This module is intentionally the only Python SDK file that imports ``ctypes``.
Public facade modules depend on narrow transport protocols and never expose
raw C ABI symbols or numeric handles to product code.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .errors import ErrorCode, RetryHint, SDKError, retryable_for_hint

EXPECTED_ABI_VERSION = 4
EASYNET_OK = 0


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
                code=ErrorCode.VERSION_INCOMPATIBLE,
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

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self.owns_handle:
            self.lib.shutdown(self.handle)

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
class CABIRuntimeTransport:
    """Runtime Core and Health transport backed by C ABI v4."""

    lib: CLILibrary
    handle: int
    owns_handle: bool = False
    _prepared_ids: dict[str, int] = field(default_factory=dict)
    _closed: bool = False

    def runtime_health(self) -> bytes:
        return self.lib.runtime_health(self._require_open())

    def invoke(self, draft_json: bytes) -> bytes:
        return self.lib.invocation_invoke(self._require_open(), draft_json)

    def open_stream(self, draft_json: bytes) -> tuple[Any, bytes]:
        _ = draft_json
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="cabi",
            retry=RetryHint.NEVER,
            message="C ABI stream transport callback adapter is not implemented",
        )

    def open_bidi(self, draft_json: bytes, streams_json: bytes) -> tuple[Any, bytes]:
        _ = (draft_json, streams_json)
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="cabi",
            retry=RetryHint.NEVER,
            message="C ABI bidi transport callback adapter is not implemented",
        )

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
        self._closed = True
        first_error: SDKError | None = None
        for prepared_id in tuple(self._prepared_ids.values()):
            try:
                self.lib.prepared_invocation_free(prepared_id)
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
        self._prepared_ids.clear()
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


def open_cabi_identity_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIIdentityTransport:
    """Open an owned C ABI identity transport using ``easynet_init``."""

    lib = CLILibrary.load(library_path)
    handle = lib.init(control_path)
    return CABIIdentityTransport(lib=lib, handle=handle, owns_handle=True)


def open_cabi_runtime_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIRuntimeTransport:
    """Open an owned C ABI runtime transport using ``easynet_init``."""

    lib = CLILibrary.load(library_path)
    handle = lib.init(control_path)
    return CABIRuntimeTransport(lib=lib, handle=handle, owns_handle=True)


def _optional_c_string(value: str) -> bytes | None:
    if not value:
        return None
    return value.encode("utf-8")


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


def _closed_error(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        message=message,
    )


def _transport_error(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )
