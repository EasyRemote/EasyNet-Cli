#!/usr/bin/env python3
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

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
LIFECYCLE_ACTIONS = {
    "start",
    "dispatch",
    "stream_open",
    "bidi_open",
    "child_dispatch",
    "cancel",
    "deadline",
    "terminal_receipt",
    "restart_recover",
}
LIFECYCLE_TRANSITION_FIELDS = {
    "allowed_source_states",
    "transition",
    "deadline_owner",
    "child_deadline_propagation",
    "cancellation_authority",
    "cancellation_ack",
    "idempotent_replay_result",
    "queue_concurrency_limits",
    "cleanup_responsibility",
    "receipt_event_observability",
}
LIFECYCLE_STATES = {
    "admitted",
    "bidi_open",
    "cancelled",
    "child_dispatched",
    "completed",
    "dispatched",
    "failed",
    "recovering",
    "running",
    "runtime_started",
    "runtime_unstarted",
    "stream_open",
    "terminal",
    "timed_out",
}
LIFECYCLE_TRANSITION_KINDS = {"next", "terminal"}
LIFECYCLE_TRANSITION_CONTRACT = {
    "bidi_open": {
        "allowed_source_states": ["dispatched", "running"],
        "transition": {"kind": "next", "state": "bidi_open"},
        "deadline_owner": "invocation_deadline_owner",
        "child_deadline_propagation": "bidi_children_inherit_remaining_parent_deadline",
        "cancellation_authority": "caller_or_parent",
        "cancellation_ack": "close_send_is_half_close_cancel_requires_explicit_cancel",
        "idempotent_replay_result": "duplicate_bidi_open_returns_existing_session_or_invalid_handle",
        "queue_concurrency_limits": "bounded_bidi_frame_queue_and_session_permit",
        "cleanup_responsibility": "release_bidi_session_permit_after_terminal_frame_or_close",
        "receipt_event_observability": "bidi_frame0_admission_and_terminal_frame_visible",
    },
    "cancel": {
        "allowed_source_states": [
            "admitted",
            "bidi_open",
            "child_dispatched",
            "dispatched",
            "running",
            "stream_open",
        ],
        "transition": {"kind": "terminal", "state": "cancelled"},
        "deadline_owner": "cancel_request_uses_runtime_control_deadline",
        "child_deadline_propagation": "parent_cancel_propagates_to_live_children",
        "cancellation_authority": "caller_parent_or_runtime_supervisor",
        "cancellation_ack": "cancel_ack_is_request_lifecycle_not_synthetic_terminal",
        "idempotent_replay_result": "duplicate_cancel_returns_existing_terminal_or_pending_intent",
        "queue_concurrency_limits": "single_cancel_winner_with_bounded_child_fanout",
        "cleanup_responsibility": "cleanup_processes_streams_and_permits_before_terminal_receipt",
        "receipt_event_observability": "cancel_event_and_cancelled_terminal_receipt_visible",
    },
    "child_dispatch": {
        "allowed_source_states": ["bidi_open", "running", "stream_open"],
        "transition": {"kind": "next", "state": "child_dispatched"},
        "deadline_owner": "parent_invocation_deadline_owner",
        "child_deadline_propagation": "child_deadline_cannot_exceed_parent_remaining_deadline",
        "cancellation_authority": "parent_invocation_control",
        "cancellation_ack": "parent_terminal_cancels_live_child_dispatch",
        "idempotent_replay_result": "child_dispatch_nonce_replay_returns_original_child_receipt",
        "queue_concurrency_limits": "bounded_child_dispatch_fanout",
        "cleanup_responsibility": "child_permits_released_before_parent_terminal_receipt",
        "receipt_event_observability": "parent_receipt_records_child_receipt_link",
    },
    "deadline": {
        "allowed_source_states": [
            "admitted",
            "bidi_open",
            "child_dispatched",
            "dispatched",
            "running",
            "stream_open",
        ],
        "transition": {"kind": "terminal", "state": "timed_out"},
        "deadline_owner": "invocation_deadline_owner",
        "child_deadline_propagation": "deadline_terminal_propagates_to_live_children",
        "cancellation_authority": "runtime_deadline_timer",
        "cancellation_ack": "deadline_terminal_is_not_user_cancel_ack",
        "idempotent_replay_result": "late_deadline_observation_returns_existing_timed_out_receipt",
        "queue_concurrency_limits": "deadline_timers_are_bounded_by_live_invocation_count",
        "cleanup_responsibility": "deadline_cleanup_completes_before_timed_out_receipt",
        "receipt_event_observability": "deadline_exceeded_event_and_timed_out_receipt_visible",
    },
    "dispatch": {
        "allowed_source_states": ["admitted"],
        "transition": {"kind": "next", "state": "dispatched"},
        "deadline_owner": "invocation_deadline_owner",
        "child_deadline_propagation": "not_applicable_until_child_dispatch",
        "cancellation_authority": "caller_or_parent",
        "cancellation_ack": "cancel_after_dispatch_records_control_intent",
        "idempotent_replay_result": "duplicate_dispatch_rejected_by_nonce_replay",
        "queue_concurrency_limits": "bounded_runtime_dispatch_queue",
        "cleanup_responsibility": "dispatch_permit_released_at_terminal_receipt",
        "receipt_event_observability": "admission_and_dispatch_events_visible",
    },
    "restart_recover": {
        "allowed_source_states": ["recovering", "runtime_started"],
        "transition": {"kind": "next", "state": "runtime_started"},
        "deadline_owner": "runtime_supervisor_recovery_deadline",
        "child_deadline_propagation": "recovered_children_keep_original_deadline_bounds",
        "cancellation_authority": "runtime_supervisor",
        "cancellation_ack": "orphan_reap_records_cleanup_without_fabricating_success",
        "idempotent_replay_result": "repeated_recovery_returns_same_replayed_terminal_facts",
        "queue_concurrency_limits": "recovery_scan_is_bounded_by_persisted_live_invocations",
        "cleanup_responsibility": "orphan_processes_and_permits_reaped_before_ready",
        "receipt_event_observability": "recovery_events_and_replayed_terminal_receipts_visible",
    },
    "start": {
        "allowed_source_states": ["runtime_unstarted"],
        "transition": {"kind": "next", "state": "runtime_started"},
        "deadline_owner": "runtime_supervisor_start_deadline",
        "child_deadline_propagation": "not_applicable_before_invocation",
        "cancellation_authority": "runtime_supervisor",
        "cancellation_ack": "start_cancel_reports_unavailable_without_invocation_receipt",
        "idempotent_replay_result": "repeat_start_returns_existing_runtime_handle",
        "queue_concurrency_limits": "runtime_start_serialized_per_process_root",
        "cleanup_responsibility": "failed_start_releases_process_root_and_sockets",
        "receipt_event_observability": "runtime_health_event_no_invocation_receipt",
    },
    "stream_open": {
        "allowed_source_states": ["dispatched", "running"],
        "transition": {"kind": "next", "state": "stream_open"},
        "deadline_owner": "invocation_deadline_owner",
        "child_deadline_propagation": "stream_children_inherit_remaining_parent_deadline",
        "cancellation_authority": "caller_or_parent",
        "cancellation_ack": "stream_cancel_is_request_then_terminal_event",
        "idempotent_replay_result": "duplicate_stream_open_returns_existing_stream_or_invalid_handle",
        "queue_concurrency_limits": "bounded_stream_callback_queue_and_stream_permit",
        "cleanup_responsibility": "release_stream_permit_after_terminal_event_or_close",
        "receipt_event_observability": "stream_open_data_and_terminal_events_visible",
    },
    "terminal_receipt": {
        "allowed_source_states": [
            "bidi_open",
            "child_dispatched",
            "dispatched",
            "running",
            "stream_open",
        ],
        "transition": {"kind": "terminal", "state": "terminal"},
        "deadline_owner": "terminal_receipt_preserves_original_deadline_owner",
        "child_deadline_propagation": "terminal_parent_closes_or_cancels_live_children",
        "cancellation_authority": "not_applicable_after_terminal_receipt",
        "cancellation_ack": "post_terminal_cancel_is_idempotent_observation",
        "idempotent_replay_result": "terminal_receipt_replay_returns_same_receipt",
        "queue_concurrency_limits": "terminal_receipt_closes_live_queue_slots",
        "cleanup_responsibility": "all_runtime_resources_released_before_terminal_receipt",
        "receipt_event_observability": "exactly_one_terminal_receipt_with_proof_facts",
    },
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


def package_identity(path: str) -> str:
    return Path(path).name


def case_contracts(cases_root: Path | None = None) -> dict[str, dict[str, Any]]:
    contracts: dict[str, dict[str, Any]] = {}
    root = cases_root or ROOT / "sdk/conformance/cases"
    for path in sorted(root.glob("*.yaml")):
        case_id = ""
        languages: list[str] = []
        actions: list[str] = []
        lifecycle_actions: list[str] = []
        section = ""
        for raw in path.read_text(encoding="utf-8").splitlines():
            line = raw.strip()
            if raw.startswith("id:"):
                case_id = raw.split(":", 1)[1].strip()
            elif raw == "required_for:":
                section = "required_for"
            elif raw == "lifecycle_actions:":
                section = "lifecycle_actions"
            elif raw == "steps:":
                section = "steps"
            elif raw and not raw.startswith(" "):
                section = ""
            elif section == "required_for" and line.startswith("- "):
                languages.append(line[2:].strip())
            elif section == "lifecycle_actions" and line.startswith("- "):
                lifecycle_actions.append(line[2:].strip())
            elif section == "steps" and line.startswith("- action:"):
                actions.append(line.split(":", 1)[1].strip())
        if not case_id or case_id in contracts or not actions:
            fail(f"invalid_case_contract:{path}")
        if lifecycle_actions != sorted(set(lifecycle_actions)):
            fail(f"invalid_lifecycle_case_actions:{path}")
        if set(lifecycle_actions) - LIFECYCLE_ACTIONS:
            fail(f"unknown_lifecycle_case_actions:{path}")
        contracts[case_id] = {
            "path": path,
            "languages": languages,
            "actions": actions,
            "lifecycle_actions": lifecycle_actions,
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
                    fail(f"canonical_inventory_product_leak:{language}:{section}:{value}")

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


def validate_lifecycle_transition_contract(concepts: dict[str, Any]) -> None:
    contract = concepts.get("lifecycle_transition_contract")
    if contract != LIFECYCLE_TRANSITION_CONTRACT:
        fail("lifecycle_transition_contract")
    if set(contract) != LIFECYCLE_ACTIONS:
        fail("lifecycle_transition_contract_actions")
    for action, entry in contract.items():
        if set(entry) != LIFECYCLE_TRANSITION_FIELDS:
            fail(f"lifecycle_transition_fields:{action}")
        sources = entry["allowed_source_states"]
        if (
            not isinstance(sources, list)
            or sources != sorted(set(sources))
            or not set(sources).issubset(LIFECYCLE_STATES)
        ):
            fail(f"lifecycle_transition_sources:{action}")
        transition = entry["transition"]
        if (
            not isinstance(transition, dict)
            or set(transition) != {"kind", "state"}
            or transition["kind"] not in LIFECYCLE_TRANSITION_KINDS
            or transition["state"] not in LIFECYCLE_STATES
        ):
            fail(f"lifecycle_transition_target:{action}")
        for field in LIFECYCLE_TRANSITION_FIELDS - {"allowed_source_states", "transition"}:
            if not isinstance(entry[field], str) or not entry[field].strip():
                fail(f"lifecycle_transition_metadata:{action}:{field}")


def validate_schema(
    concepts: dict[str, Any], *, check_paths: bool = True
) -> dict[str, Any]:
    if concepts.get("schema_version") != 4 or concepts.get("complete_inventory") is not True:
        fail("concept_schema_version")
    if concepts.get("matrix_languages") != LANGUAGES:
        fail("matrix_languages")
    if concepts.get("status_order") != STATUSES:
        fail("status_order")
    if concepts.get("status_canonical_names") != STATUS_CANONICAL_NAMES:
        fail("status_canonical_names")
    lifecycle_actions = concepts.get("lifecycle_actions")
    if lifecycle_actions != sorted(LIFECYCLE_ACTIONS):
        fail("lifecycle_actions")
    validate_lifecycle_transition_contract(concepts)
    source_revisions = concepts.get("inventory_source_revisions")
    if not isinstance(source_revisions, dict) or set(source_revisions) != set(PUBLIC_LANGUAGES):
        fail("inventory_source_revisions")
    expected_axon = concepts.get("dependency_revisions", {}).get("easynet_axon")
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
            if canonical != sorted(set(canonical)) or legacy != sorted(set(legacy or [])):
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
                    fail(f"legacy_quarantine_reason_mismatch:{language}:{section}:{value}")
                replacements = entry["canonical_replacement"]
                if not isinstance(replacements, list) or replacements != sorted(set(replacements)) or not replacements:
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
                if entry["removal_phase"] not in {"quarantined", "consumer_cutover", "removal_ready"}:
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
        expected_items = set(graph["symbols"][language]) | set(graph["members"][language])
        expected_items |= set(quarantined["languages"][language]) | set(quarantined["members"][language])
        if set(shape_hashes[language]) != expected_items:
            fail(f"shape_inventory_not_closed:{language}")
        if any(len(str(value)) != 64 for value in shape_hashes[language].values()):
            fail(f"invalid_shape_digest:{language}")

    validate_product_neutral_inventory(concepts)

    contracts = case_contracts()
    quality_cases = concepts.get("quality_gate_case_ids")
    if not isinstance(quality_cases, list) or quality_cases != sorted(set(quality_cases)):
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
        members.sort(key=lambda member: (PUBLIC_LANGUAGES.index(member["language"]), member["section"], member["item"]))
        present = sorted({member["language"] for member in members}, key=PUBLIC_LANGUAGES.index)
        first = next(item for item in supporting_items if item["shape_group"] == group_name)
        expected_support_groups[group_name] = {
            "parent_capability": first["parent_capability"],
            "role": first["role"],
            "members": members,
            "present_languages": present,
            "missing_languages": [language for language in PUBLIC_LANGUAGES if language not in present],
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
    return concepts


def validate_provider_implementations(
    concepts: dict[str, Any], *, check_paths: bool
) -> None:
    implementations = concepts.get("provider_implementations")
    if not isinstance(implementations, list):
        fail("provider_implementations_required")
    seen: set[tuple[str, str]] = set()
    revision = concepts.get("dependency_revisions", {}).get("easynet_axon")
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
            or not isinstance(identity, str) or not identity
            or not isinstance(production_owner, str) or not production_owner
            or not isinstance(owner_path, str) or not owner_path
            or not isinstance(path, str) or not path.startswith(owner_path)
            or not isinstance(interface, str) or not interface
            or not isinstance(interface_path, str) or not interface_path
            or not isinstance(capabilities, list) or not capabilities
            or set(capabilities) - set(concepts["capabilities"])
            or implementation.get("axon_revision") != revision
            or implementation.get("proof_state") != "behavior_attestation_incomplete"
            or not str(implementation.get("debt", "")).strip()
            or key in seen
        ):
            fail(f"invalid_provider_implementation:{language}:{identity}")
        seen.add(key)
        source = ROOT / path
        interface_source = ROOT / interface_path
        if check_paths and (
            not source.is_file()
            or hashlib.sha256(source.read_bytes()).hexdigest() != implementation.get("sha256")
            or not interface_source.is_file()
            or hashlib.sha256(interface_source.read_bytes()).hexdigest()
            != implementation.get("interface_sha256")
            or interface not in interface_source.read_text(encoding="utf-8")
        ):
            fail(f"provider_implementation_mismatch:{language}:{identity}")


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
            if case_ids and all((language, case_id) in bindings for case_id in case_ids):
                required_proofs[capability_id].add(language)
    for capability_id, languages in proofs.items():
        if capability_id not in capabilities or not isinstance(languages, dict):
            fail(f"unknown_provider_capability:{capability_id}")
        for language, proof in languages.items():
            if language not in PUBLIC_LANGUAGES or not isinstance(proof, dict):
                fail(f"invalid_provider_language:{capability_id}:{language}")
            if any(
                language in requirement["languages"]
                for requirement in capabilities[capability_id].get("unproven_requirements", [])
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
                or hashlib.sha256(implementation_path.read_bytes()).hexdigest() != digest
            ):
                fail(f"provider_implementation_mismatch:{capability_id}:{language}")

            interface = implementation.get("interface")
            revision = implementation.get("revision")
            if not isinstance(interface, str) or not interface:
                fail(f"provider_interface_required:{capability_id}:{language}")
            if revision != concepts.get("dependency_revisions", {}).get("easynet_axon"):
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
                fail(f"provider_implementation_not_registered:{capability_id}:{language}")
            if capability_id not in matching_implementations[0]["capability_ids"]:
                fail(f"provider_implementation_capability_mismatch:{capability_id}:{language}")
            expected_steps = {
                (case_id, action)
                for case_id in capabilities[capability_id]["case_ids"]
                if language in contracts[case_id]["languages"]
                for action in contracts[case_id]["actions"]
            }
            expected_lifecycle_actions = {
                action
                for case_id in capabilities[capability_id]["case_ids"]
                if language in contracts[case_id]["languages"]
                for action in contracts[case_id]["lifecycle_actions"]
            }
            if (
                proof.get("cutover_ready") is True
                and capabilities[capability_id]["profile"] == "runtime_core"
                and expected_lifecycle_actions != LIFECYCLE_ACTIONS
            ):
                missing = ",".join(sorted(LIFECYCLE_ACTIONS - expected_lifecycle_actions))
                fail(f"cutover_lifecycle_vectors_not_closed:{capability_id}:{language}:{missing}")
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
                if binding is None or binding.get("selector") != mapping.get("selector"):
                    fail(f"provider_step_selector_unbound:{capability_id}:{language}:{key}")
                selector = mapping.get("selector")
                if not isinstance(selector, str) or not selector:
                    fail(f"provider_step_selector_required:{capability_id}:{language}:{key}")
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
            fail(f"unclassified_package_roots:{language}:unknown={unknown}:stale={stale}")


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
        [sys.executable, str(ROOT / "sdk/conformance/public_api_inventory.py"), language],
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
    actual = {language: actual_structured_graph(language) for language in PUBLIC_LANGUAGES}
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
            missing = ",".join(sorted(set(shapes) - set(concepts["shape_sha256"][language])))
            stale = ",".join(sorted(set(concepts["shape_sha256"][language]) - set(shapes)))
            changed = ",".join(sorted(
                item for item in set(shapes) & set(concepts["shape_sha256"][language])
                if shapes[item] != concepts["shape_sha256"][language][item]
            ))
            fail(f"public_shape_mismatch:{language}:untracked={missing}:stale={stale}:changed={changed}")


def self_test(tmp: Path) -> None:
    concepts = validate_schema(load_json())

    def expect(mutated: dict[str, Any], marker: str, *, check_paths: bool = False) -> None:
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
    other = next(key for key in duplicate["capability_inventory"] if key != capability_id)
    duplicate["capability_inventory"][other]["go"][section].append(value)
    duplicate["capability_inventory"][other]["go"][section].sort()
    expect(duplicate, "multiply_owned_public_export")

    product_leak = copy.deepcopy(concepts)
    graph_section = "languages" if section == "symbols" else "members"
    leaked_value = "LeakedDaemonHandle" if section == "symbols" else "LeakedDaemonHandle.Start"
    product_leak[graph_section]["go"].remove(value)
    product_leak[graph_section]["go"].append(leaked_value)
    product_leak[graph_section]["go"].sort()
    product_leak["capability_inventory"][capability_id]["go"][section].remove(value)
    product_leak["capability_inventory"][capability_id]["go"][section].append(leaked_value)
    product_leak["capability_inventory"][capability_id]["go"][section].sort()
    product_leak["shape_sha256"]["go"][leaked_value] = product_leak["shape_sha256"]["go"].pop(value)
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
        "CABIDiscoveryTransport": "C ABI transport/provider",
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

    bad_lifecycle_actions = copy.deepcopy(concepts)
    bad_lifecycle_actions["lifecycle_actions"].remove("restart_recover")
    expect(bad_lifecycle_actions, "lifecycle_actions")

    bad_lifecycle_contract = copy.deepcopy(concepts)
    del bad_lifecycle_contract["lifecycle_transition_contract"]["cancel"]["cleanup_responsibility"]
    expect(bad_lifecycle_contract, "lifecycle_transition_contract")

    bad_lifecycle_source = copy.deepcopy(concepts)
    bad_lifecycle_source["lifecycle_transition_contract"]["dispatch"][
        "allowed_source_states"
    ] = ["invented"]
    expect(bad_lifecycle_source, "lifecycle_transition_contract")

    bad_case_action = tmp / "bad-lifecycle-case-action"
    bad_case_action.mkdir(parents=True, exist_ok=True)
    original_case = ROOT / "sdk/conformance/cases/environment-process-root.yaml"
    case_text = original_case.read_text(encoding="utf-8").replace(
        "  - start\n", "  - invented_lifecycle_action\n", 1
    )
    (bad_case_action / original_case.name).write_text(case_text, encoding="utf-8")
    try:
        case_contracts(bad_case_action)
    except ValueError as error:
        if "unknown_lifecycle_case_actions" not in str(error):
            raise
    else:
        fail("self_test_expected:unknown_lifecycle_case_actions")

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
    branded_canonical_root["canonical_packages"]["python"][0]["category"] = "public_facade"
    expect(branded_canonical_root, "product_branded_canonical_package")

    provider_capability_id = (
        "native_runtime" if "native_runtime" in concepts["capabilities"] else capability_id
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
                "revision": concepts["dependency_revisions"]["easynet_axon"],
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
                "revision": concepts["dependency_revisions"]["easynet_axon"],
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

    premature_cutover = copy.deepcopy(concepts)
    premature_cutover["provider_proofs"][provider_capability_id]["go"][
        "cutover_ready"
    ] = True
    expect(
        premature_cutover,
        f"cutover_lifecycle_vectors_not_closed:{provider_capability_id}:go",
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
                "revision": concepts["dependency_revisions"]["easynet_axon"],
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
    except (OSError, ValueError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f"sdk_concepts: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
