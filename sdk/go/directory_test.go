package easynet

import (
	"context"
	"strings"
	"testing"
)

func TestDirectoryClientDelegatesResolveToInjectedProvider(t *testing.T) {
	includeAbilities := false
	provider := &fakeDirectoryProvider{
		resolveResponse: DirectoryResolution{
			AnswerKind:   "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
			CanonicalURA: "easynet:///r/example/user/alice",
			Records: []DirectoryRecord{{
				Kind: "ID",
				URA:  "easynet:///r/example/user/alice",
			}},
		},
	}
	client, err := NewDirectoryClient(provider)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}
	resolution, err := client.Resolve(context.Background(), DirectoryResolveRequest{
		Call:             runtimeAbilityTestContext(),
		QueryURA:         "easynet:///r/example/user/alice",
		Kind:             DirectoryResolveCanonicalIdentity,
		IncludeAbilities: &includeAbilities,
	})
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	if resolution.CanonicalURA != "easynet:///r/example/user/alice" || len(resolution.Records) != 1 {
		t.Fatalf("unexpected resolution: %#v", resolution)
	}
	if provider.resolveRequest.QueryURA != "easynet:///r/example/user/alice" ||
		provider.resolveRequest.Kind != DirectoryResolveCanonicalIdentity ||
		provider.resolveRequest.IncludeAbilities == nil ||
		*provider.resolveRequest.IncludeAbilities {
		t.Fatalf("request was not delegated intact: %#v", provider.resolveRequest)
	}
}

func TestProjectDirectoryResolutionPreservesResolverFacts(t *testing.T) {
	resolution, err := ProjectDirectoryResolution(map[string]any{
		"answer": map[string]any{
			"answer_kind":    "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
			"canonical_name": "easynet:///r/example/user/alice",
			"next_hop":       map[string]any{"node_id": "node-1"},
			"selected_route": map[string]any{"route_ura": "easynet:///r/example/device/node-1"},
			"route_candidates": []any{map[string]any{
				"node_id": "node-1",
			}},
			"records": []any{map[string]any{
				"kind": "ID",
				"ura":  "easynet:///r/example/user/alice",
			}},
			"release_profile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
			"authority":       map[string]any{},
			"cache_policy":    map[string]any{},
		},
	})
	if err != nil {
		t.Fatalf("ProjectDirectoryResolution: %v", err)
	}
	if resolution.CanonicalURA != "easynet:///r/example/user/alice" ||
		resolution.NextHop["node_id"] != "node-1" ||
		len(resolution.RouteCandidates) != 1 ||
		len(resolution.Records) != 1 {
		t.Fatalf("unexpected resolution: %#v", resolution)
	}
}

func TestDirectoryHelpersRejectUnboundedCursorAndSurfaceNegativeDetail(t *testing.T) {
	if _, err := directoryCursor(strings.Repeat("x", maxDirectoryCursorLen+1)); err == nil {
		t.Fatal("oversized directory cursor was accepted")
	}
	if _, err := directoryLimit(MaxDirectoryPageLimit + 1); err == nil {
		t.Fatal("oversized directory page limit was accepted")
	}
	detail := directoryNegativeDetail(DirectoryResolution{
		Negative: map[string]any{
			"reason": "NEGATIVE_REASON_REFUSED",
			"detail": "Directory cursor does not match the current query",
		},
	})
	if !strings.Contains(detail, "cursor does not match") {
		t.Fatalf("negative detail = %q", detail)
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

type fakeDirectoryProvider struct {
	resolveRequest   DirectoryResolveRequest
	resolveResponse  DirectoryResolution
	resolveErr       error
	listRequest      DirectoryListRequest
	listResponse     DirectoryPage
	listErr          error
	subscribeRequest DirectorySubscribeRequest
	subscription     *DirectorySubscription
	subscribeErr     error
}

func (p *fakeDirectoryProvider) Resolve(_ context.Context, request DirectoryResolveRequest) (DirectoryResolution, error) {
	p.resolveRequest = request
	return p.resolveResponse, p.resolveErr
}

func (p *fakeDirectoryProvider) List(_ context.Context, request DirectoryListRequest) (DirectoryPage, error) {
	p.listRequest = request
	return p.listResponse, p.listErr
}

func (p *fakeDirectoryProvider) Subscribe(_ context.Context, request DirectorySubscribeRequest) (*DirectorySubscription, error) {
	p.subscribeRequest = request
	return p.subscription, p.subscribeErr
}
