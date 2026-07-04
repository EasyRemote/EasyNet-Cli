"""Identity projection facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Callable, Mapping, Optional, Protocol, TypeVar, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft
from ._lifecycle import ClientLifecycle

DEFAULT_SIGNING_KEY_PAGE_SIZE = 50
MAX_SIGNING_KEY_PAGE_SIZE = 500

_TAddressingResult = TypeVar("_TAddressingResult")


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
class IdentityCarrierBase:
    """Complete Invocation carrier fields for identity daemon abilities."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        _validate_identity_base(self)
        return {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
            "metadata": dict(self.metadata),
        }


@dataclass(frozen=True)
class SigningKeyRegistrationRequest:
    """Register daemon-owned public signing-key metadata."""

    owner_ura: str
    key_id: str
    algorithm: str
    public_key_base64: str
    usage: tuple[str, ...]
    role: str = "user"
    metadata: Mapping[str, object] = field(default_factory=dict)
    base: Optional[IdentityCarrierBase] = None

    def to_json_bytes(self) -> bytes:
        owner_ura = _required_clean_string(self.owner_ura, "owner_ura")
        key_id = _required_clean_string(self.key_id, "key_id")
        algorithm = _required_clean_string(self.algorithm, "algorithm")
        public_key_base64 = _required_clean_string(
            self.public_key_base64, "public_key_base64"
        )
        if not self.usage:
            raise _invalid_identity(
                "owner_ura, key_id, algorithm, public_key_base64, and usage are required"
            )
        usage = _clean_string_tuple(self.usage, "usage")
        if _contains_private_key_metadata(self.metadata):
            raise _invalid_identity(
                "private key material must not be supplied to identity facade"
            )
        role = _required_clean_string(self.role, "role")
        value: dict[str, object] = {
            "owner_ura": owner_ura,
            "key_id": key_id,
            "algorithm": algorithm,
            "public_key_base64": public_key_base64,
            "usage": list(usage),
            "role": role,
        }
        if self.base is not None:
            value.update(self.base.to_json_dict())
            if self.metadata:
                value["key_metadata"] = dict(self.metadata)
        elif self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(
            value
        )


@dataclass(frozen=True)
class SigningKeyListRequest:
    owner_ura: str = ""
    limit: int = 0
    cursor: str = ""
    base: Optional[IdentityCarrierBase] = None

    def to_json_bytes(self) -> bytes:
        if self.owner_ura.strip() != self.owner_ura or self.cursor.strip() != self.cursor:
            raise _invalid_identity(
                "owner_ura and cursor must not contain surrounding whitespace"
            )
        limit = self.limit or DEFAULT_SIGNING_KEY_PAGE_SIZE
        if limit < 1 or limit > MAX_SIGNING_KEY_PAGE_SIZE:
            raise _invalid_identity("signing-key page limit exceeds bounds")
        value: dict[str, object] = {"limit": limit}
        if self.owner_ura:
            value["owner_ura"] = self.owner_ura
        if self.cursor:
            value["cursor"] = self.cursor
        if self.base is not None:
            value.update(self.base.to_json_dict())
        return _json_bytes(value)


@dataclass(frozen=True)
class SigningKeyRevokeRequest:
    key_id: str
    reason: str
    owner_ura: str = ""
    public_key_base64: str = ""
    base: Optional[IdentityCarrierBase] = None

    def to_json_bytes(self) -> bytes:
        value: dict[str, object] = {
            "key_id": _required_clean_string(self.key_id, "key_id"),
            "reason": _required_clean_string(self.reason, "reason"),
        }
        if self.owner_ura:
            value["owner_ura"] = _required_clean_string(self.owner_ura, "owner_ura")
        if self.public_key_base64:
            value["public_key_base64"] = _required_clean_string(
                self.public_key_base64, "public_key_base64"
            )
        if self.base is not None:
            value.update(self.base.to_json_dict())
        return _json_bytes(value)


@dataclass(frozen=True)
class SignerRequest:
    owner_ura: str
    key_id: str
    usage: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        owner_ura = _required_clean_string(self.owner_ura, "owner_ura")
        key_id = _required_clean_string(self.key_id, "key_id")
        if _contains_private_key_metadata(self.metadata):
            raise _invalid_identity(
                "private key material must not be supplied to identity facade"
            )
        value: dict[str, object] = {
            "owner_ura": owner_ura,
            "key_id": key_id,
        }
        if self.usage:
            value["usage"] = _required_clean_string(self.usage, "usage")
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


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
class AbilityAddress:
    """Typed SDK projection of one canonical Ability URA."""

    ability_ura: str
    owner_ura: str
    owner_kind: str
    public_name: str
    subject_ura: str
    local_registry_ability: str = ""
    namespace: str = ""
    local_name: str = ""
    profile: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_projection(cls, projection: IdentityProjection) -> "AbilityAddress":
        if projection.kind != "ability" or not projection.ura:
            raise _invalid_identity("ability address requires an ability URA projection")
        components = projection.components
        return cls(
            ability_ura=projection.ura,
            owner_ura=_required_component_string(components, "owner_ura"),
            owner_kind=_required_component_string(components, "owner_kind"),
            public_name=_required_component_string(components, "public_name"),
            subject_ura=projection.ura,
            local_registry_ability=_optional_component_string(
                components, "local_registry_ability"
            )
            or "",
            namespace=_optional_component_string(components, "namespace") or "",
            local_name=_optional_component_string(components, "local_name") or "",
            profile=projection.profile,
            metadata=dict(projection.metadata),
        )


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


@dataclass(frozen=True)
class SigningKeyRecord:
    profile: str
    key_id: str
    owner_ura: str
    algorithm: str
    public_key_base64: str
    state: str
    usage: tuple[str, ...]
    metadata: Mapping[str, object]
    created_unix_ms: int = 0
    revoked_unix_ms: int = 0

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SigningKeyRecord":
        return cls.from_mapping(_json_object(raw, "signing-key record"))

    @classmethod
    def from_mapping(cls, decoded: Mapping[str, object]) -> "SigningKeyRecord":
        usage = _string_tuple(decoded.get("usage"), "usage")
        return cls(
            profile=_required_string(decoded, "profile"),
            key_id=_required_string(decoded, "key_id"),
            owner_ura=_required_string(decoded, "owner_ura"),
            algorithm=_required_string(decoded, "algorithm"),
            public_key_base64=_required_string(decoded, "public_key_base64"),
            state=_required_string(decoded, "state"),
            usage=usage,
            metadata=_required_mapping(decoded, "metadata"),
            created_unix_ms=_optional_int(decoded.get("created_unix_ms"), "created_unix_ms") or 0,
            revoked_unix_ms=_optional_int(decoded.get("revoked_unix_ms"), "revoked_unix_ms") or 0,
        )


@dataclass(frozen=True)
class SigningKeyPage:
    profile: str
    items: tuple[SigningKeyRecord, ...]
    next_cursor: Optional[str]
    limit: int
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SigningKeyPage":
        decoded = _json_object(raw, "signing-key page")
        items_raw = decoded.get("items")
        if not isinstance(items_raw, list):
            raise _invalid_identity("items must be a list")
        limit = _required_int(decoded, "limit")
        if limit < 1 or limit > MAX_SIGNING_KEY_PAGE_SIZE:
            raise _invalid_identity("signing-key page limit exceeds bounds")
        items: list[SigningKeyRecord] = []
        for item in items_raw:
            if not isinstance(item, dict):
                raise _invalid_identity("signing-key page item must be an object")
            items.append(SigningKeyRecord.from_mapping(item))
        return cls(
            profile=_required_string(decoded, "profile"),
            items=tuple(items),
            next_cursor=_optional_string(decoded.get("next_cursor"), "next_cursor"),
            limit=limit,
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class SigningKeyRevokeResult:
    profile: str
    key_id: str
    revoked: bool
    state: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SigningKeyRevokeResult":
        decoded = _json_object(raw, "signing-key revoke result")
        result = cls(
            profile=_required_string(decoded, "profile"),
            key_id=_required_string(decoded, "key_id"),
            revoked=_required_bool(decoded, "revoked"),
            state=_required_string(decoded, "state"),
            metadata=_required_mapping(decoded, "metadata"),
        )
        if not result.revoked:
            raise _invalid_identity("signing-key revoke result is not terminal")
        return result


@dataclass(frozen=True)
class SignerHandle:
    """Daemon-authorized signer reference, not local key material."""

    profile: str
    signer_id: str
    owner_ura: str
    key_id: str
    algorithm: str
    policy: Mapping[str, object]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SignerHandle":
        decoded = _json_object(raw, "signer handle")
        return cls(
            profile=_required_string(decoded, "profile"),
            signer_id=_required_string(decoded, "signer_id"),
            owner_ura=_required_string(decoded, "owner_ura"),
            key_id=_required_string(decoded, "key_id"),
            algorithm=_required_string(decoded, "algorithm"),
            policy=_required_mapping(decoded, "policy"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@runtime_checkable
class AddressingTransport(Protocol):
    """Axon-delegated URA and DescriptorRef projections."""

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        ...

    def build_descriptor_ref(self, request_json: bytes) -> bytes:
        ...

    def project_identity(self, request_json: bytes) -> bytes:
        ...

    def build_ura(self, request_json: bytes) -> bytes:
        ...


@runtime_checkable
class IdentityTransport(AddressingTransport, Protocol):
    """Concrete identity projections supplied by the integration layer."""

    def build_resource_ref(self, request_json: bytes) -> bytes:
        ...

    def build_register_signing_key_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_list_signing_keys_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_revoke_signing_key_invocation(self, request_json: bytes) -> bytes:
        ...

    def register_signing_key(self, request_json: bytes) -> bytes:
        ...

    def list_signing_keys(self, request_json: bytes) -> bytes:
        ...

    def revoke_signing_key(self, request_json: bytes) -> bytes:
        ...

    def signer(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class AddressingClient:
    """Axon-delegated URA and AbilityDescriptorRef helper facade."""

    transport: AddressingTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_identity("addressing transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("addressing"))

    def project_descriptor_ref(self, request: DescriptorRefRequest) -> IdentityProjection:
        self._require_open()
        try:
            raw = self.transport.project_descriptor_ref(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity descriptor projection failed", exc) from exc
        return IdentityProjection.from_json(raw)

    def parse_ura(self, ura: str) -> IdentityProjection:
        """Project a URA through the daemon/Axon identity boundary."""

        return self.project_identity(IdentityProjectionRequest(ura=ura))

    def owner_ability_ura(self, owner_ura: str, ability_name: str) -> str:
        """Build a canonical Ability URA through the identity transport."""

        self._require_open()
        request = _json_bytes(
            {
                "kind": "ability",
                "owner_ura": owner_ura,
                "ability_name": ability_name,
            }
        )
        try:
            raw = self.transport.build_ura(request)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity ability URA build failed", exc) from exc
        projection = IdentityProjection.from_json(raw)
        if projection.kind != "ability" or not projection.ura:
            raise _invalid_identity("invalid ability URA projection")
        return projection.ura

    def owner_ura_for_ability(self, ability_ura: str) -> str:
        """Return the Axon-projected owner URA for a canonical Ability URA."""

        projection = self.parse_ura(ability_ura)
        if projection.kind != "ability":
            raise _invalid_identity("ability_ura must project to an ability")
        owner_ura = projection.components.get("owner_ura")
        if not isinstance(owner_ura, str) or not owner_ura:
            raise _invalid_identity("ability projection missing owner_ura")
        return owner_ura

    def owner_ability_descriptor_ref(
        self,
        owner_ura: str,
        ability_name: str,
        descriptor_version: str = "1.0.0",
    ) -> str:
        """Build a canonical DescriptorRef for an owner ability."""

        ability_ura = self.owner_ability_ura(owner_ura, ability_name)
        return self.canonical_ability_descriptor_ref(
            ability_ura,
            descriptor_version,
        )

    def canonical_ability_descriptor_ref(
        self, value: str, descriptor_version: str = ""
    ) -> str:
        """Canonicalize or build an AbilityDescriptorRef via identity transport."""

        self._require_open()
        version = descriptor_version.strip()
        try:
            if version:
                raw = self.transport.build_descriptor_ref(
                    _json_bytes(
                        {
                            "ability_ura": value,
                            "descriptor_version": version,
                        }
                    )
                )
            else:
                raw = self.transport.project_descriptor_ref(
                    DescriptorRefRequest(value).to_json_bytes()
                )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(
                "identity descriptor-ref canonicalization failed", exc
            ) from exc
        projection = IdentityProjection.from_json(raw)
        if projection.kind != "descriptor_ref" or not projection.descriptor_ref:
            raise _invalid_identity("invalid descriptor_ref projection")
        return projection.descriptor_ref

    def ability_ura_from_descriptor_ref(self, descriptor_ref: str) -> str:
        """Project the Ability URA from an AbilityDescriptorRef via Axon helper."""

        projection = self.project_descriptor_ref(DescriptorRefRequest(descriptor_ref))
        if projection.kind != "descriptor_ref" or not projection.ability_ura:
            raise _invalid_identity("invalid descriptor_ref ability projection")
        return projection.ability_ura

    def ability_address(self, ability_ura: str) -> AbilityAddress:
        """Project an Ability URA into owner/subject facts via Axon helpers."""

        return AbilityAddress.from_projection(self.parse_ura(ability_ura))

    def project_identity(self, request: IdentityProjectionRequest) -> IdentityProjection:
        self._require_open()
        try:
            raw = self.transport.project_identity(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity projection failed", exc) from exc
        return IdentityProjection.from_json(raw)

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


@dataclass(frozen=True)
class IdentityClient:
    """Directory + Identity profile facade."""

    transport: IdentityTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)
    _addressing: AddressingClient = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_identity("identity transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("identity"))
        object.__setattr__(self, "_addressing", AddressingClient(self.transport))

    def project_descriptor_ref(self, request: DescriptorRefRequest) -> IdentityProjection:
        self._require_open()
        return self._addressing.project_descriptor_ref(request)

    def parse_ura(self, ura: str) -> IdentityProjection:
        """Project a URA through the daemon/Axon identity boundary."""

        self._require_open()
        return self._addressing.parse_ura(ura)

    def owner_ability_ura(self, owner_ura: str, ability_name: str) -> str:
        """Build a canonical Ability URA through the identity transport."""

        self._require_open()
        return self._addressing.owner_ability_ura(owner_ura, ability_name)

    def owner_ura_for_ability(self, ability_ura: str) -> str:
        """Return the Axon-projected owner URA for a canonical Ability URA."""

        self._require_open()
        return self._addressing.owner_ura_for_ability(ability_ura)

    def owner_ability_descriptor_ref(
        self,
        owner_ura: str,
        ability_name: str,
        descriptor_version: str = "1.0.0",
    ) -> str:
        """Build a canonical DescriptorRef for an owner ability."""

        self._require_open()
        return self._addressing.owner_ability_descriptor_ref(
            owner_ura,
            ability_name,
            descriptor_version,
        )

    def canonical_ability_descriptor_ref(
        self, value: str, descriptor_version: str = ""
    ) -> str:
        """Canonicalize or build an AbilityDescriptorRef via identity transport."""

        self._require_open()
        return self._addressing.canonical_ability_descriptor_ref(
            value, descriptor_version
        )

    def ability_ura_from_descriptor_ref(self, descriptor_ref: str) -> str:
        """Project the Ability URA from an AbilityDescriptorRef via identity transport."""

        self._require_open()
        return self._addressing.ability_ura_from_descriptor_ref(descriptor_ref)

    def ability_address(self, ability_ura: str) -> AbilityAddress:
        """Project an Ability URA into owner/subject facts via identity transport."""

        self._require_open()
        return self._addressing.ability_address(ability_ura)

    def project_identity(self, request: IdentityProjectionRequest) -> IdentityProjection:
        self._require_open()
        return self._addressing.project_identity(request)

    def build_resource_ref(self, request: LocalResourceRefRequest) -> ResourceRef:
        self._require_open()
        try:
            raw = self.transport.build_resource_ref(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity resource-ref build failed", exc) from exc
        return ResourceRef.from_json(raw)

    def build_register_signing_key_invocation(
        self, request: SigningKeyRegistrationRequest
    ) -> InvocationDraft:
        self._require_open()
        return self._build_invocation(
            request.to_json_bytes(),
            self.transport.build_register_signing_key_invocation,
            "identity register signing key invocation build failed",
        )

    def build_list_signing_keys_invocation(
        self, request: SigningKeyListRequest
    ) -> InvocationDraft:
        self._require_open()
        return self._build_invocation(
            request.to_json_bytes(),
            self.transport.build_list_signing_keys_invocation,
            "identity list signing keys invocation build failed",
        )

    def build_revoke_signing_key_invocation(
        self, request: SigningKeyRevokeRequest
    ) -> InvocationDraft:
        self._require_open()
        return self._build_invocation(
            request.to_json_bytes(),
            self.transport.build_revoke_signing_key_invocation,
            "identity revoke signing key invocation build failed",
        )

    def register_signing_key(
        self, request: SigningKeyRegistrationRequest
    ) -> SigningKeyRecord:
        self._require_open()
        try:
            raw = self.transport.register_signing_key(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity register signing key failed", exc) from exc
        return SigningKeyRecord.from_json(raw)

    def list_signing_keys(self, request: SigningKeyListRequest) -> SigningKeyPage:
        self._require_open()
        try:
            raw = self.transport.list_signing_keys(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity list signing keys failed", exc) from exc
        return SigningKeyPage.from_json(raw)

    def revoke_signing_key(
        self, request: SigningKeyRevokeRequest
    ) -> SigningKeyRevokeResult:
        self._require_open()
        try:
            raw = self.transport.revoke_signing_key(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity revoke signing key failed", exc) from exc
        return SigningKeyRevokeResult.from_json(raw)

    def signer(self, request: SignerRequest) -> SignerHandle:
        self._require_open()
        try:
            raw = self.transport.signer(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("identity signer failed", exc) from exc
        return SignerHandle.from_json(raw)

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _build_invocation(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> InvocationDraft:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return InvocationDraft.from_json(raw)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


def parse_ura(
    ura: str,
    *,
    library_path: str | None = None,
    control_path: str = "",
) -> IdentityProjection:
    """Project a URA through the default Axon-delegated SDK facade."""

    return _with_default_addressing(
        lambda addressing: addressing.parse_ura(ura),
        library_path=library_path,
        control_path=control_path,
    )


def owner_ability_ura(
    owner_ura: str,
    ability_name: str,
    *,
    library_path: str | None = None,
    control_path: str = "",
) -> str:
    """Build a canonical Ability URA through the default SDK facade."""

    return _with_default_addressing(
        lambda addressing: addressing.owner_ability_ura(owner_ura, ability_name),
        library_path=library_path,
        control_path=control_path,
    )


def owner_ura_for_ability(
    ability_ura: str,
    *,
    library_path: str | None = None,
    control_path: str = "",
) -> str:
    """Project the owner URA for a canonical Ability URA."""

    return _with_default_addressing(
        lambda addressing: addressing.owner_ura_for_ability(ability_ura),
        library_path=library_path,
        control_path=control_path,
    )


def owner_ability_descriptor_ref(
    owner_ura: str,
    ability_name: str,
    descriptor_version: str = "1.0.0",
    *,
    library_path: str | None = None,
    control_path: str = "",
) -> str:
    """Build a canonical AbilityDescriptorRef through the default SDK facade."""

    return _with_default_addressing(
        lambda addressing: addressing.owner_ability_descriptor_ref(
            owner_ura,
            ability_name,
            descriptor_version,
        ),
        library_path=library_path,
        control_path=control_path,
    )


def canonical_ability_descriptor_ref(
    value: str,
    descriptor_version: str = "",
    *,
    library_path: str | None = None,
    control_path: str = "",
) -> str:
    """Canonicalize or build an AbilityDescriptorRef through the SDK facade."""

    return _with_default_addressing(
        lambda addressing: addressing.canonical_ability_descriptor_ref(
            value,
            descriptor_version,
        ),
        library_path=library_path,
        control_path=control_path,
    )


def ability_ura_from_descriptor_ref(
    descriptor_ref: str,
    *,
    library_path: str | None = None,
    control_path: str = "",
) -> str:
    """Project the Ability URA from an AbilityDescriptorRef through the SDK facade."""

    return _with_default_addressing(
        lambda addressing: addressing.ability_ura_from_descriptor_ref(descriptor_ref),
        library_path=library_path,
        control_path=control_path,
    )


def ability_address(
    ability_ura: str,
    *,
    library_path: str | None = None,
    control_path: str = "",
) -> AbilityAddress:
    """Project an Ability URA into SDK owner/subject facts."""

    return _with_default_addressing(
        lambda addressing: addressing.ability_address(ability_ura),
        library_path=library_path,
        control_path=control_path,
    )


def _with_default_addressing(
    callback: Callable[[AddressingClient], _TAddressingResult],
    *,
    library_path: str | None,
    control_path: str,
) -> _TAddressingResult:
    from .environment import default_environment

    env = default_environment(library_path=library_path, control_path=control_path)
    try:
        result = callback(env.addressing_client())
    except BaseException:
        try:
            env.close()
        except Exception:
            pass
        raise
    else:
        env.close()
        return result


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


def _validate_identity_base(base: IdentityCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_identity("complete identity invocation carrier is required")


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


def _required_clean_string(value: object, field_name: str) -> str:
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_identity(f"{field_name} is required")
    if value.strip() != value:
        raise _invalid_identity(f"{field_name} must not contain surrounding whitespace")
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


def _optional_int(value: object, field_name: str) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_identity(f"{field_name} must be an integer or null")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_identity(f"{field_name} must be an object")
    return dict(value)


def _required_component_string(
    components: Mapping[str, object], field_name: str
) -> str:
    value = _optional_component_string(components, field_name)
    if not value:
        raise _invalid_identity(f"ability projection missing {field_name}")
    return value


def _optional_component_string(
    components: Mapping[str, object], field_name: str
) -> str | None:
    value = components.get(field_name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_identity(f"ability projection {field_name} must be a string")
    return value


def _string_tuple(value: object, field_name: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not value or any(not isinstance(item, str) or item.strip() == "" for item in value):
        raise _invalid_identity(f"{field_name} must be a non-empty array of strings")
    return tuple(value)


def _clean_string_tuple(value: object, field_name: str) -> tuple[str, ...]:
    if not isinstance(value, tuple) or not value:
        raise _invalid_identity(f"{field_name} must be a non-empty tuple of strings")
    items: list[str] = []
    for item in value:
        items.append(_required_clean_string(item, field_name))
    return tuple(items)


def _contains_private_key_metadata(metadata: Mapping[str, object]) -> bool:
    for key in metadata:
        normalized = key.replace("_", "").lower()
        if "privatekey" in normalized or "secret" in normalized or "seed" in normalized:
            return True
    return False


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
