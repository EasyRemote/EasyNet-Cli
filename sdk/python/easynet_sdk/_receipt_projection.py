"""Shared runtime receipt projection guards."""

from __future__ import annotations

from collections.abc import Mapping

from .errors import ErrorCode, RetryHint, SDKError


def reject_retired_top_level_receipt_alias(
    decoded: Mapping[str, object],
    projection: str,
    *,
    stage: str,
) -> None:
    """Reject the retired top-level receipt alias at SDK wire ingress."""

    if "receipt" in decoded:
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage=stage,
            retry=RetryHint.NEVER,
            retryable=False,
            message=(
                f"{projection} must use terminal_receipt; "
                "retired receipt alias is not accepted"
            ),
        )
