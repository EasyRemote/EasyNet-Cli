package easynet

import "testing"

func TestFederationRevokePayloadBuildsDaemonCarrier(t *testing.T) {
	payload := FederationRevokePayload(" easynet:///r/example/agent/alice.main ", " user requested ")
	if payload["agent_ura"] != "easynet:///r/example/agent/alice.main" {
		t.Fatalf("agent_ura = %#v", payload["agent_ura"])
	}
	if payload["reason"] != "user requested" {
		t.Fatalf("reason = %#v", payload["reason"])
	}
	if len(payload) != 2 {
		t.Fatalf("unexpected payload keys: %#v", payload)
	}
}
