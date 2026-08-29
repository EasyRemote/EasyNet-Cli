"""Generic lowering from addressed runtime abilities to Runtime Core."""

from __future__ import annotations

import base64
import binascii
from dataclasses import dataclass, field, replace
from typing import Callable, Mapping, TypeAlias

from ._identity_guards import contains_all_zero_principal
from .axon_addressing import (
    AddressingClient,
    AddressingProjection,
)
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
from ._runtime_governance import (
    ABILITY_DESCRIPTOR_PROVIDER,
    RECEIPT_HISTORY_PROVIDER,
    is_runtime_governance_read_ability,
)
from ._runtime_subjects import runtime_governance_read_subject_ura
from .runtime import (
    InvocationCancel,
    InvocationHandle,
    InvocationResult,
    RuntimeRecoveryReport,
    RuntimeRecoveryRequest,
    RuntimeClient,
    _runtime_catalogue_read_target,
)
from .runtime_authority import DraftAuthorityProvider
from ._session_authority_subjects import session_authority_admits_subject
from .signing import SignedInvocation
from .stream import LeasedStreamHandle, StreamHandle

__all__ = [
    "RuntimeAbilityClient",
    "RuntimeCallContext",
    "RuntimeInvocationAuthority",
]


RuntimeInvocationAuthority: TypeAlias = DelegationProof | SessionAuthority


@dataclass(frozen=True)
class _RuntimeAbilityDispatchPolicy:
    allow_governance_read: bool = False
    subject_policy: str = "descriptor_bound"
    descriptor_provider: str = ""


_PUBLIC_ACTION_POLICY = _RuntimeAbilityDispatchPolicy()
_GOVERNANCE_READ_POLICY = _RuntimeAbilityDispatchPolicy(
    allow_governance_read=True,
    descriptor_provider=RECEIPT_HISTORY_PROVIDER,
)
_CATALOGUE_READ_POLICY = _RuntimeAbilityDispatchPolicy(
    allow_governance_read=True,
    subject_policy="runtime_governance_read",
    descriptor_provider=ABILITY_DESCRIPTOR_PROVIDER,
)


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


@dataclass(frozen=True)
class _RuntimeAbilityProjection:
    descriptor_ref: str
    ability_ura: str
    public_name: str
    action: str

    @classmethod
    def from_descriptor_ref(
        cls,
        addressing: AddressingClient,
        callee_ura: str,
        descriptor_ref: str,
    ) -> "_RuntimeAbilityProjection":
        try:
            projection = addressing.project_descriptor_ref(descriptor_ref)
        except SDKError as exc:
            raise _invalid(
                "descriptor_ref must contain a canonical Ability URA",
                exc,
            ) from exc
        ability_ura = projection.ability_ura.strip()
        canonical_ref = projection.descriptor_ref.strip()
        if not ability_ura or not canonical_ref:
            raise _invalid("descriptor_ref must contain a canonical Ability URA")
        action = projection.action.strip() or "invoke"
        try:
            ability = addressing.project_ability_ura(ability_ura)
        except SDKError as exc:
            raise _invalid(
                "descriptor_ref must contain a canonical Ability URA",
                exc,
            ) from exc
        public_name = ""
        if ability.owner_ura.strip() == callee_ura.strip():
            public_name = ability.public_name.strip()
        return cls(
            descriptor_ref=canonical_ref,
            ability_ura=ability_ura,
            public_name=public_name,
            action=action,
        )

    def matches_scope(self, matcher: Callable[[str], bool]) -> bool:
        seen: set[str] = set()
        for candidate in (self.public_name, self.ability_ura):
            candidate = candidate.strip()
            if not candidate or candidate in seen:
                continue
            seen.add(candidate)
            if matcher(candidate):
                return True
        return False


class RuntimeAbilityClient:
    """Single generic Addressing-to-Invocation lowering path."""

    def __init__(
        self,
        runtime: RuntimeClient,
        addressing: AddressingClient,
        authority: DraftAuthorityProvider | None = None,
    ) -> None:
        if runtime is None:
            raise _invalid("runtime client is required")
        if addressing is None:
            raise _invalid("Addressing provider is required")
        self._runtime = runtime
        self._addressing = addressing
        self._authority = authority

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
        policy: _RuntimeAbilityDispatchPolicy = _PUBLIC_ACTION_POLICY,
    ) -> InvocationDraft:
        _validate_call(call)
        ability_name = _required_text(ability_name, "ability name")
        if not policy.allow_governance_read and is_runtime_governance_read_ability(
            ability_name
        ):
            raise _invalid(
                "runtime governance receipt/history/catalogue abilities must use RuntimeReceiptProvider or RuntimeAbilityDescriptorProvider"
            )
        target = _runtime_catalogue_read_target(
            callee_ura=call.callee_ura,
            subject_ura=call.subject_ura,
            ability=ability_name,
            provider=policy.descriptor_provider,
        )
        if target.callee_ura != call.callee_ura or target.subject_ura != call.subject_ura:
            call = replace(
                call,
                callee_ura=target.callee_ura,
                subject_ura=target.subject_ura,
            )
        subject_ura = self._subject_ura(call, ability_name, policy)
        descriptor_ref = self._runtime.resolve_descriptor_ref(
            callee_ura=call.callee_ura.strip(),
            ability=ability_name,
            call_mode=call_mode,
            caller_ura=call.caller_ura.strip(),
            subject_ura=self._descriptor_resolution_subject_ura(
                call, subject_ura, policy
            ),
            provider=policy.descriptor_provider,
        )
        ability = _RuntimeAbilityProjection.from_descriptor_ref(
            self._addressing,
            call.callee_ura,
            descriptor_ref,
        )
        metadata = _canonical_runtime_call_metadata(
            call,
            subject_ura,
            self._addressing.parse_ura(subject_ura),
            ability,
        )
        draft = (
            InvocationBuilder()
            .with_caller_ura(call.caller_ura.strip())
            .with_callee_ura(call.callee_ura.strip())
            .with_descriptor_ref(ability.descriptor_ref)
            .with_subject_ura(subject_ura)
            .with_nonce_base64(call.nonce_base64.strip())
            .with_causal_context(dict(call.causal_context))
            .with_json_args(arguments)
            .with_content_type("application/json")
            .with_metadata(metadata)
            .build()
        )
        return self._authority.bind(draft) if self._authority is not None else draft

    def _build_governance_read(
        self,
        call: RuntimeCallContext,
        ability_name: str,
        arguments: object,
    ) -> InvocationDraft:
        return self._build(
            call,
            ability_name,
            arguments,
            call_mode="rpc",
            policy=_GOVERNANCE_READ_POLICY,
        )

    def _build_catalogue_read(
        self,
        call: RuntimeCallContext,
        ability_name: str,
        arguments: object,
    ) -> InvocationDraft:
        return self._build(
            call,
            ability_name,
            arguments,
            call_mode="rpc",
            policy=_CATALOGUE_READ_POLICY,
        )

    def invoke(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]:
        return self.invoke_draft(self.build(call, ability_name, arguments))

    def invoke_draft(self, draft: InvocationDraft) -> dict[str, object]:
        """Submit one already-finalized draft without descriptor re-resolution."""

        result = self._runtime.invoke(draft)
        if not result.ok:
            raise _invocation_failure(result)
        return _runtime_ability_object_output(result)

    def _invoke_governance_read(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]:
        result = self._runtime._governance_read(
            self._build_governance_read(call, ability_name, arguments)
        )
        if not result.ok:
            raise _invocation_failure(result)
        return _runtime_ability_object_output(result)

    def _invoke_catalogue_read(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]:
        result = self._runtime.invoke(
            self._build_catalogue_read(call, ability_name, arguments)
        )
        if not result.ok:
            raise _invocation_failure(result)
        return _runtime_ability_object_output(result)

    def _subject_ura(
        self,
        call: RuntimeCallContext,
        ability_name: str,
        policy: _RuntimeAbilityDispatchPolicy,
    ) -> str:
        if policy.subject_policy == "runtime_owner":
            return call.callee_ura.strip()
        if policy.subject_policy == "runtime_governance_read":
            authority = _runtime_session_authority_from_call(call)
            if authority is not None:
                subject = self._addressing.parse_ura(call.subject_ura.strip())
                return runtime_governance_read_subject_ura(
                    self._addressing.user_ura(
                        subject.realm, authority.session_owner_user_id.strip()
                    ),
                    call.callee_ura,
                )
            return runtime_governance_read_subject_ura(
                call.subject_ura,
                call.callee_ura,
            )
        if policy.subject_policy != "descriptor_bound":
            raise _invalid("runtime ability subject policy is unsupported")
        subject = self._addressing.parse_ura(call.subject_ura.strip())
        if subject.kind in {"user", "authority"}:
            return self._addressing.descriptor_bound_resource_subject_ura(
                subject.ura, f"invoke/{ability_name}"
            )
        if subject.kind in {"agent", "ability", "device", "resource"}:
            return subject.ura
        raise _invalid(f"subject kind {subject.kind!r} is not descriptor-bound")

    @staticmethod
    def _descriptor_resolution_subject_ura(
        call: RuntimeCallContext,
        selected_subject_ura: str,
        policy: _RuntimeAbilityDispatchPolicy,
    ) -> str:
        if policy.descriptor_provider == ABILITY_DESCRIPTOR_PROVIDER:
            if policy.subject_policy in {"runtime_owner", "runtime_governance_read"}:
                return selected_subject_ura.strip()
            return call.subject_ura.strip()
        if policy.subject_policy == "runtime_owner":
            return selected_subject_ura.strip()
        return call.subject_ura.strip()

    def open_stream(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> StreamHandle:
        return self._runtime.invoke_stream(
            self._build(call, ability_name, arguments, call_mode="stream")
        )

    def open_leased_stream(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> LeasedStreamHandle:
        """Open an explicit ABI v9 lease stream for high-throughput payloads."""

        return self._runtime.invoke_leased_stream(
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
    _validate_runtime_call_context(call)
    nonce = _required_text(call.nonce_base64, "nonce_base64")
    try:
        decoded = base64.b64decode(nonce, validate=True)
    except (ValueError, binascii.Error) as error:
        raise _invalid("nonce_base64 must be canonical base64", error) from error
    if base64.b64encode(decoded).decode("ascii") != nonce:
        raise _invalid("nonce_base64 must be canonical base64")
    if len(decoded) != 16:
        raise _invalid("nonce_base64 must decode to 16 bytes")


def _validate_runtime_call_context(call: RuntimeCallContext) -> None:
    if not isinstance(call, RuntimeCallContext):
        raise _invalid("runtime call context is required")
    for field_name, value in (
        ("caller_ura", call.caller_ura),
        ("callee_ura", call.callee_ura),
        ("subject_ura", call.subject_ura),
        ("nonce_base64", call.nonce_base64),
    ):
        _required_text(value, field_name)
        if field_name != "nonce_base64" and contains_all_zero_principal(value):
            raise _invalid(f"{field_name} must not be all-zero")
    if not isinstance(call.causal_context, Mapping):
        raise _invalid("causal_context is required")


def _runtime_ability_object_output(result: InvocationResult) -> dict[str, object]:
    if not isinstance(result.output_json, Mapping):
        raise _invalid("runtime ability output_json must be an object")
    return dict(result.output_json)


def _canonical_runtime_call_metadata(
    call: RuntimeCallContext,
    envelope_subject_ura: str,
    envelope_subject: AddressingProjection,
    ability: _RuntimeAbilityProjection,
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
            ability,
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


def _runtime_session_authority_from_call(
    call: RuntimeCallContext,
) -> SessionAuthority | None:
    if isinstance(call.authority, SessionAuthority):
        return call.authority
    if call.authority is not None:
        return None
    session = call.metadata.get(SESSION_AUTHORITY_METADATA_KEY)
    if isinstance(session, str) and session.strip():
        return SessionAuthority.from_metadata(session)
    return None


def _validate_runtime_authority_binding(
    authority: RuntimeInvocationAuthority,
    call: RuntimeCallContext,
    envelope_subject_ura: str,
    envelope_subject: AddressingProjection,
    ability: _RuntimeAbilityProjection,
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
        if not ability.matches_scope(authority.matches_scope):
            raise _invalid("runtime delegation scopes do not admit ability")
        return
    if authority.issuer_ura.strip() != caller_ura:
        raise _invalid("runtime session authority issuer does not match caller_ura")
    if authority.callee_ura.strip() != callee_ura:
        raise _invalid("runtime session authority callee does not match callee_ura")
    if not authority.matches_audience(callee_ura):
        raise _invalid("runtime session authority audience does not admit callee_ura")
    if not session_authority_admits_subject(
        authority,
        envelope_subject_ura,
    ):
        raise _invalid(
            "runtime session authority does not admit descriptor-bound subject_ura"
        )
    if not _authority_list_admits(authority.allowed_actions, ability.action):
        raise _invalid(
            f"runtime session authority allowed_actions do not admit {ability.action}"
        )
    if not ability.matches_scope(authority.matches_scope):
        raise _invalid("runtime session authority scopes do not admit ability")


def _required_text(value: object, field_name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid(f"{field_name} is required")
    return value.strip()


def _authority_list_admits(patterns: tuple[str, ...], value: str) -> bool:
    clean_value = value.strip()
    return bool(clean_value) and any(
        pattern.strip() == clean_value for pattern in patterns
    )


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
