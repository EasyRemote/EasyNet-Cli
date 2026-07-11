package easynet

import (
	"context"
	"testing"
)

func TestProjectAbilityDescriptorMergesNestedDescriptorWithTopLevelOverride(t *testing.T) {
	projection := ProjectAbilityDescriptor(map[string]any{
		"descriptor": map[string]any{
			"name":        "skill.list",
			"owner_ura":   "easynet:///r/localhost/device/node-a",
			"ability_ura": "easynet:///r/localhost/ability/device.node-a.skill.list",
			"metadata": map[string]any{
				"tool_name":    "skill.list",
				"host_node_id": "node-a",
			},
		},
		"name": "agent.list",
	})

	if projection.Name != "agent.list" {
		t.Fatalf("top-level name must override nested descriptor name, got %q", projection.Name)
	}
	if projection.OwnerURA != "easynet:///r/localhost/device/node-a" {
		t.Fatalf("owner ura = %q", projection.OwnerURA)
	}
	if projection.AbilityURA != "easynet:///r/localhost/ability/device.node-a.skill.list" {
		t.Fatalf("ability ura = %q", projection.AbilityURA)
	}
	if projection.Metadata["host_node_id"] != "node-a" {
		t.Fatalf("metadata host node id = %#v", projection.Metadata["host_node_id"])
	}
}

func TestProjectAbilityDescriptorReadsSummaryNameHintsAndSchema(t *testing.T) {
	projection := ProjectAbilityDescriptor(map[string]any{
		"ability_ura": "easynet:///r/localhost/ability/device.node-a.agent.list",
		"owner_ura":   "easynet:///r/localhost/device/node-a",
		"namespace":   "agent",
		"local_name":  "list",
		"description": "List agents",
		"hints": map[string]any{
			"read_only":      true,
			"destructive":    false,
			"idempotent":     true,
			"streaming_only": true,
			"bidi_only":      true,
		},
		"schema_summary": map[string]any{
			"input": map[string]any{"type": "object"},
		},
	})

	if projection.Name != "agent.list" {
		t.Fatalf("name = %q", projection.Name)
	}
	if !projection.Hints.ReadOnly || !projection.Hints.Idempotent || !projection.Hints.StreamingOnly || !projection.Hints.BidiOnly {
		t.Fatalf("hints not projected: %#v", projection.Hints)
	}
	if projection.Hints.Destructive {
		t.Fatalf("destructive hint = true, want false")
	}
	if projection.InputSchema["type"] != "object" {
		t.Fatalf("input schema = %#v", projection.InputSchema)
	}
}

func TestProjectAbilityDescriptorRefDelegatesToAddressing(t *testing.T) {
	ref, err := ProjectAbilityDescriptorRef(
		context.Background(),
		NewCanonicalAddressing(),
		"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
	)
	if err != nil {
		t.Fatalf("ProjectAbilityDescriptorRef: %v", err)
	}

	if ref.AbilityURA != "easynet:///r/example/ability/device.dev-a.observe.health" || ref.Version != "1.0.0" {
		t.Fatalf("descriptor projection = %#v", ref)
	}
}

func TestParseAbilityDescriptorRefUsesCanonicalAxonProjection(t *testing.T) {
	ref, err := ParseAbilityDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
	if err != nil {
		t.Fatalf("ParseAbilityDescriptorRef: %v", err)
	}
	if ref.Raw != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" ||
		ref.AbilityURA != "easynet:///r/example/ability/device.dev-a.observe.health" ||
		ref.Version != "1.0.0" {
		t.Fatalf("descriptor ref projection = %#v", ref)
	}
}
