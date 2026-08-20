package easynet

import "testing"

func TestDirectoryEventProjectionIsRuntimeGeneric(t *testing.T) {
	event, err := ParseDirectoryEvent([]byte(`{
		"type":"snapshot",
		"agents":[{
			"agent_ura":"easynet:///r/acme/agent/alice.worker",
			"signing_authority":{"kind":"product-owned-shape"},
			"status":"active",
			"ability_count":2
		}],
		"snapshot_unix_ms":10
	}`))
	if err != nil {
		t.Fatalf("ParseDirectoryEvent: %v", err)
	}
	if event.Type != "snapshot" {
		t.Fatalf("unexpected event type: %q", event.Type)
	}
	if _, ok := event.Raw["agents"]; !ok {
		t.Fatalf("generic directory event did not preserve raw facts: %#v", event.Raw)
	}
}

func TestDirectoryEventRequiresType(t *testing.T) {
	if _, err := ParseDirectoryEvent([]byte(`{"agent_ura":"easynet:///r/acme/agent/alice.worker"}`)); err == nil {
		t.Fatal("directory event without type was accepted")
	}
}
