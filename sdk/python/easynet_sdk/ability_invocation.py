"""Ability Invocation convenience facade over Runtime Core."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Mapping

from .bidi import BidiSession, BidiStreamDescriptor
from .errors import ErrorCode, RetryHint, SDKError
from .identity import AddressingClient
from .invocation import InvocationBuilder, InvocationDraft, InvocationSignature
from .runtime import InvocationResult, RuntimeClient
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
                        code=ErrorCode.TRANSPORT,
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


def _required_string(value: object, field_name: str) -> str:
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_ability_invocation(f"{field_name} is required")
    return value


def _required_mapping(
    value: Mapping[str, object] | object, field_name: str
) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_ability_invocation(f"{field_name} must be an object")
    return dict(value)


def _invalid_ability_invocation(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="ability_invocation",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
