"""Session-authority subject admission helpers for canonical runtime calls."""

from __future__ import annotations

from .authority import SessionAuthority
from .axon_addressing import AddressingProjection, parse_ura
from .errors import SDKError


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
