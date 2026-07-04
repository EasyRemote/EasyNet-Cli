"""Private C ABI v4 transport adapter.

This module is intentionally the only Python SDK file that imports ``ctypes``.
Public facade modules depend on narrow transport protocols and never expose
raw C ABI symbols or numeric handles to product code.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import json
from dataclasses import dataclass
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


def open_cabi_identity_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
) -> CABIIdentityTransport:
    """Open an owned C ABI identity transport using ``easynet_init``."""

    lib = CLILibrary.load(library_path)
    handle = lib.init(control_path)
    return CABIIdentityTransport(lib=lib, handle=handle, owns_handle=True)


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
