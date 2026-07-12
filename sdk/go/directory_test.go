package easynet

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestRuntimeDirectoryProviderResolvesThroughCanonicalAbility(t *testing.T) {
	var seen map[string]any
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		if err := json.Unmarshal(raw, &seen); err != nil {
			return nil, err
		}
		output := `{"answer":{"answer_kind":"RESOLVE_ANSWER_KIND_NON_DISPATCHABLE","canonical_name":"easynet:///r/example/user/alice","next_hop":{"node_id":"node-1"},"selected_route":{"route_ura":"easynet:///r/example/device/node-1"},"route_candidates":[{"node_id":"node-1"}],"records":[{"kind":"ID","ura":"easynet:///r/example/user/alice"}],"release_profile":"RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL","authority":{},"cache_policy":{}}}`
		return runtimeAbilityResultJSON(true, output, "", false), nil
	}})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeDirectoryProvider(ability)
	includeAbilities := false
	client, _ := NewDirectoryClient(provider)
	resolution, err := client.Resolve(context.Background(), DirectoryResolveRequest{
		Call:             runtimeAbilityTestContext(),
		QueryURA:         "easynet:///r/example/user/alice",
		Kind:             DirectoryResolveCanonicalIdentity,
		IncludeAbilities: &includeAbilities,
	})
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	if resolution.CanonicalURA != "easynet:///r/example/user/alice" || len(resolution.Records) != 1 || resolution.NextHop["node_id"] != "node-1" || len(resolution.RouteCandidates) != 1 {
		t.Fatalf("unexpected resolution: %#v", resolution)
	}
	args := seen["args"].(map[string]any)
	if args["query_name"] != "easynet:///r/example/user/alice" || args["qtype"] != string(DirectoryResolveCanonicalIdentity) || args["include_abilities"] != false {
		t.Fatalf("canonical resolver args not preserved: %#v", args)
	}
}

func TestRuntimeDirectoryProviderListsWithCursor(t *testing.T) {
	var seen map[string]any
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		if err := json.Unmarshal(raw, &seen); err != nil {
			return nil, err
		}
		output := `{"answer_kind":"RESOLVE_ANSWER_KIND_NON_DISPATCHABLE","canonical_name":"easynet:///r/example/user/alice","records":[{"name":"easynet:///r/example/user/alice","record_type":"RECORD_TYPE_ID","value":{"id":{"ura":"easynet:///r/example/user/alice"}}}],"next_cursor":"directory:v1:cursor-2","release_profile":"RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL"}`
		return runtimeAbilityResultJSON(true, output, "", false), nil
	}})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeDirectoryProvider(ability)
	includeAbilities := false
	page, err := provider.List(context.Background(), DirectoryListRequest{
		Call:             runtimeAbilityTestContext(),
		URAPrefix:        "easynet:///r/example/user/alice",
		Limit:            1,
		Cursor:           " directory:v1:cursor-1 ",
		IncludeAbilities: &includeAbilities,
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(page.Records) != 1 || page.NextCursor != "directory:v1:cursor-2" {
		t.Fatalf("unexpected page: %#v", page)
	}
	args := seen["args"].(map[string]any)
	if args["qtype"] != string(DirectoryResolveListing) || args["limit"] != float64(1) || args["cursor"] != "directory:v1:cursor-1" || args["include_abilities"] != false {
		t.Fatalf("list args not preserved: %#v", args)
	}

	repeated := runtimeDirectoryProviderWithOutput(t, map[string]any{
		"answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
		"records":     []any{},
		"next_cursor": "directory:v1:cursor-1",
	})
	_, err = repeated.List(context.Background(), DirectoryListRequest{
		Call:   runtimeAbilityTestContext(),
		Cursor: "directory:v1:cursor-1",
	})
	if err == nil || !strings.Contains(err.Error(), "repeated cursor") {
		t.Fatalf("repeated cursor error = %v", err)
	}
}

func TestRuntimeDirectoryProviderRejectsNegativeListingAnswer(t *testing.T) {
	provider := runtimeDirectoryProviderWithOutput(t, map[string]any{
		"answer_kind": "RESOLVE_ANSWER_KIND_NEGATIVE",
		"records":     []any{},
		"negative": map[string]any{
			"reason": "NEGATIVE_REASON_REFUSED",
			"detail": "namespace.resolve Directory cursor does not match the current query",
		},
	})
	_, err := provider.List(context.Background(), DirectoryListRequest{Call: runtimeAbilityTestContext()})
	if err == nil || !strings.Contains(err.Error(), "cursor does not match") {
		t.Fatalf("negative listing error = %v", err)
	}
	if _, err := directoryCursor(strings.Repeat("x", maxDirectoryCursorLen+1)); err == nil {
		t.Fatal("oversized directory cursor was accepted")
	}
}

func TestRuntimeDirectoryProviderForwardsSubscriptionResumeCursor(t *testing.T) {
	var seen map[string]any
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{OpenStreamFunc: func(_ context.Context, raw []byte) (StreamTransport, []byte, error) {
		if err := json.Unmarshal(raw, &seen); err != nil {
			return nil, nil, err
		}
		return &memoryStreamTransport{}, []byte(`{"stream_id":"directory-1","state":"Opening","max_buffered_events":8}`), nil
	}})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeDirectoryProvider(ability)
	_, err = provider.Subscribe(context.Background(), DirectorySubscribeRequest{
		Call:         runtimeAbilityTestContext(),
		ResumeCursor: ptrDirectoryCursor(NewDirectoryCursor(4)),
	})
	if err != nil {
		t.Fatalf("Subscribe: %v", err)
	}
	args := seen["args"].(map[string]any)
	if args["resume_sequence"] != float64(4) || args["resume_token"] != "directory:4" {
		t.Fatalf("resume args not forwarded: %#v", args)
	}
}

func TestDirectorySubscriptionRequiresSnapshotThenLiveDeltas(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"event":"chunk","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"type":"snapshot","agents":[],"snapshot_unix_ms":1}}`,
		`{"sequence":2,"event":"chunk","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"type":"agent_advertised","agent_ura":"easynet:///r/example/agent/alice.worker","signing_authority":{"kind":"self_signed"},"replaced_prior":false,"unix_ms":2}}`,
	}}
	handle, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"directory-1","state":"Opening","max_buffered_events":8}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}
	subscription := newDirectorySubscription(handle)
	first, err := subscription.Next(context.Background())
	if err != nil {
		t.Fatalf("snapshot Next: %v", err)
	}
	if first.Event == nil || first.Event.Type != "snapshot" || subscription.State() != DirectorySubscriptionLive || first.Cursor.Token != "directory:1" {
		t.Fatalf("unexpected snapshot transition: event=%#v state=%s", first, subscription.State())
	}
	second, err := subscription.Next(context.Background())
	if err != nil {
		t.Fatalf("delta Next: %v", err)
	}
	if second.Event == nil || second.Event.Type != "agent_advertised" || second.Cursor.Sequence != 2 {
		t.Fatalf("unexpected live event: %#v", second)
	}
}

func TestDirectorySubscriptionFailsOnDeltaBeforeSnapshot(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"event":"chunk","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"type":"heartbeat","unix_ms":1}}`,
	}}
	handle, _ := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"directory-1","state":"Opening","max_buffered_events":8}`))
	subscription := newDirectorySubscription(handle)
	if _, err := subscription.Next(context.Background()); err == nil || !strings.Contains(err.Error(), "requires snapshot as frame zero") {
		t.Fatalf("delta-before-snapshot error = %v", err)
	}
	if subscription.State() != DirectorySubscriptionFailed {
		t.Fatalf("state = %s, want Failed", subscription.State())
	}
}

func ptrDirectoryCursor(cursor DirectoryCursor) *DirectoryCursor {
	return &cursor
}

func runtimeDirectoryProviderWithOutput(t *testing.T, output map[string]any) *RuntimeDirectoryProvider {
	t.Helper()
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		encoded, err := json.Marshal(output)
		if err != nil {
			return nil, err
		}
		return runtimeAbilityResultJSON(true, string(encoded), "", false), nil
	}})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	provider, err := NewRuntimeDirectoryProvider(ability)
	if err != nil {
		t.Fatalf("NewRuntimeDirectoryProvider: %v", err)
	}
	return provider
}
