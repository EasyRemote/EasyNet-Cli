package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryDirectoryTransport struct {
	resolveJSON   string
	devicesJSON   string
	agentsJSON    string
	abilitiesJSON string
	seenRequest   map[string]any
}

func (m *memoryDirectoryTransport) Resolve(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.resolveJSON), nil
}

func (m *memoryDirectoryTransport) ListDevices(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.devicesJSON), nil
}

func (m *memoryDirectoryTransport) ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.agentsJSON), nil
}

func (m *memoryDirectoryTransport) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.abilitiesJSON), nil
}

func baseDirectoryQuery() DirectoryQueryBase {
	return DirectoryQueryBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Cursor:            "0",
		Metadata:          map[string]any{"request_id": "directory-test"},
	}
}

func TestDirectoryListDevicesDefaultsBoundedPageSize(t *testing.T) {
	transport := &memoryDirectoryTransport{devicesJSON: `{
		"profile":"directory_identity",
		"kind":"device_page",
		"item_kind":"device",
		"items":[{"profile":"directory_identity","kind":"device","node_id":"dev-a","device_ura":"easynet:///r/example/device/dev-a","state":"online","online":true,"is_self":true,"paired":true,"tenant_id":"tenant-a","hub_endpoint":"https://hub.example","probe_status":"ok","probe_error":null,"latency_ms":12,"abilities":[],"metadata":{}}],
		"next_cursor":null,
		"limit":50,
		"source":"read_model",
		"metadata":{"source_ability":"node.list"}
	}`}
	client, err := NewDirectoryClient(transport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}

	page, err := client.ListDevices(context.Background(), DeviceQuery{DirectoryQueryBase: baseDirectoryQuery()})
	if err != nil {
		t.Fatalf("ListDevices: %v", err)
	}

	if page.Limit != DefaultDirectoryPageSize || len(page.Items) != 1 {
		t.Fatalf("unexpected page: %#v", page)
	}
	if transport.seenRequest["limit"] != float64(DefaultDirectoryPageSize) {
		t.Fatalf("default limit not sent to transport: %#v", transport.seenRequest)
	}
}

func TestDirectoryListRejectsOverMaxLimitBeforeTransport(t *testing.T) {
	transport := &memoryDirectoryTransport{}
	client, err := NewDirectoryClient(transport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}
	query := baseDirectoryQuery()
	query.Limit = MaxDirectoryPageSize + 1

	_, err = client.ListAgents(context.Background(), AgentQuery{DirectoryQueryBase: query})
	if err == nil {
		t.Fatalf("ListAgents succeeded with over-max limit")
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called despite invalid limit")
	}
}

func TestDirectoryResolveDecodesResolvedRef(t *testing.T) {
	transport := &memoryDirectoryTransport{resolveJSON: `{
		"profile":"directory_identity",
		"kind":"resolved_ref",
		"answer_kind":"RESOLVE_ANSWER_KIND_FINAL_ROUTE",
		"query_name":"easynet:///r/example/device/dev-a",
		"canonical_name":"easynet:///r/example/device/dev-a",
		"owner_ura":"easynet:///r/example/device/dev-a",
		"ability_ura":"easynet:///r/example/ability/device.dev-a.agent.list",
		"route_ura":"route-ref::easynet:///r/example/ability/device.dev-a.agent.list",
		"next_hop":{"localDeviceAbility":{"deviceUra":"easynet:///r/example/device/dev-a","dispatchName":"agent.list"}},
		"selected_route":{"reason":"ROUTE_REASON_LOCAL_DEVICE"},
		"route_candidates":[],
		"records":[],
		"negative":null,
		"release_profile":"RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
		"authority":{"authorityUra":"easynet:///r/example/hub"},
		"cache_policy":{"ttlMs":0},
		"metadata":{"source":"namespace.resolve"}
	}`}
	client, err := NewDirectoryClient(transport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}

	ref, err := client.Resolve(context.Background(), ResolveQuery{
		DirectoryQueryBase: baseDirectoryQuery(),
		QueryName:          "easynet:///r/example/device/dev-a",
		AbilityName:        "agent.list",
		QType:              "route",
	})
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}

	if ref.AnswerKind != "RESOLVE_ANSWER_KIND_FINAL_ROUTE" || ref.AbilityURA == nil || *ref.AbilityURA == "" {
		t.Fatalf("unexpected ref: %#v", ref)
	}
	if transport.seenRequest["query_name"] != "easynet:///r/example/device/dev-a" {
		t.Fatalf("query not forwarded: %#v", transport.seenRequest)
	}
}

func TestDirectoryListAbilitiesRejectsWrongPageKind(t *testing.T) {
	transport := &memoryDirectoryTransport{abilitiesJSON: `{
		"profile":"directory_identity",
		"kind":"device_page",
		"item_kind":"device",
		"items":[],
		"next_cursor":null,
		"limit":2,
		"source":"read_model",
		"metadata":{}
	}`}
	client, err := NewDirectoryClient(transport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}
	query := baseDirectoryQuery()
	query.Limit = 2

	_, err = client.ListAbilities(context.Background(), AbilityQuery{DirectoryQueryBase: query, Scope: "local"})
	if err == nil {
		t.Fatalf("ListAbilities accepted wrong page kind")
	}
}
