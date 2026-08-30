#!/usr/bin/env python3
"""Operate the public-key trust root for RemoteApp product campaigns.

Private signing keys never enter this tool. Rotation adds a separately
generated public-key record; revocation writes an immutable effective time;
installation targets the platform-owned product paths only.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import time
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent
PROVENANCE_PATH = SCRIPT_DIR / "remoteapp-evidence-provenance.py"
spec = importlib.util.spec_from_file_location("remoteapp_evidence_provenance", PROVENANCE_PATH)
if spec is None or spec.loader is None:
    raise SystemExit("cannot load RemoteApp provenance contract")
provenance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(provenance)


def read_object(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise ValueError(f"{label} is not valid JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise ValueError(f"{label} must contain a JSON object")
    return value


def validate_candidate(value: dict[str, Any], directory: Path) -> None:
    descriptor, temporary = tempfile.mkstemp(prefix=".remoteapp-trust.", dir=directory)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, sort_keys=True)
        provenance.load_trust_bundle(Path(temporary))
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def write_validated_atomic(path: Path, value: dict[str, Any], mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_symlink():
        raise ValueError("trust output must not be a symlink")
    validate_candidate(value, path.parent)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        if os.name != "nt":
            directory_fd = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def next_generation(bundle: dict[str, Any], updated_at_ms: int) -> dict[str, Any]:
    previous_updated = provenance.require_int(
        bundle.get("updated_at_ms"), "trust.updated_at_ms"
    )
    if updated_at_ms <= previous_updated:
        raise ValueError("updated_at_ms must advance monotonically")
    candidate = json.loads(json.dumps(bundle))
    candidate["generation"] = provenance.require_int(
        bundle.get("generation"), "trust.generation"
    ) + 1
    candidate["updated_at_ms"] = updated_at_ms
    return candidate


def rotate(current: Path, key_record: Path, output: Path, updated_at_ms: int) -> None:
    provenance.load_trust_bundle(current)
    bundle = next_generation(read_object(current, "current trust bundle"), updated_at_ms)
    record = read_object(key_record, "new trust key record")
    keyid = provenance.require_string(record.get("keyid"), "new trust key keyid")
    if any(row.get("keyid") == keyid for row in bundle["keys"]):
        raise ValueError(f"trust keyid {keyid!r} already exists")
    if record.get("revoked_at_ms") is not None:
        raise ValueError("a rotation successor must not already be revoked")
    bundle["keys"].append(record)
    write_validated_atomic(output, bundle)


def revoke(current: Path, keyid: str, output: Path, revoked_at_ms: int) -> None:
    provenance.load_trust_bundle(current)
    bundle = next_generation(read_object(current, "current trust bundle"), revoked_at_ms)
    matches = [row for row in bundle["keys"] if row.get("keyid") == keyid]
    if len(matches) != 1:
        raise ValueError(f"trust keyid {keyid!r} does not identify exactly one key")
    row = matches[0]
    if row.get("revoked_at_ms") is not None:
        raise ValueError(f"trust keyid {keyid!r} is already revoked")
    if revoked_at_ms <= provenance.require_int(row.get("not_before_ms"), "key.not_before_ms"):
        raise ValueError("revoked_at_ms must be after key.not_before_ms")
    row["revoked_at_ms"] = revoked_at_ms
    write_validated_atomic(output, bundle)


def authority_paths() -> tuple[Path, Path]:
    if sys.platform == "darwin":
        root = Path("/Library/Application Support/EasyNet/remoteapp")
        return root / "attestation-trust.json", root / "campaign-replay"
    if sys.platform.startswith("linux"):
        return (
            Path("/etc/easynet/remoteapp-attestation-trust.json"),
            Path("/var/lib/easynet/remoteapp-campaign-replay"),
        )
    raise ValueError("native product-authority ACL installation is not implemented on this OS")


def install(source: Path) -> None:
    if os.name == "nt" or not hasattr(os, "geteuid") or os.geteuid() != 0:
        raise ValueError("trust installation must run as root on a supported POSIX host")
    provenance.load_trust_bundle(source)
    source_metadata = source.lstat()
    if not stat.S_ISREG(source_metadata.st_mode) or source.is_symlink():
        raise ValueError("trust source must be a regular non-symlink file")
    trust_path, replay_path = authority_paths()
    trust_path.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
    replay_path.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(replay_path, 0o700)
    write_validated_atomic(trust_path, read_object(source, "trust source"), mode=0o644)
    os.chown(trust_path, 0, 0)
    os.chown(replay_path, 0, 0)
    provenance.validate_system_authority_path(
        trust_path, directory=False, label="installed RemoteApp trust bundle"
    )
    provenance.validate_system_authority_path(
        replay_path, directory=True, label="installed RemoteApp replay ledger"
    )


def validate(path: Path, at_ms: int) -> dict[str, Any]:
    keys = provenance.load_trust_bundle(path)
    statuses: dict[str, str] = {}
    for keyid, key in keys.items():
        try:
            provenance.require_trusted_key_active(
                key, signed_at_ms=at_ms, observed_at_ms=at_ms, label=f"key {keyid!r}"
            )
        except ValueError as error:
            statuses[keyid] = str(error)
        else:
            statuses[keyid] = "active"
    return {"status": "valid", "schema": provenance.TRUST_SCHEMA, "keys": statuses}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("--trust-bundle", type=Path, required=True)
    validate_parser.add_argument("--at-ms", type=int, default=int(time.time() * 1000))
    rotate_parser = subparsers.add_parser("rotate")
    rotate_parser.add_argument("--current", type=Path, required=True)
    rotate_parser.add_argument("--new-key-record", type=Path, required=True)
    rotate_parser.add_argument("--output", type=Path, required=True)
    rotate_parser.add_argument("--updated-at-ms", type=int, required=True)
    revoke_parser = subparsers.add_parser("revoke")
    revoke_parser.add_argument("--current", type=Path, required=True)
    revoke_parser.add_argument("--keyid", required=True)
    revoke_parser.add_argument("--output", type=Path, required=True)
    revoke_parser.add_argument("--revoked-at-ms", type=int, required=True)
    install_parser = subparsers.add_parser("install")
    install_parser.add_argument("--trust-bundle", type=Path, required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        if args.command == "validate":
            print(json.dumps(validate(args.trust_bundle, args.at_ms), indent=2, sort_keys=True))
        elif args.command == "rotate":
            rotate(args.current, args.new_key_record, args.output, args.updated_at_ms)
        elif args.command == "revoke":
            revoke(args.current, args.keyid, args.output, args.revoked_at_ms)
        else:
            install(args.trust_bundle)
    except (OSError, ValueError) as exc:
        raise SystemExit(f"remoteapp-attestation-trust: {exc}") from exc


if __name__ == "__main__":
    main()
