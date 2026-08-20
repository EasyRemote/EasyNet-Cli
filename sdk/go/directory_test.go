package easynet

import (
	"context"
	"fmt"
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

func TestDirectoryClientScanCollectsEveryPage(t *testing.T) {
	firstPage := make([]DirectoryRecord, 50)
	for i := range firstPage {
		firstPage[i] = DirectoryRecord{Kind: "ABILITY", AbilityURA: fmt.Sprintf("easynet:///r/example/ability/system-agent.dev-a.runtime-health.test.%02d", i)}
	}
	provider := &fakeDirectoryProvider{}
	provider.resolveFunc = func(request DirectoryResolveRequest) (DirectoryResolution, error) {
		switch request.Cursor {
		case "":
			return DirectoryResolution{
				AnswerKind: "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
				Records:    firstPage,
				NextCursor: "page-2",
			}, nil
		case "page-2":
			return DirectoryResolution{
				AnswerKind: "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
				Records:    []DirectoryRecord{{Kind: "ABILITY", AbilityURA: "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health"}},
			}, nil
		default:
			t.Fatalf("unexpected cursor %q", request.Cursor)
			return DirectoryResolution{}, nil
		}
	}
	client, err := NewDirectoryClient(provider)
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}
	snapshot, err := client.Scan(context.Background(), DirectoryResolveRequest{
		Call:     runtimeAbilityTestContext(),
		QueryURA: "easynet:///r/example/device/",
		Kind:     DirectoryResolveListing,
		Limit:    50,
	}, DirectoryScanOptions{
		MaxPages:   4,
		MaxRecords: 60,
	})
	if err != nil {
		t.Fatalf("Scan: %v", err)
	}
	if !snapshot.Complete || snapshot.Pages != 2 || snapshot.RecordCount != 51 || len(snapshot.Resolution.Records) != 51 {
		t.Fatalf("unexpected snapshot: %#v", snapshot)
	}
	if got := snapshot.Resolution.Records[50].AbilityURA; got != "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health" {
		t.Fatalf("last page ability = %q", got)
	}
	if len(provider.resolveRequests) != 2 || provider.resolveRequests[1].Cursor != "page-2" || provider.resolveRequests[1].Limit != 50 {
		t.Fatalf("scan requests = %#v", provider.resolveRequests)
	}
	if provider.resolveRequests[0].Call.NonceBase64 == provider.resolveRequests[1].Call.NonceBase64 {
		t.Fatalf("scan continuation reused Invocation nonce: %#v", provider.resolveRequests)
	}
}

func TestDirectoryClientScanRejectsRepeatedCursorAndBounds(t *testing.T) {
	provider := &fakeDirectoryProvider{
		resolveFunc: func(request DirectoryResolveRequest) (DirectoryResolution, error) {
			return DirectoryResolution{
				AnswerKind: "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
				Records:    []DirectoryRecord{{Kind: "ID", URA: "easynet:///r/example/device/dev-a"}},
				NextCursor: "same-cursor",
			}, nil
		},
	}
	client, _ := NewDirectoryClient(provider)
	_, err := client.Scan(context.Background(), DirectoryResolveRequest{
		Call: runtimeAbilityTestContext(), QueryURA: "easynet:///r/example/device/", Kind: DirectoryResolveListing,
	}, DirectoryScanOptions{MaxPages: 4, MaxRecords: 4})
	if err == nil || !strings.Contains(err.Error(), "repeated continuation cursor") {
		t.Fatalf("repeated cursor error = %v", err)
	}

	provider.resolveRequests = nil
	_, err = client.Scan(context.Background(), DirectoryResolveRequest{
		Call: runtimeAbilityTestContext(), QueryURA: "easynet:///r/example/device/", Kind: DirectoryResolveListing,
	}, DirectoryScanOptions{MaxPages: 4, MaxRecords: 1})
	if err == nil || !strings.Contains(err.Error(), "maximum record bound") {
		t.Fatalf("record bound error = %v", err)
	}
}

func TestDirectoryClientListNormalizesPageBoundsBeforeProvider(t *testing.T) {
	provider := &fakeDirectoryProvider{listResponse: DirectoryPage{Records: []DirectoryRecord{}}}
	client, _ := NewDirectoryClient(provider)
	page, err := client.List(context.Background(), DirectoryListRequest{
		Call: runtimeAbilityTestContext(), URAPrefix: "easynet:///r/example/device/",
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if provider.listRequest.Limit != DefaultDirectoryPageLimit || page.Records == nil {
		t.Fatalf("normalized list request = %#v, page = %#v", provider.listRequest, page)
	}
	_, err = client.List(context.Background(), DirectoryListRequest{
		Call: runtimeAbilityTestContext(), URAPrefix: "easynet:///r/example/device/", Limit: MaxDirectoryPageLimit + 1,
	})
	if err == nil || !strings.Contains(err.Error(), "maximum page bound") {
		t.Fatalf("oversized list limit error = %v", err)
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

func TestProjectDirectoryResolutionRejectsMalformedPresentFacts(t *testing.T) {
	base := map[string]any{
		"answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
		"records":     []any{},
	}
	cases := []struct {
		name    string
		key     string
		value   any
		message string
	}{
		{name: "answer", key: "answer", value: "not-an-object", message: "answer must be an object"},
		{name: "records", key: "records", value: "not-a-list", message: "records must be a list"},
		{name: "record missing kind", key: "records", value: []any{map[string]any{"ura": "easynet:///r/example/user/alice"}}, message: "record kind is required"},
		{name: "record missing canonical facts", key: "records", value: []any{map[string]any{"kind": "ID", "canonical_name": "easynet:///r/example/user/alice"}}, message: "record requires at least one canonical URA fact"},
		{name: "next hop", key: "next_hop", value: "not-an-object", message: "next_hop must be an object"},
		{name: "selected route", key: "selected_route", value: "not-an-object", message: "selected_route must be an object"},
		{name: "route candidates", key: "route_candidates", value: map[string]any{"node_id": "node-1"}, message: "route_candidates must be a list"},
		{name: "route candidate item", key: "route_candidates", value: []any{"not-an-object"}, message: "route_candidates item must be an object"},
		{name: "negative", key: "negative", value: "not-an-object", message: "negative must be an object"},
		{name: "authority", key: "authority", value: "not-an-object", message: "authority must be an object"},
		{name: "cache policy", key: "cache_policy", value: "not-an-object", message: "cache_policy must be an object"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			payload := make(map[string]any, len(base)+1)
			for key, value := range base {
				payload[key] = value
			}
			payload[tc.key] = tc.value
			_, err := ProjectDirectoryResolution(payload)
			if err == nil || !strings.Contains(err.Error(), tc.message) {
				t.Fatalf("ProjectDirectoryResolution error = %v, want %q", err, tc.message)
			}
		})
	}
}

func TestProjectDirectoryResolutionRejectsNegativeWithoutAnswerKind(t *testing.T) {
	_, err := ProjectDirectoryResolution(map[string]any{
		"negative": map[string]any{
			"reason": "NEGATIVE_REASON_NXDOMAIN",
			"detail": "owner is not online",
		},
		"records": []any{},
	})
	if err == nil || !strings.Contains(err.Error(), "answer_kind is required") {
		t.Fatalf("ProjectDirectoryResolution error = %v, want explicit answer_kind rejection", err)
	}
}

func TestProjectDirectoryRecordDoesNotPromoteLegacyAliases(t *testing.T) {
	record := ProjectDirectoryRecord(map[string]any{
		"type":           "URA_KIND_DEVICE",
		"canonical_name": "easynet:///r/example/device/dev-1",
	})
	if record.Kind != "" || record.URA != "" {
		t.Fatalf("legacy aliases were promoted into canonical fields: %#v", record)
	}
	if record.Raw["type"] != "URA_KIND_DEVICE" || record.Raw["canonical_name"] != "easynet:///r/example/device/dev-1" {
		t.Fatalf("raw provider facts were not preserved: %#v", record.Raw)
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
		`{"sequence":1,"kind":"data","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"type":"snapshot","agents":[],"snapshot_unix_ms":1}}`,
		`{"sequence":2,"kind":"data","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"type":"agent_advertised","agent_ura":"easynet:///r/example/agent/alice.worker","signing_authority":{"kind":"self_signed"},"replaced_prior":false,"unix_ms":2}}`,
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
		`{"sequence":1,"kind":"data","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"type":"heartbeat","unix_ms":1}}`,
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
	resolveRequests  []DirectoryResolveRequest
	resolveResponse  DirectoryResolution
	resolveErr       error
	resolveFunc      func(DirectoryResolveRequest) (DirectoryResolution, error)
	listRequest      DirectoryListRequest
	listResponse     DirectoryPage
	listErr          error
	subscribeRequest DirectorySubscribeRequest
	subscription     *DirectorySubscription
	subscribeErr     error
}

func (p *fakeDirectoryProvider) Resolve(_ context.Context, request DirectoryResolveRequest) (DirectoryResolution, error) {
	p.resolveRequest = request
	p.resolveRequests = append(p.resolveRequests, request)
	if p.resolveFunc != nil {
		return p.resolveFunc(request)
	}
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
