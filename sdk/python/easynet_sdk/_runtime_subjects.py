"""Canonical runtime subject predicates shared by SDK authority paths."""

from __future__ import annotations

from .axon_addressing import parse_ura, resource_ura, user_ura
from .errors import SDKError
from ._identity_guards import contains_all_zero_principal

RUNTIME_STATE_READ_SUBJECT_PATH = "runtime-state/read"
RETIRED_INVOCATION_HISTORY_SUBJECT_PATH = "session/invocation_history"


def runtime_state_read_subject_ura(realm: str, user_id: str) -> str:
    """Build the canonical user-owned subject for runtime-state reads."""

    clean_realm = str(realm).strip()
    clean_user_id = str(user_id).strip()
    if not clean_realm:
        _invalid_runtime_state_subject("runtime-state read subject realm is required")
    if not clean_user_id:
        _invalid_runtime_state_subject("runtime-state read subject user_id is required")
    if contains_all_zero_principal(clean_user_id):
        _invalid_runtime_state_subject(
            "runtime-state read subject user_id must not be all-zero"
        )
    try:
        owner_ura = user_ura(clean_realm, clean_user_id)
        subject = resource_ura(owner_ura, RUNTIME_STATE_READ_SUBJECT_PATH)
    except SDKError as error:
        _invalid_runtime_state_subject(
            "runtime-state read subject_ura must be canonical", error
        )
    return subject


def is_runtime_state_read_subject_ura(subject_ura: str) -> bool:
    try:
        subject = parse_ura(str(subject_ura).strip())
    except SDKError:
        return False
    owner_id = subject.components.get("owner_id")
    path = subject.components.get("path")
    return (
        subject.kind == "resource"
        and isinstance(owner_id, str)
        and owner_id.startswith("user.")
        and owner_id.removeprefix("user.").strip() != ""
        and not contains_all_zero_principal(owner_id.removeprefix("user."))
        and path == RUNTIME_STATE_READ_SUBJECT_PATH
    )


def is_runtime_governance_read_subject_ura(subject_ura: str, callee_ura: str) -> bool:
    """Return whether subject is admissible for runtime governance reads."""

    clean_subject = str(subject_ura).strip()
    clean_callee = str(callee_ura).strip()
    if is_runtime_state_read_subject_ura(clean_subject):
        return True
    try:
        subject = parse_ura(clean_subject)
        callee = parse_ura(clean_callee)
    except SDKError:
        return False
    return (
        subject.kind in {"authority", "device"}
        and subject.kind == callee.kind
        and subject.realm == callee.realm
        and clean_subject == clean_callee
    )


def is_retired_invocation_history_subject_ura(subject_ura: str) -> bool:
    try:
        subject = parse_ura(str(subject_ura).strip())
    except SDKError:
        return False
    owner_id = subject.components.get("owner_id")
    path = subject.components.get("path")
    if (
        subject.kind != "resource"
        or not isinstance(owner_id, str)
        or not owner_id.startswith("user.")
    ):
        return False
    user_id = owner_id.removeprefix("user.").strip()
    return (
        user_id != ""
        and "." not in user_id
        and not contains_all_zero_principal(user_id)
        and path == RETIRED_INVOCATION_HISTORY_SUBJECT_PATH
    )


def _invalid_runtime_state_subject(message: str, cause: Exception | None = None) -> None:
    raise SDKError(
        code="INVALID_INVOCATION",
        stage="authority",
        retry="never",
        retryable=False,
        message=message,
        cause=cause,
    )
