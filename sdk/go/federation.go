package easynet

import "strings"

// FederationRevokePayload returns the daemon carrier payload for
// `federation.revoke`.
//
// The SDK owns this carrier shape for product consumers. URA grammar remains
// owned by Axon/daemon identity projection; this helper only preserves the
// public payload keys used at the daemon boundary.
func FederationRevokePayload(agentURA string, reason string) map[string]any {
	return map[string]any{
		"agent_ura": strings.TrimSpace(agentURA),
		"reason":    strings.TrimSpace(reason),
	}
}
