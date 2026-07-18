"""Product-neutral Addressing transport backed by the Axon Python SDK."""

from __future__ import annotations

import json
from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Any, Callable, NoReturn, Protocol, TypeVar, cast

from axon_sdk.addressing import CanonicalAddressing
from axon_sdk.invocation.axiom import AbilityDescriptorRef
from axon_sdk.ura import ParsedURA, ParseError, display_id

from .errors import ErrorCode, RetryHint, SDKError

_PROFILE = "easynet-strict-v2"
_T = TypeVar("_T")


class AddressingTransport(Protocol):
    def project_descriptor_ref(self, request_json: bytes) -> bytes: ...
    def build_descriptor_ref(self, request_json: bytes) -> bytes: ...
    def project_identity(self, request_json: bytes) -> bytes: ...
    def build_ura(self, request_json: bytes) -> bytes: ...
    def close(self) -> None: ...


@dataclass(frozen=True)
class AddressingProjection:
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

    @property
    def owner_ura(self) -> str:
        return _component(self.components, "owner_ura")

    @property
    def public_name(self) -> str:
        return _component(self.components, "public_name")

    @classmethod
    def from_json(cls, raw: bytes | str) -> "AddressingProjection":
        try:
            value = json.loads(raw)
        except Exception as exc:
            _invalid_addressing(f"decode Addressing projection: {exc}", exc)
        if not isinstance(value, dict):
            _invalid_addressing("Addressing projection must be an object")
        components = value.get("components")
        metadata = value.get("metadata")
        if not isinstance(components, dict) or not isinstance(metadata, dict):
            _invalid_addressing("Addressing projection components and metadata must be objects")
        kind = value.get("kind")
        profile = value.get("profile")
        valid = value.get("valid")
        if not isinstance(kind, str) or not kind or not isinstance(profile, str) or not profile or not isinstance(valid, bool):
            _invalid_addressing("Addressing projection requires kind, profile, and valid")
        return cls(
            kind=kind,
            valid=valid,
            profile=profile,
            components=components,
            metadata=metadata,
            ura=_string(value.get("ura")),
            realm=_string(value.get("realm")),
            display_id=_string(value.get("display_id")),
            descriptor_ref=_string(value.get("descriptor_ref")),
            ability_ura=_string(value.get("ability_ura")),
            descriptor_version=_string(value.get("descriptor_version")),
        )


@dataclass(frozen=True)
class AbilityURA:
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


@dataclass
class AddressingClient:
    transport: AddressingTransport
    _closed: bool = field(default=False, init=False, repr=False)

    def _request(self, operation: str, value: Mapping[str, object]) -> AddressingProjection:
        if self._closed:
            _invalid_addressing("Addressing client is closed")
        method = getattr(self.transport, operation)
        try:
            return AddressingProjection.from_json(method(_json_bytes(value)))
        except SDKError:
            raise
        except Exception as exc:
            _invalid_addressing(f"Addressing {operation} failed: {exc}", exc)

    def parse_ura(self, ura: str) -> AddressingProjection:
        return self._request("project_identity", {"ura": _clean(ura, "ura")})

    def project_ability_ura(self, ability_ura: str) -> AddressingProjection:
        projection = self.parse_ura(ability_ura)
        if projection.kind != "ability" or not projection.ura:
            _invalid_addressing("ability_ura must project to an ability")
        _ = projection.owner_ura, projection.public_name
        return projection

    def project_descriptor_ref(self, descriptor_ref: str) -> AddressingProjection:
        return self._request("project_descriptor_ref", {"descriptor_ref": _clean(descriptor_ref, "descriptor_ref")})

    def user_ura(self, realm: str, user_id: str) -> str:
        return self._build("user", realm=_clean(realm, "realm"), user_id=_clean(user_id, "user_id"))

    def device_ura(self, realm: str, device_id: str) -> str:
        return self._build("device", realm=_clean(realm, "realm"), device_id=_clean(device_id, "device_id"))

    def agent_ura(self, realm: str, user_id: str, agent_id: str) -> str:
        return self._build("agent", owner_kind="user", realm=_clean(realm, "realm"), user_id=_clean(user_id, "user_id"), agent_id=_clean(agent_id, "agent_id"))

    def device_agent_ura(self, realm: str, device_id: str, agent_id: str) -> str:
        return self._build("agent", owner_kind="device", realm=_clean(realm, "realm"), device_id=_clean(device_id, "device_id"), agent_id=_clean(agent_id, "agent_id"))

    def hub_ura(self, realm: str) -> str:
        return self._build("hub", realm=_clean(realm, "realm"))

    def resource_ura(self, owner_ura: str, path: str) -> str:
        return self._build("resource", owner_ura=_clean(owner_ura, "owner_ura"), path=_clean(path, "path"))

    def descriptor_bound_resource_subject_ura(self, owner_ura: str, path: str) -> str:
        return self.resource_ura(owner_ura, path)

    def owner_ability_ura(self, owner_ura: str, ability_name: str) -> str:
        return self._build("ability", owner_ura=_clean(owner_ura, "owner_ura"), ability_name=_clean(ability_name, "ability_name"))

    def device_ability_ura(self, realm: str, device_id: str, namespace: str, local_name: str) -> str:
        name = ".".join(part for part in (_clean(namespace, "namespace"), _clean(local_name, "local_name")) if part)
        return self.owner_ability_ura(self.device_ura(realm, device_id), name)

    def owner_ura_for_ability(self, ability_ura: str) -> str:
        projection = self.parse_ura(ability_ura)
        owner = projection.components.get("owner_ura")
        if projection.kind != "ability" or not isinstance(owner, str) or not owner:
            _invalid_addressing("ability projection is missing owner_ura")
        return owner

    def canonical_ability_descriptor_ref(self, value: str, descriptor_version: str = "") -> str:
        if descriptor_version.strip():
            projection = self._request("build_descriptor_ref", {"ability_ura": _clean(value, "ability_ura"), "descriptor_version": _clean(descriptor_version, "descriptor_version")})
        else:
            projection = self.project_descriptor_ref(value)
        if projection.kind != "descriptor_ref" or not projection.descriptor_ref:
            _invalid_addressing("invalid descriptor_ref projection")
        return projection.descriptor_ref

    def owner_ability_descriptor_ref(self, owner_ura: str, ability_name: str, descriptor_version: str = "1.0.0") -> str:
        return self.canonical_ability_descriptor_ref(
            self.owner_ability_ura(owner_ura, ability_name), descriptor_version
        )

    def ability_ura_from_descriptor_ref(self, descriptor_ref: str) -> str:
        projection = self.project_descriptor_ref(descriptor_ref)
        if not projection.ability_ura:
            _invalid_addressing("descriptor_ref projection is missing ability_ura")
        return projection.ability_ura

    def ability_ura(self, ability_ura: str) -> AbilityURA:
        projection = self.project_ability_ura(ability_ura)
        values = projection.components
        return AbilityURA(
            ability_ura=projection.ura,
            owner_ura=_component(values, "owner_ura"),
            owner_kind=_component(values, "owner_kind"),
            public_name=_component(values, "public_name"),
            subject_ura=projection.ura,
            local_registry_ability=_string(values.get("local_registry_ability")),
            namespace=_string(values.get("namespace")),
            local_name=_string(values.get("local_name")),
            profile=projection.profile,
            metadata=projection.metadata,
        )

    def ability_address(self, ability_ura: str) -> AbilityURA:  # REQ-PROD-5 compatibility alias
        return self.ability_ura(ability_ura)

    def _build(self, kind: str, **fields: object) -> str:
        projection = self._request("build_ura", {"kind": kind, **fields})
        if projection.kind != kind or not projection.ura:
            _invalid_addressing(f"invalid {kind} URA projection")
        return projection.ura

    def close(self) -> None:
        if not self._closed:
            self.transport.close()
            self._closed = True


AbilityAddress = AbilityURA  # REQ-PROD-5 compatibility alias


def _with_client(operation: Callable[[AddressingClient], _T]) -> _T:
    client = AddressingClient(AxonAddressingTransport())
    try:
        return operation(client)
    finally:
        client.close()


def parse_ura(value: str) -> AddressingProjection:
    return _with_client(lambda client: client.parse_ura(value))


def project_descriptor_ref(value: str) -> AddressingProjection:
    return _with_client(lambda client: client.project_descriptor_ref(value))


def user_ura(realm: str, user_id: str) -> str:
    return _with_client(lambda client: client.user_ura(realm, user_id))


def device_ura(realm: str, device_id: str) -> str:
    return _with_client(lambda client: client.device_ura(realm, device_id))


def agent_ura(realm: str, user_id: str, agent_id: str) -> str:
    return _with_client(lambda client: client.agent_ura(realm, user_id, agent_id))


def device_agent_ura(realm: str, device_id: str, agent_id: str) -> str:
    return _with_client(lambda client: client.device_agent_ura(realm, device_id, agent_id))


def hub_ura(realm: str) -> str:
    return _with_client(lambda client: client.hub_ura(realm))


def resource_ura(owner_ura: str, path: str) -> str:
    return _with_client(lambda client: client.resource_ura(owner_ura, path))


def owner_ability_ura(owner_ura: str, ability_name: str) -> str:
    return _with_client(lambda client: client.owner_ability_ura(owner_ura, ability_name))


def device_ability_ura(realm: str, device_id: str, namespace: str, local_name: str) -> str:
    return _with_client(lambda client: client.device_ability_ura(realm, device_id, namespace, local_name))


def owner_ura_for_ability(ability_ura: str) -> str:
    return _with_client(lambda client: client.owner_ura_for_ability(ability_ura))


def canonical_ability_descriptor_ref(value: str, descriptor_version: str = "") -> str:
    return _with_client(lambda client: client.canonical_ability_descriptor_ref(value, descriptor_version))


class AxonAddressingTransport:
    """JSON adapter over Axon's typed canonical Addressing provider.

    This class owns only language-SDK DTO projection. URA and descriptor
    parsing/building stay entirely in ``axon_sdk``; no product profile,
    service locator, C ABI, or signing material is involved.
    """

    def __init__(self, addressing: CanonicalAddressing | None = None) -> None:
        self._addressing = addressing or CanonicalAddressing()

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        request = _request_object(request_json, "descriptor-ref projection")
        raw = _required_string(request, "descriptor_ref")
        try:
            ref = self._addressing.parse_descriptor_ref(raw)
            return _descriptor_projection(self._addressing, ref)
        except Exception as exc:
            _invalid_addressing(f"project descriptor_ref: {exc}", exc)

    def build_descriptor_ref(self, request_json: bytes) -> bytes:
        request = _request_object(request_json, "descriptor-ref build")
        ability_ura = _required_string(request, "ability_ura")
        descriptor_version = _required_string(request, "descriptor_version")
        try:
            ref = self._addressing.build_descriptor_ref(
                ability_ura, descriptor_version
            )
            return _descriptor_projection(self._addressing, ref)
        except Exception as exc:
            _invalid_addressing(f"build descriptor_ref: {exc}", exc)

    def project_identity(self, request_json: bytes) -> bytes:
        request = _request_object(request_json, "URA projection")
        raw = _required_string(request, "ura")
        if request.get("kind") not in (None, ""):
            _invalid_addressing(
                "kind is not an addressing projection selector; use build_ura"
            )
        try:
            parsed = self._addressing.parse_ura(raw)
            return _ura_projection(self._addressing, parsed)
        except ParseError as exc:
            _invalid_addressing(f"project URA: {exc}", exc)

    def build_ura(self, request_json: bytes) -> bytes:
        request = _request_object(request_json, "URA build")
        kind = _required_string(request, "kind")
        try:
            raw = self._build_typed_ura(kind, request)
            parsed = self._addressing.parse_ura(raw)
        except ParseError as exc:
            _invalid_addressing(f"build {kind} URA: {exc}", exc)
        if parsed.kind != kind:
            _invalid_addressing(
                f"built URA kind {parsed.kind!r} does not match {kind!r}"
            )
        return _ura_projection(self._addressing, parsed)

    def _build_typed_ura(
        self, kind: str, request: Mapping[str, object]
    ) -> str:
        if kind == "user":
            return cast(str, self._addressing.user_ura(
                _required_string(request, "realm"),
                _required_string(request, "user_id"),
            ))
        if kind == "device":
            return cast(str, self._addressing.device_ura(
                _required_string(request, "realm"),
                _required_string(request, "device_id"),
            ))
        if kind == "agent":
            owner_kind = _required_string(request, "owner_kind")
            realm = _required_string(request, "realm")
            agent_id = _required_string(request, "agent_id")
            if owner_kind == "user":
                return cast(str, self._addressing.agent_ura(
                    realm,
                    _required_string(request, "user_id"),
                    agent_id,
                ))
            if owner_kind == "device":
                return cast(str, self._addressing.device_agent_ura(
                    realm,
                    _required_string(request, "device_id"),
                    agent_id,
                ))
            raise ParseError(f"unsupported agent owner_kind {owner_kind!r}")
        if kind == "hub":
            return cast(str, self._addressing.hub_ura(
                _required_string(request, "realm")
            ))
        if kind == "ability":
            return cast(str, self._addressing.owner_ability_ura(
                _required_string(request, "owner_ura"),
                _required_string(request, "ability_name"),
            ))
        if kind == "resource":
            return cast(str, self._addressing.resource_ura(
                _required_string(request, "owner_ura"),
                _required_string(request, "path"),
            ))
        raise ParseError(f"unsupported URA build kind {kind!r}")

    def close(self) -> None:
        """Release no resources; present for deterministic client lifecycle."""


def _descriptor_projection(
    addressing: CanonicalAddressing, ref: AbilityDescriptorRef
) -> bytes:
    parsed = addressing.parse_ura(ref.ability_ura)
    ability = _required_ability(parsed)
    public_name = _public_ability_name(parsed)
    owner_ura = addressing.owner_ura_for_ability(ref.ability_ura)
    return _json_bytes(
        {
            "kind": "descriptor_ref",
            "valid": True,
            "ura": ref.ability_ura,
            "descriptor_ref": ref.raw,
            "ability_ura": ref.ability_ura,
            "descriptor_version": ref.version,
            "profile": _PROFILE,
            "components": {
                "ability_ura": ref.ability_ura,
                "descriptor_version": ref.version,
                "owner_ura": owner_ura,
                "owner_kind": ability.owner.kind,
                "public_name": public_name,
                "local_registry_ability": public_name,
            },
            "metadata": _metadata(),
        }
    )


def _ura_projection(
    addressing: CanonicalAddressing, parsed: ParsedURA
) -> bytes:
    components: dict[str, object] = {"realm": parsed.realm}
    projection: dict[str, object] = {
        "kind": parsed.kind,
        "valid": True,
        "ura": parsed.raw,
        "realm": parsed.realm,
        "display_id": display_id(parsed.raw),
        "profile": _PROFILE,
        "components": components,
        "metadata": _metadata(),
    }
    if parsed.kind == "ability":
        ability = _required_ability(parsed)
        public_name = _public_ability_name(parsed)
        components.update(
            {
                "owner_ura": addressing.owner_ura_for_ability(parsed.raw),
                "owner_kind": ability.owner.kind,
                "ability_name": public_name,
                "public_name": public_name,
                "local_registry_ability": public_name,
                "namespace": ability.namespace,
                "local_name": ability.local_name,
            }
        )
        projection["ability_ura"] = parsed.raw
    elif parsed.kind == "user":
        components["user_id"] = parsed.user_id or ""
    elif parsed.kind == "device":
        components["device_id"] = parsed.device_id or ""
    elif parsed.kind == "agent":
        components.update(
            {
                "owner_kind": parsed.agent_owner_kind or "",
                "agent_id": parsed.agent_id or "",
            }
        )
        if parsed.agent_owner_kind == "device":
            components["device_id"] = parsed.device_id or ""
        else:
            components["user_id"] = parsed.user_id or ""
    elif parsed.kind == "resource":
        components.update(
            {
                "owner_id": parsed.resource_owner_id or "",
                "path": parsed.resource_path or "",
            }
        )
    return _json_bytes(projection)


def _required_ability(parsed: ParsedURA) -> Any:
    if parsed.kind != "ability" or parsed.ability is None:
        raise ParseError("value is not a typed ability URA")
    return parsed.ability


def _public_ability_name(parsed: ParsedURA) -> str:
    ability = _required_ability(parsed)
    return cast(str, (
        f"{ability.namespace}.{ability.local_name}"
        if ability.namespace
        else ability.local_name
    ))


def _request_object(raw: bytes, label: str) -> dict[str, object]:
    try:
        decoded = json.loads(raw.decode("utf-8"))
    except Exception as exc:
        _invalid_addressing(f"decode {label} JSON: {exc}", exc)
    if not isinstance(decoded, dict):
        _invalid_addressing(f"{label} JSON must be an object")
    return decoded


def _required_string(value: Mapping[str, object], field: str) -> str:
    raw = value.get(field)
    if not isinstance(raw, str) or not raw.strip() or raw != raw.strip():
        _invalid_addressing(f"{field} is required and must already be trimmed")
    return raw


def _clean(value: str, field: str) -> str:
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        _invalid_addressing(f"{field} is required and must already be trimmed")
    return value


def _string(value: object) -> str:
    if value is None:
        return ""
    if not isinstance(value, str):
        _invalid_addressing("optional Addressing projection values must be strings")
    return value


def _component(value: Mapping[str, object], key: str) -> str:
    item = _string(value.get(key))
    if not item:
        _invalid_addressing(f"Addressing projection is missing {key}")
    return item


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _metadata() -> dict[str, object]:
    return {"grammar_owner": "axon", "source": "axon_sdk"}


def _invalid_addressing(
    message: str, cause: BaseException | None = None
) -> NoReturn:
    raise SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="addressing",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
