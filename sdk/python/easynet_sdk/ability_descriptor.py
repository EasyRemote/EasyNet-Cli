"""AbilityDescriptorRef projection through the Identity/Addressing facade."""

from __future__ import annotations

from dataclasses import dataclass
from .errors import ErrorCode, RetryHint, SDKError


@dataclass(frozen=True)
class AbilityDescriptorRef:
    """Descriptor identity split into ability URA and descriptor version."""

    raw: str
    ability_ura: str
    version: str


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
