package easynet

import (
	"context"
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
	"answerKind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
	"canonicalName": "easynet:///r/example/device/dev-a",
	"records": [{
		"name": "easynet:///r/example/device/dev-a",
		"recordType": "RECORD_TYPE_ID",
		"value": {"id": {"ura": "easynet:///r/example/device/dev-a", "kind": "URA_KIND_DEVICE"}},
		"metadata": {"status": "active"}
	}],
	"releaseProfile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
	"cachePolicy": {"ttlMs": 0}
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
