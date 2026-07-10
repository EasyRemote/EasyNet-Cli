"""Daemon-owned runtime signing capabilities.

This module deliberately contains no vault codec or private-key material. A
facade reaches the canonical keyring service over its local endpoint and can
only obtain a public-key projection or a signature over supplied canonical
bytes.
"""

from __future__ import annotations

import base64
import json
import os
import socket
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from .errors import ErrorCode, RetryHint, SDKError
from .identity import SignerHandle
from .invocation import InvocationSignature
from .signing import SignatureProvider, SigningMaterial

_MAX_FRAME_BYTES = 64 * 1024


def default_runtime_keyring_socket_path() -> str:
    configured = os.environ.get("EASYNET_KEYRING_SOCKET_PATH", "").strip()
    if configured:
        return configured
    return str(Path.home() / ".easynet" / "keyring.sock")


@dataclass(frozen=True)
class RuntimeSigningIdentity:
    """Opaque capability for one runtime owner URA."""

    owner_ura: str
    public_key: bytes
    socket_path: str
    timeout_seconds: float = 10.0

    def sign_canonical(self, canonical_bytes: bytes) -> bytes:
        if not canonical_bytes:
            raise _invalid("canonical bytes are required for runtime signing")
        response = _call_keyring(
            self.socket_path,
            self.timeout_seconds,
            {
                "method": "sign",
                "self_ura": self.owner_ura,
                "canonical_bytes_b64": base64.b64encode(canonical_bytes).decode("ascii"),
            },
        )
        signature = _decode_b64(response, "signature_b64", expected_len=64)
        return signature


def load_runtime_signing_identity(
    owner_ura: str, *, socket_path: str = "", timeout_seconds: float = 10.0
) -> RuntimeSigningIdentity:
    owner_ura = owner_ura.strip()
    if not owner_ura:
        raise _invalid("runtime signing identity owner URA is required")
    socket_path = socket_path or default_runtime_keyring_socket_path()
    response = _call_keyring(
        socket_path,
        timeout_seconds,
        {"method": "derive_pubkey", "self_ura": owner_ura},
    )
    return RuntimeSigningIdentity(
        owner_ura=owner_ura,
        public_key=_decode_b64(response, "public_key_b64", expected_len=32),
        socket_path=socket_path,
        timeout_seconds=timeout_seconds,
    )


def ensure_runtime_signing_identity(
    owner_ura: str,
    *,
    role_overlays: tuple[str, ...] = (),
    socket_path: str = "",
    timeout_seconds: float = 10.0,
) -> RuntimeSigningIdentity:
    owner_ura = owner_ura.strip()
    if not owner_ura:
        raise _invalid("runtime signing identity owner URA is required")
    socket_path = socket_path or default_runtime_keyring_socket_path()
    response = _call_keyring(
        socket_path,
        timeout_seconds,
        {
            "method": "ensure",
            "primary_self": owner_ura,
            "role_overlays": [item.strip() for item in role_overlays if item.strip() and item.strip() != owner_ura],
        },
    )
    return RuntimeSigningIdentity(
        owner_ura=owner_ura,
        public_key=_decode_b64(response, "public_key_b64", expected_len=32),
        socket_path=socket_path,
        timeout_seconds=timeout_seconds,
    )


@dataclass(frozen=True)
class DaemonKeyringSignatureProvider(SignatureProvider):
    """SignatureProvider backed by the daemon-owned runtime identity."""

    identity: RuntimeSigningIdentity

    def sign(self, material: SigningMaterial, handle: SignerHandle) -> InvocationSignature:
        if material.algorithm != "ed25519" or handle.algorithm != "ed25519":
            raise _invalid("daemon keyring signing requires ed25519")
        if handle.owner_ura != self.identity.owner_ura:
            raise _invalid("signer handle owner URA does not match runtime identity")
        canonical = _decode_b64_value(material.canonical_bytes_base64, "canonical_bytes_base64")
        signature = self.identity.sign_canonical(canonical)
        return InvocationSignature(
            algorithm="ed25519",
            signature_base64=base64.b64encode(signature).decode("ascii"),
            key_id_hint=handle.signer_id,
            signer_public_key_base64=base64.b64encode(self.identity.public_key).decode("ascii"),
        )


def _call_keyring(path: str, timeout: float, request: Mapping[str, Any]) -> Mapping[str, Any]:
    encoded = json.dumps(request, separators=(",", ":")).encode("utf-8")
    if len(encoded) > _MAX_FRAME_BYTES:
        raise _invalid("daemon keyring request exceeds frame limit")
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(timeout)
            connection.connect(path)
            connection.sendall(struct.pack(">I", len(encoded)) + encoded)
            length = struct.unpack(">I", _read_exact(connection, 4))[0]
            if length > _MAX_FRAME_BYTES:
                raise _invalid("daemon keyring response exceeds frame limit")
            response = json.loads(_read_exact(connection, length).decode("utf-8"))
    except SDKError:
        raise
    except OSError as exc:
        raise SDKError(
            code=ErrorCode.DAEMON_OFFLINE,
            stage="runtime_identity",
            retry=RetryHint.SAFE,
            message=f"runtime keyring unavailable at {path}: {exc}",
            retryable=True,
            cause=exc,
        ) from exc
    except (ValueError, UnicodeDecodeError) as exc:
        raise _invalid(f"decode daemon keyring response: {exc}", exc) from exc
    if not isinstance(response, dict):
        raise _invalid("daemon keyring response must be an object")
    if response.get("result") == "error":
        code = ErrorCode.NOT_FOUND if response.get("kind") == "not_found" else ErrorCode.PERMISSION_DENIED
        raise SDKError(
            code=code,
            stage="runtime_identity",
            retry=RetryHint.NEVER,
            message=str(response.get("message", "daemon keyring rejected request")),
        )
    return response


def _read_exact(connection: socket.socket, count: int) -> bytes:
    data = bytearray()
    while len(data) < count:
        chunk = connection.recv(count - len(data))
        if not chunk:
            raise OSError("unexpected EOF from daemon keyring")
        data.extend(chunk)
    return bytes(data)


def _decode_b64(response: Mapping[str, Any], field: str, *, expected_len: int) -> bytes:
    return _decode_b64_value(response.get(field), field, expected_len=expected_len)


def _decode_b64_value(value: Any, field: str, *, expected_len: int | None = None) -> bytes:
    if not isinstance(value, str) or not value:
        raise _invalid(f"daemon keyring response field {field} is required")
    try:
        decoded = base64.b64decode(value, validate=True)
    except Exception as exc:
        raise _invalid(f"{field} must be base64", exc) from exc
    if expected_len is not None and len(decoded) != expected_len:
        raise _invalid(f"{field} must be {expected_len} bytes")
    return decoded


def _invalid(message: str, cause: Exception | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_identity",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )
