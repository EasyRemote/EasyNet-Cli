#!/usr/bin/env python3
"""Validate every repository-owned system and builtin-plugin ability contract.

This gate intentionally reads the contract filesystem instead of pinning a
historical count. Adding or removing a descriptor therefore changes the audited
set automatically, while malformed, duplicate, stale, or contradictory rows
fail before Runtime compilation.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import tomllib
from typing import Any


SYSTEM_STATES = {"cutover_ready", "seam", "unsupported"}
PLUGIN_STATES = {"provider_backed"}
CALL_MODES = {"rpc", "stream", "bidi"}
BIDI_WIRE_KINDS = {"json_frames", "metadata_json_plus_binary"}
VISIBILITIES = {"PUBLIC", "SCOPED"}
EXPOSURES = {"internal", "operator", "task"}
SUBJECT_KINDS = {
    "authenticated-user",
    "route-target",
    "explicit-ura",
    "dedicated-surface",
}
RECEIPT_SEMANTICS = {"operational", "state_transition"}
REQUIRED_HINTS = {
    "read_only",
    "destructive",
    "idempotent",
    "streaming_only",
    "bidi_only",
}
DESCRIPTOR_VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


class AuditFailure(Exception):
    """One or more deterministic contract violations."""


def load_toml(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as handle:
            value = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise AuditFailure(f"{path}: cannot parse TOML: {error}") from error
    if not isinstance(value, dict):
        raise AuditFailure(f"{path}: TOML root must be a table")
    return value


def required_string(row: dict[str, Any], field: str, location: str, errors: list[str]) -> str:
    value = row.get(field)
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        errors.append(f"{location}: {field} must be a non-empty trimmed string")
        return ""
    return value


def json_object_field(
    row: dict[str, Any], field: str, location: str, errors: list[str], *, allow_null: bool = False
) -> dict[str, Any] | None:
    raw = row.get(field)
    if not isinstance(raw, str):
        errors.append(f"{location}: {field} must be encoded JSON text")
        return None
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        errors.append(f"{location}: {field} is invalid JSON: {error}")
        return None
    if value is None and allow_null:
        return None
    if not isinstance(value, dict):
        errors.append(f"{location}: {field} must decode to a JSON object")
        return None
    return value


def validate_descriptor(
    path: Path,
    row: dict[str, Any],
    source: str,
    plugin_id: str | None,
    names: dict[str, Path],
    errors: list[str],
) -> dict[str, Any]:
    location = str(path)
    name = required_string(row, "name", location, errors)
    if name:
        expected_file = f"{name}.ability.toml"
        if path.name != expected_file:
            errors.append(
                f"{location}: filename must be {expected_file!r} for ability {name!r}"
            )
        prior = names.get(name)
        if prior is not None:
            errors.append(f"{location}: duplicate ability {name!r}; first declared at {prior}")
        else:
            names[name] = path

    if str(row.get("schema_version", "")) != "3":
        errors.append(f"{location}: schema_version must be \"3\"")
    version = required_string(row, "descriptor_version", location, errors)
    if version and DESCRIPTOR_VERSION.fullmatch(version) is None:
        errors.append(f"{location}: descriptor_version must be numeric MAJOR.MINOR.PATCH")

    mode = required_string(row, "call_mode", location, errors)
    if mode not in CALL_MODES:
        errors.append(f"{location}: unsupported call_mode {mode!r}")
    state = required_string(row, "capability_state", location, errors)
    allowed_states = SYSTEM_STATES if source == "system" else PLUGIN_STATES
    if state not in allowed_states:
        errors.append(
            f"{location}: capability_state {state!r} is invalid for {source} contract"
        )

    visibility = required_string(row, "visibility", location, errors)
    if visibility not in VISIBILITIES:
        errors.append(f"{location}: unsupported visibility {visibility!r}")
    exposure = required_string(row, "exposure", location, errors)
    if exposure not in EXPOSURES:
        errors.append(f"{location}: unsupported exposure {exposure!r}")
    subject_kind = required_string(row, "subject_contract_kind", location, errors)
    if subject_kind not in SUBJECT_KINDS:
        errors.append(f"{location}: unsupported subject_contract_kind {subject_kind!r}")

    receipt_semantics = required_string(row, "receipt_semantics", location, errors)
    if receipt_semantics not in RECEIPT_SEMANTICS:
        errors.append(f"{location}: unsupported receipt_semantics {receipt_semantics!r}")
    transition_id = row.get("transition_id")
    transition_class = row.get("transition_class")
    if receipt_semantics == "state_transition":
        if not isinstance(transition_id, str) or not transition_id.strip():
            errors.append(f"{location}: state_transition receipt requires transition_id")
        if not isinstance(transition_class, str) or not transition_class.strip():
            errors.append(f"{location}: state_transition receipt requires transition_class")
    elif transition_id is not None or transition_class is not None:
        errors.append(f"{location}: operational receipt must not declare transition fields")

    input_schema = row.get("input_schema")
    if not isinstance(input_schema, dict) or input_schema.get("type") != "object":
        errors.append(f"{location}: input_schema must be a JSON-Schema object")
        input_schema = {}
    required = input_schema.get("required", [])
    properties = input_schema.get("properties", {})
    if not isinstance(required, list) or not all(isinstance(item, str) for item in required):
        errors.append(f"{location}: input_schema.required must be an array of strings")
        required = []
    if not isinstance(properties, dict):
        errors.append(f"{location}: input_schema.properties must be an object when present")
        properties = {}
    for field in required:
        if field not in properties:
            errors.append(
                f"{location}: required input field {field!r} has no properties declaration"
            )

    hints = json_object_field(row, "hints_json", location, errors)
    if hints is not None:
        missing_hints = sorted(REQUIRED_HINTS - hints.keys())
        if missing_hints:
            errors.append(f"{location}: hints_json missing {missing_hints}")
        for hint in REQUIRED_HINTS & hints.keys():
            if not isinstance(hints[hint], bool):
                errors.append(f"{location}: hints_json.{hint} must be boolean")
        if hints.get("streaming_only") is not (mode == "stream"):
            errors.append(f"{location}: streaming_only contradicts call_mode={mode!r}")
        if hints.get("bidi_only") is not (mode == "bidi"):
            errors.append(f"{location}: bidi_only contradicts call_mode={mode!r}")

    output_schema = json_object_field(
        row,
        "output_receipt_schema_json",
        location,
        errors,
        allow_null=state == "unsupported",
    )
    if state != "unsupported" and output_schema is None:
        errors.append(f"{location}: operational/provider contract needs a receipt schema")

    bidi_wire_kind = row.get("bidi_wire_kind")
    if bidi_wire_kind is not None and bidi_wire_kind not in BIDI_WIRE_KINDS:
        errors.append(f"{location}: unsupported bidi_wire_kind {bidi_wire_kind!r}")
    if mode != "bidi" and bidi_wire_kind is not None:
        errors.append(f"{location}: bidi_wire_kind requires call_mode=bidi")
    if source == "plugin" and mode == "bidi" and bidi_wire_kind is None:
        errors.append(f"{location}: plugin bidi contract must declare bidi_wire_kind")

    return {
        "name": name,
        "source": source,
        "plugin_id": plugin_id,
        "path": location,
        "descriptor_version": version,
        "call_mode": mode,
        "capability_state": state,
        "exposure": exposure,
        "dedicated_surface": row.get("dedicated_surface"),
        "subject_contract_kind": subject_kind,
        "receipt_semantics": receipt_semantics,
        "bidi_wire_kind": bidi_wire_kind,
    }


def validate_plugin_manifest(
    plugin_dir: Path,
    descriptor_rows: dict[str, dict[str, Any]],
    errors: list[str],
) -> None:
    manifest_path = plugin_dir / "plugin.toml"
    manifest = load_toml(manifest_path)
    plugin_id = required_string(manifest, "id", str(manifest_path), errors)
    metadata_rows = manifest.get("ability_metadata", [])
    if not isinstance(metadata_rows, list):
        errors.append(f"{manifest_path}: ability_metadata must be an array of tables")
        return
    metadata: dict[str, dict[str, Any]] = {}
    for index, row in enumerate(metadata_rows):
        location = f"{manifest_path}:ability_metadata[{index}]"
        if not isinstance(row, dict):
            errors.append(f"{location}: row must be a table")
            continue
        name = required_string(row, "name", location, errors)
        if name in metadata:
            errors.append(f"{location}: duplicate plugin ability metadata {name!r}")
        metadata[name] = row

    descriptor_names = set(descriptor_rows)
    metadata_names = set(metadata)
    if descriptor_names != metadata_names:
        errors.append(
            f"{manifest_path}: descriptor/metadata inventory differs; "
            f"metadata_only={sorted(metadata_names - descriptor_names)}, "
            f"descriptor_only={sorted(descriptor_names - metadata_names)}"
        )
    for name in sorted(descriptor_names & metadata_names):
        descriptor = descriptor_rows[name]
        meta = metadata[name]
        if descriptor.get("call_mode") != meta.get("call_mode"):
            errors.append(f"{manifest_path}: {name} call_mode differs from descriptor")
        if descriptor.get("bidi_wire_kind") != meta.get("bidi_wire_kind"):
            errors.append(f"{manifest_path}: {name} bidi_wire_kind differs from descriptor")
        if descriptor.get("plugin_id") != plugin_id:
            errors.append(f"{manifest_path}: {name} plugin identity projection drift")


def audit(root: Path) -> dict[str, Any]:
    system_root = root / "ability-descriptors" / "system"
    plugins_root = root / "plugins"
    if not system_root.is_dir():
        raise AuditFailure(f"missing system descriptor root: {system_root}")
    if not plugins_root.is_dir():
        raise AuditFailure(f"missing plugin root: {plugins_root}")

    errors: list[str] = []
    names: dict[str, Path] = {}
    rows: list[dict[str, Any]] = []
    system_files = sorted(system_root.rglob("*.ability.toml"))
    if not system_files:
        errors.append(f"{system_root}: inventory is empty")
    for path in system_files:
        try:
            raw = load_toml(path)
        except AuditFailure as error:
            errors.append(str(error))
            continue
        rows.append(validate_descriptor(path, raw, "system", None, names, errors))

    plugin_count = 0
    for plugin_dir in sorted(path for path in plugins_root.iterdir() if path.is_dir()):
        manifest_path = plugin_dir / "plugin.toml"
        descriptor_dir = plugin_dir / "abilities"
        if not manifest_path.is_file() or not descriptor_dir.is_dir():
            continue
        try:
            manifest = load_toml(manifest_path)
        except AuditFailure as error:
            errors.append(str(error))
            continue
        plugin_id = manifest.get("id") if isinstance(manifest.get("id"), str) else ""
        plugin_rows: dict[str, dict[str, Any]] = {}
        for path in sorted(descriptor_dir.glob("*.ability.toml")):
            try:
                raw = load_toml(path)
            except AuditFailure as error:
                errors.append(str(error))
                continue
            projected = validate_descriptor(
                path, raw, "plugin", plugin_id, names, errors
            )
            rows.append(projected)
            plugin_rows[projected["name"]] = projected
            plugin_count += 1
        validate_plugin_manifest(plugin_dir, plugin_rows, errors)

    if errors:
        raise AuditFailure("\n".join(errors))

    states: dict[str, int] = {}
    modes: dict[str, int] = {}
    for row in rows:
        states[row["capability_state"]] = states.get(row["capability_state"], 0) + 1
        modes[row["call_mode"]] = modes.get(row["call_mode"], 0) + 1
    return {
        "schema_version": 1,
        "system_contracts": len(system_files),
        "plugin_contracts": plugin_count,
        "total_contracts": len(rows),
        "capability_states": dict(sorted(states.items())),
        "call_modes": dict(sorted(modes.items())),
        "abilities": sorted(rows, key=lambda row: row["name"]),
    }


def fixture_descriptor(name: str, *, mode: str = "rpc", state: str = "cutover_ready") -> str:
    bidi = '\nbidi_wire_kind = "json_frames"' if mode == "bidi" else ""
    return f'''schema_version = "3"
name = "{name}"
descriptor_version = "1.0.0"
description = "fixture"
exposure = "task"
dedicated_surface = "none"
subject_contract_kind = "route-target"
call_mode = "{mode}"{bidi}
capability_state = "{state}"
admission_action = "invoke"
visibility = "SCOPED"
scope_subjects_kind = "any"
scope_subjects_uras = []
scope_agents_kind = "any"
scope_agents_uras = []
denied_agents = []
output_receipt_schema_json = "{{}}"
hints_json = "{{\\"read_only\\":false,\\"destructive\\":false,\\"idempotent\\":false,\\"streaming_only\\":{str(mode == 'stream').lower()},\\"bidi_only\\":{str(mode == 'bidi').lower()}}}"
receipt_semantics = "operational"

[input_schema]
type = "object"
additionalProperties = false
'''


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="easynet-ability-audit-") as temp:
        root = Path(temp)
        system = root / "ability-descriptors" / "system" / "device_control"
        plugin = root / "plugins" / "fixture"
        abilities = plugin / "abilities"
        system.mkdir(parents=True)
        abilities.mkdir(parents=True)
        (system / "fixture.rpc.ability.toml").write_text(
            fixture_descriptor("fixture.rpc"), encoding="utf-8"
        )
        (abilities / "fixture.bidi.ability.toml").write_text(
            fixture_descriptor("fixture.bidi", mode="bidi", state="provider_backed"),
            encoding="utf-8",
        )
        (plugin / "plugin.toml").write_text(
            '''schema_version = "1"
id = "fixture.plugin"
version = "1.0.0"
kind = "builtin"
entrypoint = "fixture"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = ["linux"]

[[ability_metadata]]
name = "fixture.bidi"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"
''',
            encoding="utf-8",
        )
        report = audit(root)
        if report["total_contracts"] != 2:
            raise AuditFailure("self-test valid fixture was not fully audited")
        duplicate = abilities / "fixture.rpc.ability.toml"
        duplicate.write_text(fixture_descriptor("fixture.rpc"), encoding="utf-8")
        try:
            audit(root)
        except AuditFailure as error:
            if "duplicate ability" not in str(error):
                raise AuditFailure("self-test duplicate failed for the wrong reason") from error
        else:
            raise AuditFailure("self-test failed to reject a duplicate ability")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit the full audit report")
    parser.add_argument("--self-test", action="store_true", help="run isolated gate tests")
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="repository root (defaults to this script's repository)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        self_test()
        print("check-system-ability-contract-inventory: self-test ok")
        return 0
    root = args.root
    if root is None:
        configured = os.environ.get("CHECK_SYSTEM_ABILITY_CONTRACT_INVENTORY_ROOT")
        root = Path(configured) if configured else Path(__file__).resolve().parents[2]
    try:
        report = audit(root.resolve())
    except AuditFailure as error:
        print(f"check-system-ability-contract-inventory: failed\n{error}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(
            "check-system-ability-contract-inventory: ok "
            f"system={report['system_contracts']} "
            f"plugins={report['plugin_contracts']} "
            f"total={report['total_contracts']} "
            f"modes={report['call_modes']} "
            f"states={report['capability_states']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
