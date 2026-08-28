#!/usr/bin/env python3
"""Finalize a RemoteApp product-completion candidate under product authority.

The evidence aggregator deliberately emits only an eligible candidate. This
boundary verifies an independent DSSE completion decision, consumes the signed
campaign exactly once, and only then emits the product-complete report.
"""

from __future__ import annotations

import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import time
from typing import Any, NamedTuple


CANDIDATE_SCHEMA = "easynet.remoteapp.product-completion-candidate.v1"
FINAL_REPORT_SCHEMA = "easynet.remoteapp.product-completion-report.v1"
COMPLETION_STATEMENT_SCHEMA = "easynet.remoteapp.product-completion-attestation.v1"
COMPLETION_PAYLOAD_TYPE = (
    "application/vnd.easynet.remoteapp.product-completion.v1+json"
)
COMPLETION_ROLE = "product_completion_authority"
REQUIRED_PRODUCT_DOMAIN_IDS = frozenset(
    {
        "browser_transport_resume",
        "frontend_product_flow",
        "browser_lifecycle",
        "cross_device_smoke",
        "cross_device_remoteapp",
        "cross_platform_capture",
        "input_injection",
        "media_adaptation",
        "multi_window_tracking",
        "network_fallback",
        "session_timeout_window",
        "session_timeout_application",
        "session_cancel_window",
        "session_cancel_application",
        "permission_revoke_window",
        "permission_revoke_application",
        "session_resume_window",
        "session_resume_application",
        "crash_restart_recovery",
    }
)


class CompletionAuthorization(NamedTuple):
    candidate: dict[str, Any]
    candidate_body: bytes
    verification: dict[str, Any]
    envelope: dict[str, Any]
    completion_key: dict[str, Any]
    completion_issued_at_ms: int
    candidate_sha256: str
    statement_sha256: str


class CompletionCandidate(NamedTuple):
    candidate: dict[str, Any]
    candidate_body: bytes
    verification: dict[str, Any]
    campaign_id: str
    run_id: str
    campaign_sha256: str
    issued_at_ms: int
    expires_at_ms: int
    source: dict[str, Any]
    build: dict[str, Any]
    candidate_sha256: str
    signer_uras: frozenset[str]
    signer_keyids: frozenset[str]


def load_provenance() -> Any:
    path = Path(__file__).with_name("remoteapp-evidence-provenance.py")
    spec = importlib.util.spec_from_file_location("remoteapp_evidence_provenance", path)
    if spec is None or spec.loader is None:
        raise ValueError("cannot load RemoteApp campaign verifier")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def system_authority_paths() -> tuple[Path, Path]:
    if sys.platform == "darwin":
        root = Path("/Library/Application Support/EasyNet/remoteapp")
        return root / "attestation-trust.json", root / "campaign-replay"
    if sys.platform.startswith("linux"):
        return (
            Path("/etc/easynet/remoteapp-attestation-trust.json"),
            Path("/var/lib/easynet/remoteapp-campaign-replay"),
        )
    raise ValueError("product-completion authority is supported on macOS and Linux")


def read_regular_bytes(path: Path, label: str) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise ValueError(f"{label} cannot be read: {exc}") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"{label} must be a regular non-symlink file")
    return path.read_bytes()


def parse_object(body: bytes, label: str) -> dict[str, Any]:
    try:
        value = json.loads(body.decode("utf-8"))
    except Exception as exc:
        raise ValueError(f"{label} is not UTF-8 JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be a JSON object")
    return value


def canonical_bytes(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def prepare_output(path: Path, body: bytes, label: str = "output") -> Path | None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists() or path.is_symlink():
        if read_regular_bytes(path, label) != body:
            raise ValueError(f"{label} already exists with different content")
        return None
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(body)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise
    return temporary


def publish_prepared_output(
    path: Path, temporary: Path | None, label: str = "output"
) -> None:
    if temporary is None:
        return
    try:
        try:
            os.link(temporary, path)
        except FileExistsError:
            if read_regular_bytes(path, label) != temporary.read_bytes():
                raise ValueError(f"{label} changed during publication")
        temporary.unlink()
        if os.name != "nt":
            descriptor = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def validate_completion_candidate(
    candidate_body: bytes,
    observed_at_ms: int,
    *,
    require_current_authority: bool,
) -> CompletionCandidate:
    provenance = load_provenance()
    candidate = parse_object(candidate_body, "completion candidate")

    required_candidate = {
        "schema": CANDIDATE_SCHEMA,
        "status": "passed",
        "mode": "check",
        "evidence_origin": "live_runner",
        "product_complete_eligible": True,
        "product_complete_claim": False,
        "finalization_state": "completion_signature_pending",
        "contract_fixture_mode": False,
        "campaign_verified": True,
    }
    for field, expected in required_candidate.items():
        if candidate.get(field) != expected:
            raise ValueError(f"completion candidate {field} must be {expected!r}")
    if candidate.get("errors") != []:
        raise ValueError("completion candidate errors must be empty")
    if candidate.get("script") != "tools/scripts/remoteapp-product-completion-e2e.sh":
        raise ValueError("completion candidate script identity is not canonical")
    if candidate.get("reason") != (
        "all product evidence passed; completion authority signature pending"
    ):
        raise ValueError("completion candidate reason is not claim-eligible")
    required_evidence_count = provenance.require_int(
        candidate.get("required_evidence_count"),
        "completion candidate required_evidence_count",
    )
    if required_evidence_count != len(REQUIRED_PRODUCT_DOMAIN_IDS):
        raise ValueError("completion candidate required evidence count is incomplete")
    raw_checks = candidate.get("checks")
    if not isinstance(raw_checks, list) or len(raw_checks) != required_evidence_count:
        raise ValueError("completion candidate checks do not cover every product domain")
    check_ids: set[str] = set()
    for index, raw_check in enumerate(raw_checks):
        check = provenance.require_object(raw_check, f"completion candidate checks[{index}]")
        check_id = provenance.require_string(
            check.get("id"), f"completion candidate checks[{index}].id"
        )
        if check_id in check_ids:
            raise ValueError(f"completion candidate check {check_id!r} is duplicated")
        check_ids.add(check_id)
        if check.get("status") != "passed":
            raise ValueError(f"completion candidate check {check_id!r} did not pass")
        if check.get("evidence_origin") != "live_runner":
            raise ValueError(
                f"completion candidate check {check_id!r} is not live evidence"
            )
        if check.get("errors") != []:
            raise ValueError(f"completion candidate check {check_id!r} has errors")
    if check_ids != REQUIRED_PRODUCT_DOMAIN_IDS:
        raise ValueError("completion candidate check domain set is incomplete")

    verification = provenance.require_object(
        candidate.get("campaign_verification"),
        "completion candidate campaign_verification",
    )
    if verification.get("status") != "verified":
        raise ValueError("completion candidate campaign must be receipt-verified")
    if verification.get("schema") != provenance.CAMPAIGN_BUNDLE_SCHEMA:
        raise ValueError("completion candidate campaign schema is not canonical")
    if verification.get("all_receipts_verified") is not True:
        raise ValueError("completion candidate must verify all Axon receipts")
    if verification.get("replay_ledger_reserved") is not False:
        raise ValueError("completion candidate must not consume campaign replay state")

    campaign_id = provenance.require_uuid(
        verification.get("campaign_id"), "campaign_verification.campaign_id"
    )
    run_id = provenance.require_uuid(
        verification.get("run_id"), "campaign_verification.run_id"
    )
    campaign_sha256 = provenance.validate_digest(
        verification.get("campaign_sha256"), "campaign_verification.campaign_sha256"
    )
    campaign_signer_ura = provenance.require_string(
        verification.get("campaign_signer_ura"),
        "campaign_verification.campaign_signer_ura",
    )
    campaign_keyid = provenance.require_string(
        verification.get("campaign_keyid"), "campaign_verification.campaign_keyid"
    )
    issued_at_ms = provenance.require_int(
        verification.get("issued_at_ms"), "campaign_verification.issued_at_ms"
    )
    expires_at_ms = provenance.require_int(
        verification.get("expires_at_ms"), "campaign_verification.expires_at_ms"
    )
    if require_current_authority and not (
        issued_at_ms <= observed_at_ms < expires_at_ms
    ):
        raise ValueError("signed campaign is not active at completion observation time")
    source = provenance.require_object(
        verification.get("source"), "campaign_verification.source"
    )
    build = provenance.require_object(
        verification.get("build"), "campaign_verification.build"
    )

    candidate_sha256 = provenance.sha256_bytes(candidate_body)
    signer_uras = {campaign_signer_ura}
    signer_keyids = {campaign_keyid}
    domains = provenance.require_object(
        verification.get("domains"), "campaign_verification.domains"
    )
    if not domains:
        raise ValueError("campaign_verification.domains must not be empty")
    if set(domains) != REQUIRED_PRODUCT_DOMAIN_IDS or set(domains) != check_ids:
        raise ValueError("campaign verification domains do not match product checks")
    for domain_id, domain in domains.items():
        provenance.require_string(domain_id, "campaign_verification domain id")
        row = provenance.require_object(
            domain, f"campaign_verification.domains[{domain_id!r}]"
        )
        signer_uras.add(
            provenance.require_string(
                row.get("signer_ura"),
                f"campaign_verification.domains[{domain_id!r}].signer_ura",
            )
        )
        signer_keyids.add(
            provenance.require_string(
                row.get("keyid"),
                f"campaign_verification.domains[{domain_id!r}].keyid",
            )
        )
    return CompletionCandidate(
        candidate=candidate,
        candidate_body=candidate_body,
        verification=verification,
        campaign_id=campaign_id,
        run_id=run_id,
        campaign_sha256=campaign_sha256,
        issued_at_ms=issued_at_ms,
        expires_at_ms=expires_at_ms,
        source=source,
        build=build,
        candidate_sha256=candidate_sha256,
        signer_uras=frozenset(signer_uras),
        signer_keyids=frozenset(signer_keyids),
    )


def build_completion_statement(
    candidate: CompletionCandidate, issued_at_ms: int
) -> dict[str, Any]:
    if not candidate.issued_at_ms <= issued_at_ms < candidate.expires_at_ms:
        raise ValueError("completion statement time must be inside campaign validity")
    return {
        "schema": COMPLETION_STATEMENT_SCHEMA,
        "candidate_sha256": candidate.candidate_sha256,
        "campaign_id": candidate.campaign_id,
        "run_id": candidate.run_id,
        "campaign_sha256": candidate.campaign_sha256,
        "issued_at_ms": issued_at_ms,
        "decision": "product_complete",
        "source": candidate.source,
        "build": candidate.build,
    }


def prepare_completion_signing_material(
    candidate_path: Path,
    statement_path: Path,
    pae_path: Path,
    *,
    now_ms: int | None = None,
) -> dict[str, Any]:
    if statement_path == pae_path:
        raise ValueError("statement and PAE outputs must be distinct paths")
    provenance = load_provenance()
    observed_at_ms = int(time.time() * 1000) if now_ms is None else now_ms
    candidate = validate_completion_candidate(
        read_regular_bytes(candidate_path, "completion candidate"),
        observed_at_ms,
        require_current_authority=True,
    )
    statement = build_completion_statement(candidate, observed_at_ms)
    statement_body = json.dumps(
        statement, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")
    pae_body = provenance.dsse_pae(COMPLETION_PAYLOAD_TYPE, statement_body)
    statement_temporary: Path | None = None
    pae_temporary: Path | None = None
    try:
        statement_temporary = prepare_output(
            statement_path, statement_body, "completion statement output"
        )
        pae_temporary = prepare_output(
            pae_path, pae_body, "completion PAE output"
        )
        publish_prepared_output(
            statement_path, statement_temporary, "completion statement output"
        )
        statement_temporary = None
        publish_prepared_output(pae_path, pae_temporary, "completion PAE output")
        pae_temporary = None
    finally:
        if statement_temporary is not None:
            statement_temporary.unlink(missing_ok=True)
        if pae_temporary is not None:
            pae_temporary.unlink(missing_ok=True)
    return {
        "status": "signing_material_prepared",
        "schema": COMPLETION_STATEMENT_SCHEMA,
        "candidate_sha256": candidate.candidate_sha256,
        "statement_sha256": provenance.sha256_bytes(statement_body),
        "pae_sha256": provenance.sha256_bytes(pae_body),
        "issued_at_ms": observed_at_ms,
    }


def assemble_completion_attestation(
    candidate_path: Path,
    statement_path: Path,
    signature_path: Path,
    keyid: str,
    trust_path: Path,
    output_path: Path,
    *,
    now_ms: int | None = None,
) -> dict[str, Any]:
    provenance = load_provenance()
    observed_at_ms = int(time.time() * 1000) if now_ms is None else now_ms
    keyid = provenance.require_string(keyid, "completion signature keyid")
    candidate_body = read_regular_bytes(candidate_path, "completion candidate")
    statement_body = read_regular_bytes(statement_path, "completion statement")
    signature = read_regular_bytes(signature_path, "completion signature")
    if len(signature) != 64:
        raise ValueError("completion Ed25519 signature must contain exactly 64 bytes")
    envelope = {
        "payloadType": COMPLETION_PAYLOAD_TYPE,
        "payload": base64.b64encode(statement_body).decode("ascii"),
        "signatures": [
            {"keyid": keyid, "sig": base64.b64encode(signature).decode("ascii")}
        ],
    }
    authorization = verify_completion_authorization(
        candidate_body,
        envelope,
        trust_path,
        observed_at_ms,
        require_current_authority=True,
    )
    body = canonical_bytes(envelope)
    temporary = prepare_output(
        output_path, body, "completion attestation output"
    )
    publish_prepared_output(
        output_path, temporary, "completion attestation output"
    )
    return {
        "status": "completion_attestation_assembled",
        "schema": COMPLETION_STATEMENT_SCHEMA,
        "candidate_sha256": authorization.candidate_sha256,
        "statement_sha256": authorization.statement_sha256,
        "completion_signer_ura": authorization.completion_key["signer_ura"],
        "completion_keyid": authorization.completion_key["keyid"],
        "attestation_sha256": provenance.sha256_bytes(body),
    }


def verify_completion_authorization(
    candidate_body: bytes,
    envelope: dict[str, Any],
    trust_path: Path,
    observed_at_ms: int,
    *,
    require_current_authority: bool,
) -> CompletionAuthorization:
    provenance = load_provenance()
    candidate = validate_completion_candidate(
        candidate_body,
        observed_at_ms,
        require_current_authority=require_current_authority,
    )
    if set(envelope) != {"payloadType", "payload", "signatures"}:
        raise ValueError("completion attestation envelope field set is not canonical")
    signatures = envelope.get("signatures")
    if not isinstance(signatures, list) or len(signatures) != 1:
        raise ValueError("completion attestation must contain exactly one signature")
    if not isinstance(signatures[0], dict) or set(signatures[0]) != {"keyid", "sig"}:
        raise ValueError("completion attestation signature field set is not canonical")

    trusted = provenance.load_trust_bundle(trust_path)
    statement, statement_body, completion_key = provenance.verify_dsse_envelope(
        envelope,
        "completion attestation",
        COMPLETION_PAYLOAD_TYPE,
        COMPLETION_ROLE,
        trusted,
    )
    expected_statement_fields = {
        "schema",
        "candidate_sha256",
        "campaign_id",
        "run_id",
        "campaign_sha256",
        "issued_at_ms",
        "decision",
        "source",
        "build",
    }
    if set(statement) != expected_statement_fields:
        raise ValueError("completion statement field set is not canonical")
    expected_statement = {
        "schema": COMPLETION_STATEMENT_SCHEMA,
        "candidate_sha256": candidate.candidate_sha256,
        "campaign_id": candidate.campaign_id,
        "run_id": candidate.run_id,
        "campaign_sha256": candidate.campaign_sha256,
        "decision": "product_complete",
        "source": candidate.source,
        "build": candidate.build,
    }
    for field, expected in expected_statement.items():
        if statement.get(field) != expected:
            raise ValueError(f"completion statement {field} does not match candidate")
    completion_issued_at_ms = provenance.require_int(
        statement.get("issued_at_ms"), "completion statement issued_at_ms"
    )
    if not (
        candidate.issued_at_ms <= completion_issued_at_ms < candidate.expires_at_ms
        and completion_issued_at_ms <= observed_at_ms
    ):
        raise ValueError("completion statement time is outside campaign/current bounds")
    provenance.require_trusted_key_active(
        completion_key,
        signed_at_ms=completion_issued_at_ms,
        observed_at_ms=(
            observed_at_ms if require_current_authority else completion_issued_at_ms
        ),
        label="completion authority",
    )

    if (
        completion_key["signer_ura"] in candidate.signer_uras
        or completion_key["keyid"] in candidate.signer_keyids
    ):
        raise ValueError(
            "product completion authority must have independent key and signer identity"
        )
    return CompletionAuthorization(
        candidate=candidate.candidate,
        candidate_body=candidate.candidate_body,
        verification=candidate.verification,
        envelope=envelope,
        completion_key=completion_key,
        completion_issued_at_ms=completion_issued_at_ms,
        candidate_sha256=candidate.candidate_sha256,
        statement_sha256=provenance.sha256_bytes(statement_body),
    )


def build_final_report(authorization: CompletionAuthorization) -> dict[str, Any]:
    final_report = dict(authorization.candidate)
    finalized_verification = dict(authorization.verification)
    finalized_verification["replay_ledger_reserved"] = True
    final_report.update(
        {
            "schema": FINAL_REPORT_SCHEMA,
            "reason": "independent product completion authority verified",
            "product_complete_claim": True,
            "finalization_state": "completion_authority_verified",
            "candidate_b64": base64.b64encode(authorization.candidate_body).decode(
                "ascii"
            ),
            "candidate_report_sha256": authorization.candidate_sha256,
            "completion_statement_sha256": authorization.statement_sha256,
            "completion_attestation": authorization.envelope,
            "completion_signer_ura": authorization.completion_key["signer_ura"],
            "completion_keyid": authorization.completion_key["keyid"],
            "replay_ledger_reserved": True,
            "campaign_verification": finalized_verification,
        }
    )
    return final_report


def replay_verification_for_report(
    authorization: CompletionAuthorization, final_sha256: str
) -> dict[str, Any]:
    verification = dict(authorization.verification)
    verification.update(
        {
            "completion_statement_sha256": authorization.statement_sha256,
            "final_report_sha256": final_sha256,
        }
    )
    return verification


def finalize_candidate(
    candidate_path: Path,
    envelope_path: Path,
    trust_path: Path,
    replay_dir: Path,
    output_path: Path,
    *,
    now_ms: int | None = None,
) -> dict[str, Any]:
    provenance = load_provenance()
    observed_at_ms = int(time.time() * 1000) if now_ms is None else now_ms
    authorization = verify_completion_authorization(
        read_regular_bytes(candidate_path, "completion candidate"),
        parse_object(
            read_regular_bytes(envelope_path, "completion attestation"),
            "completion attestation",
        ),
        trust_path,
        observed_at_ms,
        require_current_authority=True,
    )
    final_report = build_final_report(authorization)
    final_body = canonical_bytes(final_report)
    final_sha256 = provenance.sha256_bytes(final_body)
    temporary = prepare_output(output_path, final_body)
    replay_verification = replay_verification_for_report(
        authorization, final_sha256
    )
    try:
        provenance.reserve_campaign_replay(
            replay_dir,
            replay_verification,
            authorization.completion_issued_at_ms,
            allow_exact_existing=True,
        )
        publish_prepared_output(output_path, temporary)
    except BaseException:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
        raise
    return final_report


def verify_final_report(
    report_path: Path,
    trust_path: Path,
    replay_dir: Path,
    *,
    now_ms: int | None = None,
) -> dict[str, Any]:
    provenance = load_provenance()
    observed_at_ms = int(time.time() * 1000) if now_ms is None else now_ms
    final_body = read_regular_bytes(report_path, "product completion report")
    report = parse_object(final_body, "product completion report")
    if report.get("schema") != FINAL_REPORT_SCHEMA:
        raise ValueError(f"product completion report schema must be {FINAL_REPORT_SCHEMA!r}")
    candidate_body = provenance.decode_base64(
        report.get("candidate_b64"), "product completion report candidate_b64"
    )
    envelope = provenance.require_object(
        report.get("completion_attestation"),
        "product completion report completion_attestation",
    )
    authorization = verify_completion_authorization(
        candidate_body,
        envelope,
        trust_path,
        observed_at_ms,
        require_current_authority=False,
    )
    expected_report = build_final_report(authorization)
    if report != expected_report:
        raise ValueError("product completion report is not the canonical authorized projection")
    final_sha256 = provenance.sha256_bytes(final_body)
    replay_verification = replay_verification_for_report(
        authorization, final_sha256
    )
    provenance.verify_campaign_replay(
        replay_dir,
        replay_verification,
        authorization.completion_issued_at_ms,
    )
    return {
        "status": "verified",
        "schema": FINAL_REPORT_SCHEMA,
        "campaign_id": authorization.verification["campaign_id"],
        "run_id": authorization.verification["run_id"],
        "candidate_sha256": authorization.candidate_sha256,
        "completion_statement_sha256": authorization.statement_sha256,
        "final_report_sha256": final_sha256,
        "completion_signer_ura": authorization.completion_key["signer_ura"],
        "completion_keyid": authorization.completion_key["keyid"],
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare = subparsers.add_parser("prepare")
    prepare.add_argument("--candidate", type=Path, required=True)
    prepare.add_argument("--statement", type=Path, required=True)
    prepare.add_argument("--pae", type=Path, required=True)
    assemble = subparsers.add_parser("assemble")
    assemble.add_argument("--candidate", type=Path, required=True)
    assemble.add_argument("--statement", type=Path, required=True)
    assemble.add_argument("--signature", type=Path, required=True)
    assemble.add_argument("--keyid", required=True)
    assemble.add_argument("--output", type=Path, required=True)
    finalize = subparsers.add_parser("finalize")
    finalize.add_argument("--candidate", type=Path, required=True)
    finalize.add_argument("--attestation", type=Path, required=True)
    finalize.add_argument("--output", type=Path, required=True)
    verify = subparsers.add_parser("verify")
    verify.add_argument("--report", type=Path, required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        provenance = load_provenance()
        if args.command == "prepare":
            result = prepare_completion_signing_material(
                args.candidate,
                args.statement,
                args.pae,
            )
        else:
            trust_path, replay_dir = system_authority_paths()
            provenance.validate_system_authority_path(
                trust_path,
                directory=False,
                label="RemoteApp attestation trust bundle",
            )
            if args.command == "assemble":
                result = assemble_completion_attestation(
                    args.candidate,
                    args.statement,
                    args.signature,
                    args.keyid,
                    trust_path,
                    args.output,
                )
            else:
                provenance.validate_system_authority_path(
                    replay_dir,
                    directory=True,
                    label="RemoteApp campaign replay ledger",
                )
        if args.command == "finalize":
            result = finalize_candidate(
                args.candidate,
                args.attestation,
                trust_path,
                replay_dir,
                args.output,
            )
        elif args.command == "verify":
            result = verify_final_report(
                args.report,
                trust_path,
                replay_dir,
            )
        print(json.dumps(result, indent=2, sort_keys=True))
    except (OSError, ValueError) as exc:
        raise SystemExit(f"remoteapp-product-finalize: {exc}") from exc


if __name__ == "__main__":
    main()
