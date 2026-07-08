package easynet

import (
	"context"
	"testing"
)

func TestSurfaceRuntimeTransportBuildsInvocationThroughIdentity(t *testing.T) {
	identityTransport := newSurfaceRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: surfaceRuntimePagePageRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeSurfaceClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeSurfaceClient: %v", err)
	}

	draft, err := client.BuildCreatePageInvocation(context.Background(), SurfaceCreatePageRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		ProjectID:          "docs",
		Folder:             "/tmp/docs",
		Visibility:         "public",
	})
	if err != nil {
		t.Fatalf("BuildCreatePageInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0" {
		t.Fatalf("descriptor ref = %q", draft.DescriptorRef())
	}
	args := draft.JSONArgs().(map[string]any)
	if args["project_id"] != "docs" || args["folder"] != "/tmp/docs" || args["visibility"] != "public" {
		t.Fatalf("surface args not normalized: %#v", args)
	}
	if _, ok := args["caller_ura"]; ok {
		t.Fatalf("carrier leaked into args: %#v", args)
	}
	metadata := draft.Metadata()
	if metadata["profile"] != surfaceProfile ||
		metadata["system_ability"] != surfaceAbilityCreatePage ||
		metadata["carrier_owner"] != "daemon_sdk" {
		t.Fatalf("metadata not normalized: %#v", metadata)
	}
	if len(identityTransport.seenBuildURA) != 1 || identityTransport.seenBuildURA[0]["ability_name"] != surfaceAbilityCreatePage {
		t.Fatalf("ability URA was not delegated through identity client: %#v", identityTransport.seenBuildURA)
	}
	if len(identityTransport.seenBuildDescriptor) != 1 ||
		identityTransport.seenBuildDescriptor[0]["ability_ura"] != "easynet:///r/example/ability/alice.pages.pages.publish" ||
		identityTransport.seenBuildDescriptor[0]["descriptor_version"] != "1.0.0" {
		t.Fatalf("descriptor ref was not delegated through identity client: %#v", identityTransport.seenBuildDescriptor)
	}
}

func TestSurfaceRuntimeTransportInvokesAndProjectsRawPagesOutput(t *testing.T) {
	identityTransport := newSurfaceRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: surfaceRuntimePagePageRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeSurfaceClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeSurfaceClient: %v", err)
	}

	page, err := client.ListPages(context.Background(), SurfaceListPagesRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		Limit:              2,
	})
	if err != nil {
		t.Fatalf("ListPages: %v", err)
	}
	if len(page.Items) != 2 || page.Items[0].PageID != "docs" || page.Items[1].PageID != "blog" {
		t.Fatalf("unexpected projected page: %#v", page)
	}
	if page.Items[0].OwnerURA != "easynet:///r/example/agent/alice.pages" ||
		page.Items[0].SurfaceRef != "easynet:///r/example/resource/alice.docs" {
		t.Fatalf("owner/surface refs not projected: %#v", page.Items[0])
	}
	if page.Items[1].OwnerURA != "easynet:///r/example/agent/alice.pages" ||
		page.Items[1].SurfaceRef != "easynet:///r/example/resource/agent.alice.pages/blog" {
		t.Fatalf("missing surface_ref was not projected through identity: %#v", page.Items[1])
	}
	if page.NextCursor != nil {
		t.Fatalf("next cursor = %#v, want nil", page.NextCursor)
	}
	if len(identityTransport.seenBuildURA) != 2 ||
		identityTransport.seenBuildURA[0]["ability_name"] != surfaceAbilityListPages ||
		identityTransport.seenBuildURA[1]["kind"] != "resource" ||
		identityTransport.seenBuildURA[1]["owner_ura"] != "easynet:///r/example/agent/alice.pages" ||
		identityTransport.seenBuildURA[1]["path"] != "blog" {
		t.Fatalf("missing surface_ref did not delegate to identity BuildURA: %#v", identityTransport.seenBuildURA)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if len(args) != 0 {
		t.Fatalf("project_list should not receive carrier args: %#v", args)
	}

	runtimeTransport.outputJSON = surfaceRuntimePageRecordRawJSON
	record, err := client.CreatePage(context.Background(), SurfaceCreatePageRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		ProjectID:          "docs",
		Folder:             "/tmp/docs",
		Visibility:         "public",
	})
	if err != nil {
		t.Fatalf("CreatePage: %v", err)
	}
	if record.PublicRef == nil || *record.PublicRef != "https://example/web/alice/docs/" || record.Status == nil || *record.Status != "published" {
		t.Fatalf("unexpected projected record: %#v", record)
	}

	runtimeTransport.outputJSON = surfaceRuntimeDeleteRawJSON
	result, err := client.DeletePage(context.Background(), SurfaceDeletePageRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		ProjectID:          "docs",
	})
	if err != nil {
		t.Fatalf("DeletePage: %v", err)
	}
	if !result.Removed || result.PageID != "docs" || result.State != "deleted" {
		t.Fatalf("unexpected delete projection: %#v", result)
	}
}

func TestSurfaceRuntimeTransportProjectsHealthDescriptorFromIdentityBuiltInvocation(t *testing.T) {
	identityTransport := newSurfaceRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: surfaceRuntimeHealthRawJSONWithoutDescriptor}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeSurfaceClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeSurfaceClient: %v", err)
	}

	health, err := client.SurfaceHealth(context.Background(), SurfaceHealthRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		ProjectID:          "docs",
	})
	if err != nil {
		t.Fatalf("SurfaceHealth: %v", err)
	}
	if health.DescriptorRef != "easynet:///r/example/ability/alice.pages.pages.health@1.0.0" {
		t.Fatalf("descriptor ref = %q", health.DescriptorRef)
	}
	if len(identityTransport.seenBuildURA) != 1 ||
		identityTransport.seenBuildURA[0]["ability_name"] != surfaceAbilityHealth {
		t.Fatalf("ability URA was not delegated through identity client: %#v", identityTransport.seenBuildURA)
	}
	if len(identityTransport.seenBuildDescriptor) != 1 ||
		identityTransport.seenBuildDescriptor[0]["ability_ura"] != "easynet:///r/example/ability/alice.pages.pages.health" ||
		identityTransport.seenBuildDescriptor[0]["descriptor_version"] != "1.0.0" {
		t.Fatalf("descriptor ref was not delegated through identity client: %#v", identityTransport.seenBuildDescriptor)
	}
}

func TestSurfaceRuntimeTransportMapsTerminalFailure(t *testing.T) {
	identity, err := NewIdentityClient(newSurfaceRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{fail: true})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeSurfaceClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeSurfaceClient: %v", err)
	}

	_, err = client.ListPages(context.Background(), SurfaceListPagesRequest{SurfaceCarrierBase: surfaceBaseForTest()})
	if err == nil {
		t.Fatal("ListPages succeeded, want failure")
	}
	if !IsCode(err, ErrAdmissionDenied) {
		t.Fatalf("error code = %v, want %s", err, ErrAdmissionDenied)
	}
}

func newSurfaceRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			surfaceAbilityListPages:  "easynet:///r/example/ability/alice.pages.project_list",
			surfaceAbilityCreatePage: "easynet:///r/example/ability/alice.pages.pages.publish",
			surfaceAbilityDeletePage: "easynet:///r/example/ability/alice.pages.pages.unpublish",
			surfaceAbilityManifest:   "easynet:///r/example/ability/alice.pages.pages.get",
			surfaceAbilityHealth:     "easynet:///r/example/ability/alice.pages.pages.health",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/alice.pages.project_list":    "easynet:///r/example/ability/alice.pages.project_list@1.0.0",
			"easynet:///r/example/ability/alice.pages.pages.publish":   "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0",
			"easynet:///r/example/ability/alice.pages.pages.unpublish": "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0",
			"easynet:///r/example/ability/alice.pages.pages.get":       "easynet:///r/example/ability/alice.pages.pages.get@1.0.0",
			"easynet:///r/example/ability/alice.pages.pages.health":    "easynet:///r/example/ability/alice.pages.pages.health@1.0.0",
		},
		resourceByOwnerPath: map[string]string{
			"easynet:///r/example/agent/alice.pages\nblog": "easynet:///r/example/resource/agent.alice.pages/blog",
		},
		descriptorProjection: identityDescriptorProjectionJSON,
	}
}

const surfaceRuntimePageRecordRawJSON = `{
	"user":"alice",
	"project_id":"docs",
	"project_ura":"easynet:///r/example/resource/alice.docs",
	"url_root":"https://example/web/alice/docs/",
	"visibility":"public",
	"folder":"/tmp/docs"
}`

const surfaceRuntimePagePageRawJSON = `{
	"projects":[
		{
			"user":"alice",
			"project_id":"docs",
			"project_ura":"easynet:///r/example/resource/alice.docs",
			"url_root":"https://example/web/alice/docs/",
			"visibility":"public",
			"folder":"/tmp/docs"
		},
		{
			"user":"alice",
			"project_id":"blog",
			"url_root":"https://example/web/alice/blog/",
			"visibility":"public",
			"folder":"/tmp/blog"
		}
	]
}`

const surfaceRuntimeDeleteRawJSON = `{"project_id":"docs","removed":true}`

const surfaceRuntimeHealthRawJSONWithoutDescriptor = `{
	"owner_ura":"easynet:///r/example/agent/alice.pages",
	"surface_ref":"easynet:///r/example/resource/alice.docs",
	"descriptor_version":"1.0.0",
	"state":"ready",
	"ready":true,
	"page_count":2,
	"checks":[{"name":"pages","ready":true}]
}`
