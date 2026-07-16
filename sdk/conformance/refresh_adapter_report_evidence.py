#!/usr/bin/env python3
"""Refresh derived source digests in SDK conformance adapter reports."""

from __future__ import annotations

import argparse
import hashlib
import json
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
REPORT_DIRECTORY = Path("sdk/conformance/runner")
REPORT_GLOB = "*-action-adapter-report.json"


class EvidenceRefreshError(ValueError):
    """Raised when an adapter report cannot be refreshed safely."""


def report_paths(root: Path) -> list[Path]:
    paths = sorted((root / REPORT_DIRECTORY).glob(REPORT_GLOB))
    if not paths:
        raise EvidenceRefreshError("no adapter reports found")
    return paths


def load_report(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceRefreshError(f"read adapter report {path}: {error}") from error
    if not isinstance(value, dict) or value.get("schema_version") != 2:
        raise EvidenceRefreshError(f"invalid adapter report schema: {path}")
    if not isinstance(value.get("language"), str) or not value["language"]:
        raise EvidenceRefreshError(f"adapter report language is required: {path}")
    if not isinstance(value.get("records"), list) or not value["records"]:
        raise EvidenceRefreshError(f"adapter report records are required: {path}")
    return value


def evidence_path(root: Path, report_path: Path, ref_path: object) -> Path:
    if not isinstance(ref_path, str) or not ref_path:
        raise EvidenceRefreshError(f"evidence ref_path is required: {report_path}")
    candidate = (root / ref_path).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError as error:
        raise EvidenceRefreshError(
            f"evidence ref_path escapes repository root: {report_path}: {ref_path}"
        ) from error
    if not candidate.is_file():
        raise EvidenceRefreshError(
            f"evidence source is not a regular file: {report_path}: {ref_path}"
        )
    return candidate


def refreshed_report(root: Path, path: Path) -> tuple[dict[str, Any], list[str]]:
    report = load_report(path)
    stale: list[str] = []
    for record in report["records"]:
        if not isinstance(record, dict) or not isinstance(record.get("case_id"), str):
            raise EvidenceRefreshError(f"adapter report record is invalid: {path}")
        evidence = record.get("evidence")
        if not isinstance(evidence, list) or not evidence:
            raise EvidenceRefreshError(
                f"adapter report evidence is required: {path}: {record['case_id']}"
            )
        for item in evidence:
            if not isinstance(item, dict):
                raise EvidenceRefreshError(
                    f"adapter evidence is invalid: {path}: {record['case_id']}"
                )
            source = evidence_path(root, path, item.get("ref_path"))
            actual = hashlib.sha256(source.read_bytes()).hexdigest()
            expected = item.get("sha256")
            if expected != actual:
                stale.append(f"{path.relative_to(root)}:{record['case_id']}:{source.relative_to(root)}")
                item["sha256"] = actual
    return report, stale


def refresh(root: Path, *, write: bool) -> list[str]:
    root = root.resolve()
    stale: list[str] = []
    for path in report_paths(root):
        report, report_stale = refreshed_report(root, path)
        stale.extend(report_stale)
        if write and report_stale:
            path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return stale


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="sdk-conformance-evidence-") as raw:
        root = Path(raw)
        source = root / "sdk/go/example_test.go"
        source.parent.mkdir(parents=True)
        source.write_text("package sdk\n", encoding="utf-8")
        report = root / REPORT_DIRECTORY / "go-action-adapter-report.json"
        report.parent.mkdir(parents=True)
        report.write_text(
            json.dumps(
                {
                    "schema_version": 2,
                    "language": "go",
                    "adapter_kind": "unit_test",
                    "records": [
                        {
                            "case_id": "test/example",
                            "evidence": [
                                {
                                    "kind": "go_test",
                                    "ref_path": "sdk/go/example_test.go",
                                    "sha256": "0" * 64,
                                }
                            ],
                        }
                    ],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        stale = refresh(root, write=False)
        if stale != [
            "sdk/conformance/runner/go-action-adapter-report.json:test/example:sdk/go/example_test.go"
        ]:
            raise EvidenceRefreshError(f"unexpected check result: {stale}")
        refresh(root, write=True)
        if refresh(root, write=False):
            raise EvidenceRefreshError("refresh did not converge")

        value = load_report(report)
        value["records"][0]["evidence"][0]["ref_path"] = "../outside.go"
        report.write_text(json.dumps(value), encoding="utf-8")
        try:
            refresh(root, write=False)
        except EvidenceRefreshError as error:
            if "escapes repository root" not in str(error):
                raise
        else:
            raise EvidenceRefreshError("path traversal was accepted")


def main() -> int:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    try:
        if args.self_test:
            self_test()
            print("adapter report evidence refresh self-test ok")
            return 0
        stale = refresh(ROOT, write=args.write)
        if args.check and stale:
            for item in stale:
                print(f"stale adapter evidence: {item}")
            return 1
        print(
            "adapter report evidence refreshed" if args.write else "adapter report evidence is current"
        )
        return 0
    except EvidenceRefreshError as error:
        print(f"refresh_adapter_report_evidence: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
