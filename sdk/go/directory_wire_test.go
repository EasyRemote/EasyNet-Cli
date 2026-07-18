package easynet

import (
	"strings"
	"testing"
)

func TestDirectoryWireSnapshotRequiresTypedAuthority(t *testing.T) {
	event, err := ParseDirectoryEvent([]byte(`{
		"type":"snapshot",
		"agents":[{
			"agent_ura":"easynet:///r/acme/agent/alice.worker",
			"signing_authority":{"kind":"self_signed"},
			"status":"active",
			"ability_count":2
		}],
		"snapshot_unix_ms":10
	}`))
	if err != nil {
		t.Fatalf("ParseDirectoryEvent: %v", err)
	}
	if event.Type != "snapshot" || len(event.Agents) != 1 {
		t.Fatalf("unexpected snapshot projection: %#v", event)
	}
}

func TestDirectoryWireRejectsIncompleteTaggedVariants(t *testing.T) {
	for name, raw := range map[string]string{
		"snapshot":         `{"type":"snapshot"}`,
		"advertised":       `{"type":"agent_advertised","agent_ura":"easynet:///r/acme/agent/alice.worker"}`,
		"revoked":          `{"type":"agent_revoked","agent_ura":"easynet:///r/acme/agent/alice.worker","reason":"expired"}`,
		"owner_projection": `{"type":"owner_projection_changed","owner_ura":"easynet:///r/acme/user/alice"}`,
		"unknown":          `{"type":"product_specific_future_event"}`,
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := ParseDirectoryEvent([]byte(raw)); err == nil {
				t.Fatal("incomplete directory event was accepted")
			}
		})
	}
}

func TestDirectoryEntryPinsCanonicalProductFields(t *testing.T) {
	entry, err := ParseDirectoryEntry([]byte(`{
		"agent_ura":"easynet:///r/acme/agent/alice.worker",
		"node_id":"node-1",
		"display_name":null,
		"status":"active",
		"origin_realm":null,
		"hub_endpoint":null,
		"last_seen_unix_ms":null
	}`))
	if err != nil {
		t.Fatalf("ParseDirectoryEntry: %v", err)
	}
	canonical, err := entry.CanonicalJSON()
	if err != nil {
		t.Fatalf("CanonicalJSON: %v", err)
	}
	for _, field := range []string{
		`"agent_ura"`,
		`"node_id"`,
		`"status"`,
		`"origin_realm"`,
		`"hub_endpoint"`,
	} {
		if !strings.Contains(string(canonical), field) {
			t.Fatalf("canonical directory entry omitted %s: %s", field, canonical)
		}
	}
}
