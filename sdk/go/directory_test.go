package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryDirectoryTransport struct {
	subscriptionInvocationJSON string
	subscriptionJSON           string
	resolveJSON                string
	devicesJSON                string
	peerDevicesJSON            string
	agentsJSON                 string
	abilitiesJSON              string
	seenRequest                map[string]any
	closeCalls                 int
}

func (m *memoryDirectoryTransport) BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.subscriptionInvocationJSON), nil
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

func (m *memoryDirectoryTransport) ListPeerUserDevices(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.peerDevicesJSON), nil
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

func (m *memoryDirectoryTransport) SubscribeDirectory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.subscriptionJSON), nil
}

func (m *memoryDirectoryTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
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

func TestDirectorySubscriptionBuildsInvocationAndStateProjection(t *testing.T) {
	transport := &memoryDirectoryTransport{
		subscriptionInvocationJSON: directorySubscriptionInvocationJSON,
		subscriptionJSON:           directorySubscriptionJSON,
	}
	client, err := NewDirectoryClient(transport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}
	base := baseDirectoryQuery()
	base.Metadata = map[string]any{"request_id": "directory-subscribe"}

	draft, err := client.BuildDirectorySubscriptionInvocation(context.Background(), DirectorySubscriptionRequest{
		DirectoryQueryBase: base,
		OwnerURA:           "easynet:///r/example/device/dev-a",
		ItemKind:           "ability",
		ResumeCursor:       ptrDirectoryCursor(NewDirectorySubscriptionCursor(1)),
	})
	if err != nil {
		t.Fatalf("BuildDirectorySubscriptionInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.directory.subscribe@1.0.0" {
		t.Fatalf("descriptor = %q", draft.DescriptorRef())
	}
	if transport.seenRequest["stream"] != "directory" {
		t.Fatalf("stream not normalized: %#v", transport.seenRequest)
	}
	if transport.seenRequest["resume_cursor"].(map[string]any)["token"] != "directory:1" {
		t.Fatalf("resume cursor not forwarded: %#v", transport.seenRequest)
	}

	subscription, err := client.SubscribeDirectory(context.Background(), DirectorySubscriptionRequest{
		DirectoryQueryBase: base,
		DeviceURA:          "easynet:///r/example/device/dev-a",
		ItemKind:           "ability",
	})
	if err != nil {
		t.Fatalf("SubscribeDirectory: %v", err)
	}
	if subscription.State != DirectorySubscriptionLive || subscription.ResumeToken != "directory:3" || len(subscription.Events) != 3 {
		t.Fatalf("unexpected subscription: %#v", subscription)
	}
	if subscription.Events[2].Phase != "live" || subscription.Events[2].ItemKind != "ability" {
		t.Fatalf("unexpected live event: %#v", subscription.Events[2])
	}
}

func TestDirectorySubscriptionRejectsInvalidStateTransitions(t *testing.T) {
	if _, err := NewDirectorySubscriptionFromJSON([]byte(directorySubscriptionLiveBeforeSnapshotJSON)); err == nil {
		t.Fatalf("expected live-before-snapshot rejection")
	}
	if _, err := NewDirectorySubscriptionFromJSON([]byte(directorySubscriptionDuplicateEventJSON)); err == nil {
		t.Fatalf("expected duplicate event rejection")
	}
}

func ptrDirectoryCursor(cursor DirectorySubscriptionCursor) *DirectorySubscriptionCursor {
	return &cursor
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

func TestDirectoryListPeerUserDevicesRequiresExplicitPeerHubs(t *testing.T) {
	transport := &memoryDirectoryTransport{peerDevicesJSON: `{
		"profile":"directory_identity",
		"kind":"peer_user_device_page",
		"item_kind":"peer_user_device",
		"items":[{"profile":"directory_identity","kind":"peer_user_device","agent_ura":"easynet:///r/peer/device/dev-peer","node_id":"dev-peer","status":"active","origin_realm":"peer","hub_endpoint":"https://peer.example","last_seen_unix_ms":1783100000123,"metadata":{}}],
		"next_cursor":null,
		"limit":50,
		"source":"read_model",
		"metadata":{"source_ability":"federation.proxy_list_user_devices"}
	}`}
	client, err := NewDirectoryClient(transport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}

	page, err := client.ListPeerUserDevices(context.Background(), PeerUserDeviceQuery{
		DirectoryQueryBase: baseDirectoryQuery(),
		UserTenantID:       "user-tenant",
		PeerHubURLs:        []string{"https://peer.example"},
	})
	if err != nil {
		t.Fatalf("ListPeerUserDevices: %v", err)
	}
	if len(page.Items) != 1 || page.Items[0].NodeID != "dev-peer" || page.Items[0].OriginRealm != "peer" {
		t.Fatalf("peer page not projected: %#v", page)
	}
	if transport.seenRequest["user_tenant_id"] != "user-tenant" {
		t.Fatalf("user_tenant_id not sent: %#v", transport.seenRequest)
	}
	if _, err := client.ListPeerUserDevices(context.Background(), PeerUserDeviceQuery{
		DirectoryQueryBase: baseDirectoryQuery(),
		UserTenantID:       "user-tenant",
	}); err == nil {
		t.Fatalf("ListPeerUserDevices without peer_hub_urls succeeded")
	}
}

const directorySubscriptionInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.directory.subscribe@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"stream": "directory", "item_kind": "ability"},
  "content_type": "application/json",
  "metadata": {"request_id": "directory-subscribe", "profile": "directory_identity", "system_ability": "directory.subscribe", "carrier_owner": "daemon_sdk"}
}`

const directorySubscriptionJSON = `{
  "profile": "directory_identity",
  "kind": "directory_subscription",
  "stream": "directory",
  "state": "Live",
  "cursor": {"stream": "directory", "sequence": 3, "token": "directory:3"},
  "resume_token": "directory:3",
  "drop_count": 0,
  "events": [
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "snapshot_start",
      "event_id": "evt-1",
      "phase": "snapshot_start",
      "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
      "resume_token": "directory:1",
      "terminal": false,
      "metadata": {"source": "directory.subscribe"}
    },
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "snapshot_complete",
      "event_id": "evt-2",
      "phase": "snapshot_complete",
      "cursor": {"stream": "directory", "sequence": 2, "token": "directory:2"},
      "resume_token": "directory:2",
      "terminal": false,
      "metadata": {"source": "directory.subscribe"}
    },
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "upsert",
      "event_id": "evt-3",
      "phase": "live",
      "item_kind": "ability",
      "item": {"ability_ura": "easynet:///r/example/ability/device.dev-a.agent.list"},
      "cursor": {"stream": "directory", "sequence": 3, "token": "directory:3"},
      "resume_token": "directory:3",
      "terminal": false,
      "metadata": {"source": "directory.subscribe"}
    }
  ],
  "metadata": {"source": "directory.subscribe"}
}`

const directorySubscriptionLiveBeforeSnapshotJSON = `{
  "profile": "directory_identity",
  "kind": "directory_subscription",
  "stream": "directory",
  "state": "Live",
  "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
  "resume_token": "directory:1",
  "drop_count": 0,
  "events": [{
    "profile": "directory_identity",
    "stream": "directory",
    "kind": "upsert",
    "event_id": "evt-1",
    "phase": "live",
    "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
    "resume_token": "directory:1",
    "terminal": false,
    "metadata": {}
  }],
  "metadata": {}
}`

const directorySubscriptionDuplicateEventJSON = `{
  "profile": "directory_identity",
  "kind": "directory_subscription",
  "stream": "directory",
  "state": "Live",
  "cursor": {"stream": "directory", "sequence": 2, "token": "directory:2"},
  "resume_token": "directory:2",
  "drop_count": 0,
  "events": [
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "snapshot_complete",
      "event_id": "evt-1",
      "phase": "snapshot_complete",
      "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
      "resume_token": "directory:1",
      "terminal": false,
      "metadata": {}
    },
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "upsert",
      "event_id": "evt-1",
      "phase": "live",
      "cursor": {"stream": "directory", "sequence": 2, "token": "directory:2"},
      "resume_token": "directory:2",
      "terminal": false,
      "metadata": {}
    }
  ],
  "metadata": {}
}`

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
		PeerHubURLs:        []string{"https://peer.example:50443"},
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
	peerHubURLs, ok := transport.seenRequest["peer_hub_urls"].([]any)
	if !ok || len(peerHubURLs) != 1 || peerHubURLs[0] != "https://peer.example:50443" {
		t.Fatalf("peer hub URLs not forwarded: %#v", transport.seenRequest)
	}
	if _, err := client.Resolve(context.Background(), ResolveQuery{
		DirectoryQueryBase: baseDirectoryQuery(),
		QueryName:          "easynet:///r/example/device/dev-a",
		PeerHubURLs:        []string{" https://peer.example:50443"},
	}); err == nil {
		t.Fatalf("Resolve accepted untrimmed peer_hub_urls")
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

func TestDirectoryClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := &memoryDirectoryTransport{devicesJSON: `{
		"profile":"directory_identity",
		"kind":"device_page",
		"item_kind":"device",
		"items":[],
		"next_cursor":null,
		"limit":50,
		"source":"read_model",
		"metadata":{}
	}`}
	client, err := NewDirectoryClient(transport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.ListDevices(context.Background(), DeviceQuery{DirectoryQueryBase: baseDirectoryQuery()})
	if err == nil {
		t.Fatalf("ListDevices after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called after close: %#v", transport.seenRequest)
	}
}
