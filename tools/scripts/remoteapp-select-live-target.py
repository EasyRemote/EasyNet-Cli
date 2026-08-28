#!/usr/bin/env python3
"""Select exactly one authoritative RemoteApp target from a live inventory.

Display names, application names, and window titles are diagnostic picker text.
They can be hidden or truncated by the host OS and therefore must never narrow an
otherwise authoritative Resource URA/PID selection.  Runtime session binding is
based on the selected resource's native locator, not on these labels.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import tempfile
from typing import Any


def fail(message: str) -> None:
    raise SystemExit(message)


def metadata(resource: dict[str, Any]) -> dict[str, Any]:
    value = resource.get("metadata")
    return value if isinstance(value, dict) else {}


def is_available(resource: dict[str, Any]) -> bool:
    meta = metadata(resource)
    return meta.get("availability", resource.get("availability", "available")) == "available"


def pid_matches(resource: dict[str, Any], expected_pid: int) -> bool:
    meta = metadata(resource)
    return any(
        isinstance(value, int) and not isinstance(value, bool) and value == expected_pid
        for value in (meta.get("pid"), meta.get("primary_pid"))
    )


def diagnostic_blob(resource: dict[str, Any]) -> str:
    meta = metadata(resource)
    values = (
        resource.get("resource_ura"),
        resource.get("display_name"),
        meta.get("title"),
        meta.get("app_name"),
        meta.get("bundle_id"),
        meta.get("app_identity"),
    )
    return "\n".join(str(value).lower() for value in values if value not in (None, ""))


def require_native_locator(resource: dict[str, Any], target_kind: str) -> None:
    meta = metadata(resource)
    if target_kind == "window":
        window_id = meta.get("window_id")
        if not isinstance(window_id, int) or isinstance(window_id, bool) or window_id <= 0:
            fail("selected live window is missing a positive native window_id")
    elif target_kind == "application":
        window_ids = meta.get("resolved_window_ids")
        if not isinstance(window_ids, list) or not window_ids or any(
            not isinstance(value, int) or isinstance(value, bool) or value <= 0
            for value in window_ids
        ):
            fail("selected live application is missing resolved_window_ids")


def select_target(
    inventory: dict[str, Any],
    target_kind: str,
    resource_ura: str | None,
    target_pid: int | None,
    hint: str | None,
) -> dict[str, Any]:
    resources = inventory.get("resources")
    if not isinstance(resources, list) or any(not isinstance(item, dict) for item in resources):
        fail("resource.refresh_remote_targets response missing resources array")

    candidates = [
        resource
        for resource in resources
        if resource.get("type") == target_kind and is_available(resource)
    ]
    if resource_ura:
        candidates = [
            resource for resource in candidates if resource.get("resource_ura") == resource_ura
        ]
    if target_pid is not None:
        candidates = [resource for resource in candidates if pid_matches(resource, target_pid)]

    # Text is a picker aid only.  Once the caller supplies an authoritative
    # Resource URA or a fixture PID, hidden/truncated host labels cannot veto it.
    if not resource_ura and target_pid is None and hint:
        needle = hint.lower()
        candidates = [resource for resource in candidates if needle in diagnostic_blob(resource)]

    if len(candidates) != 1:
        sample = [
            {
                "resource_ura": resource.get("resource_ura"),
                "display_name": resource.get("display_name"),
                "pid": metadata(resource).get("pid"),
                "primary_pid": metadata(resource).get("primary_pid"),
                "window_id": metadata(resource).get("window_id"),
            }
            for resource in candidates[:12]
        ]
        fail(
            f"live {target_kind} selection must resolve exactly once; got {len(candidates)} "
            f"sample={json.dumps(sample, sort_keys=True)}"
        )

    selected = candidates[0]
    selected_ura = selected.get("resource_ura")
    if not isinstance(selected_ura, str) or not selected_ura.startswith("easynet:///"):
        fail("selected live target is missing a canonical Resource URA")
    require_native_locator(selected, target_kind)
    return selected


def write_json_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--inventory", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--kind", required=True, choices=("window", "application"))
    parser.add_argument("--resource-ura")
    parser.add_argument("--pid", type=int)
    parser.add_argument("--hint")
    args = parser.parse_args()
    if args.pid is not None and args.pid <= 0:
        fail("--pid must be a positive integer")
    with args.inventory.open(encoding="utf-8") as handle:
        inventory = json.load(handle)
    if not isinstance(inventory, dict):
        fail("live inventory root must be an object")
    selected = select_target(inventory, args.kind, args.resource_ura, args.pid, args.hint)
    write_json_atomic(args.output, selected)


if __name__ == "__main__":
    main()
