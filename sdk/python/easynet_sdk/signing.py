"""Runtime Core signing-boundary DTOs."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Mapping, Optional

from .errors import ErrorCode, RetryHint, SDKError
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

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PreparedInvocation":
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
        material = _signing_material(
            _required_object(decoded, "signing_material"),
            draft.descriptor_ref,
        )
        prepared_id = _optional_string(decoded.get("prepared_id"), "prepared_id") or ""
        request_id = _optional_string(decoded.get("request_id"), "request_id") or ""
        if prepared_id == "" and request_id == "":
            raise _invalid_prepared("prepared_id or request_id is required")
        descriptor_ref = (
            _optional_string(decoded.get("descriptor_ref"), "descriptor_ref")
            or material.descriptor_ref
        )
        if descriptor_ref == "":
            raise _invalid_prepared("descriptor_ref is required")
        expires_at_unix_ms = _optional_int(
            decoded.get("expires_at_unix_ms"), "expires_at_unix_ms"
        ) or material.expires_at_unix_ms
        return cls(
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
            canonical_hash_hex=_optional_string(
                decoded.get("canonical_hash_hex"), "canonical_hash_hex"
            )
            or "",
            expires_at_unix_ms=expires_at_unix_ms,
        )

    def submit_ready(self) -> bool:
        return False

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
        if signer_id == "":
            signer_id = signature.signer_public_key_base64 or ""
        if signer_id.strip() == "":
            raise _invalid_prepared("signer id is required")
        return SignedInvocation(
            prepared=self,
            signature=signature,
            signer_id=signer_id,
            policy=self.signing_material.signer_policy,
        )


@dataclass(frozen=True)
class SignedInvocation:
    prepared: PreparedInvocation
    signature: InvocationSignature
    signer_id: str
    policy: Optional[SignerPolicy] = None

    def submit_ready(self) -> bool:
        return True


def _signing_material(
    decoded: Mapping[str, object], fallback_descriptor_ref: str
) -> SigningMaterial:
    canonical_bytes = _required_string(decoded, "canonical_bytes_base64")
    args_digest = _required_string(decoded, "args_digest_hex")
    expires = _required_int(decoded, "expires_at_unix_ms")
    descriptor_ref = (
        _optional_string(decoded.get("descriptor_ref"), "descriptor_ref")
        or fallback_descriptor_ref
    )
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


def _signer_policy(value: object) -> SignerPolicy:
    if not isinstance(value, dict):
        raise _invalid_prepared("signer_policy must be an object")
    return SignerPolicy(
        mode=_optional_string(value.get("mode"), "mode") or "",
        signer_id=_optional_string(value.get("signer_id"), "signer_id") or "",
        policy_ref=_optional_string(value.get("policy_ref"), "policy_ref") or "",
        expires_at_unix_ms=_optional_int(
            value.get("expires_at_unix_ms"), "expires_at_unix_ms"
        )
        or 0,
    )


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
