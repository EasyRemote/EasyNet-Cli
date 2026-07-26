"""Session-authority subject admission helpers for canonical runtime calls."""

from __future__ import annotations

from .authority import SessionAuthority
from .axon_addressing import AddressingProjection, parse_ura, resource_ura, user_ura
from .errors import SDKError
from ._identity_guards import contains_all_zero_principal

_RUNTIME_STATE_READ_SUBJECT_PATH = "runtime-state/read"


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
        subject = resource_ura(owner_ura, _RUNTIME_STATE_READ_SUBJECT_PATH)
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
        and path == _RUNTIME_STATE_READ_SUBJECT_PATH
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


def session_authority_admits_subject(
    authority: SessionAuthority,
    subject_ura: str,
    subject: AddressingProjection | None = None,
) -> bool:
    """Return whether a session authority admits one descriptor-bound subject.

    The rule mirrors the Go SDK helper:
    - exact `subject_ura` equality is always accepted;
    - a user session authority may admit resources owned by that user;
    - an agent-owned resource is admitted only when the agent owner is the same
      user.

    Path substring matches are intentionally not accepted. Ownership must come
    from the canonical URA projection's `owner_id` component.
    """

    if authority.subject_ura.strip() == subject_ura.strip():
        return True
    if subject is None:
        try:
            subject = parse_ura(subject_ura.strip())
        except SDKError:
            return False
    if subject.kind != "resource":
        return False
    owner_id = subject.components.get("owner_id")
    if not isinstance(owner_id, str):
        return False
    owner_user_id = authority.session_owner_user_id.strip()
    if not owner_user_id:
        return False
    if owner_id == f"user.{owner_user_id}":
        return True
    if not owner_id.startswith("agent."):
        return False
    agent_owner = owner_id.removeprefix("agent.").split(".", 1)
    return len(agent_owner) == 2 and agent_owner[0] == owner_user_id
