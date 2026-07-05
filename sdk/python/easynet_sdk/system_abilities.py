"""SDK-owned daemon system ability identifiers.

Product packages should reference these symbols instead of hard-coding daemon
system ability names. The actual carrier construction and projection remain in
the SDK/daemon boundary.
"""

from __future__ import annotations

from enum import Enum


class AdminSystemAbility(str, Enum):
    AGENT_START = "agent.start"
    AGENT_LIST = "agent.list"
    AGENT_STOP = "agent.stop"
    AGENT_REFRESH = "agent.refresh"
    GATEWAY_STATUS = "gateway.status"
    SESSION_LIST = "session.list"
    SESSION_CREATE = "session.create"
    SESSION_DELETE = "session.delete"
    HUB_JOIN = "hub.join"
    HUB_LEAVE = "hub.leave"
    PAIRING_PREFLIGHT = "pairing.preflight"
    PAIRING_VALIDATE = "pairing.validate"
    CREDENTIAL_VERIFY = "credential.verify"
    PAIRING_CREATE = "pairing.create"
    FEDERATION_REVOKE = "federation.revoke"


class MissionSystemAbility(str, Enum):
    RUN = "mission.run"
    TRACK = "mission.track"
    CANCEL = "mission.cancel"
    EVENTS = "mission.events"
