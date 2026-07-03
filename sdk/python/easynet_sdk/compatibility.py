"""Compatibility profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from typing import Any, Mapping, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft


_PROFILE = "compatibility"


@dataclass(frozen=True)
class CompatibilityCarrierBase:
    """Complete carrier context shared by Compatibility operations."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    auth_token: str = ""
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
        if self.auth_token:
            value["auth_token"] = self.auth_token
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class CompatibilityListModelsRequest:
    base: CompatibilityCarrierBase

    def to_json_bytes(self) -> bytes:
        return _json_bytes(self.base.to_json_dict())


@dataclass(frozen=True)
class CompatibilityChatCompletionRequest:
    base: CompatibilityCarrierBase
    request: Mapping[str, object]

    def to_json_bytes(self) -> bytes:
        _validate_chat_request(self.request)
        if self.request.get("stream") is True:
            raise _invalid_compatibility("unary chat completion request must not set stream=true")
        value = self.base.to_json_dict()
        value["request"] = dict(self.request)
        return _json_bytes(value)


@dataclass(frozen=True)
class CompatibilityStreamChatCompletionRequest:
    base: CompatibilityCarrierBase
    request: Mapping[str, object]

    def to_json_bytes(self) -> bytes:
        _validate_chat_request(self.request)
        value = self.base.to_json_dict()
        stream_request = dict(self.request)
        stream_request["stream"] = True
        value["request"] = stream_request
        return _json_bytes(value)


@dataclass(frozen=True)
class CompatibilityFileUploadRequest:
    purpose: str
    id: str = ""
    file_id: str = ""
    file_ref: str = ""
    resource_ref: str = ""
    resource_ura: str = ""
    filename: str = ""
    owner_ura: str = ""
    content_type: str = ""
    content_hash: str = ""
    bytes: int = 0
    size_bytes: int = 0
    created_at: int = 0
    status: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class CompatibilityFileRequest:
    id: str = ""
    file_id: str = ""
    file_ref: str = ""
    resource_ref: str = ""
    resource_ura: str = ""
    filename: str = ""
    purpose: str = ""
    owner_ura: str = ""
    content_type: str = ""
    content_hash: str = ""
    bytes: int = 0
    size_bytes: int = 0
    created_at: int = 0
    created: int = 0
    status: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class CompatibilityFileDeleteRequest:
    deleted: bool
    id: str = ""
    file_id: str = ""
    file_ref: str = ""
    resource_ref: str = ""
    resource_ura: str = ""
    content_hash: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


ListModelsRequest = CompatibilityListModelsRequest
ChatCompletionRequest = CompatibilityChatCompletionRequest
StreamChatCompletionRequest = CompatibilityStreamChatCompletionRequest


@dataclass(frozen=True)
class CompatibilityModel:
    profile: str
    kind: str
    id: str
    object: str
    created: int
    owned_by: str
    ability_ref: str
    metadata: Mapping[str, object]


@dataclass(frozen=True)
class CompatibilityModelPage:
    profile: str
    kind: str
    object: str
    data: tuple[CompatibilityModel, ...]
    next_cursor: Optional[str]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "CompatibilityModelPage":
        decoded = _json_object(raw, "compatibility model page")
        if decoded.get("profile") != _PROFILE or decoded.get("kind") != "model_page" or decoded.get("object") != "list":
            raise _invalid_compatibility("invalid compatibility model page projection")
        data = decoded.get("data")
        if not isinstance(data, list):
            raise _invalid_compatibility("model page data must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            object=_required_string(decoded, "object"),
            data=tuple(_model(item) for item in data),
            next_cursor=_optional_string(decoded.get("next_cursor"), "next_cursor"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class CompatibilityChatCompletion:
    profile: str
    kind: str
    id: str
    object: str
    created: int
    model: str
    choices: tuple[Mapping[str, object], ...]
    usage: Mapping[str, object]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "CompatibilityChatCompletion":
        decoded = _json_object(raw, "compatibility chat completion")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "chat_completion"
            or decoded.get("object") != "chat.completion"
        ):
            raise _invalid_compatibility("invalid compatibility chat completion projection")
        choices = decoded.get("choices")
        if not isinstance(choices, list):
            raise _invalid_compatibility("choices must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            id=_required_string(decoded, "id"),
            object=_required_string(decoded, "object"),
            created=_required_non_negative_int(decoded, "created"),
            model=_required_string(decoded, "model"),
            choices=tuple(_required_mapping(item, "choice") for item in choices),
            usage=_required_mapping(decoded, "usage"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class CompatibilityChatCompletionChunk:
    profile: str
    kind: str
    id: str
    object: str
    created: int
    model: str
    choices: tuple[Mapping[str, object], ...]
    usage: object
    metadata: Mapping[str, object]


@dataclass(frozen=True)
class CompatibilityChatCompletionStream:
    profile: str
    kind: str
    stream: bool
    items: tuple[CompatibilityChatCompletionChunk, ...]
    done_sentinel: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "CompatibilityChatCompletionStream":
        decoded = _json_object(raw, "compatibility chat completion stream")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "chat_completion_stream"
            or decoded.get("stream") is not True
            or decoded.get("done_sentinel") != "[DONE]"
        ):
            raise _invalid_compatibility("invalid compatibility chat stream projection")
        items = decoded.get("items")
        if not isinstance(items, list):
            raise _invalid_compatibility("stream items must be an array")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            stream=_required_bool(decoded, "stream"),
            items=tuple(_chat_chunk(item) for item in items),
            done_sentinel=_required_string(decoded, "done_sentinel"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class CompatibilityFile:
    profile: str
    kind: str
    id: str
    object: str
    bytes: int
    created_at: int
    filename: str
    purpose: str
    status: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "CompatibilityFile":
        decoded = _json_object(raw, "compatibility file")
        return _file(decoded)


@dataclass(frozen=True)
class CompatibilityFileDeleteResult:
    profile: str
    kind: str
    id: str
    object: str
    deleted: bool
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "CompatibilityFileDeleteResult":
        decoded = _json_object(raw, "compatibility file delete result")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "file_delete_result"
            or decoded.get("object") != "file"
            or decoded.get("deleted") is not True
        ):
            raise _invalid_compatibility("invalid compatibility file delete projection")
        return cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            id=_required_string(decoded, "id"),
            object=_required_string(decoded, "object"),
            deleted=_required_bool(decoded, "deleted"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@runtime_checkable
class CompatibilityTransport(Protocol):
    """Concrete Compatibility operations supplied by the integration layer."""

    def build_list_models_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_chat_completion_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_stream_chat_completion_invocation(self, request_json: bytes) -> bytes:
        ...

    def list_models(self, request_json: bytes) -> bytes:
        ...

    def create_chat_completion(self, request_json: bytes) -> bytes:
        ...

    def stream_chat_completion(self, request_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class CompatibilityClient:
    """Compatibility profile facade."""

    transport: CompatibilityTransport

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_compatibility("compatibility transport is required")

    def build_list_models_invocation(self, request: CompatibilityListModelsRequest) -> InvocationDraft:
        return self._build_invocation(
            request.to_json_bytes(),
            self.transport.build_list_models_invocation,
            "compatibility list-models invocation failed",
        )

    def build_chat_completion_invocation(self, request: CompatibilityChatCompletionRequest) -> InvocationDraft:
        return self._build_invocation(
            request.to_json_bytes(),
            self.transport.build_chat_completion_invocation,
            "compatibility chat-completion invocation failed",
        )

    def build_stream_chat_completion_invocation(
        self, request: CompatibilityStreamChatCompletionRequest
    ) -> InvocationDraft:
        return self._build_invocation(
            request.to_json_bytes(),
            self.transport.build_stream_chat_completion_invocation,
            "compatibility stream-chat-completion invocation failed",
        )

    def list_models(self, request: CompatibilityListModelsRequest) -> CompatibilityModelPage:
        try:
            raw = self.transport.list_models(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("compatibility list models failed", exc) from exc
        return CompatibilityModelPage.from_json(raw)

    def create_chat_completion(
        self, request: CompatibilityChatCompletionRequest
    ) -> CompatibilityChatCompletion:
        try:
            raw = self.transport.create_chat_completion(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("compatibility chat completion failed", exc) from exc
        return CompatibilityChatCompletion.from_json(raw)

    def stream_chat_completion(
        self, request: CompatibilityStreamChatCompletionRequest
    ) -> CompatibilityChatCompletionStream:
        try:
            raw = self.transport.stream_chat_completion(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("compatibility stream chat completion failed", exc) from exc
        return CompatibilityChatCompletionStream.from_json(raw)

    def project_file_upload(self, request: CompatibilityFileUploadRequest) -> CompatibilityFile:
        _validate_file_upload_request(request)
        return _file_from_facts(
            id=request.id,
            file_id=request.file_id,
            file_ref=request.file_ref,
            resource_ref=request.resource_ref,
            resource_ura=request.resource_ura,
            filename=request.filename,
            purpose=request.purpose,
            owner_ura=request.owner_ura,
            content_type=request.content_type,
            content_hash=request.content_hash,
            bytes_value=request.bytes,
            size_bytes=request.size_bytes,
            created_at=request.created_at,
            created=0,
            status=request.status,
        )

    def project_file(self, request: CompatibilityFileRequest) -> CompatibilityFile:
        _validate_file_request(request)
        return _file_from_facts(
            id=request.id,
            file_id=request.file_id,
            file_ref=request.file_ref,
            resource_ref=request.resource_ref,
            resource_ura=request.resource_ura,
            filename=request.filename,
            purpose=request.purpose,
            owner_ura=request.owner_ura,
            content_type=request.content_type,
            content_hash=request.content_hash,
            bytes_value=request.bytes,
            size_bytes=request.size_bytes,
            created_at=request.created_at,
            created=request.created,
            status=request.status,
        )

    def project_file_delete_result(
        self, request: CompatibilityFileDeleteRequest
    ) -> CompatibilityFileDeleteResult:
        _validate_file_delete_request(request)
        return CompatibilityFileDeleteResult(
            profile=_PROFILE,
            kind="file_delete_result",
            id=_first_non_empty(
                request.id,
                request.file_id,
                request.file_ref,
                request.resource_ref,
                request.resource_ura,
                request.content_hash,
            ),
            object="file",
            deleted=True,
            metadata={"profile": _PROFILE, "source": "compatibility.file_delete"},
        )

    def _build_invocation(
        self, request_json: bytes, fn: Any, label: str
    ) -> InvocationDraft:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return InvocationDraft.from_json(raw)


def _validate_base(base: CompatibilityCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_compatibility("complete compatibility invocation carrier is required")


def _validate_chat_request(request: Mapping[str, object]) -> None:
    if request is None:
        raise _invalid_compatibility("compatibility chat request is required")
    model = request.get("model")
    if not isinstance(model, str) or not model:
        raise _invalid_compatibility("compatibility model is required")
    if not (model.startswith("easynet://") and "/ability/" in model):
        raise _invalid_compatibility("compatibility model must be an EasyNet ability ref")
    messages = request.get("messages")
    if not isinstance(messages, list) or not messages:
        raise _invalid_compatibility("compatibility messages are required")


def _model(raw: object) -> CompatibilityModel:
    decoded = _required_mapping(raw, "model")
    if decoded.get("profile") != _PROFILE or decoded.get("kind") != "model" or decoded.get("object") != "model":
        raise _invalid_compatibility("invalid compatibility model projection")
    return CompatibilityModel(
        profile=_required_string(decoded, "profile"),
        kind=_required_string(decoded, "kind"),
        id=_required_string(decoded, "id"),
        object=_required_string(decoded, "object"),
        created=_required_non_negative_int(decoded, "created"),
        owned_by=_required_string(decoded, "owned_by"),
        ability_ref=_required_string(decoded, "ability_ref"),
        metadata=_required_mapping(decoded, "metadata"),
    )


def _chat_chunk(raw: object) -> CompatibilityChatCompletionChunk:
    decoded = _required_mapping(raw, "chat chunk")
    if (
        decoded.get("profile") != _PROFILE
        or decoded.get("kind") != "chat_completion_chunk"
        or decoded.get("object") != "chat.completion.chunk"
    ):
        raise _invalid_compatibility("invalid compatibility chat stream chunk projection")
    choices = decoded.get("choices")
    if not isinstance(choices, list):
        raise _invalid_compatibility("chunk choices must be an array")
    return CompatibilityChatCompletionChunk(
        profile=_required_string(decoded, "profile"),
        kind=_required_string(decoded, "kind"),
        id=_required_string(decoded, "id"),
        object=_required_string(decoded, "object"),
        created=_required_non_negative_int(decoded, "created"),
        model=_required_string(decoded, "model"),
        choices=tuple(_required_mapping(item, "choice") for item in choices),
        usage=decoded.get("usage"),
        metadata=_required_mapping(decoded, "metadata"),
    )


def _file(raw: Mapping[str, object]) -> CompatibilityFile:
    if raw.get("profile") != _PROFILE or raw.get("kind") != "file" or raw.get("object") != "file":
        raise _invalid_compatibility("invalid compatibility file projection")
    return CompatibilityFile(
        profile=_required_string(raw, "profile"),
        kind=_required_string(raw, "kind"),
        id=_required_string(raw, "id"),
        object=_required_string(raw, "object"),
        bytes=_required_non_negative_int(raw, "bytes"),
        created_at=_required_non_negative_int(raw, "created_at"),
        filename=_required_string(raw, "filename"),
        purpose=_required_string(raw, "purpose"),
        status=_required_string(raw, "status"),
        metadata=_required_mapping(raw, "metadata"),
    )


def _validate_file_upload_request(request: CompatibilityFileUploadRequest) -> None:
    if not request.purpose:
        raise _invalid_compatibility("compatibility file purpose is required")
    _validate_file_facts(
        request.id,
        request.file_id,
        request.file_ref,
        request.resource_ref,
        request.resource_ura,
        request.filename,
        request.bytes,
        request.size_bytes,
        request.created_at,
        0,
    )


def _validate_file_request(request: CompatibilityFileRequest) -> None:
    _validate_file_facts(
        request.id,
        request.file_id,
        request.file_ref,
        request.resource_ref,
        request.resource_ura,
        request.filename,
        request.bytes,
        request.size_bytes,
        request.created_at,
        request.created,
    )


def _validate_file_facts(
    id: str,
    file_id: str,
    file_ref: str,
    resource_ref: str,
    resource_ura: str,
    filename: str,
    bytes_value: int,
    size_bytes: int,
    created_at: int,
    created: int,
) -> None:
    if not _first_non_empty(id, file_id):
        raise _invalid_compatibility("compatibility file id is required")
    if not _first_non_empty(file_ref, resource_ref, resource_ura):
        raise _invalid_compatibility("compatibility file ref is required")
    if not filename:
        raise _invalid_compatibility("compatibility filename is required")
    if min(bytes_value, size_bytes, created_at, created) < 0:
        raise _invalid_compatibility("compatibility file counters must be non-negative")


def _validate_file_delete_request(request: CompatibilityFileDeleteRequest) -> None:
    if not _first_non_empty(
        request.id,
        request.file_id,
        request.file_ref,
        request.resource_ref,
        request.resource_ura,
        request.content_hash,
    ):
        raise _invalid_compatibility("compatibility file identity is required")
    if request.deleted is not True:
        raise _invalid_compatibility("compatibility file delete result must be deleted")


def _file_from_facts(
    *,
    id: str,
    file_id: str,
    file_ref: str,
    resource_ref: str,
    resource_ura: str,
    filename: str,
    purpose: str,
    owner_ura: str,
    content_type: str,
    content_hash: str,
    bytes_value: int,
    size_bytes: int,
    created_at: int,
    created: int,
    status: str,
) -> CompatibilityFile:
    metadata: dict[str, object] = {"profile": _PROFILE, "source": "compatibility.file"}
    if owner_ura:
        metadata["owner_ura"] = owner_ura
    if content_type:
        metadata["content_type"] = content_type
    if content_hash:
        metadata["content_hash"] = content_hash
    ref = _first_non_empty(file_ref, resource_ref, resource_ura)
    if ref:
        metadata["file_ref"] = ref
    file = CompatibilityFile(
        profile=_PROFILE,
        kind="file",
        id=_first_non_empty(id, file_id),
        object="file",
        bytes=_first_non_zero(size_bytes, bytes_value),
        created_at=_first_non_zero(created_at, created),
        filename=filename,
        purpose=purpose,
        status=status or "processed",
        metadata=metadata,
    )
    _file(file.__dict__)
    return file


def _required_mapping(value: object, field: str) -> Mapping[str, object]:
    if not isinstance(value, dict):
        raise _invalid_compatibility(f"{field} must be an object")
    return value


def _required_string(decoded: Mapping[str, object], field: str) -> str:
    value = decoded.get(field)
    if not isinstance(value, str) or not value:
        raise _invalid_compatibility(f"{field} is required")
    return value


def _optional_string(value: object, field: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_compatibility(f"{field} must be a string")
    return value


def _required_bool(decoded: Mapping[str, object], field: str) -> bool:
    value = decoded.get(field)
    if not isinstance(value, bool):
        raise _invalid_compatibility(f"{field} must be a boolean")
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field: str) -> int:
    value = decoded.get(field)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_compatibility(f"{field} must be a non-negative integer")
    return value


def _json_object(raw: bytes | str, label: str) -> Mapping[str, object]:
    try:
        decoded = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise _invalid_compatibility(f"decode {label} JSON: {exc}") from exc
    if not isinstance(decoded, dict):
        raise _invalid_compatibility(f"{label} must be a JSON object")
    return decoded


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _first_non_empty(*values: str) -> str:
    for value in values:
        if value:
            return value
    return ""


def _first_non_zero(*values: int) -> int:
    for value in values:
        if value != 0:
            return value
    return 0


def _invalid_compatibility(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="compatibility",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.UNKNOWN,
        retryable=False,
        message=message,
        cause=cause,
    )

