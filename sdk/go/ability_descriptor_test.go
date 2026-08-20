package easynet

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestProjectAbilityDescriptorIgnoresNestedDescriptorCompatibilityShape(t *testing.T) {
	projection := ProjectAbilityDescriptor(map[string]any{
		"descriptor": map[string]any{
			"name":        "skill.list",
			"owner_ura":   "easynet:///r/localhost/agent/device.node-a.skill-management",
			"ability_ura": "easynet:///r/localhost/ability/system-agent.node-a.skill-management.skill.list",
			"metadata": map[string]any{
				"tool_name":    "skill.list",
				"host_node_id": "node-a",
			},
		},
		"name": "agent.list",
	})

	if projection.Name != "agent.list" {
		t.Fatalf("top-level name = %q", projection.Name)
	}
	if projection.OwnerURA != "" {
		t.Fatalf("nested owner ura must not be projected, got %q", projection.OwnerURA)
	}
	if projection.AbilityURA != "" {
		t.Fatalf("nested ability ura must not be projected, got %q", projection.AbilityURA)
	}
	if projection.Metadata != nil {
		t.Fatalf("nested metadata must not be projected, got %#v", projection.Metadata)
	}
}

func TestProjectAbilityDescriptorReadsSummaryNameHintsAndSchema(t *testing.T) {
	projection := ProjectAbilityDescriptor(map[string]any{
		"ability_ura": "easynet:///r/localhost/ability/system-agent.node-a.agent-management.agent.list",
		"owner_ura":   "easynet:///r/localhost/agent/device.node-a.agent-management",
		"name":        "agent.list",
		"description": "List agents",
		"hints": map[string]any{
			"read_only":      true,
			"destructive":    false,
			"idempotent":     true,
			"streaming_only": true,
			"bidi_only":      true,
		},
		"schema_summary": map[string]any{
			"title": "Agent list input",
		},
		"input_schema": map[string]any{
			"type": "object",
		},
		"scope_subjects": map[string]any{
			"kind":   "only_ura_kinds",
			"values": []any{"resource"},
		},
		"scope_agents": map[string]any{
			"kind":   "only_matching",
			"values": []any{"easynet:///r/localhost/agent/alice.operator"},
		},
		"denied_agents": []any{"easynet:///r/localhost/agent/mallory.blocked"},
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
	if projection.ScopeSubjects["kind"] != "only_ura_kinds" || projection.ScopeAgents["kind"] != "only_matching" {
		t.Fatalf("descriptor scopes not projected: subjects=%#v agents=%#v", projection.ScopeSubjects, projection.ScopeAgents)
	}
	if len(projection.DeniedAgents) != 1 || projection.DeniedAgents[0] != "easynet:///r/localhost/agent/mallory.blocked" {
		t.Fatalf("denied agents = %#v", projection.DeniedAgents)
	}
}

func TestProjectAbilityDescriptorDoesNotDeriveRetiredNameOrInputSchemaAliases(t *testing.T) {
	projection := ProjectAbilityDescriptor(map[string]any{
		"ability_ura": "easynet:///r/localhost/ability/system-agent.node-a.agent-management.agent.list",
		"owner_ura":   "easynet:///r/localhost/agent/device.node-a.agent-management",
		"namespace":   "agent",
		"local_name":  "list",
		"schema_summary": map[string]any{
			"input": map[string]any{"type": "object"},
		},
	})

	if projection.Name != "" {
		t.Fatalf("retired namespace/local_name alias derived name %q", projection.Name)
	}
	if projection.InputSchema != nil {
		t.Fatalf("retired schema_summary.input alias derived input schema %#v", projection.InputSchema)
	}
}

func TestProjectAbilityDescriptorRefDelegatesToAddressing(t *testing.T) {
	ref, err := ProjectAbilityDescriptorRef(
		context.Background(),
		NewCanonicalAddressing(),
		"easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
	)
	if err != nil {
		t.Fatalf("ProjectAbilityDescriptorRef: %v", err)
	}

	if ref.AbilityURA != "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health" ||
		ref.Version != "1.0.0" ||
		ref.DescriptorHash != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" ||
		ref.Action != "invoke" {
		t.Fatalf("descriptor projection = %#v", ref)
	}
}

func TestParseAbilityDescriptorRefUsesCanonicalAxonProjection(t *testing.T) {
	ref, err := ParseAbilityDescriptorRef("easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke")
	if err != nil {
		t.Fatalf("ParseAbilityDescriptorRef: %v", err)
	}
	if ref.Raw != "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke" ||
		ref.AbilityURA != "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health" ||
		ref.Version != "1.0.0" ||
		ref.DescriptorHash != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" ||
		ref.Action != "invoke" {
		t.Fatalf("descriptor ref projection = %#v", ref)
	}
}

func TestRuntimeAbilityDescriptorProviderListsRuntimeDescriptors(t *testing.T) {
	var seen map[string]any
	var descriptorRequests []RuntimeDescriptorRefRequest
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
			"scope_subjects":{"kind":"only_ura_kinds","values":["resource"]},
			"scope_agents":{"kind":"only_matching","values":["easynet:///r/example/agent/alice.operator"]},
			"denied_agents":["easynet:///r/example/agent/mallory.blocked"],
			"description":"Resolve names",
			"source":"kernel:built-in",
			"hints":{"read_only":true,"idempotent":true},
			"schema_summary":{"input":{"type":"object"}},
			"input_schema":{"type":"object"},
			"metadata":{"stable":"true"}
		}]}`, "", false), nil
	}, ResolveDescriptorRefFunc: func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		var request RuntimeDescriptorRefRequest
		if err := json.Unmarshal(requestJSON, &request); err != nil {
			return nil, err
		}
		descriptorRequests = append(descriptorRequests, request)
		return testResolveDescriptorRef(t)(ctx, requestJSON)
	}}
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
	if seen["subject_ura"] != "easynet:///r/example/resource/user.alice/runtime-state/read" {
		t.Fatalf("catalogue read subject_ura = %q", seen["subject_ura"])
	}
	if len(descriptorRequests) != 1 {
		t.Fatalf("descriptor resolver calls = %d, want 1", len(descriptorRequests))
	}
	if got := descriptorRequests[0].Provider; got != "ability_descriptor" {
		t.Fatalf("catalogue descriptor provider = %q, want ability_descriptor", got)
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
		got.ScopeSubjects["kind"] != "only_ura_kinds" ||
		got.ScopeAgents["kind"] != "only_matching" ||
		len(got.DeniedAgents) != 1 ||
		got.DeniedAgents[0] != "easynet:///r/example/agent/mallory.blocked" ||
		got.Metadata["stable"] != "true" {
		t.Fatalf("descriptor projection lost runtime facts: %#v", got)
	}
}

func TestRuntimeAbilityDescriptorProviderRejectsMalformedConstraintFacts(t *testing.T) {
	for _, test := range []struct {
		name        string
		constraints string
		want        string
	}{
		{
			name:        "missing subject scope",
			constraints: `"scope_agents":{"kind":"any"},"denied_agents":[]`,
			want:        "field scope_subjects must be an object",
		},
		{
			name:        "caller scope is not an object",
			constraints: `"scope_subjects":{"kind":"any"},"scope_agents":"any","denied_agents":[]`,
			want:        "field scope_agents must be an object",
		},
		{
			name:        "deny set is not strings",
			constraints: `"scope_subjects":{"kind":"any"},"scope_agents":{"kind":"any"},"denied_agents":[42]`,
			want:        "field denied_agents must be an array of strings",
		},
	} {
		t.Run(test.name, func(t *testing.T) {
			transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
				body := `{"abilities":[{
					"name":"observe.health",
					"ability_ura":"easynet:///r/example/ability/authority.observe.health",
					"descriptor_ref":"easynet:///r/example/ability/authority.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
					"owner_ura":"easynet:///r/example/authority",
					"descriptor_version":"1.0.0",
					"schema_hash":"sha256:abc",
					"descriptor_hash":"sha256:def",
					"call_mode":"rpc",
					"visibility":"public",
					` + test.constraints + `
				}]}`
				return runtimeAbilityResultJSON(true, body, "", false), nil
			}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
			runtime, _ := NewRuntimeClient(transport)
			ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
			provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

			_, err := provider.List(context.Background(), AbilityDescriptorListRequest{Call: runtimeAbilityTestContext()})
			if err == nil || !strings.Contains(err.Error(), test.want) {
				t.Fatalf("constraint error = %v, want %q", err, test.want)
			}
		})
	}
}

func TestRuntimeAbilityDescriptorProviderUsesGenericCatalogError(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"items":[]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

	_, err := provider.List(context.Background(), AbilityDescriptorListRequest{
		Call: runtimeAbilityTestContext(),
	})
	if err == nil || !strings.Contains(err.Error(), "runtime descriptor catalog output must include descriptor rows") {
		t.Fatalf("catalog error = %v", err)
	}
	if strings.Contains(err.Error(), "meta.list_abilities") {
		t.Fatalf("generic descriptor catalog error leaked provider route: %v", err)
	}
}

func TestRuntimeAbilityDescriptorProviderRejectsNestedDescriptorRows(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"abilities":[{
			"descriptor":{
				"name":"observe.health",
				"ability_ura":"easynet:///r/example/ability/authority.observe.health",
				"owner_ura":"easynet:///r/example/authority",
				"descriptor_version":"1.0.0"
			}
		}]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

	_, err := provider.List(context.Background(), AbilityDescriptorListRequest{
		Call: runtimeAbilityTestContext(),
	})
	if err == nil || !strings.Contains(err.Error(), "ability descriptor row 0 is missing identity fields") {
		t.Fatalf("nested descriptor row error = %v", err)
	}
}

func TestRuntimeAbilityDescriptorProviderRejectsRetiredNameAndInputSchemaAliases(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"abilities":[{
			"namespace":"observe",
			"local_name":"health",
			"ability_ura":"easynet:///r/example/ability/authority.observe.health",
			"descriptor_ref":"easynet:///r/example/ability/authority.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
			"owner_ura":"easynet:///r/example/authority",
			"descriptor_version":"1.0.0",
			"schema_summary":{"input":{"type":"object"}},
			"call_mode":"rpc"
		}]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

	_, err := provider.List(context.Background(), AbilityDescriptorListRequest{
		Call: runtimeAbilityTestContext(),
	})
	if err == nil || !strings.Contains(err.Error(), "ability descriptor row 0 is missing identity fields") {
		t.Fatalf("retired alias descriptor row error = %v", err)
	}
}

func TestRuntimeAbilityDescriptorProviderRejectsTypedDescriptorProjectionFields(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"abilities":[{
			"name":"observe.health",
			"ability_ura":"easynet:///r/example/ability/authority.observe.health",
			"descriptor_ref":"easynet:///r/example/ability/authority.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
			"owner_ura":"easynet:///r/example/authority",
			"descriptor_version":"1.0.0",
			"schema_hash":42,
			"call_mode":"rpc"
		}]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

	_, err := provider.List(context.Background(), AbilityDescriptorListRequest{
		Call: runtimeAbilityTestContext(),
	})
	if err == nil || !strings.Contains(err.Error(), "ability descriptor row 0 field schema_hash must be a string") {
		t.Fatalf("typed descriptor projection error = %v", err)
	}
}

func TestRuntimeAbilityDescriptorProviderRejectsLegacyVersionAlias(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"abilities":[{
			"name":"observe.health",
			"ability_ura":"easynet:///r/example/ability/authority.observe.health",
			"descriptor_ref":"easynet:///r/example/ability/authority.observe.health@2.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!rpc",
			"owner_ura":"easynet:///r/example/authority",
			"version":"1.0.0",
			"descriptor_version":"2.0.0",
			"schema_hash":"sha256:abc",
			"descriptor_hash":"sha256:def",
			"call_mode":"rpc",
			"visibility":"public",
			"scope_subjects":{"kind":"any"},
			"scope_agents":{"kind":"any"},
			"denied_agents":[]
		}]}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeAbilityDescriptorProvider(ability)

	page, err := provider.List(context.Background(), AbilityDescriptorListRequest{
		Call: runtimeAbilityTestContext(),
	})
	if err != nil {
		t.Fatalf("List with descriptor_version field: %v", err)
	}
	if got := page.Descriptors[0].Version; got != "2.0.0" {
		t.Fatalf("version = %q, want descriptor_version", got)
	}
}

func TestRuntimeAbilityDescriptorProviderGetRejectsAmbiguousDescriptors(t *testing.T) {
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		return runtimeAbilityResultJSON(true, `{"abilities":[
			{"name":"observe.health","ability_ura":"easynet:///r/example/ability/authority.observe.health","descriptor_ref":"easynet:///r/example/ability/authority.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!rpc","owner_ura":"easynet:///r/example/authority","descriptor_version":"1.0.0","schema_hash":"sha256:abc","descriptor_hash":"sha256:def","call_mode":"rpc","visibility":"public","scope_subjects":{"kind":"any"},"scope_agents":{"kind":"any"},"denied_agents":[]},
			{"name":"observe.health","ability_ura":"easynet:///r/example/ability/authority.observe.health","descriptor_ref":"easynet:///r/example/ability/authority.observe.health@2.0.0#bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb!rpc","owner_ura":"easynet:///r/example/authority","descriptor_version":"2.0.0","schema_hash":"sha256:ghi","descriptor_hash":"sha256:jkl","call_mode":"rpc","visibility":"public","scope_subjects":{"kind":"any"},"scope_agents":{"kind":"any"},"denied_agents":[]}
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
