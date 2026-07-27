package easynet

import "strings"

const (
	runtimeAbilityDescriptorProvider = "ability_descriptor"
	runtimeReceiptHistoryProvider    = "receipt_history"
)

func runtimeGovernanceDescriptorProviderForAbility(abilityName string) string {
	abilityName = strings.TrimSpace(abilityName)
	switch {
	case abilityName == "meta.list_abilities",
		abilityName == "runtime.catalog.list",
		strings.HasSuffix(abilityName, ".meta.list_abilities"),
		strings.Contains(abilityName, ".runtime.catalog."):
		return runtimeAbilityDescriptorProvider
	case strings.HasPrefix(abilityName, "invocation.history."),
		strings.HasPrefix(abilityName, "invocation.trace."),
		strings.HasPrefix(abilityName, "receipt.catalog."),
		strings.Contains(abilityName, ".invocation.history."),
		strings.Contains(abilityName, ".invocation.trace."),
		strings.Contains(abilityName, ".receipt.catalog."):
		return runtimeReceiptHistoryProvider
	default:
		return ""
	}
}

func isRuntimeGovernanceReadAbility(abilityName string) bool {
	return runtimeGovernanceDescriptorProviderForAbility(abilityName) != ""
}
