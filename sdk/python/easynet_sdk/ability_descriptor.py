"""AbilityDescriptor projection through runtime catalog facts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Mapping, Protocol, cast

from .errors import ErrorCode, RetryHint, SDKError
from .runtime_ability import RuntimeAbilityClient, RuntimeCallContext

__all__ = [
    "AbilityDescriptorClient",
    "AbilityDescriptorGetRequest",
    "AbilityDescriptorHints",
    "AbilityDescriptorListRequest",
    "AbilityDescriptorPage",
    "AbilityDescriptorProjection",
    "AbilityDescriptorProvider",
    "AbilityDescriptorRef",
    "RuntimeAbilityDescriptorProvider",
    "parse_ability_descriptor_ref",
    "project_ability_descriptor",
]

_ABILITY_DESCRIPTOR_LIST_ABILITY = "meta.list_abilities"


@dataclass(frozen=True)
class AbilityDescriptorHints:
    """Advisory transport and behavior hints from the governed descriptor."""

    read_only: bool = False
    destructive: bool = False
    idempotent: bool = False
    streaming_only: bool = False
    bidi_only: bool = False


@dataclass(frozen=True)
class AbilityDescriptorProjection:
    """Generic SDK read model for one runtime AbilityDescriptor row."""

    ability_ura: str = ""
    descriptor_ref: str = ""
    name: str = ""
    owner_ura: str = ""
    version: str = ""
    schema_hash: str = ""
    descriptor_hash: str = ""
    call_mode: str = ""
    class_: str = ""
    receipt_semantics: Mapping[str, object] = field(default_factory=dict)
    visibility: str = ""
    source: str = ""
    description: str = ""
    hints: AbilityDescriptorHints = field(default_factory=AbilityDescriptorHints)
    schema_summary: Mapping[str, object] = field(default_factory=dict)
    input_schema: Mapping[str, object] = field(default_factory=dict)
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class AbilityDescriptorRef:
    """Descriptor identity split into ability URA and descriptor version."""

    raw: str
    ability_ura: str
    version: str


@dataclass(frozen=True)
class AbilityDescriptorListRequest:
    """Request runtime descriptor catalog rows with generic filters."""

    call: RuntimeCallContext
    scope: str = ""
    owner_ura: str = ""
    ability_ura: str = ""


@dataclass(frozen=True)
class AbilityDescriptorGetRequest:
    """Resolve one canonical AbilityDescriptor row from the runtime catalog."""

    call: RuntimeCallContext
    ability_ura: str
    descriptor_version: str = ""
    call_mode: str = ""
    scope: str = ""


@dataclass(frozen=True)
class AbilityDescriptorPage:
    descriptors: tuple[AbilityDescriptorProjection, ...]


class AbilityDescriptorProvider(Protocol):
    def list(self, request: AbilityDescriptorListRequest) -> AbilityDescriptorPage: ...

    def get(self, request: AbilityDescriptorGetRequest) -> AbilityDescriptorProjection: ...


class _AddressingProjector(Protocol):
    def project_descriptor_ref(self, value: str) -> Any: ...


class AbilityDescriptorClient:
    """Stable product-neutral descriptor facade."""

    def __init__(self, provider: AbilityDescriptorProvider) -> None:
        if provider is None:
            raise _invalid_descriptor("AbilityDescriptor provider is required")
        self._provider = provider

    def list(self, request: AbilityDescriptorListRequest) -> AbilityDescriptorPage:
        return self._provider.list(request)

    def get(self, request: AbilityDescriptorGetRequest) -> AbilityDescriptorProjection:
        return self._provider.get(request)


class RuntimeAbilityDescriptorProvider:
    """Provider-backed descriptor catalog over daemon `meta.list_abilities`."""

    def __init__(self, ability: RuntimeAbilityClient) -> None:
        if ability is None:
            raise _invalid_descriptor("runtime ability client is required")
        self._ability = ability

    def list(self, request: AbilityDescriptorListRequest) -> AbilityDescriptorPage:
        if not isinstance(request, AbilityDescriptorListRequest):
            raise _invalid_descriptor("AbilityDescriptorListRequest is required")
        args: dict[str, object] = {}
        if request.scope.strip():
            args["scope"] = request.scope.strip()
        if request.owner_ura.strip():
            args["agent_ura"] = request.owner_ura.strip()
        if request.ability_ura.strip():
            args["subject_ura"] = request.ability_ura.strip()
        output = self._ability.invoke(request.call, _ABILITY_DESCRIPTOR_LIST_ABILITY, args)
        raw_abilities = output.get("abilities")
        if not isinstance(raw_abilities, list):
            raise _invalid_descriptor(
                "meta.list_abilities output must include abilities array"
            )
        descriptors: list[AbilityDescriptorProjection] = []
        for index, raw in enumerate(raw_abilities):
            if not isinstance(raw, Mapping):
                raise _invalid_descriptor(
                    f"ability descriptor row {index} must be an object"
                )
            projection = project_ability_descriptor(raw)
            if not projection.ability_ura or not projection.owner_ura or not projection.name:
                raise _invalid_descriptor(
                    f"ability descriptor row {index} is missing identity fields"
                )
            descriptors.append(projection)
        return AbilityDescriptorPage(tuple(descriptors))

    def get(self, request: AbilityDescriptorGetRequest) -> AbilityDescriptorProjection:
        if not isinstance(request, AbilityDescriptorGetRequest):
            raise _invalid_descriptor("AbilityDescriptorGetRequest is required")
        ability_ura = _required_text(request.ability_ura, "ability_ura")
        page = self.list(
            AbilityDescriptorListRequest(
                call=request.call,
                scope=request.scope,
                ability_ura=ability_ura,
            )
        )
        version = request.descriptor_version.strip()
        call_mode = request.call_mode.strip()
        matches: list[AbilityDescriptorProjection] = []
        for descriptor in page.descriptors:
            if descriptor.ability_ura != ability_ura:
                raise _invalid_descriptor(
                    "runtime returned descriptor outside requested ability_ura"
                )
            if version and descriptor.version != version:
                continue
            if call_mode and descriptor.call_mode != call_mode:
                continue
            matches.append(descriptor)
        if not matches:
            raise _not_found(ability_ura)
        if len(matches) > 1:
            raise _invalid_descriptor(
                "ability descriptor selection is ambiguous; specify descriptor_version or call_mode"
            )
        return matches[0]


def parse_ability_descriptor_ref(
    raw: str,
    *,
    addressing: object | None = None,
    library_path: str | None = None,
    control_path: str = "",
) -> AbilityDescriptorRef:
    """Project an AbilityDescriptorRef through the Axon-delegated SDK facade.

    Grammar ownership stays in the Identity/Addressing profile. Callers may
    inject an AddressingClient-like object for tests or reuse; otherwise the
    default SDK environment opens the configured C ABI facade.
    """

    try:
        if addressing is not None:
            projection = cast(_AddressingProjector, addressing).project_descriptor_ref(raw)
        else:
            from .axon_addressing import project_descriptor_ref

            projection = project_descriptor_ref(raw)
    except SDKError:
        raise
    except Exception as exc:
        raise _invalid_descriptor_ref("descriptor_ref projection failed", exc) from exc
    descriptor_ref = getattr(projection, "descriptor_ref", "")
    ability_ura = getattr(projection, "ability_ura", "")
    version = getattr(projection, "descriptor_version", "")
    if not descriptor_ref or not ability_ura or not version:
        raise _invalid_descriptor_ref("descriptor_ref projection is incomplete")
    return AbilityDescriptorRef(
        raw=descriptor_ref,
        ability_ura=ability_ura,
        version=version,
    )


def project_ability_descriptor(raw: Mapping[str, object]) -> AbilityDescriptorProjection:
    """Project one daemon descriptor row without deriving governed facts."""

    values = _merge_descriptor(raw)
    name = _text(values.get("name"))
    if not name:
        name = _join_name(_text(values.get("namespace")), _text(values.get("local_name")))
    hints = _mapping(values.get("hints"))
    schema = _mapping(values.get("schema_summary"))
    return AbilityDescriptorProjection(
        ability_ura=_text(values.get("ability_ura")),
        descriptor_ref=_text(values.get("descriptor_ref")),
        name=name,
        owner_ura=_text(values.get("owner_ura")),
        version=_first_text(values.get("version"), values.get("descriptor_version")),
        schema_hash=_text(values.get("schema_hash")),
        descriptor_hash=_text(values.get("descriptor_hash")),
        call_mode=_text(values.get("call_mode")),
        class_=_text(values.get("class")),
        receipt_semantics=_mapping(values.get("receipt_semantics")),
        visibility=_text(values.get("visibility")),
        source=_text(values.get("source")),
        description=_text(values.get("description")),
        hints=AbilityDescriptorHints(
            read_only=_bool(hints.get("read_only")),
            destructive=_bool(hints.get("destructive")),
            idempotent=_bool(hints.get("idempotent")),
            streaming_only=_bool(hints.get("streaming_only")),
            bidi_only=_bool(hints.get("bidi_only")),
        ),
        schema_summary=schema,
        input_schema=_mapping(schema.get("input")),
        metadata=_mapping(values.get("metadata")),
    )


def _invalid_descriptor_ref(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="descriptor_ref",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _merge_descriptor(raw: Mapping[str, object]) -> Mapping[str, object]:
    nested = _mapping(raw.get("descriptor"))
    if not nested:
        return dict(raw)
    merged = dict(nested)
    merged.update({key: value for key, value in raw.items() if key != "descriptor"})
    return merged


def _join_name(namespace: str, local_name: str) -> str:
    if namespace and local_name:
        return f"{namespace}.{local_name}"
    return namespace or local_name


def _text(value: object) -> str:
    return value.strip() if isinstance(value, str) else ""


def _first_text(*values: object) -> str:
    for value in values:
        text = _text(value)
        if text:
            return text
    return ""


def _bool(value: object) -> bool:
    return isinstance(value, bool) and value


def _mapping(value: object) -> dict[str, object]:
    return dict(value) if isinstance(value, Mapping) else {}


def _required_text(value: object, field_name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid_descriptor(f"{field_name} is required")
    return value.strip()


def _invalid_descriptor(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="ability_descriptor",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _not_found(ability_ura: str) -> SDKError:
    return SDKError(
        code=ErrorCode.DESCRIPTOR_NOT_FOUND,
        stage="ability_descriptor",
        retry=RetryHint.NEVER,
        retryable=False,
        message="ability descriptor not found",
        details={"ability_ura": ability_ura},
    )
