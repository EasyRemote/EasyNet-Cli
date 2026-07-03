package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memorySurfaceTransport struct {
	listInvocation     string
	createInvocation   string
	deleteInvocation   string
	manifestInvocation string
	pagePage           string
	pageRecord         string
	manifest           string
	publicRef          string
	mutationResult     string
	seen               map[string]map[string]any
}

func (m *memorySurfaceTransport) remember(name string, requestJSON []byte) {
	if m.seen == nil {
		m.seen = map[string]map[string]any{}
	}
	var decoded map[string]any
	_ = json.Unmarshal(requestJSON, &decoded)
	m.seen[name] = decoded
}

func (m *memorySurfaceTransport) BuildListPagesInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_list", requestJSON)
	return []byte(m.listInvocation), nil
}

func (m *memorySurfaceTransport) BuildCreatePageInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_create", requestJSON)
	return []byte(m.createInvocation), nil
}

func (m *memorySurfaceTransport) BuildDeletePageInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_delete", requestJSON)
	return []byte(m.deleteInvocation), nil
}

func (m *memorySurfaceTransport) BuildManifestInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_manifest", requestJSON)
	return []byte(m.manifestInvocation), nil
}

func (m *memorySurfaceTransport) ListPages(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("list_pages", requestJSON)
	return []byte(m.pagePage), nil
}

func (m *memorySurfaceTransport) CreatePage(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("create_page", requestJSON)
	return []byte(m.pageRecord), nil
}

func (m *memorySurfaceTransport) DeletePage(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("delete_page", requestJSON)
	return []byte(m.mutationResult), nil
}

func (m *memorySurfaceTransport) SurfaceManifest(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("surface_manifest", requestJSON)
	return []byte(m.manifest), nil
}

func (m *memorySurfaceTransport) PublicPageRef(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("public_page_ref", requestJSON)
	return []byte(m.publicRef), nil
}

func surfaceBaseForTest() SurfaceCarrierBase {
	return SurfaceCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/agent/alice.pages",
		SubjectURA:        "easynet:///r/example/agent/alice.pages",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "surface-list-1"},
	}
}

func TestSurfaceBuildsPageInvocations(t *testing.T) {
	transport := &memorySurfaceTransport{
		listInvocation:     surfaceListInvocationJSON,
		createInvocation:   surfaceCreateInvocationJSON,
		deleteInvocation:   surfaceDeleteInvocationJSON,
		manifestInvocation: surfaceManifestInvocationJSON,
	}
	client, err := NewSurfaceClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	listDraft, err := client.BuildListPagesInvocation(context.Background(), SurfaceListPagesRequest{SurfaceCarrierBase: surfaceBaseForTest(), Limit: 50})
	if err != nil {
		t.Fatal(err)
	}
	if listDraft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.list@1.0.0" {
		t.Fatalf("list descriptor = %q", listDraft.DescriptorRef())
	}

	createDraft, err := client.BuildCreatePageInvocation(context.Background(), SurfaceCreatePageRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		ProjectID:          "docs",
		Folder:             "/tmp/easynet-pages-docs",
		Visibility:         "public",
	})
	if err != nil {
		t.Fatal(err)
	}
	if createDraft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0" {
		t.Fatalf("create descriptor = %q", createDraft.DescriptorRef())
	}
	if transport.seen["build_create"]["project_id"] != "docs" {
		t.Fatalf("project_id not preserved: %#v", transport.seen["build_create"])
	}

	deleteDraft, err := client.BuildDeletePageInvocation(context.Background(), SurfaceDeletePageRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs"})
	if err != nil {
		t.Fatal(err)
	}
	if deleteDraft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0" {
		t.Fatalf("delete descriptor = %q", deleteDraft.DescriptorRef())
	}

	manifestDraft, err := client.BuildManifestInvocation(context.Background(), SurfaceManifestRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs"})
	if err != nil {
		t.Fatal(err)
	}
	if manifestDraft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.get@1.0.0" {
		t.Fatalf("manifest descriptor = %q", manifestDraft.DescriptorRef())
	}
}

func TestSurfaceProjectsPagesManifestRefAndMutation(t *testing.T) {
	transport := &memorySurfaceTransport{
		pagePage:       surfacePagePageJSON,
		pageRecord:     surfacePageRecordJSON,
		manifest:       surfaceManifestJSON,
		publicRef:      surfacePublicPageRefJSON,
		mutationResult: surfaceMutationResultJSON,
	}
	client, err := NewSurfaceClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	page, err := client.ListPages(context.Background(), SurfaceListPagesRequest{SurfaceCarrierBase: surfaceBaseForTest(), Limit: 50})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].PageID != "docs" || page.Source != "pages_read_model" {
		t.Fatalf("unexpected page page: %#v", page)
	}

	record, err := client.CreatePage(context.Background(), SurfaceCreatePageRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs", Folder: "/tmp/easynet-pages-docs", Visibility: "public"})
	if err != nil {
		t.Fatal(err)
	}
	if record.PageID != "docs" || record.PublicRef == nil {
		t.Fatalf("unexpected page record: %#v", record)
	}

	manifest, err := client.SurfaceManifest(context.Background(), SurfaceManifestRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs"})
	if err != nil {
		t.Fatal(err)
	}
	if manifest.Kind != "surface_manifest" || manifest.Page.PageID != "docs" {
		t.Fatalf("unexpected manifest: %#v", manifest)
	}

	ref, err := client.PublicPageRef(context.Background(), SurfacePublicPageRefRequest{Page: record})
	if err != nil {
		t.Fatal(err)
	}
	if ref.RouteKind != "hub_web" || ref.PublicRef == "" {
		t.Fatalf("unexpected public ref: %#v", ref)
	}

	result, err := client.DeletePage(context.Background(), SurfaceDeletePageRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs"})
	if err != nil {
		t.Fatal(err)
	}
	if !result.Removed || result.State != "deleted" {
		t.Fatalf("unexpected mutation result: %#v", result)
	}
}

func TestSurfaceRejectsInvalidRequests(t *testing.T) {
	client, err := NewSurfaceClient(&memorySurfaceTransport{createInvocation: surfaceCreateInvocationJSON})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.BuildCreatePageInvocation(context.Background(), SurfaceCreatePageRequest{ProjectID: "docs", Folder: "/tmp/pages"}); err == nil {
		t.Fatal("expected incomplete carrier rejection")
	}
	if _, err := client.BuildCreatePageInvocation(context.Background(), SurfaceCreatePageRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "../docs", Folder: "/tmp/pages"}); err == nil {
		t.Fatal("expected invalid project_id rejection")
	}
	if _, err := client.BuildCreatePageInvocation(context.Background(), SurfaceCreatePageRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs", Folder: "relative"}); err == nil {
		t.Fatal("expected relative folder rejection")
	}
	if _, err := client.ListPages(context.Background(), SurfaceListPagesRequest{SurfaceCarrierBase: surfaceBaseForTest(), Limit: MaxSurfacePageSize + 1}); err == nil {
		t.Fatal("expected page limit rejection")
	}
}

const surfaceListInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.list@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-list-1", "profile": "surface", "system_ability": "pages.list", "carrier_owner": "daemon_sdk"}
}`

const surfaceCreateInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"project_id": "docs", "folder": "/tmp/easynet-pages-docs", "visibility": "public"},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-create-1", "profile": "surface", "system_ability": "pages.publish", "carrier_owner": "daemon_sdk"}
}`

const surfaceDeleteInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"project_id": "docs"},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-delete-1", "profile": "surface", "system_ability": "pages.unpublish", "carrier_owner": "daemon_sdk"}
}`

const surfaceManifestInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.get@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"project_id": "docs"},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-manifest-1", "profile": "surface", "system_ability": "pages.get", "carrier_owner": "daemon_sdk"}
}`

const surfacePageRecordJSON = `{
  "profile": "surface",
  "kind": "page_record",
  "page_id": "docs",
  "owner_ura": "easynet:///r/example/agent/alice.pages",
  "surface_ref": "easynet:///r/example/resource/alice.docs",
  "public_ref": "https://example/web/alice/docs/",
  "status": "published",
  "metadata": {"profile": "surface", "source_ability": "pages.get", "user": "alice", "project_id": "docs", "visibility": "public"}
}`

const surfacePagePageJSON = `{
  "profile": "surface",
  "kind": "surface_page_page",
  "item_kind": "page_record",
  "items": [` + surfacePageRecordJSON + `],
  "next_cursor": null,
  "limit": 50,
  "source": "pages_read_model",
  "metadata": {"profile": "surface", "source_ability": "pages.list", "page_size_default": 50, "page_size_max": 500, "total_available": 1}
}`

const surfacePublicPageRefJSON = `{
  "profile": "surface",
  "kind": "public_page_ref",
  "page_id": "docs",
  "owner_ura": "easynet:///r/example/agent/alice.pages",
  "surface_ref": "easynet:///r/example/resource/alice.docs",
  "public_ref": "https://example/web/alice/docs/",
  "route_kind": "hub_web",
  "metadata": {"profile": "surface", "source_ability": "pages.get"}
}`

const surfaceManifestJSON = `{
  "profile": "surface",
  "kind": "surface_manifest",
  "page_id": "docs",
  "owner_ura": "easynet:///r/example/agent/alice.pages",
  "surface_ref": "easynet:///r/example/resource/alice.docs",
  "public_ref": "https://example/web/alice/docs/",
  "page": ` + surfacePageRecordJSON + `,
  "entrypoint": {"kind": "public_page_ref", "href": "https://example/web/alice/docs/"},
  "metadata": {"profile": "surface", "source_ability": "pages.get"}
}`

const surfaceMutationResultJSON = `{
  "profile": "surface",
  "kind": "surface_mutation_result",
  "operation": "delete",
  "page_id": "docs",
  "removed": true,
  "state": "deleted",
  "metadata": {"profile": "surface", "source_ability": "pages.unpublish"}
}`
