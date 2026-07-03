"""Identity projection facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Mapping, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError


@dataclass(frozen=True)
class DescriptorRefRequest:
    """Request a daemon/Axon DescriptorRef projection."""

    descriptor_ref: str
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        if not self.descriptor_ref:
            raise _invalid_identity("descriptor_ref is required")
        value: dict[str, object] = {"descriptor_ref": self.descriptor_ref}
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class IdentityProjectionRequest:
    """Request a daemon-owned identity projection."""

    ura: str = ""
    kind: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        if not self.ura and not self.kind:
            raise _invalid_identity("ura or kind is required")
        value: dict[str, object] = {}
        if self.ura:
            value["ura"] = self.ura
        if self.kind:
            value["kind"] = self.kind
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class LocalResourceRefRequest:
    """Request a local ResourceRef projection."""

    path: str
    capability: str

    def to_json_bytes(self) -> bytes:
        if not self.path or not self.capability:
            raise _invalid_identity("path and capability are required")
        return _json_bytes({"path": self.path, "capability": self.capability})


@dataclass(frozen=True)
class IdentityProjection:
    """SDK identity.schema.json projection."""

    kind: str
    valid: bool
    profile: str
    components: Mapping[str, object]
    metadata: Mapping[str, object]
    ura: str = ""
    realm: str = ""
    display_id: str = ""
    descriptor_ref: str = ""
    ability_ura: str = ""
    descriptor_version: str = ""

    @classmethod
    def from_json(cls, raw: bytes | str) -> "IdentityProjection":
        decoded = _json_object(raw, "identity projection")
        projection = cls(
            kind=_required_string(decoded, "kind"),
            valid=_required_bool(decoded, "valid"),
            profile=_required_string(decoded, "profile"),
            components=_required_mapping(decoded, "components"),
            metadata=_required_mapping(decoded, "metadata"),
            ura=_optional_string(decoded.get("ura"), "ura") or "",
            realm=_optional_string(decoded.get("realm"), "realm") or "",
            display_id=_optional_string(decoded.get("display_id"), "display_id") or "",
            descriptor_ref=_optional_string(
                decoded.get("descriptor_ref"), "descriptor_ref"
            )
            or "",
            ability_ura=_optional_string(decoded.get("ability_ura"), "ability_ura")
            or "",
            descriptor_version=_optional_string(
                decoded.get("descriptor_version"), "descriptor_version"
            )
            or "",
        )
        if projection.kind == "descriptor_ref" and (
            not projection.descriptor_ref
            or not projection.ability_ura
            or not projection.descriptor_version
        ):
            raise _invalid_identity("invalid descriptor_ref projection")
        return projection


@dataclass(frozen=True)
class ResourceRef:
    """SDK resource-ref.schema.json projection."""

    resource_ura: str
    owner_ura: str
    namespace: str
    capability: str
    expires_unix_ms: int
    revision: str
    display_path: str = ""

    @classmethod
    def from_json(cls, raw: bytes | str) -> "ResourceRef":
        decoded = _json_object(raw, "resource-ref")
        return cls(
            resource_ura=_required_string(decoded, "resource_ura"),
            owner_ura=_required_string(decoded, "owner_ura"),
            namespace=_required_string(decoded, "namespace"),
            capability=_required_string(decoded, "capability"),
            expires_unix_ms=_required_int(decoded, "expires_unix_ms"),
            revision=_required_string(decoded, "revision"),
            display_path=_optional_string(decoded.get("display_path"), "display_path")
            or "",
        )


@runtime_checkable
class IdentityTransport(Protocol):
    """Concrete identity projections supplied by the integration layer."""

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        ...

    def project_identity(self, request_json: bytes) -> bytes:
        ...

    def build_resource_ref(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class IdentityClient:
    """Directory + Identity projection facade."""

    transport: IdentityTransport

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_identity("identity transport is required")

    def project_descriptor_ref(self, request: DescriptorRefRequest) -> IdentityProjection:
        try:
            raw = self.transport.project_descriptor_ref(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity descriptor projection failed", exc) from exc
        return IdentityProjection.from_json(raw)

    def project_identity(self, request: IdentityProjectionRequest) -> IdentityProjection:
        try:
            raw = self.transport.project_identity(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity projection failed", exc) from exc
        return IdentityProjection.from_json(raw)

    def build_resource_ref(self, request: LocalResourceRefRequest) -> ResourceRef:
        try:
            raw = self.transport.build_resource_ref(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity resource-ref build failed", exc) from exc
        return ResourceRef.from_json(raw)


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_identity(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_identity(f"{label} JSON must be an object")
    return decoded


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_identity(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_identity(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_identity(f"{field_name} must be a boolean")
    return value


def _required_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_identity(f"{field_name} must be an integer")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_identity(f"{field_name} must be an object")
    return dict(value)


def _invalid_identity(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="directory_identity",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
