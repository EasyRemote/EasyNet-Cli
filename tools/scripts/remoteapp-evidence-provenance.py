#!/usr/bin/env python3
"""Validate RemoteApp evidence provenance and signed live campaigns.

Plain ``evidence_origin`` remains a diagnostic classification for individual
domain runners. It is never sufficient for a product-complete claim. The
``verify-campaign`` boundary accepts only DSSE Ed25519 envelopes rooted in an
external trust bundle and binds every report/artifact to one campaign, run,
source revision, and build identity.

A run-mode invocation never mints live provenance: it can only preserve a
runner declaration, while the signed campaign boundary proves live authority.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import time
from typing import Any
import uuid


LIVE_RUNNER = "live_runner"
CONTRACT_SELF_TEST = "contract_self_test"
CAMPAIGN_BUNDLE_SCHEMA = "easynet.remoteapp.campaign-bundle.v2"
CAMPAIGN_SCHEMA = "easynet.remoteapp.campaign.v2"
ATTESTATION_SCHEMA = "easynet.remoteapp.live-attestation.v2"
TRUST_SCHEMA = "easynet.remoteapp.attestation-trust.v3"
CAMPAIGN_PAYLOAD_TYPE = "application/vnd.easynet.remoteapp.campaign.v2+json"
ATTESTATION_PAYLOAD_TYPE = (
    "application/vnd.easynet.remoteapp.live-attestation.v2+json"
)
SHA256_PATTERN = re.compile(r"^sha256:[0-9a-f]{64}$")
GIT_COMMIT_PATTERN = re.compile(r"^[0-9a-f]{40}$")
TRUSTED_ROLES = frozenset(
    ("campaign_authority", "observer_runner", "product_completion_authority")
)
RECEIPT_PROOF_SET_SCHEMA = "easynet.remoteapp.receipt-proof-set.v2"
MAX_CAMPAIGN_ARGUMENTS_BYTES = 1024 * 1024
CAMPAIGN_NONCE_DOMAIN = b"easynet.remoteapp.campaign-invocation-nonce.v1\0"


def expected_origin(mode: str) -> str:
    if mode == "run":
        return LIVE_RUNNER
    if mode == "self-test":
        return CONTRACT_SELF_TEST
    raise ValueError(f"unsupported evidence mode: {mode!r}")


def read_object(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise ValueError(f"{label} is not valid JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise ValueError(f"{label} must contain a JSON object")
    return value


def verify_evidence(mode: str, path: Path) -> tuple[dict[str, Any], str]:
    evidence = read_object(path, "evidence")
    expected = expected_origin(mode)
    observed = evidence.get("evidence_origin")
    if observed != expected:
        raise ValueError(
            f"evidence_origin must be {expected}; observed {observed!r}"
        )
    return evidence, expected


def write_object_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2, sort_keys=True)
            output.write("\n")
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be a JSON object")
    return value


def require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label} must be a non-empty string")
    return value


def require_int(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{label} must be an integer")
    return value


def require_uuid(value: Any, label: str) -> str:
    text = require_string(value, label)
    try:
        parsed = uuid.UUID(text)
    except ValueError as exc:
        raise ValueError(f"{label} must be a UUID") from exc
    if str(parsed) != text.lower():
        raise ValueError(f"{label} must use canonical UUID spelling")
    return text.lower()


def decode_base64(value: Any, label: str) -> bytes:
    text = require_string(value, label)
    try:
        decoded = base64.b64decode(text, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise ValueError(f"{label} must be canonical base64") from exc
    if base64.b64encode(decoded).decode("ascii") != text:
        raise ValueError(f"{label} must use canonical padded base64")
    return decoded


def sha256_bytes(body: bytes) -> str:
    return f"sha256:{hashlib.sha256(body).hexdigest()}"


def derive_campaign_invocation_nonce(
    campaign: dict[str, str], proof: dict[str, Any]
) -> str:
    challenge = decode_base64(
        campaign.get("challenge_nonce_b64"), "campaign.challenge_nonce_b64"
    )
    if len(challenge) != 32:
        raise ValueError("campaign.challenge_nonce_b64 must contain exactly 32 bytes")
    parts = (
        campaign["campaign_id"].encode(),
        campaign["run_id"].encode(),
        challenge,
        campaign["domain_id"].encode(),
        campaign["caller_device_ura"].encode(),
        campaign["provider_device_ura"].encode(),
        proof["proof_id"].encode(),
        proof["descriptor_ref"].encode(),
        proof["subject_ura"].encode(),
        proof["caller_ura"].encode(),
        proof["callee_ura"].encode(),
        proof["session_id"].encode(),
    )
    digest_input = bytearray(CAMPAIGN_NONCE_DOMAIN)
    for part in parts:
        if len(part) > 0xFFFFFFFF:
            raise ValueError("campaign invocation nonce part exceeds u32 length")
        digest_input.extend(len(part).to_bytes(4, "big"))
        digest_input.extend(part)
    return hashlib.sha256(digest_input).digest()[:16].hex()


def validate_receipt_proof_set(
    body: bytes,
    *,
    campaign_id: str,
    run_id: str,
    challenge_nonce_b64: str,
    domain_id: str,
    caller_device_ura: str,
    provider_device_ura: str,
    label: str,
) -> dict[str, Any]:
    """Validate signed proof expectations before Rust verifies receipt bytes.

    This function does not claim receipt cryptographic validity. It establishes
    the exact campaign/topology/argument tuple the Axon-backed verifier must
    return after checking signatures, finalization, payload digest and the
    challenge-derived invocation nonce.
    """

    try:
        value = json.loads(body.decode("utf-8"))
    except Exception as exc:
        raise ValueError(f"{label} is not UTF-8 JSON: {exc}") from exc
    proof_set = require_object(value, label)
    if set(proof_set) != {"schema", "campaign", "proofs"}:
        raise ValueError(f"{label} must contain exactly schema, campaign, and proofs")
    if proof_set.get("schema") != RECEIPT_PROOF_SET_SCHEMA:
        raise ValueError(f"{label}.schema must be {RECEIPT_PROOF_SET_SCHEMA!r}")
    campaign = require_object(proof_set.get("campaign"), f"{label}.campaign")
    expected_campaign = {
        "campaign_id": campaign_id,
        "run_id": run_id,
        "challenge_nonce_b64": challenge_nonce_b64,
        "domain_id": domain_id,
        "caller_device_ura": caller_device_ura,
        "provider_device_ura": provider_device_ura,
    }
    if campaign != expected_campaign:
        raise ValueError(f"{label}.campaign does not match signed campaign/attestation")
    proofs = proof_set.get("proofs")
    if not isinstance(proofs, list) or not proofs:
        raise ValueError(f"{label}.proofs must be a non-empty array")
    expected_fields = {
        "proof_id",
        "invocation_ura",
        "descriptor_ref",
        "subject_ura",
        "caller_ura",
        "callee_ura",
        "session_id",
        "arguments_b64",
        "encoding",
        "admission_receipt_b64",
        "terminal_receipt_b64",
    }
    seen_proof_ids: set[str] = set()
    seen_invocations: set[str] = set()
    projected: list[dict[str, Any]] = []
    for index, raw_proof in enumerate(proofs):
        proof_label = f"{label}.proofs[{index}]"
        proof = require_object(raw_proof, proof_label)
        if set(proof) != expected_fields:
            raise ValueError(f"{proof_label} has an unexpected field set")
        proof_id = require_string(proof.get("proof_id"), f"{proof_label}.proof_id")
        if proof_id in seen_proof_ids:
            raise ValueError(f"{proof_label}.proof_id is duplicated")
        seen_proof_ids.add(proof_id)
        invocation_ura = require_string(
            proof.get("invocation_ura"), f"{proof_label}.invocation_ura"
        )
        if not invocation_ura.startswith("easynet:///"):
            raise ValueError(f"{proof_label}.invocation_ura must be an EasyNet URA")
        if invocation_ura in seen_invocations:
            raise ValueError(f"{proof_label}.invocation_ura is duplicated")
        seen_invocations.add(invocation_ura)
        descriptor_ref = require_string(
            proof.get("descriptor_ref"), f"{proof_label}.descriptor_ref"
        )
        if "@" not in descriptor_ref or "#" not in descriptor_ref:
            raise ValueError(f"{proof_label}.descriptor_ref must be immutable and versioned")
        for field in ("subject_ura", "caller_ura", "callee_ura"):
            ura = require_string(proof.get(field), f"{proof_label}.{field}")
            if not ura.startswith("easynet:///"):
                raise ValueError(f"{proof_label}.{field} must be an EasyNet URA")
        session_id = require_string(proof.get("session_id"), f"{proof_label}.session_id")
        arguments = decode_base64(proof.get("arguments_b64"), f"{proof_label}.arguments_b64")
        if len(arguments) > MAX_CAMPAIGN_ARGUMENTS_BYTES:
            raise ValueError(
                f"{proof_label}.arguments_b64 exceeds {MAX_CAMPAIGN_ARGUMENTS_BYTES} bytes"
            )
        try:
            arguments_value = json.loads(arguments.decode("utf-8"))
        except Exception as exc:
            raise ValueError(f"{proof_label}.arguments_b64 is not UTF-8 JSON: {exc}") from exc
        arguments_object = require_object(arguments_value, f"{proof_label}.arguments")
        if arguments_object.get("session_id") != session_id:
            raise ValueError(f"{proof_label}.arguments session_id does not match proof")
        if proof.get("encoding") != "prost.base64":
            raise ValueError(f"{proof_label}.encoding must be 'prost.base64'")
        # Syntax validation is deliberate here; signature and protobuf
        # semantics remain exclusively owned by the Rust/Axon verifier.
        decode_base64(
            proof.get("admission_receipt_b64"),
            f"{proof_label}.admission_receipt_b64",
        )
        decode_base64(
            proof.get("terminal_receipt_b64"),
            f"{proof_label}.terminal_receipt_b64",
        )
        projected.append(
            {
                "proof_id": proof_id,
                "invocation_ura": invocation_ura,
                "descriptor_ref": descriptor_ref,
                "subject_ura": proof["subject_ura"],
                "caller_ura": proof["caller_ura"],
                "callee_ura": proof["callee_ura"],
                "session_id": session_id,
                "arguments_sha256": sha256_bytes(arguments),
                "campaign_invocation_nonce": derive_campaign_invocation_nonce(
                    expected_campaign, proof
                ),
            }
        )
    return {"campaign": expected_campaign, "proofs": projected}


def canonical_invocation_arguments(value: Any, label: str) -> bytes:
    arguments = require_object(value, label)
    return json.dumps(arguments, sort_keys=True, separators=(",", ":")).encode("utf-8")


def load_campaign_and_proof_binding(
    campaign_path: Path, proof_path: Path, arguments_path: Path
) -> tuple[dict[str, str], dict[str, Any], bytes]:
    campaign = read_object(campaign_path, "campaign invocation binding")
    expected_campaign_fields = {
        "campaign_id",
        "run_id",
        "challenge_nonce_b64",
        "domain_id",
        "caller_device_ura",
        "provider_device_ura",
    }
    if set(campaign) != expected_campaign_fields:
        raise ValueError("campaign invocation binding has an unexpected field set")
    require_uuid(campaign.get("campaign_id"), "campaign.campaign_id")
    require_uuid(campaign.get("run_id"), "campaign.run_id")
    challenge = decode_base64(
        campaign.get("challenge_nonce_b64"), "campaign.challenge_nonce_b64"
    )
    if len(challenge) != 32:
        raise ValueError("campaign.challenge_nonce_b64 must contain exactly 32 bytes")
    for field in ("domain_id", "caller_device_ura", "provider_device_ura"):
        require_string(campaign.get(field), f"campaign.{field}")
    proof = read_object(proof_path, "campaign proof binding")
    expected_proof_fields = {
        "proof_id",
        "descriptor_ref",
        "subject_ura",
        "caller_ura",
        "callee_ura",
        "session_id",
    }
    if set(proof) != expected_proof_fields:
        raise ValueError("campaign proof binding has an unexpected field set")
    for field in expected_proof_fields:
        require_string(proof.get(field), f"proof.{field}")
    if "@" not in proof["descriptor_ref"] or "#" not in proof["descriptor_ref"]:
        raise ValueError("proof.descriptor_ref must be immutable and versioned")
    for field in ("subject_ura", "caller_ura", "callee_ura"):
        if not proof[field].startswith("easynet:///"):
            raise ValueError(f"proof.{field} must be an EasyNet URA")
    arguments_value = read_object(arguments_path, "invocation arguments")
    if arguments_value.get("session_id") != proof["session_id"]:
        raise ValueError("invocation arguments session_id does not match proof binding")
    arguments = canonical_invocation_arguments(arguments_value, "invocation arguments")
    return campaign, proof, arguments


def append_campaign_receipt_proof(
    proof_set_path: Path,
    campaign_path: Path,
    proof_path: Path,
    arguments_path: Path,
    invocation_meta_path: Path,
) -> None:
    campaign, proof_binding, arguments = load_campaign_and_proof_binding(
        campaign_path, proof_path, arguments_path
    )
    meta = read_object(invocation_meta_path, "verified invocation metadata")
    if meta.get("metadata_state") != "finalization_checkpoints_verified":
        raise ValueError("invocation metadata is not finalization-checkpoint verified")
    if meta.get("ledger_state") != "completed":
        raise ValueError("invocation metadata does not prove a completed invocation")
    for meta_field, proof_field in (
        ("subject_ura", "subject_ura"),
        ("caller_ura", "caller_ura"),
        ("callee_ura", "callee_ura"),
    ):
        if meta.get(meta_field) != proof_binding[proof_field]:
            raise ValueError(f"invocation metadata {meta_field} does not match proof binding")
    meta_arguments = canonical_invocation_arguments(meta.get("args"), "invocation metadata args")
    if meta_arguments != arguments:
        raise ValueError("invocation metadata args do not match campaign arguments")
    exact_arguments = decode_base64(
        meta.get("arguments_b64"), "invocation metadata arguments_b64"
    )
    if len(exact_arguments) > MAX_CAMPAIGN_ARGUMENTS_BYTES:
        raise ValueError("invocation metadata arguments_b64 exceeds product evidence limit")
    try:
        exact_arguments_value = json.loads(exact_arguments.decode("utf-8"))
    except Exception as exc:
        raise ValueError(f"invocation metadata arguments_b64 is not UTF-8 JSON: {exc}") from exc
    if canonical_invocation_arguments(
        exact_arguments_value, "invocation metadata exact arguments"
    ) != meta_arguments:
        raise ValueError("invocation metadata arguments_b64 disagrees with args projection")
    expected_nonce = derive_campaign_invocation_nonce(campaign, proof_binding)
    if meta.get("nonce") != expected_nonce:
        raise ValueError("invocation metadata nonce is not derived from campaign challenge")
    invocation_ura = require_string(meta.get("invocation_ura"), "invocation metadata invocation_ura")
    if not invocation_ura.startswith("easynet:///"):
        raise ValueError("invocation metadata invocation_ura must be an EasyNet URA")
    receipt = require_object(meta.get("receipt"), "invocation metadata receipt")
    checkpoints = require_object(
        receipt.get("verification_checkpoints"),
        "invocation metadata receipt.verification_checkpoints",
    )
    if checkpoints.get("encoding") != "prost.base64":
        raise ValueError("invocation receipt checkpoint encoding must be prost.base64")
    admission = require_string(
        checkpoints.get("admission_receipt_b64"), "admission receipt checkpoint"
    )
    terminal = require_string(
        checkpoints.get("terminal_receipt_b64"), "terminal receipt checkpoint"
    )
    decode_base64(admission, "admission receipt checkpoint")
    decode_base64(terminal, "terminal receipt checkpoint")
    if proof_set_path.exists():
        proof_set = read_object(proof_set_path, "receipt proof set")
        if proof_set.get("schema") != RECEIPT_PROOF_SET_SCHEMA:
            raise ValueError("existing receipt proof set schema is not v2")
        if proof_set.get("campaign") != campaign:
            raise ValueError("existing receipt proof set belongs to another campaign")
        proofs = proof_set.get("proofs")
        if not isinstance(proofs, list):
            raise ValueError("existing receipt proof set proofs must be an array")
    else:
        proof_set = {
            "schema": RECEIPT_PROOF_SET_SCHEMA,
            "campaign": campaign,
            "proofs": [],
        }
        proofs = proof_set["proofs"]
    if any(row.get("proof_id") == proof_binding["proof_id"] for row in proofs):
        raise ValueError(f"receipt proof_id {proof_binding['proof_id']!r} already exists")
    if any(row.get("invocation_ura") == invocation_ura for row in proofs):
        raise ValueError(f"receipt invocation_ura {invocation_ura!r} already exists")
    proofs.append(
        {
            **proof_binding,
            "invocation_ura": invocation_ura,
            "arguments_b64": base64.b64encode(exact_arguments).decode("ascii"),
            "encoding": "prost.base64",
            "admission_receipt_b64": admission,
            "terminal_receipt_b64": terminal,
        }
    )
    write_object_atomic(proof_set_path, proof_set)


def dsse_pae(payload_type: str, payload: bytes) -> bytes:
    payload_type_bytes = payload_type.encode("utf-8")
    return b" ".join(
        (
            b"DSSEv1",
            str(len(payload_type_bytes)).encode("ascii"),
            payload_type_bytes,
            str(len(payload)).encode("ascii"),
            payload,
        )
    )


def load_trust_bundle(path: Path) -> dict[str, dict[str, Any]]:
    bundle = read_object(path, "attestation trust bundle")
    if set(bundle) != {"schema", "generation", "updated_at_ms", "keys"}:
        raise ValueError(
            "attestation trust bundle must contain exactly schema, generation, "
            "updated_at_ms, and keys"
        )
    if bundle.get("schema") != TRUST_SCHEMA:
        raise ValueError(
            f"attestation trust bundle schema must be {TRUST_SCHEMA!r}"
        )
    generation = require_int(bundle.get("generation"), "attestation trust generation")
    if generation < 1:
        raise ValueError("attestation trust generation must be positive")
    require_int(bundle.get("updated_at_ms"), "attestation trust updated_at_ms")
    rows = bundle.get("keys")
    if not isinstance(rows, list) or not rows:
        raise ValueError("attestation trust bundle keys must be a non-empty array")
    trusted: dict[str, dict[str, Any]] = {}
    for index, raw in enumerate(rows):
        row = require_object(raw, f"trust keys[{index}]")
        common_fields = {
            "keyid",
            "signer_ura",
            "roles",
            "public_key_pem",
            "not_before_ms",
            "not_after_ms",
            "revoked_at_ms",
        }
        keyid = require_string(row.get("keyid"), f"trust keys[{index}].keyid")
        if keyid in trusted:
            raise ValueError(f"duplicate trusted keyid {keyid!r}")
        roles = row.get("roles")
        if not isinstance(roles, list) or not roles or not all(
            isinstance(role, str) and role for role in roles
        ):
            raise ValueError(f"trust keys[{index}].roles must be a non-empty string array")
        if len(set(roles)) != len(roles):
            raise ValueError(f"trust keys[{index}].roles contains duplicates")
        unknown_roles = sorted(set(roles) - TRUSTED_ROLES)
        if unknown_roles:
            raise ValueError(
                f"trust keys[{index}].roles contains unsupported roles {unknown_roles}"
            )
        if len(roles) != 1:
            raise ValueError(
                f"trust keys[{index}] must not combine authority roles; exactly one is required"
            )
        domains: frozenset[str] = frozenset()
        platforms: frozenset[str] = frozenset()
        if "observer_runner" in roles:
            if set(row) != common_fields | {"domains", "platforms"}:
                raise ValueError(
                    f"trust keys[{index}] observer field set is not canonical"
                )
            raw_domains = row.get("domains")
            raw_platforms = row.get("platforms")
            if not isinstance(raw_domains, list) or not raw_domains or not all(
                isinstance(domain, str) and domain for domain in raw_domains
            ):
                raise ValueError(
                    f"trust keys[{index}].domains must be a non-empty string array "
                    "for observer_runner"
                )
            if not isinstance(raw_platforms, list) or not raw_platforms or not all(
                isinstance(platform, str) and platform for platform in raw_platforms
            ):
                raise ValueError(
                    f"trust keys[{index}].platforms must be a non-empty string array "
                    "for observer_runner"
                )
            if len(set(raw_domains)) != len(raw_domains):
                raise ValueError(f"trust keys[{index}].domains contains duplicates")
            if len(set(raw_platforms)) != len(raw_platforms):
                raise ValueError(f"trust keys[{index}].platforms contains duplicates")
            domains = frozenset(raw_domains)
            platforms = frozenset(raw_platforms)
        elif set(row) != common_fields:
            raise ValueError(f"trust keys[{index}] authority field set is not canonical")
        not_before_ms = require_int(
            row.get("not_before_ms"), f"trust keys[{index}].not_before_ms"
        )
        not_after_ms = require_int(
            row.get("not_after_ms"), f"trust keys[{index}].not_after_ms"
        )
        if not_after_ms <= not_before_ms:
            raise ValueError(
                f"trust keys[{index}].not_after_ms must be after not_before_ms"
            )
        revoked_at_ms = row.get("revoked_at_ms")
        if revoked_at_ms is not None:
            revoked_at_ms = require_int(
                revoked_at_ms, f"trust keys[{index}].revoked_at_ms"
            )
            if revoked_at_ms <= not_before_ms:
                raise ValueError(
                    f"trust keys[{index}].revoked_at_ms must be after not_before_ms"
                )
        trusted[keyid] = {
            "keyid": keyid,
            "signer_ura": require_string(
                row.get("signer_ura"), f"trust keys[{index}].signer_ura"
            ),
            "roles": frozenset(roles),
            "domains": domains,
            "platforms": platforms,
            "public_key_pem": require_string(
                row.get("public_key_pem"), f"trust keys[{index}].public_key_pem"
            ),
            "not_before_ms": not_before_ms,
            "not_after_ms": not_after_ms,
            "revoked_at_ms": revoked_at_ms,
        }
    return trusted


def require_trusted_key_active(
    key: dict[str, Any], *, signed_at_ms: int, observed_at_ms: int, label: str
) -> None:
    if not (key["not_before_ms"] <= signed_at_ms < key["not_after_ms"]):
        raise ValueError(f"{label} signing key was not valid at signed time")
    if observed_at_ms >= key["not_after_ms"]:
        raise ValueError(f"{label} signing key is expired")
    revoked_at_ms = key["revoked_at_ms"]
    if revoked_at_ms is not None and observed_at_ms >= revoked_at_ms:
        raise ValueError(f"{label} signing key is revoked")


def openssl_verify_ed25519(public_key_pem: str, message: bytes, signature: bytes) -> bool:
    with tempfile.TemporaryDirectory(prefix="remoteapp-attestation-") as directory:
        root = Path(directory)
        key_path = root / "public.pem"
        message_path = root / "message.bin"
        signature_path = root / "signature.bin"
        key_path.write_text(public_key_pem, encoding="utf-8")
        message_path.write_bytes(message)
        signature_path.write_bytes(signature)
        try:
            result = subprocess.run(
                [
                    "openssl",
                    "pkeyutl",
                    "-verify",
                    "-pubin",
                    "-inkey",
                    str(key_path),
                    "-rawin",
                    "-in",
                    str(message_path),
                    "-sigfile",
                    str(signature_path),
                ],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except FileNotFoundError as exc:
            raise ValueError("openssl is required to verify Ed25519 attestations") from exc
        return result.returncode == 0


def verify_dsse_envelope(
    envelope_value: Any,
    label: str,
    expected_payload_type: str,
    required_role: str,
    trusted: dict[str, dict[str, Any]],
) -> tuple[dict[str, Any], bytes, dict[str, Any]]:
    envelope = require_object(envelope_value, label)
    if envelope.get("payloadType") != expected_payload_type:
        raise ValueError(
            f"{label}.payloadType must be {expected_payload_type!r}"
        )
    payload = decode_base64(envelope.get("payload"), f"{label}.payload")
    signatures = envelope.get("signatures")
    if not isinstance(signatures, list) or not signatures:
        raise ValueError(f"{label}.signatures must be a non-empty array")
    pae = dsse_pae(expected_payload_type, payload)
    verified_key: dict[str, Any] | None = None
    signature_errors: list[str] = []
    for index, raw_signature in enumerate(signatures):
        signature = require_object(raw_signature, f"{label}.signatures[{index}]")
        keyid = require_string(
            signature.get("keyid"), f"{label}.signatures[{index}].keyid"
        )
        key = trusted.get(keyid)
        if key is None:
            signature_errors.append(f"unknown keyid {keyid!r}")
            continue
        if required_role not in key["roles"]:
            signature_errors.append(
                f"keyid {keyid!r} is not trusted for role {required_role!r}"
            )
            continue
        signature_bytes = decode_base64(
            signature.get("sig"), f"{label}.signatures[{index}].sig"
        )
        if openssl_verify_ed25519(key["public_key_pem"], pae, signature_bytes):
            verified_key = key
            break
        signature_errors.append(f"signature from keyid {keyid!r} is invalid")
    if verified_key is None:
        raise ValueError(f"{label} has no valid trusted signature: {signature_errors}")
    try:
        decoded = json.loads(payload.decode("utf-8"))
    except Exception as exc:
        raise ValueError(f"{label}.payload is not UTF-8 JSON: {exc}") from exc
    return require_object(decoded, f"{label}.payload"), payload, verified_key


def validate_digest(value: Any, label: str) -> str:
    text = require_string(value, label)
    if SHA256_PATTERN.fullmatch(text) is None:
        raise ValueError(f"{label} must be lowercase sha256:<64-hex>")
    return text


def resolve_attested_file(root: Path, relative_value: Any, label: str) -> Path:
    relative = Path(require_string(relative_value, f"{label}.path"))
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError(f"{label}.path must be a contained relative path")
    root = root.resolve(strict=True)
    candidate = root.joinpath(relative)
    cursor = root
    for part in relative.parts:
        cursor = cursor / part
        if cursor.is_symlink():
            raise ValueError(f"{label}.path must not traverse a symlink")
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(root)
    except (FileNotFoundError, ValueError) as exc:
        raise ValueError(f"{label}.path is missing or escapes the campaign root") from exc
    if not resolved.is_file():
        raise ValueError(f"{label}.path must resolve to a regular file")
    return resolved


def verify_attested_file(root: Path, value: Any, label: str) -> dict[str, Any]:
    row = require_object(value, label)
    path = resolve_attested_file(root, row.get("path"), label)
    expected_size = require_int(row.get("size_bytes"), f"{label}.size_bytes")
    body = path.read_bytes()
    actual_size = len(body)
    if expected_size != actual_size:
        raise ValueError(
            f"{label}.size_bytes mismatch: expected {expected_size}, observed {actual_size}"
        )
    expected_digest = validate_digest(row.get("sha256"), f"{label}.sha256")
    actual_digest = sha256_bytes(body)
    if expected_digest != actual_digest:
        raise ValueError(
            f"{label}.sha256 mismatch: expected {expected_digest}, observed {actual_digest}"
        )
    return {
        "path": str(path),
        "sha256": actual_digest,
        "size_bytes": actual_size,
    }


def _manifest_entries(verification: dict[str, Any]) -> dict[str, dict[str, Any]]:
    entries: dict[str, dict[str, Any]] = {}
    domains = require_object(verification.get("domains"), "verification.domains")
    for domain_id, raw_domain in domains.items():
        domain = require_object(raw_domain, f"verification.domains.{domain_id}")
        files = domain.get("verified_files")
        if not isinstance(files, list) or not files:
            raise ValueError(
                f"verification.domains.{domain_id}.verified_files must be non-empty"
            )
        for index, raw_file in enumerate(files):
            entry = require_object(
                raw_file, f"verification.domains.{domain_id}.verified_files[{index}]"
            )
            resolved = str(
                Path(require_string(entry.get("path"), "verified file path")).resolve(
                    strict=True
                )
            )
            previous = entries.get(resolved)
            if previous is not None and previous != entry:
                raise ValueError(f"conflicting signed manifest entries for {resolved}")
            entries[resolved] = entry
    return entries


def read_verified_bytes(
    verification: dict[str, Any], path: Path, label: str
) -> bytes:
    """Read one semantic artifact and re-check it against the signed manifest.

    The product aggregator must use this function for every top-level and nested
    report. Reading and hashing the same byte buffer closes the stat/read race;
    requiring a manifest entry prevents unsigned nested evidence injection.
    """

    try:
        resolved = path.resolve(strict=True)
    except FileNotFoundError as exc:
        raise ValueError(f"{label} is missing") from exc
    entry = _manifest_entries(verification).get(str(resolved))
    if entry is None:
        raise ValueError(f"{label} is not present in the signed evidence manifest")
    body = resolved.read_bytes()
    expected_size = require_int(entry.get("size_bytes"), f"{label}.size_bytes")
    if len(body) != expected_size:
        raise ValueError(
            f"{label}.size_bytes changed after attestation: expected "
            f"{expected_size}, observed {len(body)}"
        )
    expected_digest = validate_digest(entry.get("sha256"), f"{label}.sha256")
    actual_digest = sha256_bytes(body)
    if actual_digest != expected_digest:
        raise ValueError(
            f"{label}.sha256 changed after attestation: expected "
            f"{expected_digest}, observed {actual_digest}"
        )
    return body


def read_verified_json(
    verification: dict[str, Any], path: Path, label: str
) -> dict[str, Any]:
    body = read_verified_bytes(verification, path, label)
    try:
        value = json.loads(body.decode("utf-8"))
    except Exception as exc:
        raise ValueError(f"{label} is not UTF-8 JSON: {exc}") from exc
    return require_object(value, label)


def validate_system_authority_path(path: Path, *, directory: bool, label: str) -> Path:
    """Validate a pre-provisioned, non-caller-owned product authority path."""

    if not path.is_absolute():
        raise ValueError(f"{label} must be an absolute system path")
    cursor = Path(path.anchor)
    for part in path.parts[1:]:
        cursor = cursor / part
        if cursor.is_symlink():
            raise ValueError(f"{label} must not traverse a symlink")
    try:
        metadata = path.stat()
    except FileNotFoundError as exc:
        raise ValueError(f"{label} is not provisioned at {path}") from exc
    expected_kind = stat.S_ISDIR if directory else stat.S_ISREG
    if not expected_kind(metadata.st_mode):
        kind = "directory" if directory else "regular file"
        raise ValueError(f"{label} must be a {kind}")
    if os.name != "nt":
        if metadata.st_uid != 0:
            raise ValueError(f"{label} must be owned by root")
        if metadata.st_mode & 0o022:
            raise ValueError(f"{label} must not be group/other writable")
    else:
        raise ValueError(
            f"{label} Windows ACL verification requires the native release authority"
        )
    return path


def verify_campaign_bundle(
    bundle_path: Path,
    trust_bundle_path: Path,
    campaign_root: Path,
    expected_reports: dict[str, Path],
    now_ms: int | None = None,
) -> dict[str, Any]:
    trusted = load_trust_bundle(trust_bundle_path)
    bundle = read_object(bundle_path, "campaign bundle")
    if bundle.get("schema") != CAMPAIGN_BUNDLE_SCHEMA:
        raise ValueError(f"campaign bundle schema must be {CAMPAIGN_BUNDLE_SCHEMA!r}")
    campaign, campaign_bytes, campaign_key = verify_dsse_envelope(
        bundle.get("campaign"),
        "campaign",
        CAMPAIGN_PAYLOAD_TYPE,
        "campaign_authority",
        trusted,
    )
    if campaign.get("schema") != CAMPAIGN_SCHEMA:
        raise ValueError(f"campaign payload schema must be {CAMPAIGN_SCHEMA!r}")
    campaign_id = require_uuid(campaign.get("campaign_id"), "campaign.campaign_id")
    run_id = require_uuid(campaign.get("run_id"), "campaign.run_id")
    nonce = decode_base64(campaign.get("challenge_nonce"), "campaign.challenge_nonce")
    if len(nonce) != 32:
        raise ValueError("campaign.challenge_nonce must contain exactly 32 bytes")
    issued_at_ms = require_int(campaign.get("issued_at_ms"), "campaign.issued_at_ms")
    expires_at_ms = require_int(campaign.get("expires_at_ms"), "campaign.expires_at_ms")
    if expires_at_ms <= issued_at_ms:
        raise ValueError("campaign.expires_at_ms must be after issued_at_ms")
    observed_now_ms = int(time.time() * 1000) if now_ms is None else now_ms
    if observed_now_ms < issued_at_ms or observed_now_ms > expires_at_ms:
        raise ValueError("campaign is not valid at the current time")
    require_trusted_key_active(
        campaign_key,
        signed_at_ms=issued_at_ms,
        observed_at_ms=observed_now_ms,
        label="campaign",
    )
    source = require_object(campaign.get("source"), "campaign.source")
    git_commit = require_string(source.get("git_commit"), "campaign.source.git_commit")
    if GIT_COMMIT_PATTERN.fullmatch(git_commit) is None:
        raise ValueError("campaign.source.git_commit must be 40 lowercase hex characters")
    if source.get("dirty") is not False:
        raise ValueError("campaign.source.dirty must be false")
    build = require_object(campaign.get("build"), "campaign.build")
    for field in (
        "runtime_sha256",
        "remote_desktop_plugin_sha256",
        "frontend_bundle_sha256",
        "receipt_verifier_sha256",
    ):
        validate_digest(build.get(field), f"campaign.build.{field}")
    raw_receipt_signers = campaign.get("receipt_signers")
    if not isinstance(raw_receipt_signers, list) or not raw_receipt_signers:
        raise ValueError("campaign.receipt_signers must be a non-empty array")
    receipt_signers: list[dict[str, str]] = []
    seen_receipt_signer_keys: set[tuple[str, str]] = set()
    for index, raw_signer in enumerate(raw_receipt_signers):
        signer = require_object(raw_signer, f"campaign.receipt_signers[{index}]")
        signer_ura = require_string(
            signer.get("signer_ura"), f"campaign.receipt_signers[{index}].signer_ura"
        )
        if not signer_ura.startswith("easynet:///"):
            raise ValueError(
                f"campaign.receipt_signers[{index}].signer_ura must be an EasyNet URA"
            )
        public_key_b64 = require_string(
            signer.get("ed25519_public_key_b64"),
            f"campaign.receipt_signers[{index}].ed25519_public_key_b64",
        )
        public_key = decode_base64(
            public_key_b64,
            f"campaign.receipt_signers[{index}].ed25519_public_key_b64",
        )
        if len(public_key) != 32:
            raise ValueError(
                f"campaign.receipt_signers[{index}].ed25519_public_key_b64 must "
                "contain 32 bytes"
            )
        identity = (signer_ura, public_key_b64)
        if identity in seen_receipt_signer_keys:
            raise ValueError(f"campaign.receipt_signers[{index}] is duplicated")
        seen_receipt_signer_keys.add(identity)
        receipt_signers.append(
            {
                "signer_ura": signer_ura,
                "ed25519_public_key_b64": public_key_b64,
            }
        )
    required_domains = campaign.get("required_domains")
    if not isinstance(required_domains, list) or not all(
        isinstance(domain, str) and domain for domain in required_domains
    ):
        raise ValueError("campaign.required_domains must be a string array")
    if len(set(required_domains)) != len(required_domains):
        raise ValueError("campaign.required_domains contains duplicates")
    expected_domain_set = set(expected_reports)
    if set(required_domains) != expected_domain_set:
        raise ValueError(
            "campaign.required_domains does not exactly match product-completion requirements"
        )
    campaign_digest = sha256_bytes(campaign_bytes)
    attestations = bundle.get("attestations")
    if not isinstance(attestations, list):
        raise ValueError("campaign bundle attestations must be an array")
    verified_domains: dict[str, dict[str, Any]] = {}
    for index, envelope in enumerate(attestations):
        label = f"attestations[{index}]"
        attestation, attestation_bytes, signer = verify_dsse_envelope(
            envelope,
            label,
            ATTESTATION_PAYLOAD_TYPE,
            "observer_runner",
            trusted,
        )
        if attestation.get("schema") != ATTESTATION_SCHEMA:
            raise ValueError(f"{label}.payload schema must be {ATTESTATION_SCHEMA!r}")
        if attestation.get("campaign_sha256") != campaign_digest:
            raise ValueError(f"{label} is bound to another campaign")
        if require_uuid(attestation.get("run_id"), f"{label}.run_id") != run_id:
            raise ValueError(f"{label}.run_id does not match campaign.run_id")
        domain_id = require_string(attestation.get("domain_id"), f"{label}.domain_id")
        if domain_id not in expected_domain_set:
            raise ValueError(f"{label}.domain_id {domain_id!r} is not required")
        if domain_id in verified_domains:
            raise ValueError(f"duplicate domain attestation for {domain_id!r}")
        if domain_id not in signer["domains"]:
            raise ValueError(
                f"{label} signer {signer['keyid']!r} is not trusted for domain "
                f"{domain_id!r}"
            )
        started_at_ms = require_int(attestation.get("started_at_ms"), f"{label}.started_at_ms")
        completed_at_ms = require_int(
            attestation.get("completed_at_ms"), f"{label}.completed_at_ms"
        )
        if not (issued_at_ms <= started_at_ms <= completed_at_ms <= expires_at_ms):
            raise ValueError(f"{label} time range is outside the campaign window")
        require_trusted_key_active(
            signer,
            signed_at_ms=started_at_ms,
            observed_at_ms=observed_now_ms,
            label=label,
        )
        if attestation.get("source") != source:
            raise ValueError(f"{label}.source does not match campaign.source")
        if attestation.get("build") != build:
            raise ValueError(f"{label}.build does not match campaign.build")
        producer = require_object(attestation.get("producer"), f"{label}.producer")
        if producer.get("role") != "observer_runner":
            raise ValueError(f"{label}.producer.role must be 'observer_runner'")
        if producer.get("signer_ura") != signer["signer_ura"]:
            raise ValueError(f"{label}.producer.signer_ura does not match signing key trust")
        if producer.get("key_id") != signer["keyid"]:
            raise ValueError(f"{label}.producer.key_id does not match verified signature")
        producer_platform = require_string(
            producer.get("platform"), f"{label}.producer.platform"
        )
        if producer_platform not in signer["platforms"]:
            raise ValueError(
                f"{label} signer {signer['keyid']!r} is not trusted for platform "
                f"{producer_platform!r}"
            )
        if signer["keyid"] == campaign_key["keyid"] or (
            signer["signer_ura"] == campaign_key["signer_ura"]
        ):
            raise ValueError(
                f"{label} observer authority must be independent from campaign authority"
            )
        topology = require_object(attestation.get("topology"), f"{label}.topology")
        caller_device_ura = require_string(
            topology.get("caller_device_ura"), f"{label}.topology.caller_device_ura"
        )
        provider_device_ura = require_string(
            topology.get("provider_device_ura"), f"{label}.topology.provider_device_ura"
        )
        if not caller_device_ura.startswith("easynet:///") or not provider_device_ura.startswith(
            "easynet:///"
        ):
            raise ValueError(f"{label}.topology device identities must be EasyNet URAs")
        bindings = require_object(attestation.get("bindings"), f"{label}.bindings")
        if set(bindings) != {"receipt_proof"}:
            raise ValueError(f"{label}.bindings must contain only receipt_proof")
        receipt_proof_file = verify_attested_file(
            campaign_root,
            bindings.get("receipt_proof"),
            f"{label}.bindings.receipt_proof",
        )
        report_file = verify_attested_file(
            campaign_root, attestation.get("evidence"), f"{label}.evidence"
        )
        report_path = Path(report_file["path"])
        if report_path != expected_reports[domain_id].resolve(strict=True):
            raise ValueError(
                f"{label}.evidence.path does not match configured report for {domain_id!r}"
            )
        artifacts = attestation.get("artifacts", [])
        if not isinstance(artifacts, list):
            raise ValueError(f"{label}.artifacts must be an array")
        receipt_proof_path = Path(receipt_proof_file["path"])
        receipt_expectations = validate_receipt_proof_set(
            receipt_proof_path.read_bytes(),
            campaign_id=campaign_id,
            run_id=run_id,
            challenge_nonce_b64=campaign["challenge_nonce"],
            domain_id=domain_id,
            caller_device_ura=caller_device_ura,
            provider_device_ura=provider_device_ura,
            label=f"{label}.bindings.receipt_proof",
        )
        artifact_paths: set[Path] = {report_path, receipt_proof_path}
        if report_path == receipt_proof_path:
            raise ValueError(f"{label}.bindings.receipt_proof must be distinct from evidence")
        verified_files = [report_file, receipt_proof_file]
        for artifact_index, artifact in enumerate(artifacts):
            artifact_file = verify_attested_file(
                campaign_root,
                artifact,
                f"{label}.artifacts[{artifact_index}]",
            )
            artifact_path = Path(artifact_file["path"])
            if artifact_path in artifact_paths:
                raise ValueError(f"{label}.artifacts contains a duplicate path")
            artifact_paths.add(artifact_path)
            verified_files.append(artifact_file)
        verified_domains[domain_id] = {
            "attestation_sha256": sha256_bytes(attestation_bytes),
            "signer_ura": signer["signer_ura"],
            "keyid": signer["keyid"],
            "report_path": str(report_path),
            "receipt_proof_path": str(receipt_proof_path),
            "receipt_expectations": receipt_expectations,
            "verified_files": verified_files,
        }
    missing = sorted(expected_domain_set - set(verified_domains))
    if missing:
        raise ValueError(f"campaign bundle is missing domain attestations: {missing}")
    result = {
        "status": "attestation_verified_receipts_pending",
        "schema": CAMPAIGN_BUNDLE_SCHEMA,
        "campaign_id": campaign_id,
        "run_id": run_id,
        "campaign_sha256": campaign_digest,
        "campaign_signer_ura": campaign_key["signer_ura"],
        "campaign_keyid": campaign_key["keyid"],
        "issued_at_ms": issued_at_ms,
        "expires_at_ms": expires_at_ms,
        "source": source,
        "build": build,
        "receipt_signer_keyset": {
            "schema": "easynet.remoteapp.receipt-signer-keyset.v1",
            "keys": receipt_signers,
        },
        "domains": verified_domains,
        "all_receipts_verified": False,
    }
    result["replay_ledger_reserved"] = False
    return result


def campaign_replay_record(
    verification: dict[str, Any], verified_at_ms: int
) -> dict[str, Any]:
    if verification.get("status") != "verified" or verification.get(
        "all_receipts_verified"
    ) is not True:
        raise ValueError(
            "campaign replay may be reserved only after all Axon receipts are verified"
        )
    record = {
        "schema": "easynet.remoteapp.campaign-replay.v1",
        "campaign_id": verification["campaign_id"],
        "run_id": verification["run_id"],
        "campaign_sha256": verification["campaign_sha256"],
        "verified_at_ms": verified_at_ms,
    }
    for field in ("completion_statement_sha256", "final_report_sha256"):
        if field in verification:
            record[field] = validate_digest(verification[field], field)
    return record


def read_campaign_replay_record(replay_ledger_dir: Path, campaign_id: str) -> bytes:
    ledger_path = replay_ledger_dir / f"{campaign_id}.json"
    read_flags = os.O_RDONLY
    if hasattr(os, "O_NOFOLLOW"):
        read_flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(ledger_path, read_flags)
    except OSError as exc:
        raise ValueError(f"campaign {campaign_id} replay record cannot be read") from exc
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise ValueError("campaign replay record is not a regular file")
        with os.fdopen(descriptor, "rb") as existing_file:
            body = existing_file.read()
        descriptor = -1
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    return body


def verify_campaign_replay(
    replay_ledger_dir: Path,
    verification: dict[str, Any],
    verified_at_ms: int,
) -> None:
    expected = (
        json.dumps(
            campaign_replay_record(verification, verified_at_ms),
            indent=2,
            sort_keys=True,
        )
        + "\n"
    ).encode("utf-8")
    observed = read_campaign_replay_record(
        replay_ledger_dir, require_uuid(verification.get("campaign_id"), "campaign_id")
    )
    if observed != expected:
        raise ValueError("campaign replay record does not match finalized claim")


def reserve_campaign_replay(
    replay_ledger_dir: Path,
    verification: dict[str, Any],
    verified_at_ms: int,
    *,
    allow_exact_existing: bool = False,
) -> bool:
    replay_ledger_dir.mkdir(parents=True, exist_ok=True)
    if replay_ledger_dir.is_symlink():
        raise ValueError("campaign replay ledger directory must not be a symlink")
    ledger_path = replay_ledger_dir / f"{verification['campaign_id']}.json"
    body = (
        json.dumps(
            campaign_replay_record(verification, verified_at_ms),
            indent=2,
            sort_keys=True,
        )
        + "\n"
    ).encode("utf-8")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(ledger_path, flags, 0o600)
    except FileExistsError as exc:
        if allow_exact_existing:
            existing = read_campaign_replay_record(
                replay_ledger_dir, verification["campaign_id"]
            )
            if existing == body:
                return False
        raise ValueError(
            f"campaign {verification['campaign_id']} has already been consumed"
        ) from exc
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(body)
            output.flush()
            os.fsync(output.fileno())
        if os.name != "nt":
            directory_descriptor = os.open(replay_ledger_dir, os.O_RDONLY)
            try:
                os.fsync(directory_descriptor)
            finally:
                os.close(directory_descriptor)
    except BaseException:
        try:
            ledger_path.unlink()
        except FileNotFoundError:
            pass
        raise
    return True


def project_report(mode: str, evidence_path: Path, report_path: Path) -> None:
    _, origin = verify_evidence(mode, evidence_path)
    report = read_object(report_path, "report")
    if report.get("status") != "passed":
        raise ValueError("only a passed domain report may project evidence_origin")
    report["evidence_origin"] = origin
    write_object_atomic(report_path, report)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    verify = subparsers.add_parser("verify")
    verify.add_argument("--mode", choices=("run", "self-test"), required=True)
    verify.add_argument("--evidence", type=Path, required=True)

    project = subparsers.add_parser("project-report")
    project.add_argument("--mode", choices=("run", "self-test"), required=True)
    project.add_argument("--evidence", type=Path, required=True)
    project.add_argument("--report", type=Path, required=True)

    derive = subparsers.add_parser("derive-invocation-nonce")
    derive.add_argument("--campaign-binding", type=Path, required=True)
    derive.add_argument("--proof-binding", type=Path, required=True)
    derive.add_argument("--arguments-json", type=Path, required=True)

    append_proof = subparsers.add_parser("append-receipt-proof")
    append_proof.add_argument("--proof-set", type=Path, required=True)
    append_proof.add_argument("--campaign-binding", type=Path, required=True)
    append_proof.add_argument("--proof-binding", type=Path, required=True)
    append_proof.add_argument("--arguments-json", type=Path, required=True)
    append_proof.add_argument("--invocation-meta", type=Path, required=True)

    campaign = subparsers.add_parser("verify-campaign")
    campaign.add_argument("--bundle", type=Path, required=True)
    campaign.add_argument("--trust-bundle", type=Path, required=True)
    campaign.add_argument("--campaign-root", type=Path, required=True)
    campaign.add_argument(
        "--report",
        action="append",
        default=[],
        metavar="DOMAIN_ID=PATH",
        help="required domain report binding; repeat once per domain",
    )
    campaign.add_argument("--output", type=Path)

    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        if args.command == "verify":
            verify_evidence(args.mode, args.evidence)
        elif args.command == "project-report":
            project_report(args.mode, args.evidence, args.report)
        elif args.command == "derive-invocation-nonce":
            campaign, proof, arguments = load_campaign_and_proof_binding(
                args.campaign_binding, args.proof_binding, args.arguments_json
            )
            print(derive_campaign_invocation_nonce(campaign, proof))
        elif args.command == "append-receipt-proof":
            append_campaign_receipt_proof(
                args.proof_set,
                args.campaign_binding,
                args.proof_binding,
                args.arguments_json,
                args.invocation_meta,
            )
        else:
            reports: dict[str, Path] = {}
            for binding in args.report:
                domain_id, separator, path = binding.partition("=")
                if not separator or not domain_id or not path:
                    raise ValueError("--report must use DOMAIN_ID=PATH")
                if domain_id in reports:
                    raise ValueError(f"duplicate --report domain {domain_id!r}")
                reports[domain_id] = Path(path)
            if not reports:
                raise ValueError("verify-campaign requires at least one --report")
            result = verify_campaign_bundle(
                args.bundle,
                args.trust_bundle,
                args.campaign_root,
                reports,
            )
            if args.output is not None:
                write_object_atomic(args.output, result)
            else:
                print(json.dumps(result, indent=2, sort_keys=True))
    except ValueError as exc:
        raise SystemExit(f"remoteapp-evidence-provenance: {exc}") from exc


if __name__ == "__main__":
    main()
