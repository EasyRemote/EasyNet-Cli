"""Generic AbilityDescriptorRef value object."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

from .errors import ErrorCode, RetryHint, SDKError


@dataclass(frozen=True)
class AbilityDescriptorRef:
    """Canonical descriptor identity seam: ability URA plus descriptor version."""

    raw: str
    ability_ura: str
    version: str


def parse_ability_descriptor_ref(raw: str) -> AbilityDescriptorRef:
    """Validate and split a generic AbilityDescriptorRef."""

    if not isinstance(raw, str) or raw.strip() == "":
        raise _invalid_descriptor_ref("descriptor_ref is required")
    if raw.strip() != raw:
        raise _invalid_descriptor_ref(
            "descriptor_ref must not contain surrounding whitespace"
        )
    if raw.count("@") != 1:
        raise _invalid_descriptor_ref(
            "descriptor_ref must be ability_ura@descriptor_version"
        )
    ability_ura, version = raw.split("@", 1)
    if ability_ura.strip() == "":
        raise _invalid_descriptor_ref("descriptor_ref ability_ura is required")
    if version.strip() == "":
        raise _invalid_descriptor_ref("descriptor_ref descriptor_version is required")
    if "/ability/" not in ability_ura:
        raise _invalid_descriptor_ref("descriptor_ref must bind an Ability URA")
    if not ability_ura.startswith("easynet:///r/"):
        raise _invalid_descriptor_ref(
            "descriptor_ref ability_ura must be an EasyNet URA"
        )
    return AbilityDescriptorRef(raw=raw, ability_ura=ability_ura, version=version)


def _invalid_descriptor_ref(
    message: str, cause: Optional[BaseException] = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="descriptor_ref",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
