"""Private EasyNet directory wire projections.

These product-owned JSON shapes are intentionally downstream of the canonical
Axon runtime SDK. They model EasyNet Hub directory responses, not generic
runtime protocol state.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Mapping

__all__ = [
    "DirectoryAgentSummary",
    "DirectoryEntry",
    "DirectoryEvent",
    "DirectorySigningAuthority",
    "parse_directory_entry",
    "parse_directory_event",
]


@dataclass(frozen=True)
class DirectoryEntry:
    agent_ura: str
    node_id: str
    status: str
    display_name: str | None = None
    origin_realm: str | None = None
    hub_endpoint: str | None = None
    last_seen_unix_ms: int | None = None

    def validate(self) -> None:
        _required_text(self.agent_ura, "directory entry agent_ura")
        _required_text(self.node_id, "directory entry node_id")
        _required_text(self.status, "directory entry status")

    def to_dict(self) -> dict[str, object]:
        self.validate()
        return {
            "agent_ura": self.agent_ura,
            "node_id": self.node_id,
            "status": self.status,
            "display_name": self.display_name,
            "origin_realm": self.origin_realm,
            "hub_endpoint": self.hub_endpoint,
            "last_seen_unix_ms": self.last_seen_unix_ms,
        }

    def canonical_json(self) -> bytes:
        return _canonical_json(self.to_dict())


@dataclass(frozen=True)
class DirectorySigningAuthority:
    kind: str
    host_ura: str = ""

    def validate(self) -> None:
        if self.kind == "self_signed":
            if self.host_ura:
                raise ValueError("self_signed authority cannot contain host_ura")
            return
        if self.kind == "hosted_by":
            _required_text(self.host_ura, "hosted_by authority host_ura")
            return
        raise ValueError(f"unsupported signing authority kind {self.kind!r}")

    def to_dict(self) -> dict[str, object]:
        self.validate()
        value: dict[str, object] = {"kind": self.kind}
        if self.host_ura:
            value["host_ura"] = self.host_ura
        return value


@dataclass(frozen=True)
class DirectoryAgentSummary:
    agent_ura: str
    signing_authority: DirectorySigningAuthority
    status: str
    ability_count: int

    def validate(self) -> None:
        _required_text(self.agent_ura, "directory agent agent_ura")
        _required_text(self.status, "directory agent status")
        _required_int(self.ability_count, "directory agent ability_count")
        self.signing_authority.validate()

    def to_dict(self) -> dict[str, object]:
        self.validate()
        return {
            "agent_ura": self.agent_ura,
            "signing_authority": self.signing_authority.to_dict(),
            "status": self.status,
            "ability_count": self.ability_count,
        }


@dataclass(frozen=True)
class DirectoryEvent:
    type: str
    agents: tuple[DirectoryAgentSummary, ...] | None = None
    snapshot_unix_ms: int | None = None
    agent_ura: str = ""
    signing_authority: DirectorySigningAuthority | None = None
    replaced_prior: bool | None = None
    was_active: bool | None = None
    reason: str = ""
    owner_ura: str = ""
    host_device_ura: str = ""
    projection_revision: int | None = None
    projection_digest: str = ""
    ability_count: int | None = None
    stale_count: int | None = None
    removed_count: int | None = None
    lease_expires_unix_ms: int | None = None
    unix_ms: int | None = None

    def validate(self) -> None:
        if self.type == "snapshot":
            if self.agents is None:
                raise ValueError("directory event snapshot: agents is required")
            _required_int(
                self.snapshot_unix_ms,
                "directory event snapshot_unix_ms",
            )
            for agent in self.agents:
                agent.validate()
            return
        if self.type == "agent_advertised":
            _required_text(
                self.agent_ura,
                "directory event agent_advertised agent_ura",
            )
            if self.signing_authority is None:
                raise ValueError(
                    "directory event agent_advertised: signing_authority is required"
                )
            self.signing_authority.validate()
            _required_bool(
                self.replaced_prior,
                "directory event agent_advertised replaced_prior",
            )
            _required_int(
                self.unix_ms,
                "directory event agent_advertised unix_ms",
            )
            return
        if self.type == "agent_revoked":
            _required_text(
                self.agent_ura,
                "directory event agent_revoked agent_ura",
            )
            _required_text(
                self.reason,
                "directory event agent_revoked reason",
            )
            _required_bool(
                self.was_active,
                "directory event agent_revoked was_active",
            )
            _required_int(
                self.unix_ms,
                "directory event agent_revoked unix_ms",
            )
            return
        if self.type == "heartbeat":
            _required_int(self.unix_ms, "directory event heartbeat unix_ms")
            return
        if self.type == "owner_projection_changed":
            _required_text(self.owner_ura, "directory event owner_ura")
            _required_text(
                self.host_device_ura,
                "directory event host_device_ura",
            )
            _required_text(
                self.projection_digest,
                "directory event projection_digest",
            )
            for name in (
                "projection_revision",
                "ability_count",
                "stale_count",
                "removed_count",
                "lease_expires_unix_ms",
                "unix_ms",
            ):
                _required_int(getattr(self, name), f"directory event {name}")
            return
        raise ValueError(f"directory event: unsupported type {self.type!r}")

    def to_dict(self) -> dict[str, object]:
        self.validate()
        if self.type == "snapshot":
            return {
                "type": self.type,
                "agents": [agent.to_dict() for agent in self.agents or ()],
                "snapshot_unix_ms": self.snapshot_unix_ms,
            }
        if self.type == "agent_advertised":
            authority = self.signing_authority
            assert authority is not None
            return {
                "type": self.type,
                "agent_ura": self.agent_ura,
                "signing_authority": authority.to_dict(),
                "replaced_prior": self.replaced_prior,
                "unix_ms": self.unix_ms,
            }
        if self.type == "agent_revoked":
            return {
                "type": self.type,
                "agent_ura": self.agent_ura,
                "was_active": self.was_active,
                "reason": self.reason,
                "unix_ms": self.unix_ms,
            }
        if self.type == "heartbeat":
            return {"type": self.type, "unix_ms": self.unix_ms}
        return {
            "type": self.type,
            "owner_ura": self.owner_ura,
            "host_device_ura": self.host_device_ura,
            "projection_revision": self.projection_revision,
            "projection_digest": self.projection_digest,
            "ability_count": self.ability_count,
            "stale_count": self.stale_count,
            "removed_count": self.removed_count,
            "lease_expires_unix_ms": self.lease_expires_unix_ms,
            "unix_ms": self.unix_ms,
        }

    def canonical_json(self) -> bytes:
        return _canonical_json(self.to_dict())


def parse_directory_entry(
    raw: bytes | str | Mapping[str, object],
) -> DirectoryEntry:
    value = _object(raw, "directory entry")
    entry = DirectoryEntry(
        agent_ura=_string(value, "agent_ura"),
        node_id=_string(value, "node_id"),
        status=_string(value, "status"),
        display_name=_optional_string(value.get("display_name")),
        origin_realm=_optional_string(value.get("origin_realm")),
        hub_endpoint=_optional_string(value.get("hub_endpoint")),
        last_seen_unix_ms=_optional_int(value.get("last_seen_unix_ms")),
    )
    entry.validate()
    return entry


def parse_directory_event(
    raw: bytes | str | Mapping[str, object],
) -> DirectoryEvent:
    value = _object(raw, "directory event")
    agents_value = value.get("agents")
    agents: tuple[DirectoryAgentSummary, ...] | None = None
    if agents_value is not None:
        if not isinstance(agents_value, list):
            raise ValueError("directory event agents must be a list")
        agents = tuple(
            _agent(_object(item, "directory agent")) for item in agents_value
        )
    event = DirectoryEvent(
        type=_string(value, "type"),
        agents=agents,
        snapshot_unix_ms=_optional_int(value.get("snapshot_unix_ms")),
        agent_ura=_optional_string(value.get("agent_ura")) or "",
        signing_authority=_authority(value.get("signing_authority")),
        replaced_prior=_optional_bool(value.get("replaced_prior")),
        was_active=_optional_bool(value.get("was_active")),
        reason=_optional_string(value.get("reason")) or "",
        owner_ura=_optional_string(value.get("owner_ura")) or "",
        host_device_ura=_optional_string(value.get("host_device_ura")) or "",
        projection_revision=_optional_int(value.get("projection_revision")),
        projection_digest=_optional_string(value.get("projection_digest"))
        or "",
        ability_count=_optional_int(value.get("ability_count")),
        stale_count=_optional_int(value.get("stale_count")),
        removed_count=_optional_int(value.get("removed_count")),
        lease_expires_unix_ms=_optional_int(
            value.get("lease_expires_unix_ms")
        ),
        unix_ms=_optional_int(value.get("unix_ms")),
    )
    event.validate()
    return event


def _agent(value: Mapping[str, object]) -> DirectoryAgentSummary:
    authority = _authority(value.get("signing_authority"))
    if authority is None:
        raise ValueError("directory agent signing_authority is required")
    return DirectoryAgentSummary(
        agent_ura=_string(value, "agent_ura"),
        signing_authority=authority,
        status=_string(value, "status"),
        ability_count=_required_int(
            value.get("ability_count"),
            "directory agent ability_count",
        ),
    )


def _authority(value: object) -> DirectorySigningAuthority | None:
    if value is None:
        return None
    authority = _object(value, "directory signing authority")
    return DirectorySigningAuthority(
        kind=_string(authority, "kind"),
        host_ura=_optional_string(authority.get("host_ura")) or "",
    )


def _object(raw: object, name: str) -> Mapping[str, object]:
    if isinstance(raw, (bytes, str)):
        try:
            raw = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError(f"{name}: decode JSON: {error}") from error
    if not isinstance(raw, Mapping):
        raise ValueError(f"{name} must be an object")
    return raw


def _string(value: Mapping[str, object], name: str) -> str:
    return _required_text(value.get(name), name)


def _required_text(value: object, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} is required")
    return value.strip()


def _optional_string(value: object) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError("optional directory string must be a string")
    return value


def _required_int(value: object, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError(f"{name} must be non-negative")
    return value


def _optional_int(value: object) -> int | None:
    if value is None:
        return None
    return _required_int(value, "optional directory integer")


def _required_bool(value: object, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _optional_bool(value: object) -> bool | None:
    if value is None:
        return None
    return _required_bool(value, "optional directory boolean")


def _canonical_json(value: Mapping[str, object]) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
