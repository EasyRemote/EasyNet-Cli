"""Generic lowering from addressed runtime abilities to Runtime Core."""

from __future__ import annotations

import base64
import binascii
from dataclasses import dataclass, field
from typing import Mapping, TypeAlias

from .axon_addressing import AddressingClient, AddressingProjection
from .authority import (
    DELEGATION_METADATA_KEY,
    SESSION_AUTHORITY_METADATA_KEY,
    DelegationProof,
    SessionAuthority,
    validate_authority_metadata,
)
from .bidi import BidiSession, BidiStreamDescriptor
from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationBuilder, InvocationDraft
from .runtime import (
    InvocationCancel,
    InvocationHandle,
    InvocationResult,
    RuntimeRecoveryReport,
    RuntimeRecoveryRequest,
    RuntimeClient,
)
from .signing import SignedInvocation
from .stream import StreamHandle

__all__ = [
    "RuntimeAbilityClient",
    "RuntimeCallContext",
    "RuntimeInvocationAuthority",
]


RuntimeInvocationAuthority: TypeAlias = DelegationProof | SessionAuthority


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
    authority: RuntimeInvocationAuthority | None = None


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
        return self._build(call, ability_name, arguments, call_mode="rpc")

    def _build(
        self,
        call: RuntimeCallContext,
        ability_name: str,
        arguments: object,
        *,
        call_mode: str,
    ) -> InvocationDraft:
        _validate_call(call)
        ability_name = _required_text(ability_name, "ability name")
        subject = self._addressing.parse_ura(call.subject_ura.strip())
        if subject.kind in {"user", "hub"}:
            subject_ura = self._addressing.descriptor_bound_resource_subject_ura(
                subject.ura, f"invoke/{ability_name}"
            )
        elif subject.kind in {"agent", "ability", "device", "resource"}:
            subject_ura = subject.ura
        else:
            raise _invalid(f"subject kind {subject.kind!r} is not descriptor-bound")
        metadata = _canonical_runtime_call_metadata(
            call,
            subject_ura,
            self._addressing.parse_ura(subject_ura),
            ability_name,
        )
        descriptor_ref = self._runtime.resolve_descriptor_ref(
            callee_ura=call.callee_ura.strip(),
            ability=ability_name,
            call_mode=call_mode,
            caller_ura=call.caller_ura.strip(),
            subject_ura=call.subject_ura.strip(),
        )
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
            .with_metadata(metadata)
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
        return self._runtime.invoke_stream(
            self._build(call, ability_name, arguments, call_mode="stream")
        )

    def open_bidi(
        self,
        call: RuntimeCallContext,
        ability_name: str,
        arguments: object,
        streams: tuple[BidiStreamDescriptor, ...],
    ) -> BidiSession:
        return self._runtime.open_bidi(
            self._build(call, ability_name, arguments, call_mode="bidi"),
            streams,
        )

    def submit_signed(self, signed: SignedInvocation) -> InvocationHandle:
        return self._runtime.submit_signed(signed)

    def recover(self, request: RuntimeRecoveryRequest) -> RuntimeRecoveryReport:
        return self._runtime.recover(request)

    def await_result(self, handle: InvocationHandle) -> InvocationResult:
        return self._runtime.await_result(handle)

    def cancel(self, handle: InvocationHandle, reason: str = "") -> InvocationCancel:
        return self._runtime.cancel(handle, reason)

    def events(self, handle: InvocationHandle) -> InvocationHandle:
        return self._runtime.events(handle)

    def close_handle(self, handle: InvocationHandle) -> None:
        self._runtime.close_handle(handle)


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


def _canonical_runtime_call_metadata(
    call: RuntimeCallContext,
    envelope_subject_ura: str,
    envelope_subject: AddressingProjection,
    ability_name: str,
) -> dict[str, object]:
    metadata = dict(call.metadata)
    validate_authority_metadata(metadata)
    raw_authority_present = bool(
        metadata.get(DELEGATION_METADATA_KEY)
        or metadata.get(SESSION_AUTHORITY_METADATA_KEY)
    )
    authority = call.authority
    if authority is not None:
        if raw_authority_present:
            raise _invalid(
                "runtime call authority must be supplied once as a typed authority or metadata, not both"
            )
        metadata = authority.metadata().merge_into(metadata)
    else:
        authority = _runtime_authority_from_metadata(metadata)
    if authority is not None:
        _validate_runtime_authority_binding(
            authority,
            call,
            envelope_subject_ura,
            envelope_subject,
            ability_name,
        )
    return metadata


def _runtime_authority_from_metadata(
    metadata: Mapping[str, object],
) -> RuntimeInvocationAuthority | None:
    delegation = metadata.get(DELEGATION_METADATA_KEY)
    if isinstance(delegation, str) and delegation.strip():
        return DelegationProof.from_metadata(delegation)
    session = metadata.get(SESSION_AUTHORITY_METADATA_KEY)
    if isinstance(session, str) and session.strip():
        return SessionAuthority.from_metadata(session)
    return None


def _validate_runtime_authority_binding(
    authority: RuntimeInvocationAuthority,
    call: RuntimeCallContext,
    envelope_subject_ura: str,
    envelope_subject: AddressingProjection,
    ability_name: str,
) -> None:
    caller_ura = call.caller_ura.strip()
    callee_ura = call.callee_ura.strip()
    if isinstance(authority, DelegationProof):
        if authority.caller_ura.strip() != caller_ura:
            raise _invalid("runtime delegation caller does not match caller_ura")
        if authority.subject_ura.strip() != envelope_subject_ura:
            raise _invalid(
                "runtime delegation subject does not match descriptor-bound subject_ura"
            )
        if not authority.matches_audience(callee_ura):
            raise _invalid("runtime delegation audience does not admit callee_ura")
        if not authority.matches_scope(ability_name):
            raise _invalid("runtime delegation scopes do not admit ability")
        return
    if authority.issuer_ura.strip() != caller_ura:
        raise _invalid("runtime session authority issuer does not match caller_ura")
    if authority.callee_ura.strip() != callee_ura:
        raise _invalid("runtime session authority callee does not match callee_ura")
    if not authority.matches_audience(callee_ura):
        raise _invalid("runtime session authority audience does not admit callee_ura")
    if not _session_authority_admits_subject(
        authority,
        envelope_subject_ura,
        envelope_subject,
    ):
        raise _invalid(
            "runtime session authority does not admit descriptor-bound subject_ura"
        )
    if not authority.matches_scope(ability_name):
        raise _invalid("runtime session authority scopes do not admit ability")


def _session_authority_admits_subject(
    authority: SessionAuthority,
    subject_ura: str,
    subject: AddressingProjection,
) -> bool:
    if authority.subject_ura.strip() == subject_ura.strip():
        return True
    if subject.kind != "resource":
        return False
    owner_id = subject.components.get("owner_id")
    if not isinstance(owner_id, str):
        return False
    owner_user_id = authority.session_owner_user_id.strip()
    if owner_id == f"user.{owner_user_id}":
        return True
    if not owner_id.startswith("agent."):
        return False
    agent_owner = owner_id.removeprefix("agent.").split(".", 1)
    return len(agent_owner) == 2 and agent_owner[0] == owner_user_id


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
