from easynet_sdk._runtime_governance import (
    RECEIPT_HISTORY_PROVIDER,
    governance_descriptor_provider_for_ability,
)


def test_generated_runtime_governance_routes_are_exact() -> None:
    assert (
        governance_descriptor_provider_for_ability("invocation.record.get")
        == RECEIPT_HISTORY_PROVIDER
    )
    assert governance_descriptor_provider_for_ability("invocation.history.delete") == ""
    assert (
        governance_descriptor_provider_for_ability(
            "system-agent.dev-a.runtime-governance.invocation.record.get"
        )
        == RECEIPT_HISTORY_PROVIDER
    )
