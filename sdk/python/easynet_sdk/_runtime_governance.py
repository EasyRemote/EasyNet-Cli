"""Generated runtime governance ability classification shared by SDK facades."""

from __future__ import annotations

ABILITY_DESCRIPTOR_PROVIDER = "ability_descriptor"
RECEIPT_HISTORY_PROVIDER = "receipt_history"

_RUNTIME_GOVERNANCE_ROUTES = (
    ("meta.list_abilities", ABILITY_DESCRIPTOR_PROVIDER),
    ("meta.list_resources", ABILITY_DESCRIPTOR_PROVIDER),
    ("invocation.history.list", RECEIPT_HISTORY_PROVIDER),
    ("invocation.history.get", RECEIPT_HISTORY_PROVIDER),
    ("invocation.history.path", RECEIPT_HISTORY_PROVIDER),
    ("invocation.record.get", RECEIPT_HISTORY_PROVIDER),
    ("invocation.trace.get", RECEIPT_HISTORY_PROVIDER),
)


def governance_descriptor_provider_for_ability(
    ability_name: str = "", *, ability_ura: str = ""
) -> str:
    for candidate in _ability_candidates(ability_name, ability_ura):
        for route, provider in _RUNTIME_GOVERNANCE_ROUTES:
            if candidate == route or candidate.endswith(f".{route}"):
                return provider
    return ""


def is_runtime_governance_read_ability(
    ability_name: str = "", *, ability_ura: str = ""
) -> bool:
    return bool(governance_descriptor_provider_for_ability(ability_name, ability_ura=ability_ura))


def _ability_candidates(ability_name: str, ability_ura: str) -> tuple[str, ...]:
    values: list[str] = []
    for value in (ability_name, ability_ura):
        if not isinstance(value, str):
            continue
        trimmed = value.strip()
        if trimmed and trimmed not in values:
            values.append(trimmed)
    return tuple(values)
