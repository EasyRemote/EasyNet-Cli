"""Surface profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft


_PROFILE = "surface"
_READ_MODEL = "pages_read_model"
DEFAULT_SURFACE_PAGE_SIZE = 50
MAX_SURFACE_PAGE_SIZE = 500


@dataclass(frozen=True)
class SurfaceCarrierBase:
    """Complete carrier context shared by Surface operations."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        _validate_base(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class SurfaceListPagesRequest:
    base: SurfaceCarrierBase
    limit: int = 0
    cursor: str = ""

    def to_json_bytes(self) -> bytes:
        limit = self.limit or DEFAULT_SURFACE_PAGE_SIZE
        if limit < 1 or limit > MAX_SURFACE_PAGE_SIZE:
            raise _invalid_surface("surface page limit exceeds bounds")
        value = self.base.to_json_dict()
        if self.limit:
            value["limit"] = self.limit
        if self.cursor:
            value["cursor"] = self.cursor
        return _json_bytes(value)


@dataclass(frozen=True)
class SurfaceCreatePageRequest:
    base: SurfaceCarrierBase
    project_id: str
    folder: str
    visibility: str = ""

    def to_json_bytes(self) -> bytes:
        _validate_project_id(self.project_id)
        if not self.folder.startswith("/"):
            raise _invalid_surface("surface folder must be absolute")
        if self.visibility and self.visibility not in ("public", "private"):
            raise _invalid_surface("invalid surface visibility")
        value = self.base.to_json_dict()
        value["project_id"] = self.project_id
        value["folder"] = self.folder
        if self.visibility:
            value["visibility"] = self.visibility
        return _json_bytes(value)


@dataclass(frozen=True)
class SurfaceDeletePageRequest:
    base: SurfaceCarrierBase
    project_id: str

    def to_json_bytes(self) -> bytes:
        _validate_project_id(self.project_id)
        value = self.base.to_json_dict()
        value["project_id"] = self.project_id
        return _json_bytes(value)


@dataclass(frozen=True)
class SurfaceManifestRequest:
    base: SurfaceCarrierBase
    project_id: str

    def to_json_bytes(self) -> bytes:
        _validate_project_id(self.project_id)
        value = self.base.to_json_dict()
        value["project_id"] = self.project_id
        return _json_bytes(value)


@dataclass(frozen=True)
class SurfaceHealthRequest:
    base: SurfaceCarrierBase
    project_id: str = ""
    surface_ref: str = ""

    def to_json_bytes(self) -> bytes:
        if self.project_id:
            _validate_project_id(self.project_id)
        if self.surface_ref:
            _validate_surface_ref(self.surface_ref)
        value = self.base.to_json_dict()
        if self.project_id:
            value["project_id"] = self.project_id
        if self.surface_ref:
            value["surface_ref"] = self.surface_ref
        return _json_bytes(value)


PageQuery = SurfaceListPagesRequest
CreatePageRequest = SurfaceCreatePageRequest
DeletePageRequest = SurfaceDeletePageRequest
SurfaceStatusRequest = SurfaceHealthRequest


@dataclass(frozen=True)
class SurfacePageRecord:
    profile: str
    kind: str
    page_id: str
    owner_ura: str
    surface_ref: str
    public_ref: Optional[str] = None
    status: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SurfacePageRecord":
        return _page_record(_json_object(raw, "surface page record"))

    def to_json_dict(self) -> dict[str, object]:
        _validate_page_record(self)
        return {
            "profile": self.profile,
            "kind": self.kind,
            "page_id": self.page_id,
            "owner_ura": self.owner_ura,
            "surface_ref": self.surface_ref,
            "public_ref": self.public_ref,
            "status": self.status,
            "metadata": dict(self.metadata),
        }


@dataclass(frozen=True)
class SurfacePagePage:
    profile: str
    kind: str
    item_kind: str
    items: tuple[SurfacePageRecord, ...]
    next_cursor: Optional[str]
    limit: int
    source: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SurfacePagePage":
        decoded = _json_object(raw, "surface page page")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "surface_page_page"
            or decoded.get("item_kind") != "page_record"
            or decoded.get("source") != _READ_MODEL
        ):
            raise _invalid_surface("invalid surface page projection")
        limit = _required_positive_int(decoded, "limit")
        if limit > MAX_SURFACE_PAGE_SIZE:
            raise _invalid_surface("surface page limit exceeds bounds")
        items = decoded.get("items")
        if not isinstance(items, list):
            raise _invalid_surface("items must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            item_kind=_required_string(decoded, "item_kind"),
            items=tuple(_page_record(item) for item in items),
            next_cursor=_optional_string(decoded.get("next_cursor"), "next_cursor"),
            limit=limit,
            source=_required_string(decoded, "source"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class SurfaceManifest:
    profile: str
    kind: str
    page_id: str
    owner_ura: str
    surface_ref: str
    public_ref: str
    page: SurfacePageRecord
    entrypoint: Mapping[str, object]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SurfaceManifest":
        decoded = _json_object(raw, "surface manifest")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "surface_manifest":
            raise _invalid_surface("invalid surface manifest projection")
        page = decoded.get("page")
        if not isinstance(page, dict):
            raise _invalid_surface("page must be an object")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            page_id=_required_string(decoded, "page_id"),
            owner_ura=_required_string(decoded, "owner_ura"),
            surface_ref=_required_string(decoded, "surface_ref"),
            public_ref=_required_string(decoded, "public_ref"),
            page=_page_record(page),
            entrypoint=_required_mapping(decoded, "entrypoint"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class SurfacePublicPageRef:
    profile: str
    kind: str
    page_id: str
    owner_ura: str
    surface_ref: str
    public_ref: str
    route_kind: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SurfacePublicPageRef":
        decoded = _json_object(raw, "surface public page ref")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "public_page_ref"
            or decoded.get("route_kind") != "hub_web"
        ):
            raise _invalid_surface("invalid surface public page ref projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            page_id=_required_string(decoded, "page_id"),
            owner_ura=_required_string(decoded, "owner_ura"),
            surface_ref=_required_string(decoded, "surface_ref"),
            public_ref=_required_string(decoded, "public_ref"),
            route_kind=_required_string(decoded, "route_kind"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class SurfaceMutationResult:
    profile: str
    kind: str
    operation: str
    page_id: str
    removed: bool
    state: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SurfaceMutationResult":
        decoded = _json_object(raw, "surface mutation result")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "surface_mutation_result"
            or decoded.get("operation") != "delete"
        ):
            raise _invalid_surface("invalid surface mutation result projection")
        state = _required_string(decoded, "state")
        if state not in ("deleted", "unknown"):
            raise _invalid_surface("invalid surface mutation state")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            operation=_required_string(decoded, "operation"),
            page_id=_required_string(decoded, "page_id"),
            removed=_required_bool(decoded, "removed"),
            state=state,
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class SurfaceHealthCheck:
    name: str
    state: str
    ready: bool
    message: Optional[str]
    latency_ms: int
    metadata: Mapping[str, object]


@dataclass(frozen=True)
class SurfaceHealth:
    """Daemon-governed surface readiness projection."""

    profile: str
    kind: str
    state: str
    ready: bool
    owner_ura: str
    surface_ref: str
    descriptor_ref: str
    descriptor_version: str
    page_count: int
    checks: tuple[SurfaceHealthCheck, ...]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "SurfaceHealth":
        decoded = _json_object(raw, "surface health")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "surface_health":
            raise _invalid_surface("invalid surface health projection")
        page_count = _required_non_negative_int(decoded, "page_count")
        checks = decoded.get("checks")
        if not isinstance(checks, list):
            raise _invalid_surface("checks must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            state=_required_string(decoded, "state"),
            ready=_required_bool(decoded, "ready"),
            owner_ura=_required_string(decoded, "owner_ura"),
            surface_ref=_required_string(decoded, "surface_ref"),
            descriptor_ref=_required_string(decoded, "descriptor_ref"),
            descriptor_version=_required_string(decoded, "descriptor_version"),
            page_count=page_count,
            checks=tuple(_health_check(item) for item in checks),
            metadata=_required_mapping(decoded, "metadata"),
        )


SurfaceStatus = SurfaceHealth


@runtime_checkable
class SurfaceTransport(Protocol):
    """Concrete Surface operations supplied by the integration layer."""

    def build_list_pages_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_create_page_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_delete_page_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_manifest_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_health_invocation(self, request_json: bytes) -> bytes:
        ...

    def list_pages(self, request_json: bytes) -> bytes:
        ...

    def create_page(self, request_json: bytes) -> bytes:
        ...

    def delete_page(self, request_json: bytes) -> bytes:
        ...

    def surface_manifest(self, request_json: bytes) -> bytes:
        ...

    def public_page_ref(self, request_json: bytes) -> bytes:
        ...

    def surface_health(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class SurfaceClient:
    """Surface profile facade."""

    transport: SurfaceTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_surface("surface transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("surface"))

    def build_list_pages_invocation(self, request: SurfaceListPagesRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_list_pages_invocation,
            "surface list-pages invocation failed",
        )

    def build_create_page_invocation(self, request: SurfaceCreatePageRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_create_page_invocation,
            "surface create-page invocation failed",
        )

    def build_delete_page_invocation(self, request: SurfaceDeletePageRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_delete_page_invocation,
            "surface delete-page invocation failed",
        )

    def build_manifest_invocation(self, request: SurfaceManifestRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_manifest_invocation,
            "surface manifest invocation failed",
        )

    def build_health_invocation(self, request: SurfaceHealthRequest) -> InvocationDraft:
        self._require_open()
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_health_invocation,
            "surface health invocation failed",
        )

    def list_pages(self, request: SurfaceListPagesRequest) -> SurfacePagePage:
        self._require_open()
        return self._page(
            request.to_json_bytes(), self.transport.list_pages, "surface list pages failed"
        )

    def create_page(self, request: SurfaceCreatePageRequest) -> SurfacePageRecord:
        self._require_open()
        try:
            raw = self.transport.create_page(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("surface create page failed", exc) from exc
        return SurfacePageRecord.from_json(raw)

    def delete_page(self, request: SurfaceDeletePageRequest) -> SurfaceMutationResult:
        self._require_open()
        try:
            raw = self.transport.delete_page(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("surface delete page failed", exc) from exc
        return SurfaceMutationResult.from_json(raw)

    def surface_manifest(self, request: SurfaceManifestRequest) -> SurfaceManifest:
        self._require_open()
        try:
            raw = self.transport.surface_manifest(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("surface manifest failed", exc) from exc
        return SurfaceManifest.from_json(raw)

    def public_page_ref(self, page: SurfacePageRecord) -> SurfacePublicPageRef:
        self._require_open()
        _validate_page_record(page)
        try:
            raw = self.transport.public_page_ref(_json_bytes(page.to_json_dict()))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("surface public page ref failed", exc) from exc
        return SurfacePublicPageRef.from_json(raw)

    def surface_health(self, request: SurfaceHealthRequest) -> SurfaceHealth:
        self._require_open()
        try:
            raw = self.transport.surface_health(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("surface health failed", exc) from exc
        return SurfaceHealth.from_json(raw)

    def surface_status(self, request: SurfaceStatusRequest) -> SurfaceStatus:
        return self.surface_health(request)

    def _invocation(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> InvocationDraft:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return InvocationDraft.from_json(raw)

    def _page(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> SurfacePagePage:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return SurfacePagePage.from_json(raw)

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


def _validate_base(base: SurfaceCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_surface("complete surface invocation carrier is required")


def _validate_project_id(value: str) -> None:
    if not value or len(value) > 64:
        raise _invalid_surface("invalid surface project_id")
    if not all(ch.isascii() and (ch.isalnum() or ch in "_-") for ch in value):
        raise _invalid_surface("invalid surface project_id")


def _validate_surface_ref(value: str) -> None:
    if not value or not value.strip() or value.strip() != value:
        raise _invalid_surface("surface_ref must be a clean daemon ref")
    if value.startswith("http://") or value.startswith("https://"):
        raise _invalid_surface("surface_ref must not be an HTTP route")
    if not value.startswith("easynet://"):
        raise _invalid_surface("surface_ref must be an EasyNet ref")


def _validate_page_record(record: SurfacePageRecord) -> None:
    if (
        record.profile != _PROFILE
        or record.kind != "page_record"
        or not record.page_id
        or not record.owner_ura
        or not record.surface_ref
    ):
        raise _invalid_surface("invalid surface page record projection")


def _page_record(value: object) -> SurfacePageRecord:
    if not isinstance(value, dict):
        raise _invalid_surface("page record must be an object")
    record = SurfacePageRecord(
        profile=_required_string(value, "profile"),
        kind=_required_string(value, "kind"),
        page_id=_required_string(value, "page_id"),
        owner_ura=_required_string(value, "owner_ura"),
        surface_ref=_required_string(value, "surface_ref"),
        public_ref=_optional_string(value.get("public_ref"), "public_ref"),
        status=_optional_string(value.get("status"), "status"),
        metadata=_required_mapping(value, "metadata")
        if "metadata" in value
        else {},
    )
    _validate_page_record(record)
    return record


def _health_check(value: object) -> SurfaceHealthCheck:
    if not isinstance(value, dict):
        raise _invalid_surface("surface health check must be an object")
    latency = _required_non_negative_int(value, "latency_ms")
    return SurfaceHealthCheck(
        name=_required_string(value, "name"),
        state=_required_string(value, "state"),
        ready=_required_bool(value, "ready"),
        message=_optional_string(value.get("message"), "message"),
        latency_ms=latency,
        metadata=_required_mapping(value, "metadata"),
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_surface(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_surface(f"{label} JSON must be an object")
    return decoded


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_surface(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_surface(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_surface(f"{field_name} must be a boolean")
    return value


def _required_positive_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_surface(f"{field_name} is required")
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_surface(f"{field_name} must be a non-negative integer")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_surface(f"{field_name} must be an object")
    return dict(value)


def _invalid_surface(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="surface",
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
