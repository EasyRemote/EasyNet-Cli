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
    AGENT_REFRESH = "agent.refresh"


class MissionSystemAbility(str, Enum):
    RUN = "mission.run"
    TRACK = "mission.track"
    CANCEL = "mission.cancel"
