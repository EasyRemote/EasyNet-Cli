#!/usr/bin/env python3
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

from edge_adapter_policy import (
    DEFAULT_POLICY as EDGE_ADAPTER_POLICY,
    load_json as load_edge_adapter_policy,
    validate_policy as validate_edge_adapter_policy,
)
from sdk_public_surface_policy import (
    PRODUCT_NEUTRAL_CUTOVER_REF,
    canonical_quarantine_reason,
)

ROOT = Path(__file__).resolve().parents[2]
CONCEPTS = ROOT / "sdk/conformance/canonical-public-api.json"
LANGUAGES = ["rust", "c_abi", "go", "python", "node", "java", "swift"]
PUBLIC_LANGUAGES = LANGUAGES
STATUSES = ["unsupported", "seam", "provider-backed", "cutover-ready"]
STATUS_CANONICAL_NAMES = {
    "unsupported": "Unsupported",
    "seam": "Seam",
    "provider-backed": "ProviderBacked",
    "cutover-ready": "CutoverReady",
}
PACKAGE_CATEGORIES = {
    "canonical_axon_sdk",
    "easynet_provider",
    "generated_wire",
    "provider_neutral_core",
    "provider_registry",
    "public_abi",
    "public_facade",
}
PRODUCT_NEUTRAL_PACKAGE_CATEGORIES = {"provider_neutral_core", "public_facade"}


def fail(message: str) -> None:
    raise ValueError(message)


def load_json(path: Path = CONCEPTS) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail(f"object_required:{path}")
    return value


def canonical_lifecycle_reference() -> dict[str, Any]:
    configured = os.environ.get("EASYNET_AXON_ROOT")
    axon_root = (
        Path(configured).resolve()
        if configured
        else (ROOT / "../EasyNet-Axon").resolve()
    )
    matrix_relative = Path("sdk/conformance/lifecycle/capability-matrix.v1.json")
    vectors_relative = Path("sdk/conformance/lifecycle/lifecycle-vectors.v1.json")
    matrix_path = axon_root / matrix_relative
    vectors_path = axon_root / vectors_relative
    matrix = load_json(matrix_path)
    vectors = load_json(vectors_path)
    matrix_contract = matrix.get("provider_contract")
    vector_contract = vectors.get("provider_contract")
    if (
        not isinstance(matrix_contract, dict)
        or not isinstance(vector_contract, dict)
        or matrix_contract.get("id") != "axon.canonical-runtime.lifecycle"
        or {
            "id": matrix_contract.get("id"),
            "version": matrix_contract.get("version"),
        }
        != vector_contract
    ):
        fail("canonical_lifecycle_provider_contract")
    actions = matrix_contract.get("actions")
    if (
        not isinstance(actions, list)
        or not actions
        or set(actions) != set(vectors.get("action_contracts", {}))
    ):
        fail("canonical_lifecycle_actions")
    return {
        "owner_repository": "EasyNet-Axon",
        "provider_contract": {
            "id": matrix_contract["id"],
            "version": matrix_contract["version"],
        },
        "capability_matrix": {
            "path": matrix_relative.as_posix(),
            "sha256": hashlib.sha256(matrix_path.read_bytes()).hexdigest(),
        },
        "transition_vectors": {
            "path": vectors_relative.as_posix(),
            "sha256": hashlib.sha256(vectors_path.read_bytes()).hexdigest(),
        },
    }


def package_identity(path: str) -> str:
    return Path(path).name


def case_contracts(cases_root: Path | None = None) -> dict[str, dict[str, Any]]:
    contracts: dict[str, dict[str, Any]] = {}
    root = cases_root or ROOT / "sdk/conformance/cases"
    for path in sorted(root.glob("*.yaml")):
        case_id = ""
        languages: list[str] = []
        actions: list[str] = []
        section = ""
        for raw in path.read_text(encoding="utf-8").splitlines():
            line = raw.strip()
            if raw.startswith("id:"):
                case_id = raw.split(":", 1)[1].strip()
            elif raw == "required_for:":
                section = "required_for"
            elif raw == "steps:":
                section = "steps"
            elif raw and not raw.startswith(" "):
                section = ""
            elif section == "required_for" and line.startswith("- "):
                languages.append(line[2:].strip())
            elif section == "steps" and line.startswith("- action:"):
                actions.append(line.split(":", 1)[1].strip())
        if not case_id or case_id in contracts or not actions:
            fail(f"invalid_case_contract:{path}")
        contracts[case_id] = {
            "path": path,
            "languages": languages,
            "actions": actions,
        }
    return contracts


def execution_bindings() -> dict[tuple[str, str], dict[str, Any]]:
    manifest = load_json(ROOT / "sdk/conformance/runner/execution-manifest.json")
    result: dict[tuple[str, str], dict[str, Any]] = {}
    for binding in manifest.get("bindings", []):
        key = (binding.get("language"), binding.get("case_id"))
        if key in result:
            fail(f"duplicate_case_binding:{key[0]}:{key[1]}")
        result[key] = binding
    return result


def _projection_values(
    projection: dict[str, Any], section: str, language: str
) -> list[str]:
    language_projection = projection.get(language)
    if not isinstance(language_projection, dict):
        fail(f"missing_projection:{language}")
    values = language_projection.get(section)
    if not isinstance(values, list) or values != sorted(set(values)):
        fail(f"unsorted_projection:{language}:{section}")
    return values


def validate_product_neutral_inventory(concepts: dict[str, Any]) -> None:
    graph = {
        "languages": concepts.get("languages", {}),
        "members": concepts.get("members", {}),
    }
    for section, languages in graph.items():
        for language, values in languages.items():
            for value in values:
                if canonical_quarantine_reason(value) is not None:
                    fail(
                        f"canonical_inventory_product_leak:{language}:{section}:{value}"
                    )

    inventory = concepts.get("capability_inventory", {})
    for capability_id, projection in inventory.items():
        for language, payload in projection.items():
            for section in ("symbols", "members"):
                for value in payload.get(section, []):
                    if canonical_quarantine_reason(value) is not None:
                        fail(
                            "canonical_capability_product_leak:"
                            f"{capability_id}:{language}:{section}:{value}"
                        )


def validate_schema(
    concepts: dict[str, Any], *, check_paths: bool = True
) -> dict[str, Any]:
    if (
        concepts.get("schema_version") != 5
        or concepts.get("complete_inventory") is not True
    ):
        fail("concept_schema_version")
    if concepts.get("matrix_languages") != LANGUAGES:
        fail("matrix_languages")
    if concepts.get("status_order") != STATUSES:
        fail("status_order")
    if concepts.get("status_canonical_names") != STATUS_CANONICAL_NAMES:
        fail("status_canonical_names")
    if concepts.get("canonical_lifecycle_contract") != canonical_lifecycle_reference():
        fail("canonical_lifecycle_contract")
    if "lifecycle_actions" in concepts or "lifecycle_transition_contract" in concepts:
        fail("duplicate_lifecycle_contract")
    source_revisions = concepts.get("inventory_source_revisions")
    if not isinstance(source_revisions, dict) or set(source_revisions) != set(
        PUBLIC_LANGUAGES
    ):
        fail("inventory_source_revisions")
    expected_axon = concepts.get("dependency_revisions", {}).get("axon_sdk")
    for language in ("rust", "python"):
        if source_revisions.get(language) != expected_axon:
            fail(f"inventory_source_revision_mismatch:{language}")

    graph = {
        "symbols": concepts.get("languages"),
        "members": concepts.get("members"),
    }
    quarantined = concepts.get("non_canonical")
    quarantine_metadata = concepts.get("legacy_quarantine")
    for section, values in graph.items():
        if not isinstance(values, dict) or set(values) != set(PUBLIC_LANGUAGES):
            fail(f"invalid_public_graph:{section}")
        excluded = quarantined.get("languages" if section == "symbols" else section, {})
        metadata = quarantine_metadata.get(
            "languages" if section == "symbols" else section, {}
        )
        for language in PUBLIC_LANGUAGES:
            canonical = values[language]
            legacy = excluded.get(language)
            if canonical != sorted(set(canonical)) or legacy != sorted(
                set(legacy or [])
            ):
                fail(f"unsorted_public_graph:{language}:{section}")
            if set(canonical) & set(legacy):
                fail(f"public_graph_overlap:{language}:{section}")
            if set(metadata.get(language, {})) != set(legacy):
                fail(f"legacy_metadata_not_closed:{language}:{section}")
            for value, entry in metadata.get(language, {}).items():
                required = {
                    "canonical_replacement",
                    "consumer_cutover_ref",
                    "removal_phase",
                    "reason",
                }
                if not isinstance(entry, dict) or set(entry) != required:
                    fail(f"invalid_legacy_metadata:{language}:{section}:{value}")
                policy_reason = canonical_quarantine_reason(value)
                if policy_reason is None:
                    fail(f"unapproved_legacy_quarantine:{language}:{section}:{value}")
                if entry["reason"] != policy_reason:
                    fail(
                        f"legacy_quarantine_reason_mismatch:{language}:{section}:{value}"
                    )
                replacements = entry["canonical_replacement"]
                if (
                    not isinstance(replacements, list)
                    or replacements != sorted(set(replacements))
                    or not replacements
                ):
                    fail(f"invalid_legacy_replacement:{language}:{section}:{value}")
                for reference in replacements:
                    if reference.startswith(f"languages.{language}."):
                        target = reference.split(f"languages.{language}.", 1)[1]
                        valid = target in graph["symbols"][language]
                    elif reference.startswith(f"members.{language}."):
                        target = reference.split(f"members.{language}.", 1)[1]
                        valid = target in graph["members"][language]
                    elif reference.startswith("capability_inventory."):
                        target = reference.split(".", 1)[1]
                        valid = target in concepts.get("capability_inventory", {})
                    else:
                        valid = False
                    if not valid:
                        fail(f"stale_legacy_replacement:{language}:{section}:{value}")
                cutover_ref = entry["consumer_cutover_ref"]
                cutover_path = str(cutover_ref).split("#", 1)[0]
                if not str(cutover_ref).strip() or (
                    check_paths
                    and not str(cutover_ref).startswith(("http://", "https://"))
                    and not (ROOT / cutover_path).exists()
                ):
                    fail(f"stale_consumer_cutover_ref:{language}:{section}:{value}")
                if entry["removal_phase"] not in {
                    "quarantined",
                    "consumer_cutover",
                    "removal_ready",
                }:
                    fail(f"invalid_removal_phase:{language}:{section}:{value}")
                if not str(entry["reason"]).strip():
                    fail(f"legacy_reason_required:{language}:{section}:{value}")

    capabilities = concepts.get("capabilities")
    inventory = concepts.get("capability_inventory")
    supporting_items = concepts.get("supporting_items")
    declared_support_groups = concepts.get("support_shape_groups")
    if not isinstance(capabilities, dict) or not capabilities:
        fail("capabilities_required")
    if "conformance_runner" in capabilities:
        fail("conformance_runner_is_not_runtime_capability")
    if not isinstance(inventory, dict) or set(inventory) != set(capabilities):
        fail("capability_inventory_not_closed")
    if not isinstance(supporting_items, list):
        fail("supporting_items_required")
    if not isinstance(declared_support_groups, dict):
        fail("support_shape_groups_required")
    shape_hashes = concepts.get("shape_sha256")
    if not isinstance(shape_hashes, dict) or set(shape_hashes) != set(PUBLIC_LANGUAGES):
        fail("shape_inventory_required")
    for language in PUBLIC_LANGUAGES:
        expected_items = set(graph["symbols"][language]) | set(
            graph["members"][language]
        )
        expected_items |= set(quarantined["languages"][language]) | set(
            quarantined["members"][language]
        )
        if set(shape_hashes[language]) != expected_items:
            fail(f"shape_inventory_not_closed:{language}")
        if any(len(str(value)) != 64 for value in shape_hashes[language].values()):
            fail(f"invalid_shape_digest:{language}")

    validate_product_neutral_inventory(concepts)

    contracts = case_contracts()
    quality_cases = concepts.get("quality_gate_case_ids")
    if not isinstance(quality_cases, list) or quality_cases != sorted(
        set(quality_cases)
    ):
        fail("quality_gate_cases")
    if set(quality_cases) - set(contracts):
        fail("unknown_quality_gate_case")

    ownership: dict[tuple[str, str, str], list[str]] = defaultdict(list)
    for capability_id, definition in capabilities.items():
        if not isinstance(definition, dict):
            fail(f"invalid_capability:{capability_id}")
        profile = definition.get("profile")
        case_ids = definition.get("case_ids")
        if not isinstance(profile, str) or not profile:
            fail(f"invalid_capability_profile:{capability_id}")
        if not isinstance(case_ids, list) or case_ids != sorted(set(case_ids)):
            fail(f"invalid_capability_cases:{capability_id}")
        if set(case_ids) - set(contracts):
            fail(f"unknown_capability_case:{capability_id}")
        if set(case_ids) & set(quality_cases):
            fail(f"quality_gate_used_as_runtime_capability:{capability_id}")
        unproven = definition.get("unproven_requirements", [])
        if not isinstance(unproven, list):
            fail(f"invalid_unproven_requirements:{capability_id}")
        requirement_ids: list[str] = []
        for requirement in unproven:
            if not isinstance(requirement, dict):
                fail(f"invalid_unproven_requirement:{capability_id}")
            requirement_id = requirement.get("requirement_id")
            languages = requirement.get("languages")
            steps = requirement.get("acceptance_steps")
            if (
                not isinstance(requirement_id, str)
                or requirement_id in contracts
                or not isinstance(languages, list)
                or languages != sorted(set(languages), key=LANGUAGES.index)
                or not set(languages).issubset(LANGUAGES)
                or not isinstance(steps, list)
                or not steps
                or steps != sorted(set(steps))
                or not str(requirement.get("reason", "")).strip()
            ):
                fail(f"invalid_unproven_requirement:{capability_id}:{requirement_id}")
            requirement_ids.append(requirement_id)
        if requirement_ids != sorted(set(requirement_ids)):
            fail(f"duplicate_unproven_requirement:{capability_id}")
        projection = inventory[capability_id]
        owned_count = 0
        for language in PUBLIC_LANGUAGES:
            for section in ("symbols", "members"):
                for value in _projection_values(projection, section, language):
                    ownership[(language, section, value)].append(
                        f"capability:{capability_id}"
                    )
                    owned_count += 1
        if owned_count == 0:
            fail(f"capability_without_public_export:{capability_id}")

    seen_support: set[tuple[str, str, str]] = set()
    support_groups: dict[str, set[str]] = defaultdict(set)
    for item in supporting_items:
        if not isinstance(item, dict):
            fail("invalid_supporting_item")
        language = item.get("language")
        section = item.get("section")
        value = item.get("item")
        parent = item.get("parent_capability")
        role = item.get("role")
        key = (language, section, value)
        if (
            language not in PUBLIC_LANGUAGES
            or section not in {"symbols", "members"}
            or not isinstance(value, str)
            or parent not in capabilities
            or role not in {"configuration_limit", "serialization_projection"}
            or item.get("shape_sha256") != shape_hashes[language].get(value)
            or not isinstance(item.get("shape_group"), str)
            or not item["shape_group"]
            or key in seen_support
        ):
            fail(f"invalid_supporting_item:{language}:{section}:{value}")
        seen_support.add(key)
        ownership[key].append(f"support:{role}:{parent}")
        support_groups[item["shape_group"]].add(parent)
    if any(len(parents) != 1 for parents in support_groups.values()):
        fail("support_shape_group_crosses_capabilities")
    expected_support_groups: dict[str, dict[str, Any]] = {}
    for group_name in sorted(support_groups):
        members = [
            {
                "language": item["language"],
                "section": item["section"],
                "item": item["item"],
                "shape_sha256": item["shape_sha256"],
            }
            for item in supporting_items
            if item["shape_group"] == group_name
        ]
        members.sort(
            key=lambda member: (
                PUBLIC_LANGUAGES.index(member["language"]),
                member["section"],
                member["item"],
            )
        )
        present = sorted(
            {member["language"] for member in members}, key=PUBLIC_LANGUAGES.index
        )
        first = next(
            item for item in supporting_items if item["shape_group"] == group_name
        )
        expected_support_groups[group_name] = {
            "parent_capability": first["parent_capability"],
            "role": first["role"],
            "members": members,
            "present_languages": present,
            "missing_languages": [
                language for language in PUBLIC_LANGUAGES if language not in present
            ],
        }
    if declared_support_groups != expected_support_groups:
        fail("support_shape_groups_not_closed")

    for language in PUBLIC_LANGUAGES:
        for section, values in graph.items():
            for value in values[language]:
                owners = ownership.get((language, section, value), [])
                if not owners:
                    fail(f"unowned_public_export:{language}:{section}:{value}")
                if len(owners) != 1:
                    fail(
                        f"multiply_owned_public_export:{language}:{section}:{value}:"
                        + ",".join(owners)
                    )
        for owned_language, section, value in ownership:
            if owned_language == language and value not in graph[section][language]:
                fail(f"invented_public_export:{language}:{section}:{value}")

    packages = concepts.get("canonical_packages")
    if not isinstance(packages, dict) or set(packages) != set(PUBLIC_LANGUAGES):
        fail("canonical_packages_required")
    for language in PUBLIC_LANGUAGES:
        entries = packages[language]
        if not isinstance(entries, list) or not entries:
            fail(f"canonical_packages_empty:{language}")
        paths = [entry.get("path") for entry in entries if isinstance(entry, dict)]
        if paths != sorted(set(paths)) or len(paths) != len(entries):
            fail(f"canonical_packages_not_unique:{language}")
        for entry in entries:
            path = entry.get("path")
            category = entry.get("category")
            if category not in PACKAGE_CATEGORIES or not isinstance(path, str):
                fail(f"invalid_canonical_package:{language}")
            reason = canonical_quarantine_reason(package_identity(path))
            if reason is not None and category in PRODUCT_NEUTRAL_PACKAGE_CATEGORIES:
                fail(f"product_branded_canonical_package:{language}:{category}:{path}")
            if check_paths and not (ROOT / path).exists():
                fail(f"missing_canonical_package:{language}:{path}")

    validate_provider_implementations(concepts, check_paths=check_paths)
    validate_provider_proofs(concepts, contracts, check_paths=check_paths)
    validate_edge_adapter_policy(
        load_edge_adapter_policy(EDGE_ADAPTER_POLICY),
        concepts,
        root=ROOT,
        check_sources=False,
    )
    return concepts


def validate_provider_implementations(
    concepts: dict[str, Any], *, check_paths: bool
) -> None:
    implementations = concepts.get("provider_implementations")
    if not isinstance(implementations, list):
        fail("provider_implementations_required")
    seen: set[tuple[str, str]] = set()
    revision = concepts.get("dependency_revisions", {}).get("axon_sdk")
    required_fields = {
        "language",
        "identity",
        "production_owner",
        "owner_path",
        "path",
        "sha256",
        "interface",
        "interface_path",
        "interface_sha256",
        "capability_ids",
        "axon_revision",
    }
    provider_identities: dict[str, dict[str, str]] = defaultdict(dict)
    for implementation in implementations:
        if not isinstance(implementation, dict):
            fail("invalid_provider_implementation")
        language = implementation.get("language")
        identity = implementation.get("identity")
        production_owner = implementation.get("production_owner")
        owner_path = implementation.get("owner_path")
        path = implementation.get("path")
        interface = implementation.get("interface")
        interface_path = implementation.get("interface_path")
        key = (language, identity)
        capabilities = implementation.get("capability_ids")
        if (
            language not in PUBLIC_LANGUAGES
            or not isinstance(identity, str)
            or not identity
            or not isinstance(production_owner, str)
            or not production_owner
            or not isinstance(owner_path, str)
            or not owner_path
            or not isinstance(path, str)
            or not path.startswith(owner_path)
            or not isinstance(interface, str)
            or not interface
            or not isinstance(interface_path, str)
            or not interface_path
            or not isinstance(capabilities, list)
            or not capabilities
            or set(capabilities) - set(concepts["capabilities"])
            or implementation.get("axon_revision") != revision
            or set(implementation) != required_fields
            or key in seen
        ):
            fail(f"invalid_provider_implementation:{language}:{identity}")
        if re.search(r"\beasy(?:net|remote)\b", production_owner, flags=re.I):
            fail(f"product_specific_provider_owner:{language}:{identity}")
        if identity == "direct_runtime_provider" and {
            "stream",
            "bidi",
        }.intersection(capabilities):
            fail(f"direct_runtime_claims_unsupported_streaming:{language}")
        if (
            language in {"go", "python"}
            and {"stream", "bidi"}.intersection(capabilities)
            and identity != "cabi_runtime_provider"
        ):
            fail(f"noncanonical_streaming_provider:{language}:{identity}")
        seen.add(key)
        for capability_id in capabilities:
            provider_identities[capability_id][language] = identity
        source = ROOT / path
        interface_source = ROOT / interface_path
        if check_paths and (
            not source.is_file()
            or hashlib.sha256(source.read_bytes()).hexdigest()
            != implementation.get("sha256")
            or not interface_source.is_file()
            or hashlib.sha256(interface_source.read_bytes()).hexdigest()
            != implementation.get("interface_sha256")
            or interface not in interface_source.read_text(encoding="utf-8")
        ):
            fail(f"provider_implementation_mismatch:{language}:{identity}")
    for capability_id, identities in provider_identities.items():
        shared = {
            identities[language]
            for language in ("go", "python")
            if language in identities
        }
        if len(shared) > 1:
            fail(f"cross_language_provider_identity_divergence:{capability_id}")


def validate_provider_proofs(
    concepts: dict[str, Any],
    contracts: dict[str, dict[str, Any]],
    *,
    check_paths: bool,
) -> None:
    proofs = concepts.get("provider_proofs")
    if not isinstance(proofs, dict):
        fail("provider_proofs_required")
    capabilities = concepts["capabilities"]
    bindings = execution_bindings()
    registered = concepts["provider_implementations"]
    required_proofs: dict[str, set[str]] = defaultdict(set)
    for implementation in registered:
        language = implementation["language"]
        for capability_id in implementation["capability_ids"]:
            capability = capabilities[capability_id]
            if any(
                language in requirement["languages"]
                for requirement in capability.get("unproven_requirements", [])
            ):
                continue
            case_ids = [
                case_id
                for case_id in capability["case_ids"]
                if language in contracts[case_id]["languages"]
            ]
            if case_ids and all(
                (language, case_id) in bindings for case_id in case_ids
            ):
                required_proofs[capability_id].add(language)
    for capability_id, languages in proofs.items():
        if capability_id not in capabilities or not isinstance(languages, dict):
            fail(f"unknown_provider_capability:{capability_id}")
        for language, proof in languages.items():
            if language not in PUBLIC_LANGUAGES or not isinstance(proof, dict):
                fail(f"invalid_provider_language:{capability_id}:{language}")
            if any(
                language in requirement["languages"]
                for requirement in capabilities[capability_id].get(
                    "unproven_requirements", []
                )
            ):
                fail(f"provider_has_unproven_requirement:{capability_id}:{language}")
            implementation = proof.get("implementation")
            if not isinstance(implementation, dict):
                fail(f"provider_implementation_required:{capability_id}:{language}")
            identity = implementation.get("identity")
            owner_path = implementation.get("owner_path")
            path = implementation.get("path")
            digest = implementation.get("sha256")
            if not isinstance(identity, str) or not identity:
                fail(f"provider_identity_required:{capability_id}:{language}")
            if not isinstance(owner_path, str) or not owner_path:
                fail(f"invalid_provider_owner:{capability_id}:{language}")
            if not isinstance(path, str) or not path.startswith(owner_path):
                fail(f"provider_path_outside_owner:{capability_id}:{language}")
            implementation_path = ROOT / path
            if check_paths and (
                not implementation_path.is_file()
                or hashlib.sha256(implementation_path.read_bytes()).hexdigest()
                != digest
            ):
                fail(f"provider_implementation_mismatch:{capability_id}:{language}")

            interface = implementation.get("interface")
            revision = implementation.get("revision")
            if not isinstance(interface, str) or not interface:
                fail(f"provider_interface_required:{capability_id}:{language}")
            if revision != concepts.get("dependency_revisions", {}).get("axon_sdk"):
                fail(f"provider_revision_mismatch:{capability_id}:{language}")
            matching_implementations = [
                candidate
                for candidate in registered
                if candidate["language"] == language
                and candidate["identity"] == identity
                and candidate["owner_path"] == owner_path
                and candidate["path"] == path
                and candidate["sha256"] == digest
                and candidate["interface"] == interface
            ]
            if len(matching_implementations) != 1:
                fail(
                    f"provider_implementation_not_registered:{capability_id}:{language}"
                )
            if capability_id not in matching_implementations[0]["capability_ids"]:
                fail(
                    f"provider_implementation_capability_mismatch:{capability_id}:{language}"
                )
            if capability_id in {"stream", "bidi"} and language in {"go", "python"}:
                expected_evidence = {
                    "go": ["sdk/go/cabi_runtime_test.go"],
                    "python": ["sdk/python/tests/test_cabi.py"],
                }[language]
                for case_id in capabilities[capability_id]["case_ids"]:
                    if language not in contracts[case_id]["languages"]:
                        continue
                    binding = bindings.get((language, case_id))
                    if binding is None or binding.get("evidence") != expected_evidence:
                        fail(
                            "streaming_provider_evidence_mismatch:"
                            f"{capability_id}:{language}:{case_id}"
                        )
            expected_steps = {
                (case_id, action)
                for case_id in capabilities[capability_id]["case_ids"]
                if language in contracts[case_id]["languages"]
                for action in contracts[case_id]["actions"]
            }
            mappings = proof.get("step_evidence")
            if not isinstance(mappings, list):
                fail(f"provider_step_evidence_required:{capability_id}:{language}")
            actual_steps: set[tuple[str, str]] = set()
            for mapping in mappings:
                if not isinstance(mapping, dict):
                    fail(f"invalid_provider_step:{capability_id}:{language}")
                key = (mapping.get("case_id"), mapping.get("action"))
                if key in actual_steps:
                    fail(f"duplicate_provider_step:{capability_id}:{language}:{key}")
                actual_steps.add(key)
                binding = bindings.get((language, key[0]))
                if binding is None or binding.get("selector") != mapping.get(
                    "selector"
                ):
                    fail(
                        f"provider_step_selector_unbound:{capability_id}:{language}:{key}"
                    )
                selector = mapping.get("selector")
                if not isinstance(selector, str) or not selector:
                    fail(
                        f"provider_step_selector_required:{capability_id}:{language}:{key}"
                    )
            if actual_steps != expected_steps:
                fail(f"provider_steps_not_closed:{capability_id}:{language}")
    for capability_id, languages in required_proofs.items():
        for language in sorted(languages, key=PUBLIC_LANGUAGES.index):
            if language not in proofs.get(capability_id, {}):
                fail(f"provider_proof_required:{capability_id}:{language}")


def canonical_package_paths(concepts: dict[str, Any]) -> list[str]:
    validate_schema(concepts)
    validate_package_roots(concepts)
    return [
        entry["path"]
        for language in ("go", "python")
        for entry in concepts["canonical_packages"][language]
        if entry["category"] == "provider_neutral_core"
    ]


def validate_package_roots(concepts: dict[str, Any]) -> None:
    completed = subprocess.run(
        ["go", "list", "-f", "{{.Dir}}", "./..."],
        cwd=ROOT / "sdk/go",
        check=True,
        capture_output=True,
        text=True,
    )
    actual_go = {
        str(Path(path).resolve().relative_to(ROOT))
        for path in completed.stdout.splitlines()
        if path
    }
    python_base = ROOT / "sdk/python/easynet_sdk"
    actual_python = {"sdk/python/easynet_sdk"}
    actual_python.update(
        str(path.relative_to(ROOT))
        for path in python_base.rglob("*")
        if path.is_dir() and (path / "__init__.py").is_file()
    )
    for language, actual in (("go", actual_go), ("python", actual_python)):
        declared = {entry["path"] for entry in concepts["canonical_packages"][language]}
        if actual != declared:
            unknown = ",".join(sorted(actual - declared))
            stale = ",".join(sorted(declared - actual))
            fail(
                f"unclassified_package_roots:{language}:unknown={unknown}:stale={stale}"
            )


def actual_python_graph() -> tuple[set[str], set[str]]:
    import easynet_sdk

    symbols = set(getattr(easynet_sdk, "__all__", ()))
    members: set[str] = set()
    for symbol in symbols:
        value = getattr(easynet_sdk, symbol, None)
        if isinstance(value, type):
            declared = set(vars(value))
            declared.update(getattr(value, "__annotations__", {}))
            declared.update(getattr(value, "__dataclass_fields__", {}))
            declared.update(getattr(value, "__members__", {}))
            declared.update(getattr(value, "_fields", ()))
            for member in declared:
                if not member.startswith("_"):
                    members.add(f"{symbol}.{member}")
    return symbols, members


def actual_go_graph(go_bin: str) -> tuple[set[str], set[str]]:
    completed = subprocess.run(
        [go_bin, "run", "./tools/sdk-api-inventory/main.go", "-dir", "sdk/go"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    decoded = json.loads(completed.stdout)
    return set(decoded["symbols"]), set(decoded["members"])


def actual_structured_graph(
    language: str,
) -> tuple[set[str], set[str], dict[str, str], str]:
    completed = subprocess.run(
        [
            sys.executable,
            str(ROOT / "sdk/conformance/public_api_inventory.py"),
            language,
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    decoded = json.loads(completed.stdout)
    return (
        set(decoded["symbols"]),
        set(decoded["members"]),
        {
            item: hashlib.sha256(shape.encode()).hexdigest()
            for item, shape in decoded["shapes"].items()
        },
        decoded.get("source_revision", "current_checkout"),
    )


def validate_actual(concepts: dict[str, Any], go_bin: str) -> None:
    validate_schema(concepts)
    del go_bin
    actual = {
        language: actual_structured_graph(language) for language in PUBLIC_LANGUAGES
    }
    for language, (symbols, members, shapes, source_revision) in actual.items():
        if source_revision != concepts["inventory_source_revisions"][language]:
            fail(
                f"inventory_source_revision_mismatch:{language}:"
                f"expected={concepts['inventory_source_revisions'][language]}:actual={source_revision}"
            )
        for section, found in (("symbols", symbols), ("members", members)):
            graph_section = "languages" if section == "symbols" else "members"
            expected = set(concepts[graph_section][language]) | set(
                concepts["non_canonical"][graph_section][language]
            )
            if found != expected:
                missing = ",".join(sorted(found - expected))
                stale = ",".join(sorted(expected - found))
                fail(
                    f"public_graph_mismatch:{language}:{section}:"
                    f"untracked={missing}:stale={stale}"
                )
        if shapes != concepts["shape_sha256"][language]:
            missing = ",".join(
                sorted(set(shapes) - set(concepts["shape_sha256"][language]))
            )
            stale = ",".join(
                sorted(set(concepts["shape_sha256"][language]) - set(shapes))
            )
            changed = ",".join(
                sorted(
                    item
                    for item in set(shapes) & set(concepts["shape_sha256"][language])
                    if shapes[item] != concepts["shape_sha256"][language][item]
                )
            )
            fail(
                f"public_shape_mismatch:{language}:untracked={missing}:stale={stale}:changed={changed}"
            )


def self_test(tmp: Path) -> None:
    concepts = validate_schema(load_json())

    def expect(
        mutated: dict[str, Any], marker: str, *, check_paths: bool = False
    ) -> None:
        try:
            validate_schema(mutated, check_paths=check_paths)
        except ValueError as error:
            if marker not in str(error):
                raise
        else:
            fail(f"self_test_expected:{marker}")

    capability_id = next(iter(concepts["capability_inventory"]))
    projection = concepts["capability_inventory"][capability_id]["go"]
    section = "symbols" if projection["symbols"] else "members"
    value = projection[section][0]

    unowned = copy.deepcopy(concepts)
    unowned["capability_inventory"][capability_id]["go"][section].remove(value)
    expect(unowned, "unowned_public_export")

    duplicate = copy.deepcopy(concepts)
    other = next(
        key for key in duplicate["capability_inventory"] if key != capability_id
    )
    duplicate["capability_inventory"][other]["go"][section].append(value)
    duplicate["capability_inventory"][other]["go"][section].sort()
    expect(duplicate, "multiply_owned_public_export")

    product_leak = copy.deepcopy(concepts)
    graph_section = "languages" if section == "symbols" else "members"
    leaked_value = (
        "LeakedDaemonHandle" if section == "symbols" else "LeakedDaemonHandle.Start"
    )
    product_leak[graph_section]["go"].remove(value)
    product_leak[graph_section]["go"].append(leaked_value)
    product_leak[graph_section]["go"].sort()
    product_leak["capability_inventory"][capability_id]["go"][section].remove(value)
    product_leak["capability_inventory"][capability_id]["go"][section].append(
        leaked_value
    )
    product_leak["capability_inventory"][capability_id]["go"][section].sort()
    product_leak["shape_sha256"]["go"][leaked_value] = product_leak["shape_sha256"][
        "go"
    ].pop(value)
    expect(product_leak, "canonical_inventory_product_leak")

    quarantine_cases = {
        "canonical_invocation_bytes": "Plain canonical/admission helpers",
        "axiom.canonical_invocation_bytes": "Plain canonical/admission helpers",
        "sign_invocation": "Plain canonical/admission helpers",
        "verify_invocation_signature": "Plain canonical/admission helpers",
        "verify_phase": "Plain canonical/admission helpers",
        "verify_signature": "Plain canonical/admission helpers",
        "run_admission": "Plain canonical/admission helpers",
        "default_auth_for_subject": "Process-local signer fallback",
        "GeneratedSubjectAuth": "Process-local signer fallback",
        "generate_private_agent_auth": "Process-local signer fallback",
        "generate_private_hub_auth": "Process-local signer fallback",
        "GenerateSubjectAuth": "Process-local signer fallback",
        "ProcessLocalSigner": "Process-local signer fallback",
        "PrivateKeyAuthenticator": "Process-local signer fallback",
        "start_daemon": "Daemon-bound provider",
        "ModeDevice": "Non-URA device/hub",
        "RuntimeModeHub": "Non-URA device/hub",
        "RuntimeAdminAbilityClient.RevokeDevice": "Non-URA device/hub",
    }
    for item, marker in quarantine_cases.items():
        reason = canonical_quarantine_reason(item)
        if reason is None or marker not in reason:
            fail(f"self_test_quarantine_policy:{item}:{reason}")
    if canonical_quarantine_reason("ParsedURA.DeviceID") is not None:
        fail("self_test_ura_grammar_quarantine")

    bad_status_names = copy.deepcopy(concepts)
    bad_status_names["status_canonical_names"]["seam"] = "PublicSeam"
    expect(bad_status_names, "status_canonical_names")

    bad_lifecycle_reference = copy.deepcopy(concepts)
    bad_lifecycle_reference["canonical_lifecycle_contract"]["transition_vectors"][
        "sha256"
    ] = "0" * 64
    expect(bad_lifecycle_reference, "canonical_lifecycle_contract")

    duplicate_lifecycle_contract = copy.deepcopy(concepts)
    duplicate_lifecycle_contract["lifecycle_actions"] = ["invented"]
    expect(duplicate_lifecycle_contract, "duplicate_lifecycle_contract")

    unapproved_quarantine = copy.deepcopy(concepts)
    unapproved_value = next(
        value
        for value in unapproved_quarantine["languages"]["go"]
        if canonical_quarantine_reason(value) is None
    )
    unapproved_quarantine["languages"]["go"].remove(unapproved_value)
    unapproved_quarantine["non_canonical"]["languages"]["go"].append(unapproved_value)
    unapproved_quarantine["non_canonical"]["languages"]["go"].sort()
    unapproved_quarantine["legacy_quarantine"]["languages"]["go"][unapproved_value] = {
        "canonical_replacement": ["capability_inventory.runtime_lifecycle"],
        "consumer_cutover_ref": PRODUCT_NEUTRAL_CUTOVER_REF,
        "removal_phase": "quarantined",
        "reason": "test-only quarantine entry",
    }
    expect(unapproved_quarantine, "unapproved_legacy_quarantine")

    mismatched_quarantine = copy.deepcopy(concepts)
    quarantined_value = mismatched_quarantine["non_canonical"]["languages"]["go"][0]
    mismatched_quarantine["legacy_quarantine"]["languages"]["go"][quarantined_value][
        "reason"
    ] = "test-only mismatched quarantine reason"
    expect(mismatched_quarantine, "legacy_quarantine_reason_mismatch")

    invented = copy.deepcopy(concepts)
    invented["capabilities"]["invented"] = {"profile": "runtime_core", "case_ids": []}
    invented["capability_inventory"]["invented"] = {
        language: {"symbols": [], "members": []} for language in PUBLIC_LANGUAGES
    }
    expect(invented, "capability_without_public_export")

    missing_root = copy.deepcopy(concepts)
    missing_root["canonical_packages"]["go"][0]["path"] = "sdk/go/000-missing-core"
    expect(missing_root, "missing_canonical_package", check_paths=True)

    branded_canonical_root = copy.deepcopy(concepts)
    branded_canonical_root["canonical_packages"]["python"][0]["category"] = (
        "public_facade"
    )
    expect(branded_canonical_root, "product_branded_canonical_package")

    duplicated_provider_state = copy.deepcopy(concepts)
    duplicated_provider_state["provider_implementations"][0]["proof_state"] = (
        "provider-backed"
    )
    expect(
        duplicated_provider_state,
        "invalid_provider_implementation",
        check_paths=True,
    )

    product_specific_provider_owner = copy.deepcopy(concepts)
    product_specific_provider_owner["provider_implementations"][0][
        "production_owner"
    ] = "EasyNet daemon provider"
    expect(
        product_specific_provider_owner,
        "product_specific_provider_owner",
        check_paths=True,
    )

    direct_streaming_claim = copy.deepcopy(concepts)
    direct_provider = next(
        implementation
        for implementation in direct_streaming_claim["provider_implementations"]
        if implementation["language"] == "go"
        and implementation["identity"] == "direct_runtime_provider"
    )
    direct_provider["capability_ids"].append("stream")
    expect(
        direct_streaming_claim,
        "direct_runtime_claims_unsupported_streaming",
        check_paths=True,
    )

    divergent_streaming_provider = copy.deepcopy(concepts)
    python_streaming_provider = next(
        implementation
        for implementation in divergent_streaming_provider[
            "provider_implementations"
        ]
        if implementation["language"] == "python"
        and implementation["identity"] == "cabi_runtime_provider"
    )
    python_streaming_provider["identity"] = "python_streaming_provider"
    expect(
        divergent_streaming_provider,
        "noncanonical_streaming_provider",
        check_paths=True,
    )

    provider_capability_id = (
        "native_runtime"
        if "native_runtime" in concepts["capabilities"]
        else capability_id
    )
    implementation_path = ROOT / "sdk/go/direct_runtime.go"
    implementation_sha = hashlib.sha256(implementation_path.read_bytes()).hexdigest()
    provider = copy.deepcopy(concepts)
    provider["provider_proofs"][provider_capability_id] = {
        "go": {
            "implementation": {
                "identity": "direct_runtime_provider",
                "owner_path": "sdk/go/",
                "path": "sdk/go/missing.go",
                "sha256": "0" * 64,
                "interface": "RuntimeConnector",
                "revision": concepts["dependency_revisions"]["axon_sdk"],
            },
            "step_evidence": [],
        }
    }
    expect(provider, "provider_implementation_mismatch", check_paths=True)

    incomplete_steps = copy.deepcopy(concepts)
    incomplete_steps["provider_proofs"][provider_capability_id] = {
        "go": {
            "implementation": {
                "identity": "direct_runtime_provider",
                "owner_path": "sdk/go/",
                "path": "sdk/go/direct_runtime.go",
                "sha256": implementation_sha,
                "interface": "RuntimeConnector",
                "revision": concepts["dependency_revisions"]["axon_sdk"],
            },
            "step_evidence": [],
        }
    }
    expect(
        incomplete_steps,
        f"provider_steps_not_closed:{provider_capability_id}:go",
        check_paths=True,
    )

    missing_required_proof = copy.deepcopy(concepts)
    del missing_required_proof["provider_proofs"][provider_capability_id]["go"]
    expect(
        missing_required_proof,
        f"provider_proof_required:{provider_capability_id}:go",
        check_paths=True,
    )

    irrelevant_implementation = copy.deepcopy(concepts)
    irrelevant_implementation["provider_proofs"]["runtime_health"] = {
        "go": {
            "implementation": {
                "identity": "direct_runtime_provider",
                "owner_path": "sdk/go/",
                "path": "sdk/go/direct_runtime.go",
                "sha256": implementation_sha,
                "interface": "RuntimeConnector",
                "revision": concepts["dependency_revisions"]["axon_sdk"],
            },
            "step_evidence": [],
        }
    }
    expect(
        irrelevant_implementation,
        "provider_implementation_capability_mismatch",
        check_paths=True,
    )
    tmp.mkdir(parents=True, exist_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--validate-schema", action="store_true")
    parser.add_argument("--validate-actual", action="store_true")
    parser.add_argument("--print-neutrality-roots", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--manifest", type=Path, default=CONCEPTS)
    parser.add_argument("--go-bin", default="go")
    parser.add_argument("--tmp", type=Path)
    args = parser.parse_args()
    try:
        concepts = load_json(args.manifest)
        if args.validate_schema:
            validate_schema(concepts)
            print("sdk concepts schema ok")
        elif args.validate_actual:
            validate_actual(concepts, args.go_bin)
            print("sdk concepts public graph ok")
        elif args.print_neutrality_roots:
            print("\n".join(canonical_package_paths(concepts)))
        elif args.self_test:
            if args.tmp is None:
                fail("self_test_tmp_required")
            self_test(args.tmp)
            print("sdk concepts self-test ok")
        else:
            parser.error("one mode is required")
    except (
        OSError,
        ValueError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as error:
        print(f"sdk_concepts: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
