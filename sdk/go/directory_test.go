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
		output := `{"answer_kind":"RESOLVE_ANSWER_KIND_NON_DISPATCHABLE","canonical_name":"easynet:///r/example/user/alice","records":[{"kind":"ID","ura":"easynet:///r/example/user/alice"}],"release_profile":"RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL","authority":{},"cache_policy":{}}`
		return runtimeAbilityResultJSON(true, output, "", false), nil
	}})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeDirectoryProvider(ability)
	client, _ := NewDirectoryClient(provider)
	resolution, err := client.Resolve(context.Background(), DirectoryResolveRequest{
		Call:     runtimeAbilityTestContext(),
		QueryURA: "easynet:///r/example/user/alice",
		Kind:     DirectoryResolveCanonicalIdentity,
	})
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	if resolution.CanonicalURA != "easynet:///r/example/user/alice" || len(resolution.Records) != 1 {
		t.Fatalf("unexpected resolution: %#v", resolution)
	}
	args := seen["args"].(map[string]any)
	if args["query_name"] != "easynet:///r/example/user/alice" || args["qtype"] != string(DirectoryResolveCanonicalIdentity) {
		t.Fatalf("canonical resolver args not preserved: %#v", args)
	}
}

func TestRuntimeDirectoryProviderKeepsUnavailableCursorSeamsExplicit(t *testing.T) {
	provider := &RuntimeDirectoryProvider{}
	if _, err := provider.List(context.Background(), DirectoryListRequest{Cursor: "directory-page:1"}); err == nil || !strings.Contains(err.Error(), "cursor provider is not available") {
		t.Fatalf("list cursor seam error = %v", err)
	}
	if _, err := provider.Subscribe(context.Background(), DirectorySubscribeRequest{ResumeCursor: ptrDirectoryCursor(NewDirectoryCursor(4))}); err == nil || !strings.Contains(err.Error(), "resume provider is not available") {
		t.Fatalf("resume seam error = %v", err)
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
