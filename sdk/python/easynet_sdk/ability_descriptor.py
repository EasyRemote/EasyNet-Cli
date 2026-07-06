"""AbilityDescriptorRef validation and Identity/Addressing projection."""

from __future__ import annotations

from dataclasses import dataclass
from .errors import ErrorCode, RetryHint, SDKError


@dataclass(frozen=True)
class AbilityDescriptorRef:
    """Descriptor identity split into ability URA and descriptor version."""

    raw: str
    ability_ura: str
    version: str


def validate_ability_descriptor_ref_shape(raw: str) -> AbilityDescriptorRef:
    """Validate and split the local AbilityDescriptorRef carrier shape."""

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
            "descriptor_ref ability_ura must use the runtime URA resource form"
        )
    return AbilityDescriptorRef(raw=raw, ability_ura=ability_ura, version=version)


def parse_ability_descriptor_ref(
    raw: str,
    *,
    addressing: object | None = None,
    library_path: str | None = None,
    control_path: str = "",
) -> AbilityDescriptorRef:
    """Project an AbilityDescriptorRef through the Axon-delegated SDK facade.

    Grammar ownership stays in the Identity/Addressing profile. Callers may
    inject an AddressingClient-like object for tests or reuse; otherwise the
    default SDK environment opens the configured C ABI facade.
    """

    try:
        if addressing is not None:
            from .identity import DescriptorRefRequest

            projection = addressing.project_descriptor_ref(DescriptorRefRequest(raw))
        else:
            from .identity import project_descriptor_ref

            projection = project_descriptor_ref(
                raw,
                library_path=library_path,
                control_path=control_path,
            )
    except SDKError:
        raise
    except Exception as exc:
        raise _invalid_descriptor_ref("descriptor_ref projection failed", exc) from exc
    descriptor_ref = getattr(projection, "descriptor_ref", "")
    ability_ura = getattr(projection, "ability_ura", "")
    version = getattr(projection, "descriptor_version", "")
    if not descriptor_ref or not ability_ura or not version:
        raise _invalid_descriptor_ref("descriptor_ref projection is incomplete")
    return AbilityDescriptorRef(
        raw=descriptor_ref,
        ability_ura=ability_ura,
        version=version,
    )


def _invalid_descriptor_ref(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="descriptor_ref",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
