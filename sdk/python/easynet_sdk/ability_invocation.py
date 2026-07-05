"""Ability Invocation convenience facade over Runtime Core."""

from __future__ import annotations

import base64
from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import Any, Mapping

from .bidi import BidiSession, BidiStreamDescriptor
from .errors import ErrorCode, RetryHint, SDKError, is_code
from .identity import AddressingClient, DescriptorRefRequest
from .invocation import InvocationBuilder, InvocationDraft, InvocationSignature
from .receipt import ReceiptClient
from .runtime import (
    InvocationCancel,
    InvocationHandle,
    InvocationResult,
    PrepareOptions,
    RuntimeClient,
)
from .signing import PreparedInvocation, SignedInvocation, Signer, SigningMaterial
from .stream import StreamHandle


@dataclass(frozen=True)
class AbilityCallRequest:
    """Complete ability Invocation request with an Axon-delegated selector."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    descriptor_ref: str = ""
    ability_ura: str = ""
    ability_name: str = ""
    descriptor_version: str = "1.0.0"
    content_type: str = "application/json"
    args: Any = field(default_factory=dict)
    arguments_base64: str | None = None
    metadata: Mapping[str, object] = field(default_factory=dict)
    caller_signature: InvocationSignature | None = None


@dataclass(frozen=True)
class AbilityTargetRequest:
    """Ability Invocation request where the SDK resolves callee/subject facts."""

    caller_ura: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    descriptor_ref: str = ""
    ability_ura: str = ""
    owner_ura: str = ""
    ability_name: str = ""
    subject_ura: str = ""
    descriptor_version: str = "1.0.0"
    content_type: str = "application/json"
    args: Any = field(default_factory=dict)
    arguments_base64: str | None = None
    metadata: Mapping[str, object] = field(default_factory=dict)
    caller_signature: InvocationSignature | None = None


@dataclass(frozen=True)
class ResolvedAbilityTarget:
    """Daemon/Axon-projected target facts for an ability Invocation."""

    ability_ura: str
    descriptor_ref: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str


@dataclass(frozen=True)
class AbilityChildContext:
    """Child Invocation context anchored to a daemon/Axon parent receipt."""

    invoker: "AbilityInvocationClient"
    caller_ura: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def target_request(
        self,
        *,
        descriptor_ref: str = "",
        ability_ura: str = "",
        owner_ura: str = "",
        ability_name: str = "",
        subject_ura: str = "",
        descriptor_version: str = "1.0.0",
        content_type: str = "application/json",
        args: Any = None,
        arguments_base64: str | None = None,
        metadata: Mapping[str, object] | None = None,
        caller_signature: InvocationSignature | None = None,
    ) -> AbilityTargetRequest:
        """Build a child ability target request."""

        return AbilityTargetRequest(
            caller_ura=_required_string(self.caller_ura, "caller_ura"),
            nonce_base64=_required_string(self.nonce_base64, "nonce_base64"),
            causal_context=_required_mapping(self.causal_context, "causal_context"),
            descriptor_ref=descriptor_ref,
            ability_ura=ability_ura,
            owner_ura=owner_ura,
            ability_name=ability_name,
            subject_ura=subject_ura,
            descriptor_version=descriptor_version,
            content_type=content_type,
            args={} if args is None else args,
            arguments_base64=arguments_base64,
            metadata=_merged_metadata(self.metadata, metadata),
            caller_signature=caller_signature,
        )

    def call_request(
        self,
        *,
        callee_ura: str,
        subject_ura: str,
        descriptor_ref: str = "",
        ability_ura: str = "",
        ability_name: str = "",
        descriptor_version: str = "1.0.0",
        content_type: str = "application/json",
        args: Any = None,
        arguments_base64: str | None = None,
        metadata: Mapping[str, object] | None = None,
        caller_signature: InvocationSignature | None = None,
    ) -> AbilityCallRequest:
        """Build a direct child ability request with an inherited causal anchor."""

        return AbilityCallRequest(
            caller_ura=_required_string(self.caller_ura, "caller_ura"),
            callee_ura=_required_string(callee_ura, "callee_ura"),
            subject_ura=_required_string(subject_ura, "subject_ura"),
            nonce_base64=_required_string(self.nonce_base64, "nonce_base64"),
            causal_context=_required_mapping(self.causal_context, "causal_context"),
            descriptor_ref=descriptor_ref,
            ability_ura=ability_ura,
            ability_name=ability_name,
            descriptor_version=descriptor_version,
            content_type=content_type,
            args={} if args is None else args,
            arguments_base64=arguments_base64,
            metadata=_merged_metadata(self.metadata, metadata),
            caller_signature=caller_signature,
        )

    def build_target_invocation(self, **kwargs: object) -> InvocationDraft:
        return self.invoker.build_target_invocation(self.target_request(**kwargs))

    def invoke_target(self, **kwargs: object) -> InvocationResult:
        return self.invoker.invoke_target(self.target_request(**kwargs))

    def stream_target(self, **kwargs: object) -> StreamHandle:
        return self.invoker.stream_target(self.target_request(**kwargs))

    def build_invocation(self, **kwargs: object) -> InvocationDraft:
        return self.invoker.build_invocation(self.call_request(**kwargs))

    def invoke(self, **kwargs: object) -> InvocationResult:
        return self.invoker.invoke(self.call_request(**kwargs))

    def stream(self, **kwargs: object) -> StreamHandle:
        return self.invoker.stream(self.call_request(**kwargs))


@dataclass
class AbilityInvocationClient:
    """Build and dispatch complete ability Invocations through SDK clients."""

    runtime: RuntimeClient
    addressing: AddressingClient
    _closed: bool = False

    def build_invocation(self, request: AbilityCallRequest) -> InvocationDraft:
        """Build a complete `InvocationDraft` without dispatching it."""

        self._require_open()
        descriptor_ref = self._descriptor_ref(request)
        builder = (
            InvocationBuilder()
            .with_caller_ura(_required_string(request.caller_ura, "caller_ura"))
            .with_callee_ura(_required_string(request.callee_ura, "callee_ura"))
            .with_descriptor_ref(descriptor_ref)
            .with_subject_ura(_required_string(request.subject_ura, "subject_ura"))
            .with_nonce_base64(_required_string(request.nonce_base64, "nonce_base64"))
            .with_causal_context(
                _required_mapping(request.causal_context, "causal_context")
            )
            .with_content_type(_required_string(request.content_type, "content_type"))
            .with_metadata(dict(request.metadata))
        )
        if request.arguments_base64 is None:
            builder.with_json_args(request.args)
        else:
            builder.with_arguments_base64(
                _required_string(request.arguments_base64, "arguments_base64")
            )
        if request.caller_signature is not None:
            builder.with_caller_signature(request.caller_signature)
        return builder.build()

    def resolve_target(self, request: AbilityTargetRequest) -> ResolvedAbilityTarget:
        """Resolve a generic ability target through daemon/Axon identity helpers."""

        self._require_open()
        selector = self._target_selector(request)
        version = _required_string(request.descriptor_version, "descriptor_version")
        if selector == "descriptor_ref":
            projection = self.addressing.project_descriptor_ref(
                DescriptorRefRequest(
                    _required_string(request.descriptor_ref, "descriptor_ref")
                )
            )
            descriptor_ref = projection.descriptor_ref
            ability_ura = projection.ability_ura
            version = projection.descriptor_version
        elif selector == "ability_ura":
            ability_ura = _required_string(request.ability_ura, "ability_ura")
            descriptor_ref = self.addressing.canonical_ability_descriptor_ref(
                ability_ura,
                version,
            )
        else:
            ability_ura = self.addressing.owner_ability_ura(
                _required_string(request.owner_ura, "owner_ura"),
                _required_string(request.ability_name, "ability_name"),
            )
            descriptor_ref = self.addressing.canonical_ability_descriptor_ref(
                ability_ura,
                version,
            )
        address = self.addressing.ability_address(ability_ura)
        subject_ura = request.subject_ura or address.subject_ura
        return ResolvedAbilityTarget(
            ability_ura=ability_ura,
            descriptor_ref=descriptor_ref,
            callee_ura=address.owner_ura,
            subject_ura=_required_string(subject_ura, "subject_ura"),
            descriptor_version=version,
        )

    def build_target_invocation(
        self, request: AbilityTargetRequest
    ) -> InvocationDraft:
        """Resolve an ability target and build a complete Invocation draft."""

        target = self.resolve_target(request)
        builder = (
            InvocationBuilder()
            .with_caller_ura(_required_string(request.caller_ura, "caller_ura"))
            .with_callee_ura(target.callee_ura)
            .with_descriptor_ref(target.descriptor_ref)
            .with_subject_ura(target.subject_ura)
            .with_nonce_base64(_required_string(request.nonce_base64, "nonce_base64"))
            .with_causal_context(
                _required_mapping(request.causal_context, "causal_context")
            )
            .with_content_type(_required_string(request.content_type, "content_type"))
            .with_metadata(dict(request.metadata))
        )
        if request.arguments_base64 is None:
            builder.with_json_args(request.args)
        else:
            builder.with_arguments_base64(
                _required_string(request.arguments_base64, "arguments_base64")
            )
        if request.caller_signature is not None:
            builder.with_caller_signature(request.caller_signature)
        return builder.build()

    def invoke_target(self, request: AbilityTargetRequest) -> InvocationResult:
        """Resolve and submit one unary ability Invocation."""

        return self._require_open().invoke(self.build_target_invocation(request))

    def stream_target(self, request: AbilityTargetRequest) -> StreamHandle:
        """Resolve and open one server-stream ability Invocation."""

        return self._require_open().invoke_stream(
            self.build_target_invocation(request)
        )

    def bidi_target(
        self,
        request: AbilityTargetRequest,
        streams: tuple[BidiStreamDescriptor, ...] = (),
    ) -> BidiSession:
        """Resolve and open one bidirectional ability Invocation."""

        return self._require_open().open_bidi(
            self.build_target_invocation(request), streams
        )

    def prepare_target(
        self,
        request: AbilityTargetRequest,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        """Resolve and prepare one target ability Invocation for signing."""

        return self._require_open().prepare(
            self.build_target_invocation(request), options
        )

    def prepare_and_sign_target(
        self,
        request: AbilityTargetRequest,
        signer: Signer,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[SignedInvocation, SigningMaterial]:
        """Resolve, prepare, and sign one target ability Invocation."""

        return self._require_open().prepare_and_sign(
            self.build_target_invocation(request), signer, options
        )

    def child_context(
        self,
        parent: InvocationResult,
        receipts: ReceiptClient,
        *,
        caller_ura: str,
        nonce_base64: str,
        metadata: Mapping[str, object] | None = None,
    ) -> AbilityChildContext:
        """Create a child-call context from a parent Invocation receipt."""

        self._require_open()
        if receipts is None:
            raise _invalid_ability_invocation("receipt client is required")
        causal_context = receipts.causal_context_from_invocation_result(parent)
        return AbilityChildContext(
            invoker=self,
            caller_ura=_required_string(caller_ura, "caller_ura"),
            nonce_base64=_required_string(nonce_base64, "nonce_base64"),
            causal_context=causal_context,
            metadata=dict(metadata or {}),
        )

    def invoke(self, request: AbilityCallRequest) -> InvocationResult:
        """Build and submit one unary ability Invocation."""

        return self._require_open().invoke(self.build_invocation(request))

    def stream(self, request: AbilityCallRequest) -> StreamHandle:
        """Build and open one server-stream ability Invocation."""

        return self._require_open().invoke_stream(self.build_invocation(request))

    def bidi(
        self,
        request: AbilityCallRequest,
        streams: tuple[BidiStreamDescriptor, ...] = (),
    ) -> BidiSession:
        """Build and open one bidirectional ability Invocation."""

        return self._require_open().open_bidi(self.build_invocation(request), streams)

    def prepare(
        self,
        request: AbilityCallRequest,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        """Build and prepare one ability Invocation for signing."""

        return self._require_open().prepare(self.build_invocation(request), options)

    def prepare_and_sign(
        self,
        request: AbilityCallRequest,
        signer: Signer,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[SignedInvocation, SigningMaterial]:
        """Build, prepare, and sign one ability Invocation."""

        return self._require_open().prepare_and_sign(
            self.build_invocation(request), signer, options
        )

    def submit_signed(self, signed: SignedInvocation) -> InvocationHandle:
        """Submit a signed Invocation and return its observation handle."""

        return self._require_open().submit_signed(signed)

    def await_result(self, handle: InvocationHandle) -> InvocationResult:
        """Await a submitted Invocation handle."""

        return self._require_open().await_result(handle)

    def cancel(self, handle: InvocationHandle, reason: str = "") -> InvocationCancel:
        """Cancel a submitted Invocation handle."""

        return self._require_open().cancel(handle, reason)

    def events(self, handle: InvocationHandle) -> InvocationHandle:
        """Fetch the latest submitted Invocation handle events."""

        return self._require_open().events(handle)

    def close_handle(self, handle: InvocationHandle) -> None:
        """Release a submitted Invocation handle."""

        self._require_open().close_handle(handle)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        first_error: SDKError | None = None
        for owned in (self.runtime, self.addressing):
            try:
                owned.close()
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
            except Exception as exc:
                if first_error is None:
                    first_error = SDKError(
                        code=ErrorCode.ROUTE_UNAVAILABLE,
                        stage="ability_invocation",
                        retry=RetryHint.SAFE,
                        retryable=True,
                        message="ability invocation client close failed",
                        cause=exc,
                    )
        if first_error is not None:
            raise first_error

    def __enter__(self) -> "AbilityInvocationClient":
        self._require_open()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _descriptor_ref(self, request: AbilityCallRequest) -> str:
        selectors = tuple(
            value.strip()
            for value in (
                request.descriptor_ref,
                request.ability_ura,
                request.ability_name,
            )
            if value.strip()
        )
        if len(selectors) != 1:
            raise _invalid_ability_invocation(
                "exactly one of descriptor_ref, ability_ura, or ability_name is required"
            )
        if request.descriptor_ref.strip():
            return self.addressing.canonical_ability_descriptor_ref(
                request.descriptor_ref
            )
        version = _required_string(request.descriptor_version, "descriptor_version")
        if request.ability_ura.strip():
            return self.addressing.canonical_ability_descriptor_ref(
                request.ability_ura,
                version,
            )
        return self.addressing.owner_ability_descriptor_ref(
            _required_string(request.callee_ura, "callee_ura"),
            _required_string(request.ability_name, "ability_name"),
            version,
        )

    def _target_selector(self, request: AbilityTargetRequest) -> str:
        descriptor_ref = _selector_string(request.descriptor_ref, "descriptor_ref")
        ability_ura = _selector_string(request.ability_ura, "ability_ura")
        owner_ura = _selector_string(request.owner_ura, "owner_ura")
        ability_name = _selector_string(request.ability_name, "ability_name")
        selector_count = sum(
            (
                bool(descriptor_ref),
                bool(ability_ura),
                bool(owner_ura or ability_name),
            )
        )
        if selector_count != 1:
            raise _invalid_ability_invocation(
                "exactly one of descriptor_ref, ability_ura, or owner_ura plus "
                "ability_name is required"
            )
        if bool(owner_ura) != bool(ability_name):
            raise _invalid_ability_invocation(
                "owner_ura and ability_name must be provided together"
            )
        if descriptor_ref:
            return "descriptor_ref"
        if ability_ura:
            return "ability_ura"
        return "owner_ability"

    def _require_open(self) -> RuntimeClient:
        if self._closed:
            raise SDKError(
                code=ErrorCode.CANCELLED,
                stage="ability_invocation",
                retry=RetryHint.NEVER,
                retryable=False,
                message="ability invocation client is closed",
            )
        return self.runtime


@dataclass(frozen=True)
class InvocationObjectAdapter:
    """Adapt invocation tuple-like objects into SDK Invocation DTOs.

    The adapter reads a host application's seven-tuple object shape. Descriptor
    reference and owner Ability URA construction still go through
    `AbilityInvocationClient` and its `AddressingClient`.
    """

    invoker: AbilityInvocationClient
    descriptor_version: str = "1.0.0"

    def build_invocation(
        self,
        tuple_: object,
        *,
        metadata: Mapping[str, object] | None = None,
        caller_signature: InvocationSignature | Mapping[str, object] | object | None = None,
        descriptor_version: str = "",
    ) -> InvocationDraft:
        """Build a complete SDK `InvocationDraft` from an invocation object."""

        version = descriptor_version or self.descriptor_version
        direct_request = self._request_from_tuple(
            tuple_,
            metadata=metadata,
            caller_signature=caller_signature,
            descriptor_version=version,
            selector="descriptor_ref",
        )
        try:
            return self.invoker.build_invocation(direct_request)
        except SDKError as exc:
            if not _is_descriptor_selector_rejection(exc):
                raise
        return self.invoker.build_invocation(
            self._request_from_tuple(
                tuple_,
                metadata=metadata,
                caller_signature=caller_signature,
                descriptor_version=version,
                selector="ability_name",
            )
        )

    def to_wire_dict(
        self,
        tuple_: object,
        *,
        metadata: Mapping[str, object] | None = None,
        caller_signature: InvocationSignature | Mapping[str, object] | object | None = None,
        bidi_streams: Sequence[Mapping[str, object] | BidiStreamDescriptor | object] | None = None,
        descriptor_version: str = "",
    ) -> dict[str, object]:
        """Return the daemon Invocation JSON DTO."""

        value = self.build_invocation(
            tuple_,
            metadata=metadata,
            caller_signature=caller_signature,
            descriptor_version=descriptor_version,
        ).to_json_dict()
        if bidi_streams:
            value["bidi_streams"] = [
                _stream_descriptor_dict(stream) for stream in bidi_streams
            ]
        return value

    def invoke(
        self,
        tuple_: object,
        *,
        metadata: Mapping[str, object] | None = None,
        caller_signature: InvocationSignature | Mapping[str, object] | object | None = None,
        descriptor_version: str = "",
    ) -> InvocationResult:
        """Build and submit one invocation object."""

        return self.invoker.runtime.invoke(
            self.build_invocation(
                tuple_,
                metadata=metadata,
                caller_signature=caller_signature,
                descriptor_version=descriptor_version,
            )
        )

    def stream(
        self,
        tuple_: object,
        *,
        metadata: Mapping[str, object] | None = None,
        caller_signature: InvocationSignature | Mapping[str, object] | object | None = None,
        descriptor_version: str = "",
    ) -> StreamHandle:
        """Build and open one invocation object as a server stream."""

        return self.invoker.runtime.invoke_stream(
            self.build_invocation(
                tuple_,
                metadata=metadata,
                caller_signature=caller_signature,
                descriptor_version=descriptor_version,
            )
        )

    def bidi(
        self,
        tuple_: object,
        *,
        streams: Sequence[Mapping[str, object] | BidiStreamDescriptor | object] = (),
        metadata: Mapping[str, object] | None = None,
        caller_signature: InvocationSignature | Mapping[str, object] | object | None = None,
        descriptor_version: str = "",
    ) -> BidiSession:
        """Build and open one invocation object as a bidirectional session."""

        return self.invoker.runtime.open_bidi(
            self.build_invocation(
                tuple_,
                metadata=metadata,
                caller_signature=caller_signature,
                descriptor_version=descriptor_version,
            ),
            tuple(_coerce_bidi_stream_descriptor(stream) for stream in streams),
        )

    def _request_from_tuple(
        self,
        tuple_: object,
        *,
        metadata: Mapping[str, object] | None,
        caller_signature: InvocationSignature | Mapping[str, object] | object | None,
        descriptor_version: str,
        selector: str,
    ) -> AbilityCallRequest:
        arguments = _tuple_value(tuple_, "arguments")
        argument_kwargs = _argument_kwargs(arguments)
        ability = _required_string(_tuple_value(tuple_, "ability"), "ability")
        selector_kwargs: dict[str, str] = {}
        if selector == "descriptor_ref":
            selector_kwargs["descriptor_ref"] = ability
        elif selector == "ability_name":
            selector_kwargs["ability_name"] = ability
        else:
            raise _invalid_ability_invocation("unknown invocation object selector")
        return AbilityCallRequest(
            caller_ura=_required_string(_tuple_value(tuple_, "caller"), "caller"),
            callee_ura=_required_string(_tuple_value(tuple_, "callee"), "callee"),
            subject_ura=_required_string(_tuple_value(tuple_, "subject"), "subject"),
            nonce_base64=_nonce_base64(_tuple_value(tuple_, "nonce")),
            causal_context=_causal_context(_tuple_value(tuple_, "causal")),
            descriptor_version=_required_string(
                descriptor_version, "descriptor_version"
            ),
            metadata=dict(metadata or {}),
            caller_signature=_coerce_invocation_signature(caller_signature),
            **selector_kwargs,
            **argument_kwargs,
        )


def _required_string(value: object, field_name: str) -> str:
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_ability_invocation(f"{field_name} is required")
    if value.strip() != value:
        raise _invalid_ability_invocation(
            f"{field_name} must not contain surrounding whitespace"
        )
    return value


def _selector_string(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise _invalid_ability_invocation(f"{field_name} must be a string")
    if value.strip() != value:
        raise _invalid_ability_invocation(
            f"{field_name} must not contain surrounding whitespace"
        )
    return value


def _required_mapping(
    value: Mapping[str, object] | object, field_name: str
) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_ability_invocation(f"{field_name} must be an object")
    return dict(value)


def _merged_metadata(
    base: Mapping[str, object],
    extra: Mapping[str, object] | None,
) -> Mapping[str, object]:
    value = dict(base)
    if extra:
        value.update(dict(extra))
    return value


def _invalid_ability_invocation(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="ability_invocation",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _is_descriptor_selector_rejection(error: SDKError) -> bool:
    return is_code(error, ErrorCode.INVALID_ARGUMENT) and error.stage in {
        "directory_identity",
        "identity",
    }


def _tuple_value(tuple_: object, field_name: str) -> object:
    if isinstance(tuple_, Mapping):
        if field_name not in tuple_:
            raise _invalid_ability_invocation(f"{field_name} is required")
        return tuple_[field_name]
    if not hasattr(tuple_, field_name):
        raise _invalid_ability_invocation(f"{field_name} is required")
    return getattr(tuple_, field_name)


def _nonce_base64(value: object) -> str:
    if isinstance(value, str):
        return _required_string(value, "nonce_base64")
    if not isinstance(value, (bytes, bytearray)):
        raise _invalid_ability_invocation("nonce must be bytes or nonce_base64 string")
    nonce = bytes(value)
    if len(nonce) != 16:
        raise _invalid_ability_invocation("nonce must be 16 bytes")
    if not any(nonce):
        raise _invalid_ability_invocation("nonce must not be all-zero")
    return base64.b64encode(nonce).decode("ascii")


def _argument_kwargs(arguments: object) -> dict[str, object]:
    if isinstance(arguments, Mapping):
        if "arguments_base64" in arguments:
            return {
                "arguments_base64": _required_string(
                    arguments.get("arguments_base64"), "arguments_base64"
                ),
                "content_type": _required_string(
                    arguments.get("content_type"), "content_type"
                ),
            }
        if "args" in arguments:
            return {
                "args": arguments["args"],
                "content_type": _optional_content_type(arguments.get("content_type")),
            }
        raise _invalid_ability_invocation(
            "arguments mapping must contain args or arguments_base64"
        )
    raw = getattr(arguments, "raw", None)
    is_json = getattr(arguments, "is_json", raw is None)
    if is_json:
        return {
            "args": getattr(arguments, "json_value", None),
            "content_type": _optional_content_type(
                getattr(arguments, "content_type", None)
            ),
        }
    if raw is None:
        raw = b""
    if not isinstance(raw, (bytes, bytearray)):
        raise _invalid_ability_invocation("binary arguments must be bytes")
    return {
        "arguments_base64": base64.b64encode(bytes(raw)).decode("ascii"),
        "content_type": _required_string(
            getattr(arguments, "content_type", None), "content_type"
        ),
    }


def _optional_content_type(value: object) -> str:
    if value is None or value == "":
        return "application/json"
    return _required_string(value, "content_type")


def _causal_context(value: object) -> Mapping[str, object]:
    if value is None:
        return {"form": "none"}
    if isinstance(value, Mapping):
        return dict(value)
    if _has_callable(value, "to_wire"):
        return {"form": "scalar", **_object_wire_dict(value)}
    if hasattr(value, "root") and hasattr(value, "proof_ura"):
        root = getattr(value, "root")
        if not isinstance(root, (bytes, bytearray)) or len(root) != 32:
            raise _invalid_ability_invocation("merkle root must be 32 bytes")
        return {
            "form": "merkle",
            "root_hex": bytes(root).hex(),
            "proof_ura": _required_string(getattr(value, "proof_ura"), "proof_ura"),
        }
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        refs = list(value)
        if not refs:
            raise _invalid_ability_invocation(
                "causal list must not be empty; use None for a root invocation"
            )
        return {
            "form": "list",
            "prior": [_object_wire_dict(ref) for ref in refs],
        }
    raise _invalid_ability_invocation("unsupported causal context")


def _coerce_invocation_signature(
    value: InvocationSignature | Mapping[str, object] | object | None,
) -> InvocationSignature | None:
    if value is None:
        return None
    if isinstance(value, InvocationSignature):
        return value
    wire = dict(value) if isinstance(value, Mapping) else _object_wire_dict(value)
    return InvocationSignature(
        algorithm=_required_string(wire.get("algorithm"), "caller_signature.algorithm"),
        signature_base64=_required_string(
            wire.get("signature_base64"), "caller_signature.signature_base64"
        ),
        key_id_hint=_optional_wire_string(wire.get("key_id_hint")),
        signer_public_key_base64=_optional_wire_string(
            wire.get("signer_public_key_base64")
        ),
    )


def _coerce_bidi_stream_descriptor(
    value: Mapping[str, object] | BidiStreamDescriptor | object,
) -> BidiStreamDescriptor:
    if isinstance(value, BidiStreamDescriptor):
        return value
    wire = dict(value) if isinstance(value, Mapping) else _object_wire_dict(value)
    stream_id = wire.get("stream_id")
    if not isinstance(stream_id, int):
        raise _invalid_ability_invocation("stream_id must be an integer")
    return BidiStreamDescriptor(
        stream_id=stream_id,
        content_type=_required_string(wire.get("content_type"), "content_type"),
        ordering=_required_string(wire.get("ordering", "STRICT"), "ordering"),
        codec_params=_optional_wire_string(wire.get("codec_params")) or "",
    )


def _stream_descriptor_dict(
    value: Mapping[str, object] | BidiStreamDescriptor | object,
) -> dict[str, object]:
    descriptor = _coerce_bidi_stream_descriptor(value)
    result: dict[str, object] = {
        "stream_id": descriptor.stream_id,
        "content_type": descriptor.content_type,
        "ordering": descriptor.ordering,
    }
    if descriptor.codec_params:
        result["codec_params"] = descriptor.codec_params
    return result


def _object_wire_dict(value: object) -> dict[str, object]:
    if not _has_callable(value, "to_wire"):
        raise _invalid_ability_invocation("object does not expose to_wire")
    wire = value.to_wire()
    if not isinstance(wire, Mapping):
        raise _invalid_ability_invocation("to_wire must return a mapping")
    return dict(wire)


def _has_callable(value: object, name: str) -> bool:
    return callable(getattr(value, name, None))


def _optional_wire_string(value: object) -> str | None:
    if value is None:
        return None
    if value == "":
        return ""
    return _required_string(value, "optional string")
