"""Shared receipt-history admission guard for SDK providers."""

from __future__ import annotations

from typing import Mapping

from ._session_authority_subjects import is_runtime_state_read_subject_ura, session_authority_admits_subject
from .authority import (
    DELEGATION_METADATA_KEY,
    SESSION_AUTHORITY_METADATA_KEY,
    DelegationProof,
    SessionAuthority,
    validate_authority_metadata,
)
from .errors import ErrorCode, RetryHint, SDKError
from .runtime_ability import RuntimeCallContext, RuntimeInvocationAuthority, _validate_runtime_call_context


def validate_receipt_history_request(
    call: RuntimeCallContext,
    receipt_filter: object,
    required_scope: str,
) -> None:
    required_scope = required_scope.strip()
    if not required_scope:
        raise _history_error(
            ErrorCode.PROVIDER_UNAVAILABLE,
            "receipt history authority scope is required",
        )
    _validate_receipt_history_call(call, required_scope)
    _validate_receipt_history_filter_binding(call, receipt_filter)


def _validate_receipt_history_call(
    call: RuntimeCallContext,
    required_scope: str,
) -> None:
    try:
        _validate_runtime_call_context(call)
    except SDKError as error:
        raise _history_error(
            ErrorCode.INVALID_INVOCATION,
            error.message,
            _runtime_call_details(call) if isinstance(call, RuntimeCallContext) else None,
            error,
        ) from error
    if not is_runtime_state_read_subject_ura(call.subject_ura):
        raise _history_error(
            ErrorCode.INVALID_INVOCATION,
            "session history call.subject_ura must be a user-owned runtime-state read subject",
            _runtime_call_details(call),
        )
    authority = _runtime_call_authority(call)
    if authority is None:
        raise _history_error(
            ErrorCode.AUTHORITY_DENIED,
            "session history requires runtime authority bound to the receipt query tuple",
            _runtime_call_details(call),
        )
    _validate_receipt_history_authority_binding(authority, call, required_scope)


def _validate_receipt_history_filter_binding(
    call: RuntimeCallContext,
    receipt_filter: object,
) -> None:
    if receipt_filter is None:
        return
    caller_ura = call.caller_ura.strip()
    callee_ura = call.callee_ura.strip()
    details = _runtime_call_details(call)
    filter_caller = str(getattr(receipt_filter, "caller_ura", "")).strip()
    if filter_caller and filter_caller != caller_ura:
        details["filter_caller_ura"] = getattr(receipt_filter, "caller_ura", "")
        raise _history_error(
            ErrorCode.AUTHORITY_DENIED,
            "receipt filter caller_ura does not match receipt query caller_ura",
            details,
        )
    filter_callee = str(getattr(receipt_filter, "callee_ura", "")).strip()
    if filter_callee and filter_callee != callee_ura:
        details["filter_callee_ura"] = getattr(receipt_filter, "callee_ura", "")
        raise _history_error(
            ErrorCode.AUTHORITY_DENIED,
            "receipt filter callee_ura does not match receipt query callee_ura",
            details,
        )


def _runtime_call_authority(
    call: RuntimeCallContext,
) -> RuntimeInvocationAuthority | None:
    metadata = dict(call.metadata)
    validate_authority_metadata(metadata)
    raw_present = bool(
        metadata.get(DELEGATION_METADATA_KEY)
        or metadata.get(SESSION_AUTHORITY_METADATA_KEY)
    )
    if call.authority is not None:
        if raw_present:
            raise _history_error(
                ErrorCode.INVALID_INVOCATION,
                "runtime call authority must be supplied once as a typed authority or metadata, not both",
                _runtime_call_details(call),
            )
        return call.authority
    if metadata.get(DELEGATION_METADATA_KEY):
        return DelegationProof.from_metadata(str(metadata[DELEGATION_METADATA_KEY]))
    if metadata.get(SESSION_AUTHORITY_METADATA_KEY):
        return SessionAuthority.from_metadata(str(metadata[SESSION_AUTHORITY_METADATA_KEY]))
    return None


def _validate_receipt_history_authority_binding(
    authority: RuntimeInvocationAuthority,
    call: RuntimeCallContext,
    required_scope: str,
) -> None:
    caller_ura = call.caller_ura.strip()
    callee_ura = call.callee_ura.strip()
    subject_ura = call.subject_ura.strip()
    details = _runtime_call_details(call)
    details["required_scope"] = required_scope.strip()
    if isinstance(authority, DelegationProof):
        if authority.caller_ura.strip() != caller_ura:
            raise _history_error(
                ErrorCode.AUTHORITY_DENIED,
                "delegation authority caller does not match receipt query caller_ura",
                details,
            )
        if authority.subject_ura.strip() != subject_ura:
            raise _history_error(
                ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
                "delegation authority subject does not match receipt query subject_ura",
                details,
            )
        if not authority.matches_audience(callee_ura):
            raise _history_error(
                ErrorCode.AUTHORITY_DENIED,
                "delegation authority audience does not admit receipt query callee_ura",
                details,
            )
        if not authority.matches_scope(required_scope):
            raise _history_error(
                ErrorCode.AUTHORITY_DENIED,
                "delegation authority scopes do not admit receipt history list authority scope",
                details,
            )
        return
    if isinstance(authority, SessionAuthority):
        details["authority_session_subject"] = authority.subject_ura
        if authority.issuer_ura.strip() != caller_ura:
            raise _history_error(
                ErrorCode.AUTHORITY_DENIED,
                "session authority issuer does not match receipt query caller_ura",
                details,
            )
        if authority.callee_ura.strip() != callee_ura:
            raise _history_error(
                ErrorCode.AUTHORITY_DENIED,
                "session authority callee does not match receipt query callee_ura",
                details,
            )
        if not authority.matches_audience(callee_ura):
            raise _history_error(
                ErrorCode.AUTHORITY_DENIED,
                "session authority audience does not admit receipt query callee_ura",
                details,
            )
        if not session_authority_admits_subject(authority, subject_ura):
            raise _history_error(
                ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
                "session authority subject does not admit receipt query subject_ura",
                details,
            )
        if not authority.matches_scope(required_scope):
            raise _history_error(
                ErrorCode.AUTHORITY_DENIED,
                "session authority scopes do not admit receipt history list authority scope",
                details,
            )
        return
    raise SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime",
        retry=RetryHint.NEVER,
        retryable=False,
        message="runtime call authority has an unsupported canonical type",
    )


def _runtime_call_details(call: RuntimeCallContext) -> dict[str, object]:
    return {
        "caller_ura": call.caller_ura,
        "callee_ura": call.callee_ura,
        "subject_ura": call.subject_ura,
    }


def _history_error(
    code: ErrorCode,
    message: str,
    details: Mapping[str, object] | None = None,
    cause: BaseException | None = None,
) -> SDKError:
    return SDKError(
        code=code,
        stage="history",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=dict(details or {}),
        cause=cause,
    )
