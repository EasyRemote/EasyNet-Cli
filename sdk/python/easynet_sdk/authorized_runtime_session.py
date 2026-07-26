"""Canonical authorized runtime session model."""

from __future__ import annotations

import hashlib
import json
import time
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol

from .authority import (
    DelegationProof,
    SessionAuthority,
)
from .bidi import BidiSession, BidiStreamDescriptor
from .errors import ErrorCode, RetryHint, SDKError
from ._identity_guards import contains_all_zero_principal
from .invocation import InvocationBuilder, InvocationDraft, new_invocation_nonce_base64
from .receipt import (
    ReceiptGetRequest,
    ReceiptGetResult,
    ReceiptListRequest,
    ReceiptHistoryPage,
    ReceiptProvider,
    ReceiptTraceRequest,
    ReceiptTraceResult,
)
from .runtime import (
    InvocationCancel,
    InvocationHandle,
    InvocationResult,
    PrepareOptions,
    RuntimeClient,
    RuntimeReceipt,
    StreamHandle,
)
from ._session_authority_subjects import session_authority_admits_subject
from ._receipt_history_admission import validate_receipt_history_request
from .signing import PreparedInvocation, SignedInvocation, Signer, SigningMaterial


class RuntimeSessionState(StrEnum):
    INTENT = "Intent"
    PREPARED = "Prepared"
    AUTHORIZED = "Authorized"
    SIGNED = "Signed"
    SUBMITTED = "Submitted"
    TERMINAL = "Terminal"


@dataclass(frozen=True)
class PrincipalRef:
    ura: str


@dataclass(frozen=True)
class CallerIdentityRef:
    principal: PrincipalRef


@dataclass(frozen=True)
class ActingPrincipalRef:
    principal: PrincipalRef


@dataclass(frozen=True)
class RuntimeTargetRef:
    ura: str


@dataclass(frozen=True)
class AbilityRef:
    name: str


@dataclass(frozen=True)
class SubjectRef:
    ura: str
    derivation_rule: str = ""


@dataclass(frozen=True)
class InvocationIntent:
    caller_identity: CallerIdentityRef
    acting_principal: ActingPrincipalRef
    target: RuntimeTargetRef
    ability: AbilityRef
    subject: SubjectRef
    call_mode: str
    arguments: object
    deadline_unix_ms: int
    idempotency_key: str
    causal_context: Mapping[str, object]
    content_type: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)


class DescriptorResolutionState(StrEnum):
    RESOLVED = "Resolved"
    NOT_FOUND = "NotFound"
    OWNER_OFFLINE = "OwnerOffline"
    MODE_UNSUPPORTED = "ModeUnsupported"
    STALE = "Stale"
    UNAVAILABLE = "Unavailable"


@dataclass(frozen=True)
class DescriptorResolutionRequest:
    caller_identity: CallerIdentityRef
    acting_principal: ActingPrincipalRef
    target: RuntimeTargetRef
    ability: AbilityRef
    subject: SubjectRef
    call_mode: str
    deadline_unix_ms: int
    idempotency_key: str
    causal_context: Mapping[str, object]


@dataclass(frozen=True)
class DescriptorResolution:
    state: DescriptorResolutionState
    descriptor_ref: str = ""
    descriptor_fingerprint: str = ""
    owner_principal: PrincipalRef = PrincipalRef("")
    reason: str = ""


@dataclass(frozen=True)
class PreparedInvocationState:
    intent: InvocationIntent
    draft: InvocationDraft
    descriptor_ref: str
    descriptor_fingerprint: str
    owner_principal: PrincipalRef
    preparation_fingerprint: str


@dataclass(frozen=True)
class AuthorityArtifact:
    authority: DelegationProof | SessionAuthority
    subject: SubjectRef
    fingerprint: str = ""
    owner: PrincipalRef = PrincipalRef("")
    admission: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class AuthorizedInvocation:
    prepared: PreparedInvocationState
    draft: InvocationDraft
    artifact: AuthorityArtifact


@dataclass(frozen=True)
class SignedInvocationState:
    authorized: AuthorizedInvocation
    prepared: PreparedInvocation
    signed: SignedInvocation
    signer_id: str


@dataclass(frozen=True)
class SubmittedInvocation:
    signed: SignedInvocationState
    handle: InvocationHandle


@dataclass(frozen=True)
class TerminalReceipt:
    submitted: SubmittedInvocation
    result: InvocationResult
    receipt: RuntimeReceipt


class RuntimeProvider(Protocol):
    def prepare_for_signing(
        self, draft: InvocationDraft, options: PrepareOptions
    ) -> tuple[PreparedInvocation, SigningMaterial]: ...

    def submit_signed(self, signed: SignedInvocation) -> InvocationHandle: ...

    def await_terminal(self, handle: InvocationHandle) -> InvocationResult: ...

    def open_stream(self, signed: SignedInvocation) -> StreamHandle: ...

    def open_bidi(
        self, signed: SignedInvocation, streams: tuple[BidiStreamDescriptor, ...]
    ) -> BidiSession: ...

    def cancel(self, handle: InvocationHandle, reason: str = "") -> InvocationCancel: ...

    def events(self, handle: InvocationHandle) -> InvocationHandle: ...

    def diagnostics(self) -> Mapping[str, object]: ...


class DescriptorProvider(Protocol):
    def resolve_descriptor(
        self, request: DescriptorResolutionRequest
    ) -> DescriptorResolution: ...


class AuthorizationProvider(Protocol):
    def authorize_invocation(
        self, prepared: PreparedInvocationState
    ) -> AuthorityArtifact: ...


class SignerProvider(Protocol):
    def caller_signer(
        self, authorized: AuthorizedInvocation, material: SigningMaterial
    ) -> Signer: ...


class IdentityProvider(Protocol):
    def caller_identity(self) -> CallerIdentityRef: ...


class ClockIdempotencySource(Protocol):
    def now_unix_ms(self) -> int: ...

    def new_idempotency_key(self) -> str: ...

    def new_nonce_base64(self) -> str: ...


class AuthorizedRuntimeSession:
    def __init__(
        self,
        *,
        runtime: RuntimeProvider,
        descriptor: DescriptorProvider,
        authorization: AuthorizationProvider,
        signer: SignerProvider,
        receipts: ReceiptProvider,
        identity: IdentityProvider,
        clock: ClockIdempotencySource,
    ) -> None:
        if runtime is None:
            raise _session_error(
                ErrorCode.PROVIDER_UNAVAILABLE, "runtime", "runtime provider is required"
            )
        if descriptor is None:
            raise _session_error(
                ErrorCode.PROVIDER_UNAVAILABLE,
                "descriptor",
                "descriptor provider is required",
            )
        if authorization is None:
            raise _session_error(
                ErrorCode.PROVIDER_UNAVAILABLE,
                "authorization",
                "authorization provider is required",
            )
        if signer is None:
            raise _session_error(
                ErrorCode.CALLER_SIGNER_UNAVAILABLE,
                "sign",
                "signer provider is required",
            )
        if receipts is None:
            raise _session_error(
                ErrorCode.PROVIDER_UNAVAILABLE, "receipt", "receipt provider is required"
            )
        if identity is None:
            raise _session_error(
                ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
                "identity",
                "identity provider is required",
            )
        if clock is None:
            raise _session_error(
                ErrorCode.PROVIDER_UNAVAILABLE,
                "clock",
                "clock/idempotency source is required",
            )
        self._runtime = runtime
        self._descriptor = descriptor
        self._authorization = authorization
        self._signer = signer
        self._receipts = receipts
        self._identity = identity
        self._clock = clock
        self.abilities = SessionAbilityOperations(self)
        self.invoke = SessionInvokeOperations(self)
        self.streams = SessionStreamOperations(self)
        self.bidi = SessionBidiOperations(self)
        self.receipts = SessionReceiptOperations(self)
        self.history = SessionHistoryOperations(self)
        self.cancellation = SessionCancellationOperations(self)
        self.diagnostics = SessionDiagnosticsOperations(self)

    def prepare(self, intent: InvocationIntent) -> PreparedInvocationState:
        intent = self._normalized_intent(intent)
        resolution = self._descriptor.resolve_descriptor(
            _descriptor_request_from_intent(intent)
        )
        _validate_descriptor_resolution(resolution)
        metadata = _runtime_session_intent_metadata(intent, resolution)
        draft = (
            InvocationBuilder()
            .with_caller_ura(intent.caller_identity.principal.ura)
            .with_callee_ura(intent.target.ura)
            .with_subject_ura(intent.subject.ura)
            .with_descriptor_ref(resolution.descriptor_ref)
            .with_nonce_base64(self._clock.new_nonce_base64())
            .with_causal_context(intent.causal_context)
            .with_json_args(intent.arguments)
            .with_content_type(intent.content_type.strip() or "application/json")
            .with_metadata(metadata)
            .build()
        )
        return _prepared_state(intent, draft, resolution)

    def authorize(self, prepared: PreparedInvocationState) -> AuthorizedInvocation:
        if not isinstance(prepared, PreparedInvocationState):
            raise _session_error(
                ErrorCode.INVALID_INVOCATION,
                "authorize",
                "prepared invocation state is required",
            )
        artifact = self._authorization.authorize_invocation(prepared)
        if artifact is None or artifact.authority is None:
            raise _session_error(
                ErrorCode.AUTHORITY_DENIED,
                "authorize",
                "authority artifact is required",
                _prepared_details(prepared),
            )
        subject = artifact.subject if artifact.subject.ura else prepared.intent.subject
        owner = artifact.owner if artifact.owner.ura else prepared.owner_principal
        artifact = AuthorityArtifact(
            authority=artifact.authority,
            subject=subject,
            fingerprint=artifact.fingerprint,
            owner=owner,
            admission=dict(artifact.admission),
        )
        _validate_authorized_binding(artifact, prepared)
        metadata = artifact.authority.metadata().merge_into(prepared.draft.metadata)
        draft = _rebuild_draft_with_metadata(prepared.draft, metadata)
        return AuthorizedInvocation(prepared=prepared, draft=draft, artifact=artifact)

    def sign(
        self,
        authorized: AuthorizedInvocation,
        options: PrepareOptions = PrepareOptions(),
    ) -> SignedInvocationState:
        if not isinstance(authorized, AuthorizedInvocation):
            raise _session_error(
                ErrorCode.AUTHORITY_DENIED, "sign", "authorized invocation is required"
            )
        prepared, material = self._runtime.prepare_for_signing(
            authorized.draft, options
        )
        try:
            signer = self._signer.caller_signer(authorized, material)
        except SDKError as error:
            if error.code == ErrorCode.CALLER_SIGNER_UNAVAILABLE:
                raise
            raise _session_error(
                ErrorCode.CALLER_SIGNER_UNAVAILABLE,
                "sign",
                "caller signer unavailable",
                _authorized_details(authorized),
                error,
            ) from error
        except Exception as error:
            raise _session_error(
                ErrorCode.CALLER_SIGNER_UNAVAILABLE,
                "sign",
                "caller signer unavailable",
                _authorized_details(authorized),
                error,
            ) from error
        if signer.handle.owner_ura.strip() != authorized.prepared.intent.caller_identity.principal.ura.strip():
            raise _session_error(
                ErrorCode.CALLER_SIGNER_UNAVAILABLE,
                "sign",
                "signer owner does not match caller identity",
                _authorized_details(authorized),
            )
        signed = signer.sign(prepared)
        return SignedInvocationState(
            authorized=authorized,
            prepared=prepared,
            signed=signed,
            signer_id=signed.signer_id,
        )

    def submit(self, signed: SignedInvocationState) -> SubmittedInvocation:
        if not isinstance(signed, SignedInvocationState) or not signed.signed.submit_ready():
            raise _session_error(
                ErrorCode.INVALID_INVOCATION,
                "submit",
                "signed invocation is not submit-ready",
            )
        return SubmittedInvocation(
            signed=signed,
            handle=self._runtime.submit_signed(signed.signed),
        )

    def await_terminal(self, submitted: SubmittedInvocation) -> TerminalReceipt:
        result = self._runtime.await_terminal(submitted.handle)
        receipt = result.terminal_receipt_summary
        if receipt is None:
            raise _session_error(
                ErrorCode.TERMINAL_RECEIPT_UNAVAILABLE,
                "terminal",
                "terminal receipt is required",
            )
        try:
            receipt.validate_proof_facts()
        except SDKError as error:
            raise _session_error(
                ErrorCode.RECEIPT_PROOF_FACTS_MISSING,
                "receipt",
                "terminal receipt proof facts are missing",
                cause=error,
            ) from error
        return TerminalReceipt(submitted=submitted, result=result, receipt=receipt)

    def _normalized_intent(self, intent: InvocationIntent) -> InvocationIntent:
        if not isinstance(intent, InvocationIntent):
            raise _session_error(
                ErrorCode.INVALID_INVOCATION, "intent", "invocation intent is required"
            )
        if not intent.caller_identity.principal.ura.strip():
            try:
                intent = InvocationIntent(
                    caller_identity=self._identity.caller_identity(),
                    acting_principal=intent.acting_principal,
                    target=intent.target,
                    ability=intent.ability,
                    subject=intent.subject,
                    call_mode=intent.call_mode,
                    arguments=intent.arguments,
                    deadline_unix_ms=intent.deadline_unix_ms,
                    idempotency_key=intent.idempotency_key,
                    causal_context=dict(intent.causal_context),
                    content_type=intent.content_type,
                    metadata=dict(intent.metadata),
                )
            except Exception as error:
                raise _session_error(
                    ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
                    "identity",
                    "caller identity unavailable",
                    cause=error,
                ) from error
        _validate_intent(intent)
        return intent


class SessionAbilityOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def resolve(self, intent: InvocationIntent) -> PreparedInvocationState:
        return self._session.prepare(intent)


class SessionInvokeOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def submit(
        self, intent: InvocationIntent, options: PrepareOptions = PrepareOptions()
    ) -> SubmittedInvocation:
        signed = self._sign(intent, options)
        return self._session.submit(signed)

    def run(
        self, intent: InvocationIntent, options: PrepareOptions = PrepareOptions()
    ) -> TerminalReceipt:
        return self._session.await_terminal(self.submit(intent, options))

    def _sign(
        self, intent: InvocationIntent, options: PrepareOptions
    ) -> SignedInvocationState:
        prepared = self._session.prepare(intent)
        authorized = self._session.authorize(prepared)
        return self._session.sign(authorized, options)


class SessionStreamOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def open(
        self, intent: InvocationIntent, options: PrepareOptions = PrepareOptions()
    ) -> StreamHandle:
        signed = self._session.invoke._sign(intent, options)
        return self._session._runtime.open_stream(signed.signed)


class SessionBidiOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def open(
        self,
        intent: InvocationIntent,
        streams: tuple[BidiStreamDescriptor, ...] = (),
        options: PrepareOptions = PrepareOptions(),
    ) -> BidiSession:
        signed = self._session.invoke._sign(intent, options)
        return self._session._runtime.open_bidi(signed.signed, streams)


class SessionReceiptOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def get(self, request: ReceiptGetRequest) -> ReceiptGetResult:
        return self._session._receipts.get(request)

    def trace(self, request: ReceiptTraceRequest) -> ReceiptTraceResult:
        return self._session._receipts.trace(request)


class SessionHistoryOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def list(self, request: ReceiptListRequest) -> ReceiptHistoryPage:
        required_scope = _receipt_history_list_authority_scope(self._session._receipts)
        if not isinstance(request, ReceiptListRequest):
            raise _session_error(
                ErrorCode.INVALID_INVOCATION,
                "history",
                "Receipt list request is required",
            )
        validate_receipt_history_request(request.call, request.filter, required_scope)
        return self._session._receipts.list(request)


class SessionCancellationOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def cancel(
        self, submitted: SubmittedInvocation, reason: str = ""
    ) -> InvocationCancel:
        return self._session._runtime.cancel(submitted.handle, reason)

    def events(self, submitted: SubmittedInvocation) -> InvocationHandle:
        return self._session._runtime.events(submitted.handle)


class SessionDiagnosticsOperations:
    def __init__(self, session: AuthorizedRuntimeSession) -> None:
        self._session = session

    def read(self) -> Mapping[str, object]:
        return self._session._runtime.diagnostics()


class RuntimeClientSessionRuntimeProvider:
    def __init__(self, client: RuntimeClient) -> None:
        if client is None:
            raise _session_error(
                ErrorCode.PROVIDER_UNAVAILABLE, "runtime", "runtime client is required"
            )
        self._client = client

    def prepare_for_signing(
        self, draft: InvocationDraft, options: PrepareOptions
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        return self._client.prepare(draft, options)

    def submit_signed(self, signed: SignedInvocation) -> InvocationHandle:
        return self._client.submit_signed(signed)

    def await_terminal(self, handle: InvocationHandle) -> InvocationResult:
        return self._client.await_result(handle)

    def open_stream(self, signed: SignedInvocation) -> StreamHandle:
        return self._client.open_signed_stream(signed)

    def open_bidi(
        self, signed: SignedInvocation, streams: tuple[BidiStreamDescriptor, ...]
    ) -> BidiSession:
        return self._client.open_signed_bidi(signed, streams)

    def cancel(self, handle: InvocationHandle, reason: str = "") -> InvocationCancel:
        return self._client.cancel(handle, reason)

    def events(self, handle: InvocationHandle) -> InvocationHandle:
        return self._client.events(handle)

    def diagnostics(self) -> Mapping[str, object]:
        return {"runtime_provider": "runtime_client"}


class RuntimeClientDescriptorProvider:
    def __init__(self, client: RuntimeClient) -> None:
        if client is None:
            raise _session_error(
                ErrorCode.PROVIDER_UNAVAILABLE,
                "descriptor",
                "runtime client is required",
            )
        self._client = client

    def resolve_descriptor(
        self, request: DescriptorResolutionRequest
    ) -> DescriptorResolution:
        try:
            ref = self._client.resolve_descriptor_ref(
                callee_ura=request.target.ura,
                ability=request.ability.name,
                call_mode=request.call_mode,
                caller_ura=request.caller_identity.principal.ura,
                subject_ura=request.subject.ura,
            )
        except Exception as error:
            return _descriptor_resolution_from_error(error)
        return DescriptorResolution(
            state=DescriptorResolutionState.RESOLVED,
            descriptor_ref=ref,
            descriptor_fingerprint=_descriptor_fingerprint(ref),
        )


class StaticCallerIdentityProvider:
    def __init__(self, caller: CallerIdentityRef) -> None:
        self._caller = caller

    def caller_identity(self) -> CallerIdentityRef:
        if not self._caller.principal.ura.strip():
            raise _session_error(
                ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
                "identity",
                "caller identity unavailable",
            )
        return self._caller


class SystemClockIdempotencySource:
    def now_unix_ms(self) -> int:
        return int(time.time() * 1000)

    def new_idempotency_key(self) -> str:
        return "idem-" + new_invocation_nonce_base64().rstrip("=").replace("+", "-").replace("/", "_")

    def new_nonce_base64(self) -> str:
        return new_invocation_nonce_base64()


def _prepared_state(
    intent: InvocationIntent,
    draft: InvocationDraft,
    resolution: DescriptorResolution,
) -> PreparedInvocationState:
    partial = {
        "caller": intent.caller_identity.principal.ura,
        "acting_principal": intent.acting_principal.principal.ura,
        "target": intent.target.ura,
        "ability": intent.ability.name,
        "subject": intent.subject.ura,
        "call_mode": intent.call_mode,
        "deadline_unix_ms": intent.deadline_unix_ms,
        "idempotency_key": intent.idempotency_key,
        "descriptor_ref": resolution.descriptor_ref,
        "descriptor_fingerprint": resolution.descriptor_fingerprint,
        "causal_context": dict(intent.causal_context),
    }
    fingerprint = hashlib.sha256(
        json.dumps(partial, separators=(",", ":"), sort_keys=True).encode("utf-8")
    ).hexdigest()
    return PreparedInvocationState(
        intent=intent,
        draft=draft,
        descriptor_ref=resolution.descriptor_ref,
        descriptor_fingerprint=resolution.descriptor_fingerprint,
        owner_principal=resolution.owner_principal,
        preparation_fingerprint=fingerprint,
    )


def _descriptor_request_from_intent(
    intent: InvocationIntent,
) -> DescriptorResolutionRequest:
    return DescriptorResolutionRequest(
        caller_identity=intent.caller_identity,
        acting_principal=intent.acting_principal,
        target=intent.target,
        ability=intent.ability,
        subject=intent.subject,
        call_mode=intent.call_mode,
        deadline_unix_ms=intent.deadline_unix_ms,
        idempotency_key=intent.idempotency_key,
        causal_context=dict(intent.causal_context),
    )


def _receipt_history_list_authority_scope(receipts: ReceiptProvider) -> str:
    capability = getattr(receipts, "receipt_history_list_authority_scope", None)
    if not callable(capability):
        raise _session_error(
            ErrorCode.PROVIDER_UNAVAILABLE,
            "history",
            "receipt provider does not expose receipt history authority scope",
        )
    try:
        scope = str(capability()).strip()
    except Exception as error:
        raise _session_error(
            ErrorCode.PROVIDER_UNAVAILABLE,
            "history",
            "receipt provider history authority scope unavailable",
            cause=error,
        ) from error
    if not scope:
        raise _session_error(
            ErrorCode.PROVIDER_UNAVAILABLE,
            "history",
            "receipt provider history authority scope is required",
        )
    return scope


def _runtime_session_intent_metadata(
    intent: InvocationIntent,
    resolution: DescriptorResolution,
) -> dict[str, object]:
    metadata = dict(intent.metadata)
    metadata["canonical_runtime_session"] = {
        "state": RuntimeSessionState.PREPARED.value,
        "caller_ura": intent.caller_identity.principal.ura,
        "acting_principal_ura": intent.acting_principal.principal.ura,
        "target_ura": intent.target.ura,
        "ability": intent.ability.name,
        "subject_ura": intent.subject.ura,
        "subject_derivation": intent.subject.derivation_rule,
        "call_mode": intent.call_mode,
        "deadline_unix_ms": intent.deadline_unix_ms,
        "idempotency_key": intent.idempotency_key,
        "descriptor_ref": resolution.descriptor_ref,
        "descriptor_fingerprint": resolution.descriptor_fingerprint,
        "owner_principal_ura": resolution.owner_principal.ura,
    }
    return metadata


def _validate_intent(intent: InvocationIntent) -> None:
    _validate_principal(intent.caller_identity.principal, "caller identity")
    _validate_principal(intent.acting_principal.principal, "acting principal")
    if not intent.target.ura.strip() or contains_all_zero_principal(intent.target.ura):
        raise _session_error(
            ErrorCode.INVALID_INVOCATION,
            "intent",
            "target URA is required",
            _intent_details(intent),
        )
    if not intent.ability.name.strip():
        raise _session_error(
            ErrorCode.INVALID_INVOCATION,
            "intent",
            "ability is required",
            _intent_details(intent),
        )
    if not intent.subject.ura.strip() or contains_all_zero_principal(intent.subject.ura):
        raise _session_error(
            ErrorCode.INVALID_INVOCATION,
            "intent",
            "subject URA is required",
            _intent_details(intent),
        )
    if not intent.call_mode.strip():
        raise _session_error(
            ErrorCode.INVALID_INVOCATION,
            "intent",
            "call mode is required",
            _intent_details(intent),
        )
    if intent.deadline_unix_ms <= 0:
        raise _session_error(
            ErrorCode.INVALID_INVOCATION,
            "intent",
            "deadline_unix_ms is required",
            _intent_details(intent),
        )
    if not intent.idempotency_key.strip():
        raise _session_error(
            ErrorCode.INVALID_INVOCATION,
            "intent",
            "idempotency key is required",
            _intent_details(intent),
        )
    if intent.causal_context is None:
        raise _session_error(
            ErrorCode.INVALID_INVOCATION,
            "intent",
            "causal context is required",
            _intent_details(intent),
        )


def _validate_principal(ref: PrincipalRef, label: str) -> None:
    if not ref.ura.strip():
        raise _session_error(
            ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
            "intent",
            f"{label} URA is required",
        )
    if contains_all_zero_principal(ref.ura):
        raise _session_error(
            ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
            "intent",
            f"{label} must not be all-zero",
            {"principal_ura": ref.ura},
        )


def _validate_descriptor_resolution(resolution: DescriptorResolution) -> None:
    if resolution.state is DescriptorResolutionState.RESOLVED:
        if not resolution.descriptor_ref.strip():
            raise _session_error(
                ErrorCode.DESCRIPTOR_NOT_FOUND,
                "descriptor",
                "resolved descriptor omitted descriptor_ref",
            )
        return
    mapping = {
        DescriptorResolutionState.NOT_FOUND: (
            ErrorCode.DESCRIPTOR_NOT_FOUND,
            "descriptor not found",
        ),
        DescriptorResolutionState.OWNER_OFFLINE: (
            ErrorCode.DESCRIPTOR_OWNER_OFFLINE,
            "descriptor owner offline",
        ),
        DescriptorResolutionState.MODE_UNSUPPORTED: (
            ErrorCode.DESCRIPTOR_MODE_UNSUPPORTED,
            "descriptor mode unsupported",
        ),
        DescriptorResolutionState.STALE: (ErrorCode.DESCRIPTOR_STALE, "descriptor stale"),
        DescriptorResolutionState.UNAVAILABLE: (
            ErrorCode.PROVIDER_UNAVAILABLE,
            "descriptor provider unavailable",
        ),
    }
    code, message = mapping.get(
        resolution.state,
        (ErrorCode.PROVIDER_UNAVAILABLE, "descriptor provider returned unknown state"),
    )
    raise _session_error(code, "descriptor", message, {"reason": resolution.reason})


def _validate_authorized_binding(
    artifact: AuthorityArtifact,
    prepared: PreparedInvocationState,
) -> None:
    details = _prepared_details(prepared)
    details["authority_session_subject"] = artifact.subject.ura
    details["owner_principal"] = artifact.owner.ura
    authority = artifact.authority
    intent = prepared.intent
    if isinstance(authority, DelegationProof):
        details["authority_session_subject"] = authority.subject_ura
        if authority.caller_ura.strip() != intent.caller_identity.principal.ura.strip():
            raise _session_error(
                ErrorCode.AUTHORITY_DENIED,
                "authorize",
                "authority caller does not match caller identity",
                details,
            )
        if authority.subject_ura.strip() != intent.subject.ura.strip():
            raise _session_error(
                ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
                "authorize",
                "authority subject does not admit invocation subject",
                details,
            )
        if not authority.matches_audience(intent.target.ura) or not authority.matches_scope(intent.ability.name):
            raise _session_error(
                ErrorCode.AUTHORITY_DENIED,
                "authorize",
                "authority does not admit target or ability",
                details,
            )
        return
    details["authority_session_subject"] = authority.subject_ura
    if authority.issuer_ura.strip() != intent.caller_identity.principal.ura.strip():
        raise _session_error(
            ErrorCode.AUTHORITY_DENIED,
            "authorize",
            "authority issuer does not match caller identity",
            details,
        )
    if authority.callee_ura.strip() != intent.target.ura.strip():
        raise _session_error(
            ErrorCode.AUTHORITY_DENIED,
            "authorize",
            "authority target does not match invocation target",
            details,
        )
    if not session_authority_admits_subject(authority, intent.subject.ura):
        raise _session_error(
            ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
            "authorize",
            "authority subject does not admit invocation subject",
            details,
        )
    if not authority.matches_audience(intent.target.ura) or not authority.matches_scope(intent.ability.name):
        raise _session_error(
            ErrorCode.AUTHORITY_DENIED,
            "authorize",
            "authority does not admit target or ability",
            details,
        )


def _rebuild_draft_with_metadata(
    draft: InvocationDraft, metadata: Mapping[str, object]
) -> InvocationDraft:
    builder = (
        InvocationBuilder()
        .with_caller_ura(draft.caller_ura)
        .with_callee_ura(draft.callee_ura)
        .with_descriptor_ref(draft.descriptor_ref)
        .with_subject_ura(draft.subject_ura)
        .with_nonce_base64(draft.nonce_base64)
        .with_causal_context(draft.causal_context)
        .with_content_type(draft.content_type)
        .with_metadata(metadata)
    )
    if draft._has_args:
        builder.with_json_args(draft.args)
    else:
        assert draft.arguments_base64 is not None
        builder.with_arguments_base64(draft.arguments_base64)
    return builder.build()


def _descriptor_fingerprint(ref: str) -> str:
    return hashlib.sha256(ref.strip().encode("utf-8")).hexdigest()


def _descriptor_resolution_from_error(error: BaseException) -> DescriptorResolution:
    text = str(error)
    if isinstance(error, SDKError):
        if error.code == ErrorCode.DESCRIPTOR_OWNER_OFFLINE:
            return DescriptorResolution(DescriptorResolutionState.OWNER_OFFLINE, reason=text)
        if error.code == ErrorCode.DESCRIPTOR_MODE_UNSUPPORTED:
            return DescriptorResolution(DescriptorResolutionState.MODE_UNSUPPORTED, reason=text)
        if error.code == ErrorCode.DESCRIPTOR_NOT_FOUND:
            return DescriptorResolution(DescriptorResolutionState.NOT_FOUND, reason=text)
        if error.code == ErrorCode.DESCRIPTOR_STALE:
            return DescriptorResolution(DescriptorResolutionState.STALE, reason=text)
    return DescriptorResolution(DescriptorResolutionState.UNAVAILABLE, reason=text)


def _intent_details(intent: InvocationIntent) -> dict[str, object]:
    return {
        "caller": intent.caller_identity.principal.ura,
        "acting_principal": intent.acting_principal.principal.ura,
        "target": intent.target.ura,
        "ability": intent.ability.name,
        "subject": intent.subject.ura,
        "call_mode": intent.call_mode,
        "idempotency_key": intent.idempotency_key,
    }


def _prepared_details(prepared: PreparedInvocationState) -> dict[str, object]:
    details = _intent_details(prepared.intent)
    details["descriptor_ref"] = prepared.descriptor_ref
    details["descriptor_fingerprint"] = prepared.descriptor_fingerprint
    details["preparation_fingerprint"] = prepared.preparation_fingerprint
    details["owner_principal"] = prepared.owner_principal.ura
    return details


def _authorized_details(authorized: AuthorizedInvocation) -> dict[str, object]:
    details = _prepared_details(authorized.prepared)
    details["authority_artifact_fingerprint"] = authorized.artifact.fingerprint
    details["authority_session_subject"] = authorized.artifact.subject.ura
    return details


def _session_error(
    code: ErrorCode,
    stage: str,
    message: str,
    details: Mapping[str, object] | None = None,
    cause: BaseException | None = None,
) -> SDKError:
    return SDKError(
        code=code,
        stage=stage,
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=dict(details or {}),
        cause=cause,
    )
