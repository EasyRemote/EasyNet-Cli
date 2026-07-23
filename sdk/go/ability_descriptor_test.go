package easynet

import (
	"context"
	"encoding/json"
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

func TestRuntimeAbilityDescriptorProviderListsRuntimeDescriptors(t *testing.T) {
	var seen map[string]any
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		if err := json.Unmarshal(raw, &seen); err != nil {
			return nil, err
		}
		args := seen["args"].(map[string]any)
		if args["owner_ura"] != "easynet:///r/example/authority" || args["scope"] != "realm" {
			t.Fatalf("provider did not lower filters to runtime catalog args: %#v", args)
		}
		return runtimeAbilityResultJSON(true, `{"abilities":[{
			"name":"namespace.resolve",
			"ability_ura":"easynet:///r/example/ability/authority.namespace.resolve",
			"descriptor_ref":"easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
			"owner_ura":"easynet:///r/example/authority",
			"descriptor_version":"1.0.0",
			"schema_hash":"sha256:abc",
			"descriptor_hash":"sha256:def",
			"call_mode":"rpc",
			"class":"runtime",
			"receipt_semantics":{"kind":"terminal"},
			"visibility":"public",
			"description":"Resolve names",
			"source":"kernel:built-in",
			"hints":{"read_only":true,"idempotent":true},
			"schema_summary":{"input":{"type":"object"}},
			"metadata":{"stable":"true"}
		}]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	provider, err := NewRuntimeAbilityDescriptorProvider(ability)
	if err != nil {
		t.Fatalf("NewRuntimeAbilityDescriptorProvider: %v", err)
	}
	page, err := provider.List(context.Background(), AbilityDescriptorListRequest{
		Call:     runtimeAbilityTestContext(),
		Scope:    "realm",
		OwnerURA: "easynet:///r/example/authority",
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if seen["descriptor_ref"] != "easynet:///r/example/ability/authority.meta.list_abilities@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read" {
		t.Fatalf("descriptor_ref = %q", seen["descriptor_ref"])
	}
	if len(page.Descriptors) != 1 {
		t.Fatalf("descriptor count = %d", len(page.Descriptors))
	}
	got := page.Descriptors[0]
	if got.AbilityURA != "easynet:///r/example/ability/authority.namespace.resolve" ||
		got.DescriptorRef != "easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read" ||
		got.Version != "1.0.0" ||
		got.Class != "runtime" ||
		got.SchemaHash != "sha256:abc" ||
		got.CallMode != "rpc" ||
		!got.Hints.ReadOnly ||
		got.SchemaSummary["input"] == nil ||
		got.InputSchema["type"] != "object" ||
		got.Metadata["stable"] != "true" {
			t.Fatalf("descriptor projection lost runtime facts: %#v", got)
		}
	}

func TestRuntimeAbilityDescriptorProviderGetRejectsAmbiguousDescriptors(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"abilities":[
			{"name":"observe.health","ability_ura":"easynet:///r/example/ability/authority.observe.health","owner_ura":"easynet:///r/example/authority","version":"1.0.0","call_mode":"rpc"},
			{"name":"observe.health","ability_ura":"easynet:///r/example/ability/authority.observe.health","owner_ura":"easynet:///r/example/authority","version":"2.0.0","call_mode":"rpc"}
		]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

	_, err := provider.Get(context.Background(), AbilityDescriptorGetRequest{
		Call:       runtimeAbilityTestContext(),
		AbilityURA: "easynet:///r/example/ability/authority.observe.health",
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ambiguous descriptor error = %v", err)
	}

	got, err := provider.Get(context.Background(), AbilityDescriptorGetRequest{
		Call:              runtimeAbilityTestContext(),
		AbilityURA:        "easynet:///r/example/ability/authority.observe.health",
		DescriptorVersion: "2.0.0",
	})
	if err != nil {
		t.Fatalf("Get with descriptor_version: %v", err)
	}
	if got.Version != "2.0.0" {
		t.Fatalf("version = %q", got.Version)
	}
}

func TestRuntimeAbilityDescriptorProviderGetReportsDescriptorNotFound(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"abilities":[]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

	_, err := provider.Get(context.Background(), AbilityDescriptorGetRequest{
		Call:       runtimeAbilityTestContext(),
		AbilityURA: "easynet:///r/example/ability/authority.observe.health",
	})
	if !IsCode(err, ErrDescriptorNotFound) {
		t.Fatalf("descriptor miss error = %v, want DESCRIPTOR_NOT_FOUND", err)
	}
}
