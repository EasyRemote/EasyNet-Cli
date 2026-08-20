"""Shared identity guards for canonical SDK facades."""

_ALL_ZERO_PRINCIPAL_ID = "00000000-0000-0000-0000-000000000000"


def contains_all_zero_principal(value: str) -> bool:
    return _ALL_ZERO_PRINCIPAL_ID in value.lower().strip()

