//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

func TestCABISurfaceTransportBuildsInvokesAndProjects(t *testing.T) {
	libraryPath := buildFakeCABISurfaceLibrary(t)
	client, transport, err := NewCABISurfaceClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABISurfaceClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI surface transport: %v", err)
		}
	}()

	draft, err := client.BuildCreatePageInvocation(context.Background(), SurfaceCreatePageRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		ProjectID:          "docs",
		Folder:             "/tmp/easynet-pages-docs",
		Visibility:         "public",
	})
	if err != nil {
		t.Fatalf("BuildCreatePageInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0" {
		t.Fatalf("create descriptor_ref = %q", draft.DescriptorRef())
	}

	page, err := client.ListPages(context.Background(), SurfaceListPagesRequest{SurfaceCarrierBase: surfaceBaseForTest(), Limit: 50})
	if err != nil {
		t.Fatalf("ListPages: %v", err)
	}
	if len(page.Items) != 1 || page.Items[0].PageID != "docs" {
		t.Fatalf("page = %#v", page)
	}

	record, err := client.CreatePage(context.Background(), SurfaceCreatePageRequest{
		SurfaceCarrierBase: surfaceBaseForTest(),
		ProjectID:          "docs",
		Folder:             "/tmp/easynet-pages-docs",
		Visibility:         "public",
	})
	if err != nil {
		t.Fatalf("CreatePage: %v", err)
	}
	if record.PageID != "docs" || record.PublicRef == nil {
		t.Fatalf("record = %#v", record)
	}

	manifest, err := client.SurfaceManifest(context.Background(), SurfaceManifestRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs"})
	if err != nil {
		t.Fatalf("SurfaceManifest: %v", err)
	}
	if manifest.Page.PageID != "docs" || manifest.Entrypoint == nil {
		t.Fatalf("manifest = %#v", manifest)
	}

	publicRef, err := client.PublicPageRef(context.Background(), SurfacePublicPageRefRequest{Page: record})
	if err != nil {
		t.Fatalf("PublicPageRef: %v", err)
	}
	if publicRef.RouteKind != "hub_web" || publicRef.PublicRef == "" {
		t.Fatalf("public ref = %#v", publicRef)
	}

	health, err := client.SurfaceHealth(context.Background(), SurfaceHealthRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs"})
	if err != nil {
		t.Fatalf("SurfaceHealth: %v", err)
	}
	if !health.Ready || health.PageCount != 1 || len(health.Checks) != 2 {
		t.Fatalf("health = %#v", health)
	}

	deleted, err := client.DeletePage(context.Background(), SurfaceDeletePageRequest{SurfaceCarrierBase: surfaceBaseForTest(), ProjectID: "docs"})
	if err != nil {
		t.Fatalf("DeletePage: %v", err)
	}
	if !deleted.Removed || deleted.State != "deleted" {
		t.Fatalf("delete result = %#v", deleted)
	}
}

func TestCABISurfaceTransportRejectsClosedUse(t *testing.T) {
	libraryPath := buildFakeCABISurfaceLibrary(t)
	client, transport, err := NewCABISurfaceClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABISurfaceClient: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := client.BuildListPagesInvocation(context.Background(), SurfaceListPagesRequest{SurfaceCarrierBase: surfaceBaseForTest(), Limit: 50}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("BuildListPagesInvocation after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func buildFakeCABISurfaceLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_surface.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABISurfaceSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI surface source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI surface library: %v\n%s", err, out)
	}
	return output
}

const fakeCABISurfaceSource = `
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

static char *dup_json(const char *s) {
	size_t n = strlen(s);
	char *out = (char *)malloc(n + 1);
	if (out == 0) return 0;
	memcpy(out, s, n + 1);
	return out;
}

uint32_t easynet_abi_version(void) { return 4u; }
void easynet_string_free(char *s) { free(s); }
int32_t easynet_last_error_json(char **out_error_json) {
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI surface error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 1101;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "pages.list") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"pages\":[{\"page_id\":\"docs\"}]},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "pages.publish") != 0 || strstr(invocation_json, "pages.get") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"page_id\":\"docs\"},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "pages.unpublish") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"page_id\":\"docs\",\"removed\":true},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "pages.health") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"page_count\":1},\"error\":null}");
		return 0;
	}
	return 10;
}
int32_t easynet_surface_build_list_pages_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/agent/alice.pages\",\"descriptor_ref\":\"easynet:///r/example/ability/alice.pages.pages.list@1.0.0\",\"subject_ura\":\"easynet:///r/example/agent/alice.pages\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"surface\",\"system_ability\":\"pages.list\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_surface_build_create_page_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "docs") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/agent/alice.pages\",\"descriptor_ref\":\"easynet:///r/example/ability/alice.pages.pages.publish@1.0.0\",\"subject_ura\":\"easynet:///r/example/agent/alice.pages\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"project_id\":\"docs\",\"folder\":\"/tmp/easynet-pages-docs\",\"visibility\":\"public\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"surface\",\"system_ability\":\"pages.publish\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_surface_build_delete_page_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/agent/alice.pages\",\"descriptor_ref\":\"easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0\",\"subject_ura\":\"easynet:///r/example/agent/alice.pages\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"project_id\":\"docs\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"surface\",\"system_ability\":\"pages.unpublish\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_surface_build_manifest_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/agent/alice.pages\",\"descriptor_ref\":\"easynet:///r/example/ability/alice.pages.pages.get@1.0.0\",\"subject_ura\":\"easynet:///r/example/agent/alice.pages\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"project_id\":\"docs\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"surface\",\"system_ability\":\"pages.get\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_surface_build_health_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/agent/alice.pages\",\"descriptor_ref\":\"easynet:///r/example/ability/alice.pages.pages.health@1.0.0\",\"subject_ura\":\"easynet:///r/example/agent/alice.pages\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"project_id\":\"docs\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"surface\",\"system_ability\":\"pages.health\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_surface_project_page_record(uint64_t handle, const char *page_json, char **out_page_json) {
	(void)handle; (void)page_json;
	*out_page_json = dup_json("{\"profile\":\"surface\",\"kind\":\"page_record\",\"page_id\":\"docs\",\"owner_ura\":\"easynet:///r/example/agent/alice.pages\",\"surface_ref\":\"easynet:///r/example/resource/alice.docs\",\"public_ref\":\"https://example/web/alice/docs/\",\"status\":\"published\",\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.get\",\"user\":\"alice\",\"project_id\":\"docs\",\"visibility\":\"public\"}}");
	return 0;
}
int32_t easynet_surface_project_page_page(uint64_t handle, const char *pages_json, char **out_page_json) {
	(void)handle; (void)pages_json;
	*out_page_json = dup_json("{\"profile\":\"surface\",\"kind\":\"surface_page_page\",\"item_kind\":\"page_record\",\"items\":[{\"profile\":\"surface\",\"kind\":\"page_record\",\"page_id\":\"docs\",\"owner_ura\":\"easynet:///r/example/agent/alice.pages\",\"surface_ref\":\"easynet:///r/example/resource/alice.docs\",\"public_ref\":\"https://example/web/alice/docs/\",\"status\":\"published\",\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.list\"}}],\"next_cursor\":null,\"limit\":50,\"source\":\"pages_read_model\",\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.list\",\"total_available\":1}}");
	return 0;
}
int32_t easynet_surface_project_manifest(uint64_t handle, const char *page_json, char **out_manifest_json) {
	(void)handle; (void)page_json;
	*out_manifest_json = dup_json("{\"profile\":\"surface\",\"kind\":\"surface_manifest\",\"page_id\":\"docs\",\"owner_ura\":\"easynet:///r/example/agent/alice.pages\",\"surface_ref\":\"easynet:///r/example/resource/alice.docs\",\"public_ref\":\"https://example/web/alice/docs/\",\"page\":{\"profile\":\"surface\",\"kind\":\"page_record\",\"page_id\":\"docs\",\"owner_ura\":\"easynet:///r/example/agent/alice.pages\",\"surface_ref\":\"easynet:///r/example/resource/alice.docs\",\"public_ref\":\"https://example/web/alice/docs/\",\"status\":\"published\",\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.get\"}},\"entrypoint\":{\"kind\":\"public_page_ref\",\"href\":\"https://example/web/alice/docs/\"},\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.get\"}}");
	return 0;
}
int32_t easynet_surface_project_public_page_ref(uint64_t handle, const char *page_json, char **out_ref_json) {
	(void)handle; (void)page_json;
	*out_ref_json = dup_json("{\"profile\":\"surface\",\"kind\":\"public_page_ref\",\"page_id\":\"docs\",\"owner_ura\":\"easynet:///r/example/agent/alice.pages\",\"surface_ref\":\"easynet:///r/example/resource/alice.docs\",\"public_ref\":\"https://example/web/alice/docs/\",\"route_kind\":\"hub_web\",\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.get\"}}");
	return 0;
}
int32_t easynet_surface_project_mutation_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle; (void)result_json;
	*out_result_json = dup_json("{\"profile\":\"surface\",\"kind\":\"surface_mutation_result\",\"operation\":\"delete\",\"page_id\":\"docs\",\"removed\":true,\"state\":\"deleted\",\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.unpublish\"}}");
	return 0;
}
int32_t easynet_surface_project_health(uint64_t handle, const char *health_json, char **out_health_json) {
	(void)handle; (void)health_json;
	*out_health_json = dup_json("{\"profile\":\"surface\",\"kind\":\"surface_health\",\"state\":\"ready\",\"ready\":true,\"owner_ura\":\"easynet:///r/example/agent/alice.pages\",\"surface_ref\":\"easynet:///r/example/resource/alice.docs\",\"descriptor_ref\":\"easynet:///r/example/ability/alice.pages.pages.health@1.0.0\",\"descriptor_version\":\"1.0.0\",\"page_count\":1,\"checks\":[{\"name\":\"manifest\",\"state\":\"ready\",\"ready\":true,\"message\":null,\"latency_ms\":3,\"metadata\":{\"source\":\"pages.get\"}},{\"name\":\"public_ref\",\"state\":\"ready\",\"ready\":true,\"message\":null,\"latency_ms\":1,\"metadata\":{\"route_kind\":\"hub_web\"}}],\"metadata\":{\"profile\":\"surface\",\"source_ability\":\"pages.health\",\"rendering_owner\":\"backend\"}}");
	return 0;
}
`
