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

func TestCABIHostBindingTransportProjectsCodecAndHash(t *testing.T) {
	libraryPath := buildFakeCABIHostBindingLibrary(t)
	client, transport, err := NewCABIHostBindingClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABIHostBindingClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI host binding transport: %v", err)
		}
	}()

	binding, err := client.BuildHostStreamBinding(context.Background(), HostStreamBindingRequest{
		BindingID:     "binding-weather-1",
		DescriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
		Endpoint:      "/tmp/easynet-weather.sock",
		FrameSchema:   hostStreamFrameSchema,
		Metadata:      map[string]any{"source": "fixture"},
	})
	if err != nil {
		t.Fatalf("BuildHostStreamBinding: %v", err)
	}
	if binding.Lifecycle["frame_contract_owner"] != "daemon_sdk" ||
		binding.Metadata["hash_algorithm"] != hostStreamHashAlgorithm {
		t.Fatalf("binding projection = %#v", binding)
	}

	request, err := client.DecodeRequest(context.Background(), HostStreamEnvelope{
		Request: HostStreamEnvelopeRequest{
			Fn:     "weather.stream",
			Args:   map[string]any{"city": "Singapore"},
			CallID: "call-weather-1",
			Caller: "easynet:///r/example/user/alice",
		},
	})
	if err != nil {
		t.Fatalf("DecodeRequest: %v", err)
	}
	if request.Function != "weather.stream" || request.Metadata["wire"] != "host_stream_request_v1" {
		t.Fatalf("request projection = %#v", request)
	}

	item, err := client.EncodeItem(context.Background(), 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("EncodeItem: %v", err)
	}
	if item.FrameType != "item" || item.Seq == nil || *item.Seq != 0 {
		t.Fatalf("item frame = %#v", item)
	}

	errorFrame, err := client.EncodeError(context.Background(), &SDKError{
		Code:    ErrInvalidArgument,
		Stage:   "host",
		Retry:   RetryNever,
		Message: "bad input",
		Details: map[string]any{},
	})
	if err != nil {
		t.Fatalf("EncodeError: %v", err)
	}
	if errorFrame.FrameType != "error" || errorFrame.Error == nil || errorFrame.Error.Code != ErrInvalidArgument {
		t.Fatalf("error frame = %#v", errorFrame)
	}

	state := HostStreamHashState{
		Algorithm:  hostStreamHashAlgorithm,
		OutputHash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
		Frames:     0,
	}
	folded, err := client.FoldOutputHash(context.Background(), state, 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("FoldOutputHash: %v", err)
	}
	if folded.OutputHash != "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15" ||
		folded.LastSeq == nil || *folded.LastSeq != 0 || folded.CanonicalJSON != `{"token":"hello"}` {
		t.Fatalf("folded hash = %#v", folded)
	}

	terminal, err := client.EncodeTerminal(context.Background(), HostStreamTerminalSummary{
		OutputHash: folded.OutputHash,
		Frames:     folded.Frames,
		Metadata:   map[string]any{"canonical_json": folded.CanonicalJSON},
	})
	if err != nil {
		t.Fatalf("EncodeTerminal: %v", err)
	}
	if terminal.FrameType != "terminal" || terminal.OutputHash == nil || *terminal.OutputHash != folded.OutputHash {
		t.Fatalf("terminal frame = %#v", terminal)
	}
}

func TestCABIHostBindingTransportRejectsClosedUse(t *testing.T) {
	libraryPath := buildFakeCABIHostBindingLibrary(t)
	client, transport, err := NewCABIHostBindingClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABIHostBindingClient: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}

	_, err = client.EncodeItem(context.Background(), 0, map[string]any{"token": "hello"})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("EncodeItem after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func buildFakeCABIHostBindingLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_host_binding.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIHostBindingSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI host binding source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI host binding library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIHostBindingSource = `
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
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI host binding error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 808;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_host_binding_build(uint64_t handle, const char *request_json, char **out_binding_json) {
	(void)handle;
	if (strstr(request_json, "binding-weather-1") == 0) return 10;
	*out_binding_json = dup_json("{\"binding_id\":\"binding-weather-1\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0\",\"endpoint\":\"/tmp/easynet-weather.sock\",\"frame_schema\":\"host-stream-frame.schema.json\",\"cleanup\":{},\"timeout_ms\":null,\"readiness\":{\"state\":\"declared\",\"checked\":false,\"endpoint_ready\":null},\"lifecycle\":{\"endpoint_owner\":\"product_host\",\"process_owner\":\"product_host\",\"frame_contract_owner\":\"daemon_sdk\"},\"metadata\":{\"profile\":\"host_binding\",\"frame_schema\":\"host-stream-frame.schema.json\",\"hash_algorithm\":\"sha256(prev_hash || seq_be || canonical_json(value))\"}}");
	return 0;
}
int32_t easynet_host_binding_decode_request(uint64_t handle, const char *envelope_json, char **out_request_json) {
	(void)handle;
	if (strstr(envelope_json, "weather.stream") == 0) return 10;
	*out_request_json = dup_json("{\"function\":\"weather.stream\",\"args\":{\"city\":\"Singapore\"},\"call_id\":\"call-weather-1\",\"caller\":\"easynet:///r/example/user/alice\",\"metadata\":{\"wire\":\"host_stream_request_v1\",\"frame_contract_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_host_binding_encode_item(uint64_t handle, const char *item_json, char **out_frame_json) {
	(void)handle;
	if (strstr(item_json, "\"seq\":0") == 0) return 10;
	*out_frame_json = dup_json("{\"frame_type\":\"item\",\"seq\":0,\"value\":{\"token\":\"hello\"},\"error\":null,\"terminal\":null,\"output_hash\":null}");
	return 0;
}
int32_t easynet_host_binding_encode_error(uint64_t handle, const char *error_json, char **out_frame_json) {
	(void)handle;
	if (strstr(error_json, "bad input") == 0) return 10;
	*out_frame_json = dup_json("{\"frame_type\":\"error\",\"seq\":null,\"value\":null,\"error\":{\"code\":\"INVALID_ARGUMENT\",\"stage\":\"host\",\"message\":\"bad input\",\"retry\":\"never\",\"details\":{}},\"terminal\":null,\"output_hash\":null}");
	return 0;
}
int32_t easynet_host_binding_encode_terminal(uint64_t handle, const char *terminal_json, char **out_frame_json) {
	(void)handle;
	if (strstr(terminal_json, "8196e03ca122") == 0) return 10;
	*out_frame_json = dup_json("{\"frame_type\":\"terminal\",\"seq\":1,\"value\":null,\"error\":null,\"terminal\":{\"output_hash\":\"sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15\",\"frames\":1,\"metadata\":{\"canonical_json\":\"{\\\"token\\\":\\\"hello\\\"}\"}},\"output_hash\":\"sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15\"}");
	return 0;
}
int32_t easynet_host_binding_fold_output_hash(uint64_t handle, const char *fold_json, char **out_state_json) {
	(void)handle;
	if (strstr(fold_json, "\"seq\":0") == 0) return 10;
	*out_state_json = dup_json("{\"algorithm\":\"sha256(prev_hash || seq_be || canonical_json(value))\",\"output_hash\":\"sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15\",\"frames\":1,\"last_seq\":0,\"canonical_json\":\"{\\\"token\\\":\\\"hello\\\"}\"}");
	return 0;
}
`
