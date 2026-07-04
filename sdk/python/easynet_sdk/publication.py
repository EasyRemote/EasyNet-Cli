"""Publication profile facade."""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field, replace
from typing import Any, Callable, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
from .errors import ErrorCode, RetryHint, SDKError
from .identity import LocalResourceRefRequest, ResourceRef
from .invocation import InvocationDraft
from .runtime import RuntimeClient


DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE = 50
MAX_PUBLISHED_ABILITY_PAGE_SIZE = 500
_PROFILE = "publication"


@dataclass(frozen=True)
class AbilityPackageManifest:
    """Input manifest projected by the Publication profile."""

    name: str
    namespace: str
    description: str
    input_schema: Mapping[str, object]
    descriptor_version: str = ""
    output_schema: Any = None
    exec: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        if not self.name or not self.namespace or self.input_schema is None:
            raise _invalid_publication("name, namespace, and input_schema are required")
        value: dict[str, object] = {
            "name": self.name,
            "namespace": self.namespace,
            "description": self.description,
            "input_schema": dict(self.input_schema),
        }
        if self.descriptor_version:
            value["descriptor_version"] = self.descriptor_version
        if self.output_schema is not None:
            value["output_schema"] = self.output_schema
        if self.exec:
            value["exec"] = dict(self.exec)
        return value


@dataclass(frozen=True)
class ValidatePackageOptions:
    """Facade-owned validation inputs forwarded to the daemon boundary."""

    manifest: Optional[AbilityPackageManifest] = None
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class PackageValidationManifest:
    name: str
    namespace: str
    wire_key: str
    descriptor_version: str
    description: str
    exec_kind: str
    input_schema: Mapping[str, object]
    timeout_seconds: Optional[int] = None
    output_schema: Any = None


@dataclass(frozen=True)
class PackageValidation:
    """SDK package-validation.schema.json projection."""

    profile: str
    kind: str
    valid: bool
    package_path: str
    manifest_path: str
    manifest_hash: str
    manifest: PackageValidationManifest
    errors: tuple[object, ...]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PackageValidation":
        decoded = _json_object(raw, "package validation")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "package_validation":
            raise _invalid_publication("invalid package validation projection")
        manifest = _required_mapping(decoded, "manifest")
        errors = decoded.get("errors")
        if not isinstance(errors, list):
            raise _invalid_publication("errors must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            valid=_required_bool(decoded, "valid"),
            package_path=_required_string(decoded, "package_path"),
            manifest_path=_required_string(decoded, "manifest_path"),
            manifest_hash=_required_string(decoded, "manifest_hash"),
            manifest=PackageValidationManifest(
                name=_required_string(manifest, "name"),
                namespace=_required_string(manifest, "namespace"),
                wire_key=_required_string(manifest, "wire_key"),
                descriptor_version=_required_string(manifest, "descriptor_version"),
                description=_required_string(manifest, "description", allow_empty=True),
                exec_kind=_required_string(manifest, "exec_kind"),
                timeout_seconds=_optional_int(
                    manifest.get("timeout_seconds"), "timeout_seconds"
                ),
                input_schema=_required_mapping(manifest, "input_schema"),
                output_schema=manifest.get("output_schema"),
            ),
            errors=tuple(errors),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class AbilityDeployRequest:
    """Complete carrier for daemon ability deployment."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    resource_ref: ResourceRef
    node_id: str
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        _validate_deploy_request(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
            "resource_ref": _resource_ref_dict(self.resource_ref),
            "node_id": self.node_id,
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class AbilityDeployResult:
    """Daemon deploy execution projection."""

    profile: str
    kind: str
    public_name: str
    namespace: str
    ability_ura: str
    node_id: str
    install_id: str
    state: str
    mutated_by: str = ""
    bundle: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "AbilityDeployResult":
        decoded = _json_object(raw, "ability deploy result")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "ability_deploy_result"
        ):
            raise _invalid_publication("invalid ability deploy result projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            public_name=_required_string(decoded, "public_name"),
            namespace=_required_string(decoded, "namespace"),
            ability_ura=_required_string(decoded, "ability_ura"),
            node_id=_required_string(decoded, "node_id"),
            install_id=_required_string(decoded, "install_id"),
            state=_required_string(decoded, "state"),
            mutated_by=_optional_string(decoded.get("mutated_by"), "mutated_by") or "",
            bundle=_optional_string(decoded.get("bundle"), "bundle") or "",
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class AbilityImplID:
    """One executable binding for enable/disable operations."""

    impl_id: str
    ability_ura: str
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        if not self.impl_id or not self.ability_ura:
            raise _invalid_publication("impl_id and ability_ura are required")
        value: dict[str, object] = {
            "impl_id": self.impl_id,
            "ability_ura": self.ability_ura,
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class PublishedAbility:
    """SDK published-ability.schema.json projection."""

    descriptor: Mapping[str, object]
    implementation: Mapping[str, object]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PublishedAbility":
        decoded = _json_object(raw, "published ability")
        return _published_ability(decoded)


@dataclass(frozen=True)
class PublishedAbilityQuery:
    """Bounded read-model query over published abilities."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    limit: int = 0
    cursor: str = ""
    owner_ura: str = ""
    ability_ura: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    def with_default_limit(self) -> "PublishedAbilityQuery":
        if self.limit == 0:
            return replace(self, limit=DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE)
        return self

    def to_json_bytes(self) -> bytes:
        query = self.with_default_limit()
        _validate_publication_query(query)
        value: dict[str, object] = {
            "caller_ura": query.caller_ura,
            "callee_ura": query.callee_ura,
            "subject_ura": query.subject_ura,
            "descriptor_version": query.descriptor_version,
            "nonce_base64": query.nonce_base64,
            "causal_context": dict(query.causal_context),
            "limit": query.limit,
        }
        if query.cursor:
            value["cursor"] = query.cursor
        if query.owner_ura:
            value["owner_ura"] = query.owner_ura
        if query.ability_ura:
            value["ability_ura"] = query.ability_ura
        if query.metadata:
            value["metadata"] = dict(query.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class PublishedAbilityPage:
    """Bounded daemon read-model page."""

    profile: str
    kind: str
    item_kind: str
    items: tuple[PublishedAbility, ...]
    limit: int
    source: str
    metadata: Mapping[str, object]
    next_cursor: Optional[str] = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PublishedAbilityPage":
        decoded = _json_object(raw, "published ability page")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("item_kind") != "published_ability"
        ):
            raise _invalid_publication("invalid published ability page projection")
        limit = _required_positive_int(decoded, "limit")
        if limit > MAX_PUBLISHED_ABILITY_PAGE_SIZE:
            raise _invalid_publication("publication query limit exceeds bounds")
        raw_items = decoded.get("items")
        if not isinstance(raw_items, list):
            raise _invalid_publication("items must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            item_kind=_required_string(decoded, "item_kind"),
            items=tuple(_published_ability(item) for item in raw_items),
            next_cursor=_optional_string(decoded.get("next_cursor"), "next_cursor"),
            limit=limit,
            source=_required_string(decoded, "source"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class UnpublishAbilityRequest:
    """Complete carrier for daemon ability unpublish."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    ability_ura: str
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        _validate_unpublish_request(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
            "ability_ura": self.ability_ura,
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return _json_bytes(value)


@dataclass(frozen=True)
class PublicationRecord:
    profile: str
    kind: str
    metadata: Mapping[str, object]
    descriptor_ref: str = ""
    owner_ura: str = ""
    resource_ref: Optional[str] = None
    status: Optional[str] = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PublicationRecord":
        decoded = _json_object(raw, "publication record")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_publication("invalid publication record projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            descriptor_ref=_optional_string(
                decoded.get("descriptor_ref"), "descriptor_ref"
            )
            or "",
            owner_ura=_optional_string(decoded.get("owner_ura"), "owner_ura") or "",
            resource_ref=_optional_string(decoded.get("resource_ref"), "resource_ref"),
            status=_optional_string(decoded.get("status"), "status"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class InstallOptions:
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class PluginInstallResult:
    profile: str
    kind: str
    source: str
    install_id: str
    status: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "PluginInstallResult":
        decoded = _json_object(raw, "plugin install result")
        if decoded.get("profile") != _PROFILE:
            raise _invalid_publication("invalid plugin install projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            source=_required_string(decoded, "source"),
            install_id=_required_string(decoded, "install_id"),
            status=_required_string(decoded, "status"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@runtime_checkable
class PublicationTransport(Protocol):
    """Concrete publication operations supplied by the integration layer."""

    def build_resource_ref(self, request_json: bytes) -> bytes:
        ...

    def validate_package(self, request_json: bytes) -> bytes:
        ...

    def deploy_ability(self, request_json: bytes) -> bytes:
        ...

    def build_deploy_invocation(self, request_json: bytes) -> bytes:
        ...

    def project_deploy_result(self, result_json: bytes) -> bytes:
        ...

    def install_plugin(self, request_json: bytes) -> bytes:
        ...

    def list_abilities(self, request_json: bytes) -> bytes:
        ...

    def build_list_abilities_invocation(self, request_json: bytes) -> bytes:
        ...

    def project_ability_page(self, page_json: bytes) -> bytes:
        ...

    def project_unpublish_result(self, result_json: bytes) -> bytes:
        ...

    def show_ability(self, request_json: bytes) -> bytes:
        ...

    def enable_ability_impl(self, request_json: bytes) -> bytes:
        ...

    def disable_ability_impl(self, request_json: bytes) -> bytes:
        ...

    def build_unpublish_invocation(self, request_json: bytes) -> bytes:
        ...

    def unpublish_ability(self, request_json: bytes) -> bytes:
        ...


@dataclass
class RuntimePublicationTransport:
    """Publication transport that dispatches complete operations through Runtime Core."""

    carrier: PublicationTransport
    runtime: RuntimeClient
    _closed: bool = field(default=False, init=False, repr=False)

    def build_resource_ref(self, request_json: bytes) -> bytes:
        return self._delegate("build_resource_ref", request_json)

    def validate_package(self, request_json: bytes) -> bytes:
        return self._delegate("validate_package", request_json)

    def deploy_ability(self, request_json: bytes) -> bytes:
        self._require_open()
        draft = InvocationDraft.from_json(
            self.carrier.build_deploy_invocation(request_json)
        )
        result = self.runtime.invoke(draft)
        if not result.ok:
            raise SDKError(
                code=ErrorCode.ABILITY_FAILED,
                stage="publication",
                retry=RetryHint.UNKNOWN,
                retryable=False,
                message="publication deploy invocation failed",
                cause=result.error,
            )
        output = result.output_json
        if not isinstance(output, dict):
            raise _invalid_publication("publication deploy output must be an object")
        return self.carrier.project_deploy_result(_json_bytes(output))

    def build_deploy_invocation(self, request_json: bytes) -> bytes:
        return self._delegate("build_deploy_invocation", request_json)

    def project_deploy_result(self, result_json: bytes) -> bytes:
        return self._delegate("project_deploy_result", result_json)

    def install_plugin(self, request_json: bytes) -> bytes:
        return self._delegate("install_plugin", request_json)

    def list_abilities(self, request_json: bytes) -> bytes:
        return self._invoke_projected(
            request_json,
            build_method="build_list_abilities_invocation",
            project_method="project_ability_page",
        )

    def project_ability_page(self, page_json: bytes) -> bytes:
        return self._delegate("project_ability_page", page_json)

    def project_unpublish_result(self, result_json: bytes) -> bytes:
        return self._delegate("project_unpublish_result", result_json)

    def show_ability(self, request_json: bytes) -> bytes:
        return self._delegate("show_ability", request_json)

    def enable_ability_impl(self, request_json: bytes) -> bytes:
        return self._delegate("enable_ability_impl", request_json)

    def disable_ability_impl(self, request_json: bytes) -> bytes:
        return self._delegate("disable_ability_impl", request_json)

    def build_unpublish_invocation(self, request_json: bytes) -> bytes:
        return self._delegate("build_unpublish_invocation", request_json)

    def unpublish_ability(self, request_json: bytes) -> bytes:
        return self._invoke_projected(
            request_json,
            build_method="build_unpublish_invocation",
            project_method="project_unpublish_result",
            projection_keys=("descriptor_version", "ability_ura"),
            failure_message="publication unpublish invocation failed",
            output_message="publication unpublish output must be an object",
        )

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        first_error: SDKError | None = None
        for owned in (self.runtime, self.carrier):
            try:
                owned.close()
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
            except Exception as exc:
                if first_error is None:
                    first_error = SDKError(
                        code=ErrorCode.TRANSPORT,
                        stage="publication",
                        retry=RetryHint.SAFE,
                        retryable=True,
                        message="publication runtime transport close failed",
                        cause=exc,
                    )
        if first_error is not None:
            raise first_error

    def _delegate(self, method_name: str, request_json: bytes) -> bytes:
        self._require_open()
        return getattr(self.carrier, method_name)(request_json)

    def _invoke_projected(
        self,
        request_json: bytes,
        *,
        build_method: str,
        project_method: str,
        projection_keys: tuple[str, ...] = ("limit", "cursor"),
        failure_message: str = "publication read-model invocation failed",
        output_message: str = "publication read-model output must be an object",
    ) -> bytes:
        self._require_open()
        draft = InvocationDraft.from_json(getattr(self.carrier, build_method)(request_json))
        result = self.runtime.invoke(draft)
        if not result.ok:
            raise SDKError(
                code=ErrorCode.ABILITY_FAILED,
                stage="publication",
                retry=RetryHint.UNKNOWN,
                retryable=False,
                message=failure_message,
                cause=result.error,
            )
        output = result.output_json
        if not isinstance(output, dict):
            raise _invalid_publication(output_message)
        projection = _projection_request(request_json, output, projection_keys)
        return getattr(self.carrier, project_method)(_json_bytes(projection))

    def _require_open(self) -> None:
        if self._closed:
            raise _invalid_publication("publication runtime transport is closed")


@dataclass(frozen=True)
class PublicationClient:
    """Publication profile facade."""

    transport: PublicationTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_publication("publication transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("publication"))

    def build_local_resource_ref(self, request: LocalResourceRefRequest) -> ResourceRef:
        self._require_open()
        if not os.path.isabs(request.path):
            raise _invalid_publication("absolute resource path is required")
        try:
            raw = self.transport.build_resource_ref(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication resource-ref build failed", exc) from exc
        return ResourceRef.from_json(raw)

    def validate_package(
        self, path: str = "", options: Optional[ValidatePackageOptions] = None
    ) -> PackageValidation:
        self._require_open()
        options = options or ValidatePackageOptions()
        if not path and options.manifest is None:
            raise _invalid_publication("package path or manifest is required")
        value: dict[str, object] = {}
        if path:
            value["path"] = path
        if options.manifest is not None:
            value["manifest"] = options.manifest.to_json_dict()
        if options.metadata:
            value["metadata"] = dict(options.metadata)
        try:
            raw = self.transport.validate_package(_json_bytes(value))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication validate package failed", exc) from exc
        return PackageValidation.from_json(raw)

    def deploy_ability(self, request: AbilityDeployRequest) -> AbilityDeployResult:
        self._require_open()
        try:
            raw = self.transport.deploy_ability(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication deploy failed", exc) from exc
        return AbilityDeployResult.from_json(raw)

    def build_deploy_invocation(self, request: AbilityDeployRequest) -> InvocationDraft:
        self._require_open()
        try:
            raw = self.transport.build_deploy_invocation(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication deploy invocation failed", exc) from exc
        return InvocationDraft.from_json(raw)

    def install_plugin(
        self, source: str, options: Optional[InstallOptions] = None
    ) -> PluginInstallResult:
        self._require_open()
        options = options or InstallOptions()
        if not source:
            raise _invalid_publication("plugin source is required")
        value: dict[str, object] = {"source": source}
        if options.metadata:
            value["metadata"] = dict(options.metadata)
        try:
            raw = self.transport.install_plugin(_json_bytes(value))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication install plugin failed", exc) from exc
        return PluginInstallResult.from_json(raw)

    def list_abilities(self, query: PublishedAbilityQuery) -> PublishedAbilityPage:
        self._require_open()
        try:
            raw = self.transport.list_abilities(query.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication list abilities failed", exc) from exc
        return PublishedAbilityPage.from_json(raw)

    def show_ability(self, descriptor_ref: str) -> PublishedAbility:
        self._require_open()
        if not descriptor_ref:
            raise _invalid_publication("descriptor_ref is required")
        try:
            raw = self.transport.show_ability(_json_bytes({"descriptor_ref": descriptor_ref}))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication show ability failed", exc) from exc
        return PublishedAbility.from_json(raw)

    def enable_ability_impl(self, impl_id: AbilityImplID) -> None:
        self._require_open()
        self._expect_record(
            self.transport.enable_ability_impl,
            impl_id.to_json_bytes(),
            "ability_impl_enabled",
            "publication enable ability impl failed",
        )

    def disable_ability_impl(self, impl_id: AbilityImplID) -> None:
        self._require_open()
        self._expect_record(
            self.transport.disable_ability_impl,
            impl_id.to_json_bytes(),
            "ability_impl_disabled",
            "publication disable ability impl failed",
        )

    def build_unpublish_invocation(
        self, request: UnpublishAbilityRequest
    ) -> InvocationDraft:
        self._require_open()
        try:
            raw = self.transport.build_unpublish_invocation(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("publication unpublish invocation failed", exc) from exc
        return InvocationDraft.from_json(raw)

    def unpublish_ability(self, target: str | UnpublishAbilityRequest) -> None:
        self._require_open()
        if isinstance(target, UnpublishAbilityRequest):
            request_json = target.to_json_bytes()
        else:
            if not target:
                raise _invalid_publication("descriptor_ref is required")
            request_json = _json_bytes({"descriptor_ref": target})
        self._expect_record(
            self.transport.unpublish_ability,
            request_json,
            "ability_unpublished",
            "publication unpublish ability failed",
        )

    def _expect_record(
        self,
        fn: Callable[[bytes], bytes],
        request_json: bytes,
        expected_kind: str,
        label: str,
    ) -> None:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        record = PublicationRecord.from_json(raw)
        if record.kind != expected_kind:
            raise _invalid_publication("invalid publication record projection")

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


def _validate_deploy_request(request: AbilityDeployRequest) -> None:
    if (
        not request.caller_ura
        or not request.callee_ura
        or not request.subject_ura
        or not request.descriptor_version
        or not request.nonce_base64
        or request.causal_context is None
        or not request.node_id
    ):
        raise _invalid_publication("complete deploy invocation carrier is required")
    _validate_resource_ref(request.resource_ref)


def _validate_unpublish_request(request: UnpublishAbilityRequest) -> None:
    if (
        not request.caller_ura
        or not request.callee_ura
        or not request.subject_ura
        or not request.descriptor_version
        or not request.nonce_base64
        or request.causal_context is None
        or not request.ability_ura
    ):
        raise _invalid_publication("complete unpublish invocation carrier is required")


def _validate_publication_query(query: PublishedAbilityQuery) -> None:
    if (
        not query.caller_ura
        or not query.callee_ura
        or not query.subject_ura
        or not query.descriptor_version
        or not query.nonce_base64
        or query.causal_context is None
    ):
        raise _invalid_publication("complete publication query carrier is required")
    if query.limit <= 0 or query.limit > MAX_PUBLISHED_ABILITY_PAGE_SIZE:
        raise _invalid_publication("publication query limit exceeds bounds")


def _validate_resource_ref(ref: ResourceRef) -> None:
    if (
        not ref.resource_ura
        or not ref.owner_ura
        or not ref.namespace
        or not ref.capability
        or not ref.revision
    ):
        raise _invalid_publication("valid resource_ref is required")
    if ref.namespace.lower() in {"axon", "daemon", "easynet", "internal", "system"}:
        raise _invalid_publication("resource_ref namespace is reserved")


def _resource_ref_dict(ref: ResourceRef) -> dict[str, object]:
    _validate_resource_ref(ref)
    value: dict[str, object] = {
        "resource_ura": ref.resource_ura,
        "owner_ura": ref.owner_ura,
        "namespace": ref.namespace,
        "capability": ref.capability,
        "expires_unix_ms": ref.expires_unix_ms,
        "revision": ref.revision,
    }
    if ref.display_path:
        value["display_path"] = ref.display_path
    return value


def _published_ability(value: object) -> PublishedAbility:
    if not isinstance(value, dict):
        raise _invalid_publication("published ability must be an object")
    return PublishedAbility(
        descriptor=_required_mapping(value, "descriptor"),
        implementation=_required_mapping(value, "implementation"),
        metadata=_required_mapping(value, "metadata"),
    )


def _projection_request(
    request_json: bytes,
    result: Mapping[str, object],
    passthrough_keys: tuple[str, ...] = ("limit", "cursor"),
) -> dict[str, object]:
    request = _json_object(request_json, "publication projection request")
    projection: dict[str, object] = {"result": dict(result)}
    for key in passthrough_keys:
        value = request.get(key)
        if value is not None:
            projection[key] = value
    return projection


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_publication(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_publication(f"{label} JSON must be an object")
    return decoded


def _required_string(
    decoded: Mapping[str, object], field_name: str, *, allow_empty: bool = False
) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or (not allow_empty and value.strip() == ""):
        raise _invalid_publication(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_publication(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_publication(f"{field_name} must be a boolean")
    return value


def _optional_int(value: object, field_name: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_publication(f"{field_name} must be an integer or null")
    return value


def _required_positive_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_publication(f"{field_name} must be a positive integer")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_publication(f"{field_name} must be an object")
    return dict(value)


def _invalid_publication(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="publication",
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
