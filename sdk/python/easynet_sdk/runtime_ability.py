"""Generic lowering from addressed runtime abilities to Runtime Core."""

from __future__ import annotations

import base64
import binascii
from dataclasses import dataclass, field
from typing import Mapping

from .axon_addressing import AddressingClient
from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationBuilder, InvocationDraft
from .runtime import InvocationResult, RuntimeClient
from .stream import StreamHandle

__all__ = ["RuntimeAbilityClient", "RuntimeCallContext"]


@dataclass(frozen=True)
class RuntimeCallContext:
    """Complete caller-controlled context for one runtime ability call."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    descriptor_version: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


class RuntimeAbilityClient:
    """Single generic Addressing-to-Invocation lowering path."""

    def __init__(self, runtime: RuntimeClient, addressing: AddressingClient) -> None:
        if runtime is None:
            raise _invalid("runtime client is required")
        if addressing is None:
            raise _invalid("Addressing provider is required")
        self._runtime = runtime
        self._addressing = addressing

    def build(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> InvocationDraft:
        _validate_call(call)
        ability_name = _required_text(ability_name, "ability name")
        version = call.descriptor_version.strip() or "1.0.0"
        descriptor_ref = self._addressing.owner_ability_descriptor_ref(
            call.callee_ura.strip(), ability_name, version
        )
        subject = self._addressing.parse_ura(call.subject_ura.strip())
        if subject.kind in {"user", "hub"}:
            subject_ura = self._addressing.descriptor_bound_resource_subject_ura(
                subject.ura, f"invoke/{ability_name}"
            )
        elif subject.kind in {"agent", "ability", "device", "resource"}:
            subject_ura = subject.ura
        else:
            raise _invalid(f"subject kind {subject.kind!r} is not descriptor-bound")
        return (
            InvocationBuilder()
            .with_caller_ura(call.caller_ura.strip())
            .with_callee_ura(call.callee_ura.strip())
            .with_descriptor_ref(descriptor_ref)
            .with_subject_ura(subject_ura)
            .with_nonce_base64(call.nonce_base64.strip())
            .with_causal_context(dict(call.causal_context))
            .with_json_args(arguments)
            .with_content_type("application/json")
            .with_metadata(dict(call.metadata))
            .build()
        )

    def invoke(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]:
        result = self._runtime.invoke(self.build(call, ability_name, arguments))
        if not result.ok:
            raise _invocation_failure(result)
        if not isinstance(result.output_json, Mapping):
            raise _invalid("runtime ability output_json must be an object")
        return dict(result.output_json)

    def open_stream(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> StreamHandle:
        return self._runtime.invoke_stream(self.build(call, ability_name, arguments))


def _validate_call(call: RuntimeCallContext) -> None:
    if not isinstance(call, RuntimeCallContext):
        raise _invalid("runtime call context is required")
    _required_text(call.caller_ura, "caller_ura")
    _required_text(call.callee_ura, "callee_ura")
    _required_text(call.subject_ura, "subject_ura")
    nonce = _required_text(call.nonce_base64, "nonce_base64")
    try:
        if not base64.b64decode(nonce, validate=True):
            raise ValueError("empty nonce")
    except (ValueError, binascii.Error) as error:
        raise _invalid("nonce_base64 must be canonical base64", error) from error
    if not isinstance(call.causal_context, Mapping):
        raise _invalid("causal_context is required")


def _required_text(value: object, field_name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid(f"{field_name} is required")
    return value.strip()


def _invocation_failure(result: InvocationResult) -> SDKError:
    failure = result.error
    code = ErrorCode.EXECUTION_FAILED
    if failure and failure.code:
        try:
            code = ErrorCode(failure.code)
        except ValueError:
            pass
    return SDKError(
        code=code,
        stage=failure.stage if failure and failure.stage else "runtime",
        retry=RetryHint.SAFE if failure and failure.retryable else RetryHint.NEVER,
        retryable=bool(failure and failure.retryable),
        message=(
            failure.message
            if failure and failure.message
            else "runtime ability invocation failed"
        ),
        details={
            "terminal_state": result.terminal_state,
            **({"runtime_code": failure.code} if failure and failure.code else {}),
        },
    )


def _invalid(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_ability",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )
