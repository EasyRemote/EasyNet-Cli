package easynet

import "testing"

func TestGeneratedRuntimeGovernanceRoutesAreExact(t *testing.T) {
	if got := runtimeGovernanceDescriptorProviderForAbility("invocation.record.get"); got != runtimeReceiptHistoryProvider {
		t.Fatalf("invocation.record.get provider = %q", got)
	}
	if got := runtimeGovernanceDescriptorProviderForAbility("invocation.history.delete"); got != "" {
		t.Fatalf("unregistered history verb must not inherit governance provider: %q", got)
	}
	if got := runtimeGovernanceDescriptorProviderForAbility("system-agent.dev-a.runtime-governance.invocation.record.get"); got != runtimeReceiptHistoryProvider {
		t.Fatalf("system-agent-qualified invocation.record.get provider = %q", got)
	}
}
