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

func TestCABIPublicationTransportEnablesAndDisablesAbilityImpl(t *testing.T) {
	libraryPath := buildFakeCABIPublicationLibrary(t)
	client, transport, err := NewCABIPublicationClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABIPublicationClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI publication transport: %v", err)
		}
	}()

	enabled, err := client.EnableAbilityImplWithRequest(context.Background(), baseAbilityImplLifecycleRequest())
	if err != nil {
		t.Fatalf("EnableAbilityImplWithRequest: %v", err)
	}
	if enabled.Kind != "ability_impl_enabled" || enabled.Status == nil || *enabled.Status != "enabled" {
		t.Fatalf("enabled record = %#v", enabled)
	}
	if enabled.Metadata["source_ability"] != publicationAbilityImplEnable {
		t.Fatalf("enabled metadata = %#v", enabled.Metadata)
	}

	disabled, err := client.DisableAbilityImplWithRequest(context.Background(), baseAbilityImplLifecycleRequest())
	if err != nil {
		t.Fatalf("DisableAbilityImplWithRequest: %v", err)
	}
	if disabled.Kind != "ability_impl_disabled" || disabled.Status == nil || *disabled.Status != "disabled" {
		t.Fatalf("disabled record = %#v", disabled)
	}
	if disabled.Metadata["source_ability"] != publicationAbilityImplDisable {
		t.Fatalf("disabled metadata = %#v", disabled.Metadata)
	}

	if err := client.EnableAbilityImpl(context.Background(), AbilityImplID{
		ImplID:     "impl-1",
		AbilityURA: "easynet:///r/example/ability/device.dev-a.er.weather",
	}); !IsCode(err, ErrGeneric) {
		t.Fatalf("minimal EnableAbilityImpl error = %v, want %s", err, ErrGeneric)
	}
}

func buildFakeCABIPublicationLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_publication.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIPublicationSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI publication source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI publication library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIPublicationSource = `
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
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI publication error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 707;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "ability.impl.enable") != 0 || strstr(invocation_json, "ability.impl.disable") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"terminal_state\":\"Completed\",\"output_json\":{\"ok\":true,\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.er.weather\",\"impl_id\":\"impl-1\",\"owner_ura\":\"easynet:///r/example/device/dev-a\",\"resource_ref\":\"easynet:///r/example/resource/device.dev-a/fs/tmp/weather\"},\"error\":null}");
		return 0;
	}
	*out_result_json = dup_json("{\"ok\":true,\"terminal_state\":\"Completed\",\"output_json\":{},\"error\":null}");
	return 0;
}
int32_t easynet_publication_build_resource_ref(uint64_t handle, const char *request_json, char **out_ref_json) {
	(void)handle; (void)request_json;
	*out_ref_json = dup_json("{\"resource_ura\":\"easynet:///r/example/resource/device.dev-a/fs/tmp/pkg\",\"owner_ura\":\"easynet:///r/example/device/dev-a\",\"namespace\":\"fs\",\"display_path\":\"tmp/pkg\",\"capability\":\"read\",\"expires_unix_ms\":4102444800000,\"revision\":\"fs-local-mapping-v1\"}");
	return 0;
}
int32_t easynet_publication_validate_package(uint64_t handle, const char *request_json, char **out_validation_json) {
	(void)handle; (void)request_json;
	*out_validation_json = dup_json("{\"profile\":\"publication\",\"kind\":\"package_validation\",\"valid\":true,\"package_path\":\"/tmp/pkg\",\"manifest_path\":\"/tmp/pkg/ability.json\",\"manifest_hash\":\"sha256:abc\",\"manifest\":{\"name\":\"weather\",\"namespace\":\"er\",\"wire_key\":\"er.weather\",\"descriptor_version\":\"1.0.0\",\"description\":\"\",\"exec_kind\":\"python\",\"input_schema\":{}},\"errors\":[],\"metadata\":{}}");
	return 0;
}
int32_t easynet_publication_install_plugin(uint64_t handle, const char *request_json, char **out_result_json) {
	(void)handle; (void)request_json;
	*out_result_json = dup_json("{\"profile\":\"publication\",\"kind\":\"plugin_install\",\"source\":\"file:///tmp/plugin\",\"install_id\":\"plugin@0.1.0\",\"status\":\"installed\",\"metadata\":{}}");
	return 0;
}
int32_t easynet_publication_build_deploy_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"metadata\":{\"profile\":\"publication\",\"system_ability\":\"ability.deploy\",\"carrier_owner\":\"daemon_sdk\"},\"args\":{},\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0\"}");
	return 0;
}
int32_t easynet_publication_project_deploy_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle; (void)result_json;
	*out_result_json = dup_json("{\"profile\":\"publication\",\"kind\":\"ability_deploy_result\",\"public_name\":\"weather\",\"namespace\":\"er\",\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.er.weather\",\"node_id\":\"local\",\"install_id\":\"install-1\",\"state\":\"enabled\",\"metadata\":{}}");
	return 0;
}
int32_t easynet_publication_build_list_abilities_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"metadata\":{\"profile\":\"publication\",\"system_ability\":\"meta.list_abilities\",\"carrier_owner\":\"daemon_sdk\"},\"args\":{},\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0\"}");
	return 0;
}
int32_t easynet_publication_project_ability_page(uint64_t handle, const char *page_json, char **out_page_json) {
	(void)handle; (void)page_json;
	*out_page_json = dup_json("{\"profile\":\"publication\",\"kind\":\"published_ability_page\",\"item_kind\":\"published_ability\",\"items\":[],\"next_cursor\":null,\"limit\":50,\"source\":\"read_model\",\"metadata\":{}}");
	return 0;
}
int32_t easynet_publication_build_show_ability_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"metadata\":{\"profile\":\"publication\",\"system_ability\":\"meta.list_abilities\",\"carrier_owner\":\"daemon_sdk\"},\"args\":{},\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0\"}");
	return 0;
}
int32_t easynet_publication_project_ability_record(uint64_t handle, const char *record_json, char **out_ability_json) {
	(void)handle; (void)record_json;
	*out_ability_json = dup_json("{\"descriptor\":{\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0\"},\"implementation\":{},\"metadata\":{}}");
	return 0;
}
int32_t easynet_publication_build_enable_ability_impl_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "caller_ura") == 0 || strstr(request_json, "impl-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.ability.impl.enable@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"impl_id\":\"impl-1\",\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.er.weather\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"publication\",\"system_ability\":\"ability.impl.enable\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_publication_project_enable_ability_impl_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle; (void)result_json;
	*out_result_json = dup_json("{\"profile\":\"publication\",\"kind\":\"ability_impl_enabled\",\"owner_ura\":\"easynet:///r/example/device/dev-a\",\"resource_ref\":\"easynet:///r/example/resource/device.dev-a/fs/tmp/weather\",\"status\":\"enabled\",\"metadata\":{\"profile\":\"publication\",\"source_ability\":\"ability.impl.enable\",\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.er.weather\",\"impl_id\":\"impl-1\"}}");
	return 0;
}
int32_t easynet_publication_build_disable_ability_impl_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "caller_ura") == 0 || strstr(request_json, "impl-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.ability.impl.disable@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"impl_id\":\"impl-1\",\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.er.weather\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"publication\",\"system_ability\":\"ability.impl.disable\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_publication_project_disable_ability_impl_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle; (void)result_json;
	*out_result_json = dup_json("{\"profile\":\"publication\",\"kind\":\"ability_impl_disabled\",\"owner_ura\":\"easynet:///r/example/device/dev-a\",\"resource_ref\":\"easynet:///r/example/resource/device.dev-a/fs/tmp/weather\",\"status\":\"disabled\",\"metadata\":{\"profile\":\"publication\",\"source_ability\":\"ability.impl.disable\",\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.er.weather\",\"impl_id\":\"impl-1\"}}");
	return 0;
}
int32_t easynet_publication_build_unpublish_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"metadata\":{\"profile\":\"publication\",\"system_ability\":\"ability.unpublish\",\"carrier_owner\":\"daemon_sdk\"},\"args\":{},\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0\"}");
	return 0;
}
int32_t easynet_publication_project_unpublish_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle; (void)result_json;
	*out_result_json = dup_json("{\"profile\":\"publication\",\"kind\":\"ability_unpublished\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0\",\"metadata\":{}}");
	return 0;
}
`
