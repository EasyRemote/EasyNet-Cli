"""Generic provider-authorized signing handle."""

from __future__ import annotations

import base64
import binascii
import json
from collections.abc import Mapping
from dataclasses import dataclass

from .errors import ErrorCode, RetryHint, SDKError


@dataclass(frozen=True)
class SignerHandle:
    profile: str
    signer_id: str
    owner_ura: str
    key_id: str
    algorithm: str
    policy: Mapping[str, object]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SignerHandle":
        try:
            value = json.loads(raw)
        except Exception as exc:
            raise _invalid(f"decode signer handle: {exc}") from exc
        if not isinstance(value, dict):
            raise _invalid("signer handle must be an object")
        handle = cls(
            profile=_required(value, "profile"),
            signer_id=_required(value, "signer_id"),
            owner_ura=_required(value, "owner_ura"),
            key_id=_required(value, "key_id"),
            algorithm=_required(value, "algorithm"),
            policy=_mapping(value, "policy"),
            metadata=_mapping(value, "metadata"),
        )
        error = signer_handle_provenance_error(handle)
        if error:
            raise _invalid(error)
        return handle


def signer_handle_provenance_error(handle: SignerHandle) -> str:
    if handle.profile.strip() != "signing":
        return "signer handle profile is unsupported"
    mode = handle.policy.get("mode")
    source = handle.metadata.get("source")
    if source != "provider_key_inventory":
        return "signer handle source must be provider key inventory"
    if mode != "provider_managed_signing":
        return "signer handle policy mode is not supported"
    if handle.policy.get("usage") != "invocation.sign":
        return "signer handle policy usage is not supported"
    policy_signer_id = handle.policy.get("signer_id")
    if isinstance(policy_signer_id, str) and policy_signer_id and policy_signer_id != handle.signer_id:
        return "signer handle policy.signer_id must match signer_id"
    policy_ref = handle.policy.get("policy_ref")
    if not isinstance(policy_ref, str) or not policy_ref:
        return "signer handle policy_ref is required"
    metadata_policy_ref = handle.metadata.get("policy_ref")
    if isinstance(metadata_policy_ref, str) and metadata_policy_ref and metadata_policy_ref != policy_ref:
        return "signer handle metadata policy_ref must match policy.policy_ref"
    if handle.algorithm.strip().lower() != "ed25519":
        return "signer handle algorithm must be ed25519"
    owner = handle.policy.get("inventory_owner_ura")
    if owner != handle.owner_ura:
        return "signer handle inventory_owner_ura must match owner_ura"
    if handle.policy.get("key_state") != "active":
        return "signer handle key_state must be active"
    public_key = handle.metadata.get("public_key_base64")
    if public_key is not None and (not isinstance(public_key, str) or not _is_ed25519_public_key(public_key)):
        return "signer handle public_key_base64 must be a 32-byte Ed25519 public key"
    return ""


def _required(value: Mapping[str, object], key: str) -> str:
    item = value.get(key)
    if not isinstance(item, str) or not item.strip():
        raise _invalid(f"signer handle {key} is required")
    return item


def _mapping(value: Mapping[str, object], key: str) -> Mapping[str, object]:
    item = value.get(key)
    if not isinstance(item, dict):
        raise _invalid(f"signer handle {key} must be an object")
    return item


def _is_ed25519_public_key(value: str) -> bool:
    try:
        decoded = base64.b64decode(value, validate=True)
        return len(decoded) == 32 and base64.b64encode(decoded).decode("ascii") == value
    except (binascii.Error, ValueError):
        return False


def _invalid(message: str) -> SDKError:
    return SDKError(code=ErrorCode.INVALID_ARGUMENT, stage="signing", retry=RetryHint.NEVER, retryable=False, message=message)
