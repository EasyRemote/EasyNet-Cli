"""Runtime Core signing-boundary DTOs."""

from __future__ import annotations

import base64
import binascii
import hashlib
import json
from dataclasses import dataclass, field, replace
from typing import TYPE_CHECKING, Any, Mapping, Optional, Protocol, cast

if TYPE_CHECKING:
    from .runtime import InvocationHandle, RuntimeClient

from .errors import ErrorCode, RetryHint, SDKError
from .signer_handle import SignerHandle, signer_handle_provenance_error
from .invocation import InvocationDraft, InvocationSignature


@dataclass(frozen=True)
class SignerPolicy:
    mode: str = ""
    signer_id: str = ""
    policy_ref: str = ""
    expires_at_unix_ms: int = 0


@dataclass(frozen=True)
class SigningMaterial:
    canonical_bytes_base64: str
    args_digest_hex: str
    expires_at_unix_ms: int
    algorithm: str = ""
    descriptor_ref: str = ""
    nonce_base64: str = ""
    signed_fields: tuple[str, ...] = field(default_factory=tuple)
    signer_policy: Optional[SignerPolicy] = None


@dataclass(frozen=True)
class PreparedInvocation:
    tuple: InvocationDraft
    signing_material: SigningMaterial
    prepared_id: str = ""
    request_id: str = ""
    descriptor_ref: str = ""
    descriptor_hash_hex: str = ""
    schema_hash_hex: str = ""
    canonical_hash_hex: str = ""
    expires_at_unix_ms: int = 0
    _runtime: Any = field(default=None, compare=False, repr=False)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PreparedInvocation":
        return _prepared_invocation_from_json(raw, require_prepared_id=True)

    def submit_ready(self) -> bool:
        return False

    def sign(self, signer: "Signer") -> "SignedInvocation":
        """Sign this prepared Invocation through an SDK signer workflow."""

        if signer is None:
            raise _invalid_prepared("signer is required")
        return signer.sign(self)

    def sign_with_caller_signature(
        self, signature: InvocationSignature
    ) -> "SignedInvocation":
        if signature.algorithm.strip() == "":
            raise _invalid_prepared("signature.algorithm is required")
        if signature.signature_base64.strip() == "":
            raise _invalid_prepared("signature.signature_base64 is required")
        signer_id = signature.key_id_hint or ""
        if self.signing_material.signer_policy and self.signing_material.signer_policy.signer_id:
            signer_id = self.signing_material.signer_policy.signer_id
        if signer_id.strip() == "":
            raise _invalid_prepared("signer id is required")
        return SignedInvocation(
            prepared=self,
            signature=signature,
            signer_id=signer_id,
            policy=self.signing_material.signer_policy,
            _runtime=self._runtime,
        )

    def _bind_runtime(self, runtime: object) -> "PreparedInvocation":
        return replace(self, tuple=self.tuple._bind_runtime(runtime), _runtime=runtime)


class SignatureProvider(Protocol):
    """Produces caller signatures over canonical signing material."""

    def sign(
        self, material: SigningMaterial, handle: SignerHandle
    ) -> InvocationSignature:
        ...


@dataclass(frozen=True)
class Signer:
    """SDK signer workflow object over a provider-authorized signer handle."""

    handle: SignerHandle
    provider: SignatureProvider

    def sign(self, prepared: PreparedInvocation) -> "SignedInvocation":
        if prepared is None:
            raise _invalid_prepared("prepared invocation is required")
        signature = self.sign_material(prepared.signing_material)
        return self.sign_with_signature(prepared, signature)

    def sign_material(self, material: SigningMaterial) -> InvocationSignature:
        """Sign stateless canonical material for direct stream dispatch."""

        if material is None:
            raise _invalid_prepared("signing material is required")
        if self.handle is None:
            raise _invalid_prepared("signer handle is required")
        if self.provider is None:
            raise _invalid_prepared("signature provider is required")
        _validate_signer_handle(self.handle)
        _validate_signer_policy(material.signer_policy, self.handle)
        signature = self.provider.sign(material, self.handle)
        if not isinstance(signature, InvocationSignature):
            raise _invalid_prepared(
                "signature provider must return InvocationSignature"
            )
        return _normalize_signature(self.handle, signature)

    def sign_with_signature(
        self, prepared: PreparedInvocation, signature: InvocationSignature
    ) -> "SignedInvocation":
        _validate_signer_handle(self.handle)
        _validate_signer_policy(prepared.signing_material.signer_policy, self.handle)
        normalized = _normalize_signature(self.handle, signature)
        signed = prepared.sign_with_caller_signature(normalized)
        return replace(signed, signer_id=self.handle.signer_id)


@dataclass(frozen=True)
class SignedInvocation:
    prepared: PreparedInvocation
    signature: InvocationSignature
    signer_id: str
    policy: Optional[SignerPolicy] = None
    _runtime: Any = field(default=None, compare=False, repr=False)

    def submit_ready(self) -> bool:
        return (
            self.signer_id.strip() != ""
            and self.signature.algorithm.strip() != ""
            and self.signature.signature_base64.strip() != ""
            and self.prepared.descriptor_ref.strip() != ""
            and self.prepared.signing_material.canonical_bytes_base64.strip() != ""
        )

    def to_json_dict(self) -> dict[str, object]:
        if not self.submit_ready():
            raise _invalid_prepared("signed invocation is not submit-ready")
        value: dict[str, object] = {
            "signer_id": self.signer_id,
            "prepared": {
                "prepared_id": self.prepared.prepared_id,
                "request_id": self.prepared.request_id,
                "descriptor_ref": self.prepared.descriptor_ref,
                "canonical_hash_hex": self.prepared.canonical_hash_hex,
                "expires_at_unix_ms": self.prepared.expires_at_unix_ms,
                "canonical_bytes_base64": (
                    self.prepared.signing_material.canonical_bytes_base64
                ),
                "tuple": self.prepared.tuple.to_json_dict(),
            },
            "signature": self.signature.to_json_dict(),
        }
        if self.policy is not None:
            value["policy"] = {
                "mode": self.policy.mode,
                "signer_id": self.policy.signer_id,
                "policy_ref": self.policy.policy_ref,
                "expires_at_unix_ms": self.policy.expires_at_unix_ms,
            }
        return value

    def to_json(self) -> str:
        return json.dumps(self.to_json_dict(), separators=(",", ":"), sort_keys=True)

    def to_invocation_draft(self) -> InvocationDraft:
        """Project the signed workflow envelope to the canonical Invocation DTO.

        Submit keeps the complete signed envelope for handle/audit admission.
        Stream and bidi transports accept only the invocation tuple with
        caller_signature attached, so signer policy and prepared-workflow
        fields must not cross that boundary.
        """

        if not self.submit_ready():
            raise _invalid_prepared("signed invocation is not submit-ready")
        return replace(self.prepared.tuple, caller_signature=self.signature)

    def submit(self) -> "InvocationHandle":
        """Submit this signed Invocation through its bound RuntimeClient."""

        return _require_runtime(self._runtime).submit_signed(self)

    def _bind_runtime(self, runtime: object) -> "SignedInvocation":
        return replace(
            self,
            prepared=self.prepared._bind_runtime(runtime),
            _runtime=runtime,
        )


def _signing_material(decoded: Mapping[str, object]) -> SigningMaterial:
    canonical_bytes = _required_string(decoded, "canonical_bytes_base64")
    _decode_base64_field(canonical_bytes, "canonical_bytes_base64")
    args_digest = _required_string(decoded, "args_digest_hex")
    expires = _required_int(decoded, "expires_at_unix_ms")
    descriptor_ref = _required_string(decoded, "descriptor_ref")
    signed_fields = decoded.get("signed_fields", [])
    if not isinstance(signed_fields, list) or any(
        not isinstance(item, str) for item in signed_fields
    ):
        raise _invalid_prepared("signed_fields must be an array of strings")
    policy_raw = decoded.get("signer_policy")
    policy = _signer_policy(policy_raw) if policy_raw is not None else None
    return SigningMaterial(
        canonical_bytes_base64=canonical_bytes,
        args_digest_hex=args_digest,
        expires_at_unix_ms=expires,
        algorithm=_optional_string(decoded.get("algorithm"), "algorithm") or "",
        descriptor_ref=descriptor_ref,
        nonce_base64=_optional_string(decoded.get("nonce_base64"), "nonce_base64") or "",
        signed_fields=tuple(signed_fields),
        signer_policy=policy,
    )


def signing_material_from_prepare_json(raw: bytes | str) -> SigningMaterial:
    """Decode a stateless prepare projection without a retained native handle."""
    return _prepared_invocation_from_json(raw, require_prepared_id=False).signing_material


def _prepared_invocation_from_json(
    raw: bytes | str, *, require_prepared_id: bool
) -> PreparedInvocation:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_prepared(f"decode prepared invocation JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_prepared("prepared invocation JSON must be an object")
    if decoded.get("submit_ready") is True:
        raise _invalid_prepared("PreparedInvocation must not be submit-ready")

    draft = InvocationDraft.from_json(json.dumps(_required_object(decoded, "tuple")))
    material = _signing_material(_required_object(decoded, "signing_material"))
    if material.descriptor_ref != draft.descriptor_ref:
        raise _invalid_prepared(
            "signing_material.descriptor_ref must match tuple descriptor_ref"
        )
    prepared_id = _optional_string(decoded.get("prepared_id"), "prepared_id") or ""
    request_id = _optional_string(decoded.get("request_id"), "request_id") or ""
    if require_prepared_id and prepared_id == "":
        raise _invalid_prepared("prepared_id is required")
    descriptor_ref = _required_string(decoded, "descriptor_ref")
    if descriptor_ref == "":
        raise _invalid_prepared("descriptor_ref is required")
    if descriptor_ref != material.descriptor_ref:
        raise _invalid_prepared(
            "signing_material.descriptor_ref must match tuple descriptor_ref"
        )
    expires_at_unix_ms = _required_int(decoded, "expires_at_unix_ms")
    if expires_at_unix_ms != material.expires_at_unix_ms:
        raise _invalid_prepared(
            "expires_at_unix_ms must match signing_material.expires_at_unix_ms"
        )
    canonical_hash_hex = _optional_string(
        decoded.get("canonical_hash_hex"), "canonical_hash_hex"
    ) or ""
    _validate_canonical_material_hash(
        material.canonical_bytes_base64,
        canonical_hash_hex,
    )
    return PreparedInvocation(
        tuple=draft,
        signing_material=material,
        prepared_id=prepared_id,
        request_id=request_id,
        descriptor_ref=descriptor_ref,
        descriptor_hash_hex=_optional_string(
            decoded.get("descriptor_hash_hex"), "descriptor_hash_hex"
        )
        or "",
        schema_hash_hex=_optional_string(decoded.get("schema_hash_hex"), "schema_hash_hex")
        or "",
        canonical_hash_hex=canonical_hash_hex,
        expires_at_unix_ms=expires_at_unix_ms,
    )


def _signer_policy(value: object) -> SignerPolicy:
    if not isinstance(value, dict):
        raise _invalid_prepared("signer_policy must be an object")
    policy = SignerPolicy(
        mode=_optional_string(value.get("mode"), "mode") or "",
        signer_id=_optional_string(value.get("signer_id"), "signer_id") or "",
        policy_ref=_optional_string(value.get("policy_ref"), "policy_ref") or "",
        expires_at_unix_ms=_optional_int(
            value.get("expires_at_unix_ms"), "expires_at_unix_ms"
        )
        or 0,
    )
    if policy.mode.strip() == "provider_managed_signing":
        if policy.signer_id.strip() == "":
            raise _invalid_prepared(
                "provider-managed signer_policy requires signer_id"
            )
        if policy.policy_ref.strip() == "":
            raise _invalid_prepared(
                "provider-managed signer_policy requires policy_ref"
            )
    return policy


def _validate_signer_handle(handle: SignerHandle) -> None:
    if handle is None:
        raise _invalid_prepared("signer handle is required")
    if handle.signer_id.strip() == "":
        raise _invalid_prepared("signer handle signer_id is required")
    if handle.key_id.strip() == "":
        raise _invalid_prepared("signer handle key_id is required")
    if handle.owner_ura.strip() == "":
        raise _invalid_prepared("signer handle owner_ura is required")
    error = signer_handle_provenance_error(handle)
    if error:
        raise _invalid_prepared(error)


def _validate_signer_policy(
    policy: SignerPolicy | None, handle: SignerHandle
) -> None:
    if policy is None:
        return
    if policy.signer_id and policy.signer_id != handle.signer_id:
        raise _invalid_prepared("prepared signer policy does not match signer handle")
    handle_mode = handle.policy.get("mode")
    if (
        policy.mode
        and isinstance(handle_mode, str)
        and handle_mode
        and policy.mode != handle_mode
    ):
        raise _invalid_prepared("prepared signer policy mode does not match handle")


def _normalize_signature(
    handle: SignerHandle, signature: InvocationSignature
) -> InvocationSignature:
    algorithm = signature.algorithm or handle.algorithm
    if algorithm.strip() == "":
        raise _invalid_prepared("signature.algorithm is required")
    if handle.algorithm and algorithm != handle.algorithm:
        raise _invalid_prepared("signature algorithm does not match signer handle")
    public_key_base64 = _handle_public_key_base64(handle)
    if signature.signer_public_key_base64:
        if public_key_base64 and signature.signer_public_key_base64 != public_key_base64:
            raise _invalid_prepared("signature public key does not match signer handle")
        public_key_base64 = signature.signer_public_key_base64
    if not public_key_base64:
        raise _invalid_prepared("signer handle public_key_base64 is required")
    key_id_hint = signature.key_id_hint or public_key_base64
    if key_id_hint not in {handle.signer_id, handle.key_id, public_key_base64}:
        raise _invalid_prepared("signature key_id_hint does not match signer handle")
    return InvocationSignature(
        algorithm=algorithm,
        signature_base64=signature.signature_base64,
        key_id_hint=public_key_base64,
        signer_public_key_base64=public_key_base64,
    )


def _handle_public_key_base64(handle: SignerHandle) -> str:
    public_key = handle.metadata.get("public_key_base64")
    if not isinstance(public_key, str) or not public_key.strip():
        return ""
    return public_key.strip()


def _validate_canonical_material_hash(
    canonical_bytes_base64: str, canonical_hash_hex: str
) -> None:
    if canonical_hash_hex == "":
        return
    canonical_hash = _normalize_optional_sha256_hex(
        canonical_hash_hex,
        "canonical_hash_hex",
    )
    canonical_bytes = _decode_base64_field(
        canonical_bytes_base64,
        "canonical_bytes_base64",
    )
    actual = hashlib.sha256(canonical_bytes).hexdigest()
    if actual != canonical_hash:
        raise _invalid_prepared(
            "canonical_hash_hex does not match canonical_bytes_base64"
        )


def _normalize_optional_sha256_hex(value: str, field_name: str) -> str:
    raw = value[7:] if value.startswith("sha256:") else value
    if len(raw) != 64:
        raise _invalid_prepared(f"{field_name} must be a sha256 hex digest")
    try:
        bytes.fromhex(raw)
    except ValueError as exc:
        raise _invalid_prepared(f"{field_name} must be hex", exc) from exc
    return raw.lower()


def _decode_base64_field(value: str, field_name: str) -> bytes:
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_prepared(f"{field_name} is required")
    try:
        decoded = base64.b64decode(value, validate=True)
    except binascii.Error as exc:
        raise _invalid_prepared(f"{field_name} must be base64", exc) from exc
    if base64.b64encode(decoded).decode("ascii") != value:
        raise _invalid_prepared(f"{field_name} must be canonical base64")
    return decoded


def _required_object(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_prepared(f"{field_name} must be an object")
    return value


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_prepared(f"{field_name} is required")
    return value


def _required_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_prepared(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_prepared(f"{field_name} must be a string or null")
    return value


def _optional_int(value: object, field_name: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_prepared(f"{field_name} must be an integer or null")
    return value


def _invalid_prepared(
    message: str, cause: Optional[BaseException] = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="prepare",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _require_runtime(runtime: object | None) -> "RuntimeClient":
    if runtime is None:
        raise SDKError(
            code=ErrorCode.INVALID_HANDLE,
            stage="prepare",
            retry=RetryHint.NEVER,
            retryable=False,
            message="invocation is not bound to a RuntimeClient",
        )
    return cast("RuntimeClient", runtime)
