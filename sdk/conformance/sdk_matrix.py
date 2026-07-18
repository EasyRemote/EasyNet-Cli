#!/usr/bin/env python3
from __future__ import annotations

import argparse
import functools
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

from sdk_concepts import (
    CONCEPTS,
    LANGUAGES,
    STATUS_CANONICAL_NAMES,
    STATUSES,
    canonical_lifecycle_reference,
    case_contracts,
    self_test as concepts_self_test,
    validate_schema,
)
from source_revision import AXON_REVISION_ROOTS, axon_root, git_source_revision

ROOT = Path(__file__).resolve().parents[2]
SOURCE = CONCEPTS
MATRIX = ROOT / "sdk/conformance/sdk-parity-matrix.json"


@functools.lru_cache(maxsize=1)
def tree_sha256() -> str:
    output = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    ).stdout
    material = bytearray()
    for raw in sorted(path for path in output.split(b"\0") if path):
        material.extend(raw)
        material.append(0)
        path = ROOT / raw.decode()
        material.extend(path.read_bytes() if path.is_file() else b"<deleted>")
        material.append(0)
    return hashlib.sha256(material).hexdigest()


@functools.lru_cache(maxsize=None)
def toolchain_attestation(language: str) -> tuple[str, str]:
    python = os.environ.get("SDK_CONFORMANCE_PYTHON", sys.executable)
    commands = {
        "rust": ["rustc", "--version"],
        "c_abi": ["rustc", "--version"],
        "go": ["go", "version"],
        "python": [python, "--version"],
        "node": ["node", "--version"],
        "java": ["java", "-version"],
        "swift": ["swift", "--version"],
    }
    completed = subprocess.run(
        commands[language], check=True, capture_output=True, text=True
    )
    version = (completed.stdout or completed.stderr).strip()
    contract = (ROOT / "sdk/conformance/toolchains.json").read_bytes()
    payload = {
        "contract_sha256": hashlib.sha256(contract).hexdigest(),
        "language": language,
        "version": version,
    }
    digest = hashlib.sha256(
        json.dumps(
            payload,
            separators=(",", ":"),
            sort_keys=True,
        ).encode()
    ).hexdigest()
    return digest, version


def fail(message: str) -> None:
    raise ValueError(message)


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail(f"object_required:{path}")
    return value


def case_languages() -> dict[str, set[str]]:
    result: dict[str, set[str]] = {}
    for path in sorted((ROOT / "sdk/conformance/cases").glob("*.yaml")):
        case_id = ""
        languages: set[str] = set()
        required = False
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith("id:"):
                case_id = line.split(":", 1)[1].strip()
            elif line == "required_for:":
                required = True
            elif required and line.startswith("  - "):
                languages.add(line[4:].strip())
            elif required and line and not line.startswith(" "):
                required = False
        if not case_id or case_id in result:
            fail(f"invalid_or_duplicate_case:{path}")
        result[case_id] = languages
    return result


def generate() -> dict[str, Any]:
    source = validate_schema(load_json(SOURCE))
    capabilities = source["capabilities"]
    inventory = source["capability_inventory"]
    provider_proofs = source["provider_proofs"]
    known_cases = case_languages()
    contracts = case_contracts()
    capability_ids = list(capabilities)
    cells: list[dict[str, Any]] = []
    for capability_id, capability in capabilities.items():
        profile = capability["profile"]
        ids = capability["case_ids"]
        for language in LANGUAGES:
            evidence = [case_id for case_id in ids if language in known_cases[case_id]]
            unproven = [
                requirement["requirement_id"]
                for requirement in capability.get("unproven_requirements", [])
                if language in requirement["languages"]
            ]
            projection = inventory[capability_id][language]
            public_items = projection["symbols"] + projection["members"]
            has_public_surface = bool(public_items)
            proof = provider_proofs.get(capability_id, {}).get(language)
            if proof is not None:
                status = (
                    "cutover-ready"
                    if proof.get("cutover_ready") is True
                    else "provider-backed"
                )
                if not has_public_surface or evidence != ids or unproven:
                    fail(f"provider_status_not_closed:{capability_id}:{language}")
            elif has_public_surface or evidence:
                status = "seam"
            else:
                status = "unsupported"
                evidence = []
            cells.append(
                {
                    "capability_id": capability_id,
                    "language": language,
                    "profile": profile,
                    "status": status,
                    "evidence_case_ids": evidence,
                    "unproven_requirement_ids": unproven,
                    "shape_evidence": [
                        {
                            "item": item,
                            "sha256": source["shape_sha256"][language][item],
                        }
                        for item in public_items
                    ],
                    "step_shape_evidence": (
                        [
                            {
                                "case_id": case_id,
                                "action": action,
                                "execution_case_bound": case_id in evidence,
                                "items": [
                                    {
                                        "item": item,
                                        "sha256": source["shape_sha256"][language][
                                            item
                                        ],
                                    }
                                    for item in public_items
                                ],
                            }
                            for case_id in ids
                            for action in contracts[case_id]["actions"]
                        ]
                        if has_public_surface
                        else []
                    ),
                    "public_api_ref": f"sdk/conformance/canonical-public-api.json#capability_inventory/{capability_id}",
                    "provider_proof_ref": (
                        f"sdk/conformance/canonical-public-api.json#provider_proofs/{capability_id}/{language}"
                        if proof is not None
                        else None
                    ),
                }
            )
    return {
        "schema_version": 5,
        "source": "sdk/conformance/canonical-public-api.json",
        "languages": LANGUAGES,
        "status_order": STATUSES,
        "status_canonical_names": STATUS_CANONICAL_NAMES,
        "canonical_lifecycle_contract": canonical_lifecycle_reference(),
        "capability_ids": capability_ids,
        "cells": cells,
    }


def case_paths() -> dict[str, Path]:
    result: dict[str, Path] = {}
    for path in sorted((ROOT / "sdk/conformance/cases").glob("*.yaml")):
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith("id:"):
                result[line.split(":", 1)[1].strip()] = path
                break
    return result


def validate_execution(record: dict[str, Any], language: str, case_id: str) -> None:
    if record.get("status") != "passed":
        fail(f"evidence_not_passed:{language}:{case_id}:{record.get('status')}")
    selector = record.get("selector")
    if not isinstance(selector, str) or not selector:
        fail(f"missing_selector:{language}:{case_id}")
    if record.get("collected_tests") != [selector]:
        fail(f"selector_not_collected_once:{language}:{case_id}")
    case_path = case_paths().get(case_id)
    if (
        case_path is None
        or record.get("case_sha256")
        != hashlib.sha256(case_path.read_bytes()).hexdigest()
    ):
        fail(f"missing_case_digest:{language}:{case_id}")
    evidence = record.get("evidence")
    if not isinstance(evidence, list) or not evidence:
        fail(f"missing_evidence:{language}:{case_id}")
    if any(
        not isinstance(item, dict) or len(str(item.get("sha256", ""))) != 64
        for item in evidence
    ):
        fail(f"missing_evidence_hash:{language}:{case_id}")
    for item in evidence:
        evidence_path = (ROOT / str(item.get("ref_path", ""))).resolve()
        try:
            evidence_path.relative_to(ROOT)
        except ValueError:
            fail(f"evidence_outside_repo:{language}:{case_id}")
        if (
            not evidence_path.is_file()
            or hashlib.sha256(evidence_path.read_bytes()).hexdigest() != item["sha256"]
        ):
            fail(f"evidence_hash_mismatch:{language}:{case_id}")
    executions = record.get("executions")
    if not isinstance(executions, list) or not executions:
        fail(f"zero_execution:{language}:{case_id}")
    executed = [
        proof
        for proof in executions
        if isinstance(proof, dict) and proof.get("phase") == "execution"
    ]
    if not executed or any(
        proof.get("exit_code") != 0 or len(str(proof.get("output_sha256", ""))) != 64
        for proof in executed
    ):
        fail(f"execution_failed_or_unbound:{language}:{case_id}")
    payload = {
        "case_id": case_id,
        "case_sha256": record["case_sha256"],
        "language": language,
        "selector": selector,
        "evidence": evidence,
        "collected_tests": record["collected_tests"],
        "executions": executions,
        "execution_failure": None,
    }
    command_attestation = hashlib.sha256(
        json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
    ).hexdigest()
    run_context = {
        "run_nonce": record.get("run_nonce"),
        "tree_sha256": record.get("tree_sha256"),
        "toolchain_sha256": record.get("toolchain_sha256"),
        "toolchain_version": record.get("toolchain_version"),
        "axon_revision": record.get("axon_revision"),
    }
    if any(not isinstance(value, str) or not value for value in run_context.values()):
        fail(f"missing_run_context:{language}:{case_id}")
    expected_attestation = hashlib.sha256(
        json.dumps(
            {
                "command_attestation_sha256": command_attestation,
                "run_context": run_context,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode()
    ).hexdigest()
    if record.get("attestation_sha256") != expected_attestation:
        fail(f"attestation_mismatch:{language}:{case_id}")


def validate(
    matrix_path: Path,
    results_dir: Path,
    languages: list[str] | None = None,
    check_checkout: bool = True,
    allow_snapshot_results: bool = False,
) -> None:
    expected = generate()
    concepts = validate_schema(load_json(SOURCE))
    selected_languages = expected["languages"] if languages is None else languages
    if (
        not selected_languages
        or len(selected_languages) != len(set(selected_languages))
        or not set(selected_languages).issubset(expected["languages"])
    ):
        fail("invalid_language_slice")
    actual = load_json(matrix_path)
    if actual != expected:
        fail("matrix_not_canonical_generated_output")
    indexed_results: dict[str, dict[str, dict[str, Any]]] = {}
    current_tree = tree_sha256()
    expected_tree = current_tree
    attestation_manifest = results_dir / "source-attestation.json"
    if attestation_manifest.is_file():
        if not allow_snapshot_results:
            fail("snapshot_results_not_allowed")
        manifest = load_json(attestation_manifest)
        if manifest.get("schema_version") != 1:
            fail("invalid_source_attestation_schema")
        if manifest.get("source_state") != "captured_source_snapshot":
            fail("invalid_source_attestation_state")
        snapshot_tree = manifest.get("tree_sha256")
        if (
            not isinstance(snapshot_tree, str)
            or len(snapshot_tree) != 64
            or any(ch not in "0123456789abcdefABCDEF" for ch in snapshot_tree)
        ):
            fail("invalid_source_attestation_tree")
        expected_tree = snapshot_tree
    missing_results = [
        language
        for language in selected_languages
        if not (results_dir / f"{language}.json").is_file()
    ]
    if missing_results:
        fail("missing_live_results:" + ",".join(missing_results))
    expected_axon = concepts["dependency_revisions"]["axon_sdk"]
    current_axon = git_source_revision(axon_root(), AXON_REVISION_ROOTS)
    if check_checkout and current_axon != expected_axon:
        fail(
            f"axon_checkout_revision_mismatch:expected={expected_axon}:actual={current_axon}"
        )
    observed_nonces: set[str] = set()
    for language in selected_languages:
        result_path = results_dir / f"{language}.json"
        records = json.loads(result_path.read_text(encoding="utf-8"))
        if not isinstance(records, list) or not records:
            fail(f"empty_live_result:{language}")
        indexed: dict[str, dict[str, Any]] = {}
        for record in records:
            if not isinstance(record, dict) or record.get("language") != language:
                fail(f"invalid_live_record:{language}")
            case_id = record.get("case_id")
            if not isinstance(case_id, str) or case_id in indexed:
                fail(f"missing_or_duplicate_live_case:{language}:{case_id}")
            if record.get("status") == "skipped":
                fail(f"skipped_status_forbidden:{language}:{case_id}")
            nonce = record.get("run_nonce")
            if not isinstance(nonce, str) or len(nonce) != 64:
                fail(f"invalid_run_nonce:{language}:{case_id}")
            observed_nonces.add(nonce)
            if record.get("tree_sha256") != expected_tree:
                fail(f"replayed_tree_attestation:{language}:{case_id}")
            if record.get("axon_revision") != expected_axon:
                fail(f"axon_attestation_revision_mismatch:{language}:{case_id}")
            digest, version = toolchain_attestation(language)
            if (
                record.get("toolchain_sha256") != digest
                or record.get("toolchain_version") != version
            ):
                fail(f"toolchain_attestation_mismatch:{language}:{case_id}")
            indexed[case_id] = record
        current_cases = set(case_paths())
        if set(indexed) != current_cases:
            missing = sorted(current_cases - set(indexed))
            stale = sorted(set(indexed) - current_cases)
            fail(
                f"stale_live_case_set:{language}:missing={','.join(missing)}:stale={','.join(stale)}"
            )
        indexed_results[language] = indexed
    if len(observed_nonces) != 1:
        fail("mixed_run_nonce")
    referenced: set[tuple[str, str]] = set()
    for cell in expected["cells"]:
        if cell["language"] not in selected_languages:
            continue
        if cell["status"] == "unsupported":
            if cell["evidence_case_ids"]:
                fail(
                    f"unsupported_with_evidence:{cell['capability_id']}:{cell['language']}"
                )
            continue
        for case_id in cell["evidence_case_ids"]:
            referenced.add((cell["language"], case_id))
            record = indexed_results[cell["language"]].get(case_id)
            if record is None:
                fail(
                    f"missing_executable_evidence:{cell['capability_id']}:{cell['language']}:{case_id}"
                )
            validate_execution(record, cell["language"], case_id)
        if cell["status"] in {"provider-backed", "cutover-ready"}:
            proof = concepts["provider_proofs"][cell["capability_id"]][cell["language"]]
            for mapping in proof["step_evidence"]:
                record = indexed_results[cell["language"]][mapping["case_id"]]
                if record.get("selector") != mapping["selector"]:
                    fail(
                        f"provider_step_not_live:{cell['capability_id']}:"
                        f"{cell['language']}:{mapping['case_id']}:{mapping['action']}"
                    )
    quality_cases = set(concepts["quality_gate_case_ids"])
    for language, records in indexed_results.items():
        for case_id, record in records.items():
            if record.get("status") == "passed" and case_id in quality_cases:
                validate_execution(record, language, case_id)
                referenced.add((language, case_id))
            if (
                record.get("status") == "passed"
                and (language, case_id) not in referenced
            ):
                fail(f"unmodeled_passed_case:{language}:{case_id}")


def synthetic_results(directory: Path, matrix: dict[str, Any]) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    manifest = load_json(ROOT / "sdk/conformance/runner/execution-manifest.json")
    selectors = {
        (item["language"], item["case_id"]): item["selector"]
        for item in manifest["bindings"]
    }
    evidence_by_language: dict[str, set[str]] = {
        language: set() for language in matrix["languages"]
    }
    for cell in matrix["cells"]:
        evidence_by_language[cell["language"]].update(cell["evidence_case_ids"])
    concepts = validate_schema(load_json(SOURCE))
    quality_cases = set(concepts["quality_gate_case_ids"])
    contracts = case_languages()
    for language in matrix["languages"]:
        evidence_by_language[language].update(
            case_id for case_id in quality_cases if language in contracts[case_id]
        )
    source_ref = "sdk/conformance/canonical-public-api.json"
    source_digest = hashlib.sha256((ROOT / source_ref).read_bytes()).hexdigest()
    paths = case_paths()
    current_tree = tree_sha256()
    expected_axon = concepts["dependency_revisions"]["axon_sdk"]
    for language, evidence_case_ids in evidence_by_language.items():
        toolchain_digest, toolchain_version = toolchain_attestation(language)
        run_context = {
            "run_nonce": "a" * 64,
            "tree_sha256": current_tree,
            "toolchain_sha256": toolchain_digest,
            "toolchain_version": toolchain_version,
            "axon_revision": expected_axon,
        }
        records = []
        for case_id in sorted(paths):
            case_digest = hashlib.sha256(paths[case_id].read_bytes()).hexdigest()
            if case_id not in evidence_case_ids:
                records.append(
                    {
                        "case_id": case_id,
                        "language": language,
                        "profile": "self_test",
                        "case_sha256": case_digest,
                        "selector": None,
                        "evidence": [],
                        "collected_tests": [],
                        "attestation_sha256": None,
                        "status": "unsupported",
                        "error_code": "CAPABILITY_UNSUPPORTED",
                        "message": "self-test unsupported",
                        "executions": [],
                        **run_context,
                    }
                )
                continue
            selector = selectors.get((language, case_id))
            if not selector:
                fail(f"self_test_missing_binding:{language}:{case_id}")
            evidence = [
                {"kind": "self_test", "ref_path": source_ref, "sha256": source_digest}
            ]
            executions = [
                {
                    "phase": "execution",
                    "command": ["self-test"],
                    "working_directory": ".",
                    "exit_code": 0,
                    "output_sha256": source_digest,
                }
            ]
            payload = {
                "case_id": case_id,
                "case_sha256": case_digest,
                "language": language,
                "selector": selector,
                "evidence": evidence,
                "collected_tests": [selector],
                "executions": executions,
                "execution_failure": None,
            }
            command_attestation = hashlib.sha256(
                json.dumps(payload, separators=(",", ":"), sort_keys=True).encode(
                    "utf-8"
                )
            ).hexdigest()
            attestation = hashlib.sha256(
                json.dumps(
                    {
                        "command_attestation_sha256": command_attestation,
                        "run_context": run_context,
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode()
            ).hexdigest()
            records.append(
                {
                    "case_id": case_id,
                    "language": language,
                    "profile": "self_test",
                    "case_sha256": case_digest,
                    "selector": selector,
                    "evidence": evidence,
                    "collected_tests": [selector],
                    "attestation_sha256": attestation,
                    "status": "passed",
                    "error_code": None,
                    "message": None,
                    "executions": executions,
                    **run_context,
                }
            )
        (directory / f"{language}.json").write_text(
            json.dumps(records), encoding="utf-8"
        )


def self_test(tmp: Path) -> None:
    concepts_self_test(tmp / "concepts")
    fake_python = tmp / "python-toolchain"
    fake_python.write_text(
        "#!/usr/bin/env bash\n"
        'if [[ "${1:-}" == "--version" ]]; then\n'
        "  printf 'Python conformance-fixture\\n'\n"
        "  exit 0\n"
        "fi\n"
        "exit 1\n",
        encoding="utf-8",
    )
    fake_python.chmod(0o755)
    original_python = os.environ.get("SDK_CONFORMANCE_PYTHON")
    try:
        os.environ["SDK_CONFORMANCE_PYTHON"] = str(fake_python)
        toolchain_attestation.cache_clear()
        _, selected_version = toolchain_attestation("python")
        if selected_version != "Python conformance-fixture":
            fail("self_test_explicit_python_toolchain_was_ignored")
    finally:
        if original_python is None:
            os.environ.pop("SDK_CONFORMANCE_PYTHON", None)
        else:
            os.environ["SDK_CONFORMANCE_PYTHON"] = original_python
        toolchain_attestation.cache_clear()

    matrix = generate()
    good_matrix = tmp / "matrix.json"
    good_matrix.write_text(json.dumps(matrix, indent=2) + "\n", encoding="utf-8")
    results = tmp / "results"
    synthetic_results(results, matrix)
    validate(good_matrix, results, check_checkout=False)

    snapshot_manifest = results / "source-attestation.json"
    first_result = json.loads(
        (results / f"{matrix['languages'][0]}.json").read_text(encoding="utf-8")
    )
    snapshot_manifest.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "source_state": "captured_source_snapshot",
                "tree_sha256": first_result[0]["tree_sha256"],
            }
        ),
        encoding="utf-8",
    )
    try:
        validate(good_matrix, results, check_checkout=False)
    except ValueError as exc:
        if "snapshot_results_not_allowed" not in str(exc):
            raise
    else:
        fail("self_test_snapshot_results_were_accepted_without_opt_in")
    validate(
        good_matrix,
        results,
        check_checkout=False,
        allow_snapshot_results=True,
    )
    snapshot_manifest.unlink()

    replay_path = results / f"{matrix['languages'][0]}.json"
    replayed = json.loads(replay_path.read_text(encoding="utf-8"))
    replayed[0]["tree_sha256"] = "0" * 64
    replay_path.write_text(json.dumps(replayed), encoding="utf-8")
    try:
        validate(good_matrix, results, check_checkout=False)
    except ValueError as exc:
        if "replayed_tree_attestation" not in str(exc):
            raise
    else:
        fail("self_test_replayed_tree_was_accepted")

    synthetic_results(results, matrix)
    mixed = json.loads(replay_path.read_text(encoding="utf-8"))
    mixed[0]["run_nonce"] = "b" * 64
    replay_path.write_text(json.dumps(mixed), encoding="utf-8")
    try:
        validate(good_matrix, results, check_checkout=False)
    except ValueError as exc:
        if "mixed_run_nonce" not in str(exc):
            raise
    else:
        fail("self_test_mixed_run_nonce_was_accepted")

    synthetic_results(results, matrix)
    first_language = matrix["languages"][0]
    result_path = results / f"{first_language}.json"
    records = json.loads(result_path.read_text(encoding="utf-8"))
    next(record for record in records if record["status"] == "passed")[
        "executions"
    ] = []
    result_path.write_text(json.dumps(records), encoding="utf-8")
    try:
        validate(good_matrix, results, check_checkout=False)
    except ValueError as exc:
        if "zero_execution" not in str(exc):
            raise
    else:
        fail("self_test_zero_execution_was_accepted")

    synthetic_results(results, matrix)
    broken = json.loads(good_matrix.read_text(encoding="utf-8"))
    broken["cells"].pop()
    broken_matrix = tmp / "missing-cell.json"
    broken_matrix.write_text(json.dumps(broken), encoding="utf-8")
    try:
        validate(broken_matrix, results, check_checkout=False)
    except ValueError as exc:
        if "matrix_not_canonical" not in str(exc):
            raise
    else:
        fail("self_test_missing_cell_was_accepted")

    invented = json.loads(good_matrix.read_text(encoding="utf-8"))
    invented["capability_ids"].append("invented_capability")
    invented_matrix = tmp / "invented-capability.json"
    invented_matrix.write_text(json.dumps(invented), encoding="utf-8")
    try:
        validate(invented_matrix, results, check_checkout=False)
    except ValueError as exc:
        if "matrix_not_canonical" not in str(exc):
            raise
    else:
        fail("self_test_invented_capability_was_accepted")

    synthetic_results(results, matrix)
    result_path.write_text(
        (ROOT / "sdk/conformance/runner/rust-action-adapter-report.json").read_text(
            encoding="utf-8"
        ),
        encoding="utf-8",
    )
    try:
        validate(good_matrix, results, check_checkout=False)
    except ValueError as exc:
        if "empty_live_result" not in str(exc):
            raise
    else:
        fail("self_test_committed_report_was_accepted_as_live_result")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--generate", action="store_true")
    parser.add_argument("--validate", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--validate-slice", nargs="+")
    parser.add_argument("--matrix", type=Path, default=MATRIX)
    parser.add_argument("--results-dir", type=Path)
    parser.add_argument("--tmp", type=Path)
    parser.add_argument("--allow-snapshot-results", action="store_true")
    args = parser.parse_args()
    try:
        if args.generate:
            print(json.dumps(generate(), indent=2))
        elif args.self_test:
            if args.tmp is None:
                fail("self_test_tmp_required")
            self_test(args.tmp)
            print("sdk parity matrix self-test ok")
        elif args.validate:
            if args.results_dir is None:
                fail("live_results_required")
            validate(
                args.matrix.resolve(),
                args.results_dir.resolve(),
                allow_snapshot_results=args.allow_snapshot_results,
            )
            print(f"sdk parity matrix ok: {args.matrix}")
        elif args.validate_slice is not None:
            if args.results_dir is None:
                fail("live_results_required")
            validate(
                args.matrix.resolve(),
                args.results_dir.resolve(),
                args.validate_slice,
                allow_snapshot_results=args.allow_snapshot_results,
            )
            print(
                "sdk parity matrix language slice ok: " + ",".join(args.validate_slice)
            )
        else:
            parser.error("one mode is required")
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        print(f"sdk_parity_matrix: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
