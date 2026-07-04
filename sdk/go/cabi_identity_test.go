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

const cabiIdentityPublicKey = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="

func TestCABIIdentityTransportProjectsAddressingAndResourceRefs(t *testing.T) {
	libraryPath := buildFakeCABIIdentityLibrary(t)
	client, transport, err := NewCABIIdentityClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABIIdentityClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI identity transport: %v", err)
		}
	}()

	abilityURA, err := client.OwnerAbilityURA(context.Background(), "easynet:///r/example/device/dev-a", "observe.health")
	if err != nil {
		t.Fatalf("OwnerAbilityURA: %v", err)
	}
	if abilityURA != "easynet:///r/example/ability/device.dev-a.observe.health" {
		t.Fatalf("ability URA = %q", abilityURA)
	}
	descriptorRef, err := client.OwnerAbilityDescriptorRef(context.Background(), "easynet:///r/example/device/dev-a", "observe.health", "1.0.0")
	if err != nil {
		t.Fatalf("OwnerAbilityDescriptorRef: %v", err)
	}
	if descriptorRef != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" {
		t.Fatalf("descriptor ref = %q", descriptorRef)
	}
	projectedAbility, err := client.AbilityURAFromDescriptorRef(context.Background(), descriptorRef)
	if err != nil {
		t.Fatalf("AbilityURAFromDescriptorRef: %v", err)
	}
	if projectedAbility != abilityURA {
		t.Fatalf("projected ability = %q, want %q", projectedAbility, abilityURA)
	}
	resource, err := client.BuildResourceRef(context.Background(), LocalResourceRefRequest{
		Path:       "/tmp/easynet-weather-package",
		Capability: "read",
	})
	if err != nil {
		t.Fatalf("BuildResourceRef: %v", err)
	}
	if resource.ResourceURA == "" || resource.Capability != "read" || resource.Revision == "" {
		t.Fatalf("resource ref = %#v", resource)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := client.ProjectIdentity(context.Background(), IdentityProjectionRequest{URA: "easynet:///r/example/device/dev-a"}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ProjectIdentity after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestCABIIdentityTransportSigningKeyLifecycleAndSigner(t *testing.T) {
	libraryPath := buildFakeCABIIdentityLibrary(t)
	client, transport, err := NewCABIIdentityClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABIIdentityClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI identity transport: %v", err)
		}
	}()
	base := cabiIdentityCarrierBase()

	record, err := client.RegisterSigningKey(context.Background(), SigningKeyRegistrationRequest{
		IdentityCarrierBase: base,
		OwnerURA:            "easynet:///r/example/agent/alice.sdk",
		KeyID:               "alice-key-1",
		Algorithm:           "ed25519",
		PublicKeyBase64:     cabiIdentityPublicKey,
		Usage:               []string{"invocation.sign"},
	})
	if err != nil {
		t.Fatalf("RegisterSigningKey: %v", err)
	}
	if record.KeyID != "alice-key-1" || record.OwnerURA != "easynet:///r/example/agent/alice.sdk" || record.State != "active" {
		t.Fatalf("record = %#v", record)
	}
	page, err := client.ListSigningKeys(context.Background(), SigningKeyListRequest{
		IdentityCarrierBase: base,
		OwnerURA:            "easynet:///r/example/agent/alice.sdk",
		Limit:               1,
	})
	if err != nil {
		t.Fatalf("ListSigningKeys: %v", err)
	}
	if len(page.Items) != 1 || page.Items[0].KeyID != "alice-key-1" || page.Limit != 1 {
		t.Fatalf("page = %#v", page)
	}
	revoked, err := client.RevokeSigningKey(context.Background(), SigningKeyRevokeRequest{
		IdentityCarrierBase: base,
		OwnerURA:            "easynet:///r/example/agent/alice.sdk",
		KeyID:               "alice-key-1",
		PublicKeyBase64:     cabiIdentityPublicKey,
		Reason:              "rotation",
	})
	if err != nil {
		t.Fatalf("RevokeSigningKey: %v", err)
	}
	if !revoked.Revoked || revoked.KeyID != "alice-key-1" || revoked.State != "revoked" {
		t.Fatalf("revoked = %#v", revoked)
	}
	signer, err := client.Signer(context.Background(), SignerRequest{
		IdentityCarrierBase: base,
		OwnerURA:            "easynet:///r/example/agent/alice.sdk",
		KeyID:               "alice-key-1",
		Usage:               "invocation.sign",
	})
	if err != nil {
		t.Fatalf("Signer: %v", err)
	}
	if signer.SignerID != "signer-alice-key-1" || signer.KeyID != "alice-key-1" || signer.Algorithm != "ed25519" {
		t.Fatalf("signer = %#v", signer)
	}
}

func cabiIdentityCarrierBase() IdentityCarrierBase {
	return IdentityCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/agent/alice.sdk",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
	}
}

func buildFakeCABIIdentityLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_identity.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIIdentitySource), 0o600); err != nil {
		t.Fatalf("write fake C ABI identity source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI identity library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIIdentitySource = `
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
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI identity error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 919;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }

int32_t easynet_identity_project_ura(uint64_t handle, const char *ura, char **out_identity_json) {
	(void)handle;
	if (strstr(ura, "observe.health") != 0) {
		*out_identity_json = dup_json("{\"kind\":\"ability\",\"valid\":true,\"ura\":\"easynet:///r/example/ability/device.dev-a.observe.health\",\"profile\":\"directory_identity\",\"components\":{\"owner_ura\":\"easynet:///r/example/device/dev-a\"},\"metadata\":{}}");
		return 0;
	}
	*out_identity_json = dup_json("{\"kind\":\"device\",\"valid\":true,\"ura\":\"easynet:///r/example/device/dev-a\",\"profile\":\"directory_identity\",\"components\":{},\"metadata\":{}}");
	return 0;
}
int32_t easynet_identity_build_ura(uint64_t handle, const char *request_json, char **out_identity_json) {
	(void)handle;
	if (strstr(request_json, "observe.health") == 0) return 10;
	*out_identity_json = dup_json("{\"kind\":\"ability\",\"valid\":true,\"ura\":\"easynet:///r/example/ability/device.dev-a.observe.health\",\"profile\":\"directory_identity\",\"components\":{\"owner_ura\":\"easynet:///r/example/device/dev-a\"},\"metadata\":{}}");
	return 0;
}
int32_t easynet_identity_project_descriptor_ref(uint64_t handle, const char *descriptor_ref, char **out_descriptor_json) {
	(void)handle;
	if (strstr(descriptor_ref, "@1.0.0") == 0) return 10;
	*out_descriptor_json = dup_json("{\"kind\":\"descriptor_ref\",\"valid\":true,\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0\",\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.observe.health\",\"descriptor_version\":\"1.0.0\",\"profile\":\"directory_identity\",\"components\":{\"owner_ura\":\"easynet:///r/example/device/dev-a\"},\"metadata\":{}}");
	return 0;
}
int32_t easynet_identity_build_descriptor_ref(uint64_t handle, const char *request_json, char **out_descriptor_json) {
	(void)handle;
	if (strstr(request_json, "descriptor_version") == 0) return 10;
	*out_descriptor_json = dup_json("{\"kind\":\"descriptor_ref\",\"valid\":true,\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0\",\"ability_ura\":\"easynet:///r/example/ability/device.dev-a.observe.health\",\"descriptor_version\":\"1.0.0\",\"profile\":\"directory_identity\",\"components\":{\"owner_ura\":\"easynet:///r/example/device/dev-a\"},\"metadata\":{}}");
	return 0;
}
int32_t easynet_publication_build_resource_ref(uint64_t handle, const char *request_json, char **out_ref_json) {
	(void)handle;
	if (strstr(request_json, "easynet-weather-package") == 0) return 10;
	*out_ref_json = dup_json("{\"resource_ura\":\"easynet:///r/example/resource/local/weather-package\",\"owner_ura\":\"easynet:///r/example/agent/alice.sdk\",\"namespace\":\"local\",\"display_path\":\"/tmp/easynet-weather-package\",\"capability\":\"read\",\"expires_unix_ms\":1783100000123,\"revision\":\"sha256:abc\"}");
	return 0;
}

int32_t easynet_identity_build_register_signing_key_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "caller_ura") == 0 || strstr(request_json, "public_key_base64") == 0) return 10;
	*out_invocation_json = dup_json("{\"metadata\":{\"system_ability\":\"identity.register_pubkey\"},\"args\":{\"agent_ura\":\"easynet:///r/example/agent/alice.sdk\"}}");
	return 0;
}
int32_t easynet_identity_build_list_signing_keys_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "caller_ura") == 0 || strstr(request_json, "owner_ura") == 0) return 10;
	*out_invocation_json = dup_json("{\"metadata\":{\"system_ability\":\"identity.list_user_pubkeys\"},\"args\":{\"agent_ura\":\"easynet:///r/example/agent/alice.sdk\"}}");
	return 0;
}
int32_t easynet_identity_build_revoke_signing_key_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "public_key_base64") == 0 || strstr(request_json, "caller_ura") == 0) return 10;
	*out_invocation_json = dup_json("{\"metadata\":{\"system_ability\":\"identity.revoke_user_pubkey\"},\"args\":{\"agent_ura\":\"easynet:///r/example/agent/alice.sdk\"}}");
	return 0;
}
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "identity.register_pubkey") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"output_json\":{\"ok\":true}}");
		return 0;
	}
	if (strstr(invocation_json, "identity.list_user_pubkeys") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"output_json\":{\"agent_ura\":\"easynet:///r/example/agent/alice.sdk\",\"keys\":[{\"key_id\":\"alice-key-1\",\"public_key_b64\":\"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\",\"added_at_unix_ms\":1783100000123}],\"rotation_epoch\":3}}");
		return 0;
	}
	if (strstr(invocation_json, "identity.revoke_user_pubkey") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"output_json\":{\"ok\":true,\"removed\":true}}");
		return 0;
	}
	return 10;
}
int32_t easynet_identity_project_signing_key_record(uint64_t handle, const char *result_json, char **out_record_json) {
	(void)handle;
	if (strstr(result_json, "public_key_base64") == 0) return 10;
	*out_record_json = dup_json("{\"profile\":\"directory_identity\",\"key_id\":\"alice-key-1\",\"owner_ura\":\"easynet:///r/example/agent/alice.sdk\",\"algorithm\":\"ed25519\",\"public_key_base64\":\"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\",\"state\":\"active\",\"usage\":[\"invocation.sign\"],\"created_unix_ms\":1783100000123,\"metadata\":{\"source\":\"fake\"}}");
	return 0;
}
int32_t easynet_identity_project_signing_key_page(uint64_t handle, const char *result_json, char **out_page_json) {
	(void)handle;
	if (strstr(result_json, "keys") == 0) return 10;
	*out_page_json = dup_json("{\"profile\":\"directory_identity\",\"items\":[{\"profile\":\"directory_identity\",\"key_id\":\"alice-key-1\",\"owner_ura\":\"easynet:///r/example/agent/alice.sdk\",\"algorithm\":\"ed25519\",\"public_key_base64\":\"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\",\"state\":\"active\",\"usage\":[\"invocation.sign\"],\"created_unix_ms\":1783100000123,\"metadata\":{\"source\":\"fake\"}}],\"next_cursor\":null,\"limit\":1,\"metadata\":{\"source\":\"fake\"}}");
	return 0;
}
int32_t easynet_identity_project_signing_key_revoke_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle;
	if (strstr(result_json, "reason") == 0) return 10;
	*out_result_json = dup_json("{\"profile\":\"directory_identity\",\"key_id\":\"alice-key-1\",\"revoked\":true,\"state\":\"revoked\",\"metadata\":{\"source\":\"fake\"}}");
	return 0;
}
int32_t easynet_identity_project_signer_handle(uint64_t handle, const char *result_json, char **out_signer_json) {
	(void)handle;
	if (strstr(result_json, "invocation.sign") == 0 || strstr(result_json, "keys") == 0) return 10;
	*out_signer_json = dup_json("{\"profile\":\"directory_identity\",\"signer_id\":\"signer-alice-key-1\",\"owner_ura\":\"easynet:///r/example/agent/alice.sdk\",\"key_id\":\"alice-key-1\",\"algorithm\":\"ed25519\",\"policy\":{\"mode\":\"local_daemon_signing\",\"usage\":\"invocation.sign\",\"signer_id\":\"signer-alice-key-1\"},\"metadata\":{\"source\":\"fake\",\"public_key_base64\":\"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\"}}");
	return 0;
}
`
