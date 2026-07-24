"""Runtime-owned signing capabilities.

This module deliberately contains no vault codec or private-key material. A
facade reaches the canonical keyring service over its local endpoint and can
only obtain a public-key projection or a signature over supplied canonical
bytes.
"""

from __future__ import annotations

import base64
import hashlib
from dataclasses import dataclass
from typing import Callable, TypeVar

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from .key_service import (
    KeyServiceClient,
    MAX_KEY_SERVICE_CANONICAL_BYTES,
    decode_base64_field,
    decode_base64_value,
    invalid_key_service_input,
    invalid_key_service_payload,
    require_response_shape,
)
from ...errors import ErrorCode, SDKError
from ...signer_handle import SignerHandle
from ...invocation import InvocationSignature
from ...signing import SignatureProvider, SigningMaterial

_T = TypeVar("_T")


@dataclass(frozen=True)
class RuntimeSigningIdentity:
    """Opaque capability for one runtime owner URA."""

    owner_ura: str
    public_key: bytes
    socket_path: str
    timeout_seconds: float = 10.0

    def sign_canonical(self, canonical_bytes: bytes) -> bytes:
        if not isinstance(canonical_bytes, bytes) or not canonical_bytes:
            raise _invalid("canonical bytes are required for runtime signing")
        if len(canonical_bytes) > MAX_KEY_SERVICE_CANONICAL_BYTES:
            raise _invalid("canonical bytes exceed the 64 MiB signing limit")

        def operation() -> bytes:
            public_key_b64 = base64.b64encode(self.public_key).decode("ascii")
            response = KeyServiceClient(self.socket_path, self.timeout_seconds).call(
                {
                    "method": "sign",
                    "self_ura": self.owner_ura,
                    "public_key_b64": public_key_b64,
                    "signer_policy_ref": _runtime_signer_policy_ref(
                        self.owner_ura, public_key_b64
                    ),
                    "canonical_bytes_b64": base64.b64encode(canonical_bytes).decode(
                        "ascii"
                    ),
                },
            )
            require_response_shape(response, "signature", required=("signature_b64",))
            signature = decode_base64_field(response, "signature_b64", expected_len=64)
            try:
                Ed25519PublicKey.from_public_bytes(self.public_key).verify(
                    signature, canonical_bytes
                )
            except (ValueError, InvalidSignature) as error:
                raise invalid_key_service_payload(
                    "provider key service returned a signature that does not verify "
                    "against the bound runtime identity",
                    error,
                ) from error
            return signature

        return _runtime_identity_operation(operation)


def load_runtime_signing_identity(
    owner_ura: str, *, socket_path: str = "", timeout_seconds: float = 10.0
) -> RuntimeSigningIdentity:
    owner_ura = _owner_ura(owner_ura)
    client = KeyServiceClient(socket_path, timeout_seconds)

    def operation() -> bytes:
        response = client.call({"method": "derive_pubkey", "self_ura": owner_ura})
        require_response_shape(response, "public_key", required=("public_key_b64",))
        return decode_base64_field(response, "public_key_b64", expected_len=32)

    public_key = _runtime_identity_operation(operation)
    return RuntimeSigningIdentity(
        owner_ura=owner_ura,
        public_key=public_key,
        socket_path=client.socket_path,
        timeout_seconds=client.timeout_seconds,
    )


def ensure_runtime_signing_identity(
    owner_ura: str,
    *,
    socket_path: str = "",
    timeout_seconds: float = 10.0,
) -> RuntimeSigningIdentity:
    owner_ura = _owner_ura(owner_ura)
    client = KeyServiceClient(socket_path, timeout_seconds)

    def operation() -> bytes:
        response = client.call(
            {
                "method": "ensure",
                "primary_self": owner_ura,
            },
        )
        require_response_shape(response, "public_key", required=("public_key_b64",))
        return decode_base64_field(response, "public_key_b64", expected_len=32)

    public_key = _runtime_identity_operation(operation)
    return RuntimeSigningIdentity(
        owner_ura=owner_ura,
        public_key=public_key,
        socket_path=client.socket_path,
        timeout_seconds=client.timeout_seconds,
    )


@dataclass(frozen=True)
class RuntimeKeyringSignatureProvider(SignatureProvider):
    """SignatureProvider backed by a runtime-owned signing identity."""

    identity: RuntimeSigningIdentity

    def sign(
        self, material: SigningMaterial, handle: SignerHandle
    ) -> InvocationSignature:
        if material.algorithm != "ed25519" or handle.algorithm != "ed25519":
            raise _invalid("runtime keyring signing requires ed25519")
        if handle.owner_ura != self.identity.owner_ura:
            raise _invalid("signer handle owner URA does not match runtime identity")
        canonical = _runtime_identity_operation(
            lambda: decode_base64_value(
                material.canonical_bytes_base64, "canonical_bytes_base64"
            )
        )
        signature = self.identity.sign_canonical(canonical)
        return InvocationSignature(
            algorithm="ed25519",
            signature_base64=base64.b64encode(signature).decode("ascii"),
            key_id_hint=handle.signer_id,
            signer_public_key_base64=base64.b64encode(self.identity.public_key).decode(
                "ascii"
            ),
        )


def _invalid(message: str, cause: Exception | None = None) -> SDKError:
    shared = invalid_key_service_input(message, cause)
    return SDKError(
        code=shared.code,
        stage="runtime_identity",
        retry=shared.retry,
        retryable=shared.retryable,
        message=shared.message,
        details=shared.details,
        cause=shared.cause,
    )


def _owner_ura(value: object) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid("runtime signing identity owner URA is required")
    return value.strip()


def _runtime_signer_policy_ref(owner_ura: str, public_key_b64: str) -> str:
    digest = hashlib.sha256(
        owner_ura.encode("utf-8")
        + b"\0"
        + owner_ura.encode("utf-8")
        + b"\0"
        + public_key_b64.encode("ascii")
    ).hexdigest()[:32]
    return f"provider-key-inventory:sha256:{digest}"


def _runtime_identity_operation(operation: Callable[[], _T]) -> _T:
    try:
        return operation()
    except SDKError as error:
        raise _runtime_identity_error(error) from error


def _runtime_identity_error(error: SDKError) -> SDKError:
    code = error.code
    if code == ErrorCode.NOT_FOUND:
        code = ErrorCode.CALLER_SIGNER_UNAVAILABLE
    return SDKError(
        code=code,
        stage="runtime_identity",
        retry=error.retry,
        retryable=error.retryable,
        message=error.message,
        source=error.source,
        invocation_id=error.invocation_id,
        receipt_ura=error.receipt_ura,
        details=error.details,
        cause=error,
    )
