package easynet

import (
	"context"
	"strings"
	"testing"
)

func TestDirectoryRuntimeTransportResolvesThroughRuntime(t *testing.T) {
	identityTransport := newDirectoryRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: directoryRuntimeResolveRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryClient: %v", err)
	}

	ref, err := client.Resolve(context.Background(), ResolveQuery{
		DirectoryQueryBase: directoryBaseForTest(),
		QueryName:          "easynet:///r/example/device/dev-a",
		QType:              "directory_listing",
		RealmHint:          "example",
	})
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	if ref.Kind != "resolved_ref" || ref.AnswerKind != "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE" {
		t.Fatalf("resolved ref not projected: %#v", ref)
	}
	if len(ref.Records) != 1 {
		t.Fatalf("records = %#v", ref.Records)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if args["queryName"] != "easynet:///r/example/device/dev-a" ||
		args["qtype"] != directoryResolveTypeDirectoryListing ||
		args["realmHint"] != "example" {
		t.Fatalf("resolve args not normalized: %#v", args)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.namespace.resolve@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
	if len(identityTransport.seenBuildURA) != 1 ||
		identityTransport.seenBuildURA[0]["ability_name"] != directoryAbilityNamespaceResolve {
		t.Fatalf("identity lookup not delegated: %#v", identityTransport.seenBuildURA)
	}
}

func TestDirectoryRuntimeTransportRejectsLegacyResolveAliases(t *testing.T) {
	identity, err := NewIdentityClient(newDirectoryRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: `{
		"answerKind": "RESOLVE_ANSWER_KIND_FINAL_ROUTE",
		"canonicalName": "easynet:///r/example/device/dev-a",
		"ownerUra": "easynet:///r/example/device/dev-a",
		"abilityUra": "easynet:///r/example/ability/device.dev-a.agent.list",
		"records": []
	}`})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryClient: %v", err)
	}

	_, err = client.Resolve(context.Background(), ResolveQuery{
		DirectoryQueryBase: directoryBaseForTest(),
		QueryName:          "easynet:///r/example/device/dev-a",
		QType:              "directory_listing",
	})
	if err == nil {
		t.Fatalf("Resolve succeeded for alias-only provider output")
	}
	if sdkErr, ok := err.(*SDKError); !ok || sdkErr.Code != ErrInvalidArgument || !strings.Contains(sdkErr.Message, "answer_kind is required") {
		t.Fatalf("Resolve error = %#v, want invalid argument mentioning answer_kind is required", err)
	}
}

func TestDirectoryRuntimeTransportProxyResolvesThroughRuntime(t *testing.T) {
	identityTransport := newDirectoryRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: directoryRuntimeResolveRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryClient: %v", err)
	}

	ref, err := client.Resolve(context.Background(), ResolveQuery{
		DirectoryQueryBase: directoryBaseForTest(),
		QueryName:          "easynet:///r/example/agent/alice.",
		QType:              "directory_listing",
		RealmHint:          "example",
		PeerHubURLs:        []string{"https://peer-hub.example:50443"},
	})
	if err != nil {
		t.Fatalf("Resolve proxy: %v", err)
	}
	if ref.Metadata["source"] != directoryAbilityProxyResolve {
		t.Fatalf("metadata source = %#v", ref.Metadata)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	peerHubURLs, ok := args["peer_hub_urls"].([]any)
	if !ok || len(peerHubURLs) != 1 || peerHubURLs[0] != "https://peer-hub.example:50443" {
		t.Fatalf("peer_hub_urls arg = %#v", args["peer_hub_urls"])
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.namespace.proxy_resolve@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
	if len(identityTransport.seenBuildURA) != 1 ||
		identityTransport.seenBuildURA[0]["ability_name"] != directoryAbilityProxyResolve {
		t.Fatalf("identity lookup not delegated to proxy resolve: %#v", identityTransport.seenBuildURA)
	}
}

func TestDirectoryRuntimeTransportBuildsDevicePageInvocation(t *testing.T) {
	identity, err := NewIdentityClient(newDirectoryRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: directoryRuntimeDeviceListRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryClient: %v", err)
	}

	page, err := client.ListDevices(context.Background(), DeviceQuery{DirectoryQueryBase: directoryBaseForTest()})
	if err != nil {
		t.Fatalf("ListDevices: %v", err)
	}
	if page.Kind != "device_page" || len(page.Items) != 1 || page.Items[0].NodeID != "dev-a" {
		t.Fatalf("device page not projected: %#v", page)
	}
	if page.Metadata["source_ability"] != directoryAbilityNodeList {
		t.Fatalf("metadata = %#v", page.Metadata)
	}
}

func TestDirectoryRuntimeTransportRejectsLegacyDeviceAliases(t *testing.T) {
	identity, err := NewIdentityClient(newDirectoryRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: `{
		"nodes": [{
			"nodeId": "dev-a",
			"deviceUra": "easynet:///r/example/device/dev-a",
			"status": "online",
			"hubEndpoint": "https://hub.example",
			"probeStatus": "ok"
		}]
	}`})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryClient: %v", err)
	}

	_, err = client.ListDevices(context.Background(), DeviceQuery{DirectoryQueryBase: directoryBaseForTest()})
	if err == nil {
		t.Fatalf("ListDevices succeeded for alias-only provider output")
	}
	if sdkErr, ok := err.(*SDKError); !ok || sdkErr.Code != ErrInvalidArgument || !strings.Contains(sdkErr.Message, "device_ura") {
		t.Fatalf("ListDevices error = %#v, want invalid argument mentioning device_ura", err)
	}
}

func TestDirectoryRuntimeTransportListsPeerUserDevices(t *testing.T) {
	identity, err := NewIdentityClient(newDirectoryRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: directoryRuntimePeerDeviceRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryClient: %v", err)
	}

	page, err := client.ListPeerUserDevices(context.Background(), PeerUserDeviceQuery{
		DirectoryQueryBase: directoryBaseForTest(),
		UserTenantID:       "user-tenant",
		PeerHubURLs:        []string{"https://peer-hub.example:50443"},
	})
	if err != nil {
		t.Fatalf("ListPeerUserDevices: %v", err)
	}
	if page.Kind != "peer_user_device_page" || len(page.Items) != 1 ||
		page.Items[0].AgentURA != "easynet:///r/peer-realm.local/device/dev-peer" {
		t.Fatalf("peer page not projected: %#v", page)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if args["realm"] != "user-tenant" {
		t.Fatalf("realm arg = %#v", args["realm"])
	}
	peerHubURLs, ok := args["peer_hub_urls"].([]any)
	if !ok || len(peerHubURLs) != 1 || peerHubURLs[0] != "https://peer-hub.example:50443" {
		t.Fatalf("peer_hub_urls arg = %#v", args["peer_hub_urls"])
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.federation.proxy_list_user_devices@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
}

func TestDirectoryRuntimeTransportOpensDirectorySubscriptionStream(t *testing.T) {
	identity, err := NewIdentityClient(newDirectoryRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	recvCount := 0
	runtimeTransport := &compatibilityRuntimeInvokeTransport{
		streamTransport: StreamTransportFunc{
			RecvFunc: func(ctx context.Context) ([]byte, error) {
				recvCount++
				switch recvCount {
				case 1:
					return []byte(directoryRuntimeSnapshotCompleteFrameJSON), nil
				case 2:
					return []byte(directoryRuntimeLiveFrameJSON), nil
				default:
					t.Fatalf("unexpected recv call %d", recvCount)
					return nil, nil
				}
			},
			CancelFunc: func(ctx context.Context, reason string) ([]byte, error) {
				if reason != "done" {
					t.Fatalf("cancel reason = %q, want done", reason)
				}
				return []byte(`{"stream_id":"runtime-stream-1","cancelled":true,"state":"Cancelled","terminal":true}`), nil
			},
		},
	}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryClient: %v", err)
	}

	subscription, err := client.SubscribeDirectory(context.Background(), DirectorySubscriptionRequest{
		DirectoryQueryBase: directoryBaseForTest(),
		DeviceURA:          "easynet:///r/example/device/dev-a",
		ItemKind:           "ability",
	})
	if err != nil {
		t.Fatalf("SubscribeDirectory: %v", err)
	}
	if subscription.State != DirectorySubscriptionOpening || subscription.MetadataStreamID() != "runtime-stream-1" {
		t.Fatalf("unexpected subscription open state: %#v", subscription)
	}
	if !runtimeTransport.openStreamCalled || runtimeTransport.seenStreamDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.directory.subscribe@1.0.0" {
		t.Fatalf("runtime stream was not opened with directory draft: %#v", runtimeTransport.seenStreamDraft)
	}

	snapshotComplete, err := subscription.Next(context.Background())
	if err != nil {
		t.Fatalf("Next snapshot: %v", err)
	}
	if snapshotComplete.Phase != "snapshot_complete" || subscription.State != DirectorySubscriptionLive {
		t.Fatalf("unexpected snapshot event/state: event=%#v subscription=%#v", snapshotComplete, subscription)
	}
	live, err := subscription.Next(context.Background())
	if err != nil {
		t.Fatalf("Next live: %v", err)
	}
	if live.Phase != "live" || live.ItemKind != "ability" || subscription.ResumeToken != "directory:2" {
		t.Fatalf("unexpected live event/state: event=%#v subscription=%#v", live, subscription)
	}
	cancel, err := subscription.Cancel(context.Background(), "done")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	if !cancel.Cancelled() || subscription.State != DirectorySubscriptionClosed {
		t.Fatalf("unexpected cancel/subscription state: cancel=%#v subscription=%#v", cancel, subscription)
	}
	if err := subscription.Close(context.Background()); err != nil {
		t.Fatalf("Close after cancel should be idempotent: %v", err)
	}
}

func TestDirectorySubscriptionReleasesRuntimeHandleOnTerminalEvent(t *testing.T) {
	released := ""
	handle, err := NewStreamHandleFromJSON(StreamTransportFunc{
		RecvFunc: func(ctx context.Context) ([]byte, error) {
			return []byte(directoryRuntimeTerminalFrameJSON), nil
		},
	}, []byte(`{"stream_id":"runtime-stream-terminal","state":"Open","max_buffered_events":16}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}
	subscription := DirectorySubscription{
		Profile:     directoryIdentityProfile,
		Kind:        "directory_subscription",
		Stream:      directorySubscriptionStream,
		State:       DirectorySubscriptionLive,
		Cursor:      NewDirectorySubscriptionCursor(0),
		ResumeToken: "directory:0",
		Events:      []DirectorySubscriptionEvent{},
		DropCount:   0,
		Metadata:    map[string]any{"runtime_stream_id": "runtime-stream-terminal"},
		handle:      handle,
		release: func(streamID string) {
			released = streamID
		},
	}

	event, err := subscription.Next(context.Background())
	if err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	if !event.Terminal || subscription.State != DirectorySubscriptionClosed {
		t.Fatalf("unexpected terminal event/state: event=%#v subscription=%#v", event, subscription)
	}
	if released != "runtime-stream-terminal" || subscription.handle != nil {
		t.Fatalf("runtime handle was not released: released=%q handle=%#v", released, subscription.handle)
	}
}

func TestDirectoryRuntimeTransportReleasesTerminalDirectorySubscriptionStream(t *testing.T) {
	identity, err := NewIdentityClient(newDirectoryRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{
		streamTransport: StreamTransportFunc{
			RecvFunc: func(ctx context.Context) ([]byte, error) {
				return []byte(directoryRuntimeTerminalFrameJSON), nil
			},
		},
	}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	directoryTransport, err := NewDirectoryRuntimeTransport(runtime, identity)
	if err != nil {
		t.Fatalf("NewDirectoryRuntimeTransport: %v", err)
	}
	client, err := NewDirectoryClient(directoryTransport)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}

	subscription, err := client.SubscribeDirectory(context.Background(), DirectorySubscriptionRequest{
		DirectoryQueryBase: directoryBaseForTest(),
		DeviceURA:          "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatalf("SubscribeDirectory: %v", err)
	}
	event, err := subscription.Next(context.Background())
	if err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	if !event.Terminal || subscription.State != DirectorySubscriptionClosed {
		t.Fatalf("terminal event did not close subscription: event=%#v subscription=%#v", event, subscription)
	}
	if subscription.handle != nil {
		t.Fatalf("terminal subscription retained runtime stream handle")
	}
	directoryTransport.mu.Lock()
	streamCount := len(directoryTransport.streams)
	directoryTransport.mu.Unlock()
	if streamCount != 0 {
		t.Fatalf("terminal subscription retained %d transport streams", streamCount)
	}
}

func newDirectoryRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			directoryAbilityNamespaceResolve:     "easynet:///r/example/ability/device.dev-a.namespace.resolve",
			directoryAbilityProxyResolve:         "easynet:///r/example/ability/device.dev-a.namespace.proxy_resolve",
			directoryAbilityNodeList:             "easynet:///r/example/ability/device.dev-a.node.list",
			directoryAbilityProxyListUserDevices: "easynet:///r/example/ability/device.dev-a.federation.proxy_list_user_devices",
			directoryAbilityAgentList:            "easynet:///r/example/ability/device.dev-a.agent.list",
			directoryAbilityMetaListAbilities:    "easynet:///r/example/ability/device.dev-a.meta.list_abilities",
			directoryAbilitySubscribeDirectory:   "easynet:///r/example/ability/device.dev-a.directory.subscribe",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.namespace.resolve":                  "easynet:///r/example/ability/device.dev-a.namespace.resolve@1.0.0",
			"easynet:///r/example/ability/device.dev-a.namespace.proxy_resolve":            "easynet:///r/example/ability/device.dev-a.namespace.proxy_resolve@1.0.0",
			"easynet:///r/example/ability/device.dev-a.node.list":                          "easynet:///r/example/ability/device.dev-a.node.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.federation.proxy_list_user_devices": "easynet:///r/example/ability/device.dev-a.federation.proxy_list_user_devices@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.list":                         "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.meta.list_abilities":                "easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0",
			"easynet:///r/example/ability/device.dev-a.directory.subscribe":                "easynet:///r/example/ability/device.dev-a.directory.subscribe@1.0.0",
		},
		descriptorProjection: identityDescriptorProjectionJSON,
	}
}

func directoryBaseForTest() DirectoryQueryBase {
	return DirectoryQueryBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/user/alice",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Limit:             50,
		Metadata:          map[string]any{"request_id": "directory-runtime-test"},
	}
}

const directoryRuntimeResolveRawJSON = `{
	"answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
	"canonical_name": "easynet:///r/example/device/dev-a",
	"records": [{
		"name": "easynet:///r/example/device/dev-a",
		"recordType": "RECORD_TYPE_ID",
		"value": {"id": {"ura": "easynet:///r/example/device/dev-a", "kind": "URA_KIND_DEVICE"}},
		"metadata": {"status": "active"}
	}],
	"release_profile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
	"cache_policy": {"ttl_ms": 0}
}`

const directoryRuntimeDeviceListRawJSON = `{
	"nodes": [{
		"node_id": "dev-a",
		"device_ura": "easynet:///r/example/device/dev-a",
		"state": "online",
		"tenant_id": "example",
		"hub_endpoint": "https://hub.example",
		"abilities": ["agent.list"]
	}]
}`

const directoryRuntimePeerDeviceRawJSON = `{
	"devices": [{
		"agent_ura": "easynet:///r/peer-realm.local/device/dev-peer",
		"node_id": "dev-peer",
		"display_name": "peer device",
		"status": "active",
		"origin_realm": "peer-realm.local",
		"hub_endpoint": "https://peer-hub.example:50443",
		"last_seen_unix_ms": 1783100000123
	}]
}`

const directoryRuntimeSnapshotCompleteFrameJSON = `{
	"sequence": 1,
	"kind": "data",
	"state": "Open",
	"terminal": false,
	"payload_content_type": "application/json",
	"payload_json": {
		"profile": "directory_identity",
		"stream": "directory",
		"kind": "snapshot_complete",
		"event_id": "dir-evt-1",
		"phase": "snapshot_complete",
		"cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
		"resume_token": "directory:1",
		"terminal": false,
		"metadata": {"source": "runtime"}
	}
}`

const directoryRuntimeLiveFrameJSON = `{
	"sequence": 2,
	"kind": "data",
	"state": "Open",
	"terminal": false,
	"payload_content_type": "application/json",
	"payload_json": {
		"profile": "directory_identity",
		"stream": "directory",
		"kind": "upsert",
		"event_id": "dir-evt-2",
		"phase": "live",
		"item_kind": "ability",
		"item": {"ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health"},
		"cursor": {"stream": "directory", "sequence": 2, "token": "directory:2"},
		"resume_token": "directory:2",
		"terminal": false,
		"metadata": {"source": "runtime"}
	}
}`

const directoryRuntimeTerminalFrameJSON = `{
	"sequence": 1,
	"kind": "data",
	"state": "TerminalFrameSeen",
	"terminal": true,
	"payload_content_type": "application/json",
	"payload_json": {
		"profile": "directory_identity",
		"stream": "directory",
		"kind": "snapshot_complete",
		"event_id": "dir-evt-terminal",
		"phase": "snapshot_complete",
		"cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
		"resume_token": "directory:1",
		"terminal": true,
		"metadata": {"source": "runtime"}
	}
}`
