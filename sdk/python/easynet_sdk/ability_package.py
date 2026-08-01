"""Canonical runtime ability package manifest builder.

This module owns only the generic runtime deploy-bundle shape consumed by a
runtime ability package installer. Product repositories supply the function,
socket, schemas and namespace; the SDK guarantees the emitted manifest does not
accumulate product metadata or retired descriptor fields.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Mapping

from .errors import ErrorCode, RetryHint, SDKError

__all__ = [
    "HostStreamExec",
    "RuntimeAbilityPackageManifest",
]


@dataclass(frozen=True)
class HostStreamExec:
    """External host_stream executor binding for one runtime ability."""

    host_socket: str
    function: str

    def to_mapping(self) -> dict[str, object]:
        host_socket = _required_text(self.host_socket, "host_socket")
        function = _required_text(self.function, "function")
        return {
            "kind": "host_stream",
            "host_socket": host_socket,
            "function": function,
        }


@dataclass(frozen=True)
class RuntimeAbilityPackageManifest:
    """Product-neutral runtime ability package manifest.

    `name` is the verb-local ability name. `namespace` is the deploy-envelope
    namespace segment used by the runtime installer to derive the public ability
    key. The canonical manifest hash excludes that namespace on the daemon side;
    the SDK still carries it here because package authors need one object to
    write the complete deploy bundle.
    """

    name: str
    namespace: str
    description: str
    admission_action: str
    input_schema: Mapping[str, Any]
    exec: HostStreamExec
    output_schema: Mapping[str, Any] | None = None
    exposure: str | None = None
    descriptor_version: str | None = None
    timeout_seconds: int | None = None
    schema_version: str = "1"

    def to_mapping(self) -> dict[str, object]:
        manifest: dict[str, object] = {
            "schema_version": _required_text(self.schema_version, "schema_version"),
            "name": _required_text(self.name, "name"),
            "namespace": _required_text(self.namespace, "namespace"),
            "description": _required_text(self.description, "description"),
            "admission_action": _required_text(
                self.admission_action,
                "admission_action",
            ),
            "input_schema": _required_schema_object(
                self.input_schema,
                "input_schema",
            ),
            "exec": self.exec.to_mapping(),
        }
        if self.output_schema is not None:
            manifest["output_schema"] = _required_schema_object(
                self.output_schema,
                "output_schema",
            )
        if self.exposure is not None:
            exposure = _required_text(self.exposure, "exposure")
            if exposure not in {"task", "operator", "internal"}:
                raise _invalid_manifest(
                    "exposure must be one of task, operator, or internal"
                )
            manifest["exposure"] = exposure
        if self.descriptor_version is not None:
            manifest["descriptor_version"] = _required_text(
                self.descriptor_version,
                "descriptor_version",
            )
        if self.timeout_seconds is not None:
            if self.timeout_seconds <= 0:
                raise _invalid_manifest("timeout_seconds must be positive")
            manifest["timeout_seconds"] = self.timeout_seconds
        return manifest

    def to_json(self, *, indent: int | None = None) -> str:
        return json.dumps(
            self.to_mapping(),
            indent=indent,
            separators=None if indent is not None else (",", ":"),
            sort_keys=True,
        )


def _required_text(value: str, field: str) -> str:
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        raise _invalid_manifest(f"{field} must be a non-empty trimmed string")
    return value


def _required_schema_object(value: Mapping[str, Any], field: str) -> dict[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_manifest(f"{field} must be an object")
    copied = dict(value)
    if not copied:
        raise _invalid_manifest(f"{field} must be non-empty")
    return copied


def _invalid_manifest(message: str) -> SDKError:
    return SDKError(
        ErrorCode.INVALID_ARGUMENT,
        stage="ability_package",
        retry=RetryHint.NEVER,
        message=message,
    )
