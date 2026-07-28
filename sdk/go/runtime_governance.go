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
		strings.HasSuffix(abilityName, ".meta.list_abilities"),
		abilityName == "meta.list_resources",
		strings.HasSuffix(abilityName, ".meta.list_resources"):
		return runtimeAbilityDescriptorProvider
	case strings.HasPrefix(abilityName, "invocation.history."),
		strings.HasPrefix(abilityName, "invocation.trace."),
		strings.Contains(abilityName, ".invocation.history."),
		strings.Contains(abilityName, ".invocation.trace."):
		return runtimeReceiptHistoryProvider
	default:
		return ""
	}
}

func isRuntimeGovernanceReadAbility(abilityName string) bool {
	return runtimeGovernanceDescriptorProviderForAbility(abilityName) != ""
}
