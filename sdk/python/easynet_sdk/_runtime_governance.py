"""Runtime governance ability classification shared by SDK facades."""

from __future__ import annotations

ABILITY_DESCRIPTOR_PROVIDER = "ability_descriptor"
RECEIPT_HISTORY_PROVIDER = "receipt_history"


def governance_descriptor_provider_for_ability(
    ability_name: str = "", *, ability_ura: str = ""
) -> str:
    """Return the runtime governance provider required for an ability."""

    for candidate in _ability_candidates(ability_name, ability_ura):
        if _is_catalogue_read(candidate):
            return ABILITY_DESCRIPTOR_PROVIDER
        if _is_receipt_read(candidate):
            return RECEIPT_HISTORY_PROVIDER
    return ""


def is_runtime_governance_read_ability(
    ability_name: str = "", *, ability_ura: str = ""
) -> bool:
    return bool(
        governance_descriptor_provider_for_ability(
            ability_name, ability_ura=ability_ura
        )
    )


def _ability_candidates(ability_name: str, ability_ura: str) -> tuple[str, ...]:
    values: list[str] = []
    for value in (ability_name, ability_ura):
        if not isinstance(value, str):
            continue
        trimmed = value.strip()
        if trimmed and trimmed not in values:
            values.append(trimmed)
    return tuple(values)


def _is_catalogue_read(value: str) -> bool:
    return (
        value == "meta.list_abilities"
        or value.endswith(".meta.list_abilities")
    )


def _is_receipt_read(value: str) -> bool:
    return (
        value.startswith("invocation.history.")
        or value.startswith("invocation.trace.")
        or ".invocation.history." in value
        or ".invocation.trace." in value
    )
