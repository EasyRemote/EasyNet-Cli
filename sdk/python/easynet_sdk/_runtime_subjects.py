"""Canonical runtime subject predicates shared by SDK authority paths."""

from __future__ import annotations

from .axon_addressing import parse_ura
from .errors import SDKError
from ._identity_guards import contains_all_zero_principal

RUNTIME_STATE_READ_SUBJECT_PATH = "runtime-state/read"
RETIRED_INVOCATION_HISTORY_SUBJECT_PATH = "session/invocation_history"


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
