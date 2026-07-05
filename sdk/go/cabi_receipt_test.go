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

func TestCABIReceiptTransportFetchUsesCarrierInvokeAndProjection(t *testing.T) {
	libraryPath := buildFakeCABIReceiptLibrary(t)
	transport, err := OpenCABIReceiptTransport(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("OpenCABIReceiptTransport: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI receipt transport: %v", err)
		}
	}()
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}

	summary, err := client.Fetch(context.Background(), baseReceiptFetchRequest())
	if err != nil {
		t.Fatalf("Fetch: %v", err)
	}

	if summary.InvocationID == nil || *summary.InvocationID != "inv-example-1" {
		t.Fatalf("invocation id = %#v, want inv-example-1", summary.InvocationID)
	}
	if summary.State != "completed" || summary.Verified {
		t.Fatalf("summary = %#v, want completed non-verifying projection", summary)
	}
	output, ok := summary.Output.(map[string]any)
	if !ok || output["ok"] != true {
		t.Fatalf("output = %#v, want ok=true object", summary.Output)
	}
}

func TestCABIReceiptTransportReadsHistoryAndTrace(t *testing.T) {
	libraryPath := buildFakeCABIReceiptLibrary(t)
	transport, err := OpenCABIReceiptTransport(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("OpenCABIReceiptTransport: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI receipt transport: %v", err)
		}
	}()
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}
	req := ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"key": map[string]any{"request_id": "inv-example-1"}},
	}

	draft, err := client.BuildListHistoryInvocation(context.Background(), req)
	if err != nil {
		t.Fatalf("BuildListHistoryInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0" {
		t.Fatalf("list descriptor ref = %q", draft.DescriptorRef())
	}
	history, err := client.GetHistory(context.Background(), req)
	if err != nil {
		t.Fatalf("GetHistory: %v", err)
	}
	if history["record"] == nil {
		t.Fatalf("history output = %#v, want record", history)
	}
	trace, err := client.GetTrace(context.Background(), ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"key": map[string]any{"trace_id": "trace-1"}},
	})
	if err != nil {
		t.Fatalf("GetTrace: %v", err)
	}
	if trace["trace_id"] != "trace-1" {
		t.Fatalf("trace output = %#v, want trace_id", trace)
	}
}

func TestCABIReceiptTransportProjectsAndFailsClosed(t *testing.T) {
	libraryPath := buildFakeCABIReceiptLibrary(t)
	client, transport, err := NewCABIReceiptClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABIReceiptClient: %v", err)
	}

	verification, err := client.Verify(context.Background(), []byte(`{"receipt_ura":"easynet:///r/example/receipt/receipt-1","state":"completed"}`))
	if err != nil {
		t.Fatalf("Verify: %v", err)
	}
	if verification.Method != "summary_projection" || verification.Verified {
		t.Fatalf("verification = %#v", verification)
	}
	causal, err := client.CausalRef(context.Background(), []byte(`{"receipt_ura":"easynet:///r/example/receipt/receipt-1","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`))
	if err != nil {
		t.Fatalf("CausalRef: %v", err)
	}
	if causal.CausalRef == "" || causal.Form != "receipt_ref" {
		t.Fatalf("causal ref = %#v", causal)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := client.Project(context.Background(), []byte(`{"state":"completed","output":{}}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Project after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestCABIReceiptOutputJSONRequiresObject(t *testing.T) {
	_, err := outputJSONFromInvocationResult([]byte(`{"ok":true,"output_json":null}`))
	if err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("null output_json error = %v, want %s", err, ErrInvalidArgument)
	}
	_, err = outputJSONFromInvocationResult([]byte(`{"ok":true,"output_json":[]}`))
	if err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("array output_json error = %v, want %s", err, ErrInvalidArgument)
	}
}

func buildFakeCABIReceiptLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_receipt.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIReceiptSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI receipt source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI receipt library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIReceiptSource = `
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
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI receipt error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 909;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "invocation.trace.get") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"trace_id\":\"trace-1\",\"nodes\":[],\"edges\":[],\"edge_semantics\":\"Axon causal links\"},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "invocation.history.get") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"record\":{\"invocation_id\":\"inv-example-1\",\"state\":\"completed\"}},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "invocation.history.list") == 0) return 10;
	*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"receipt_ura\":null,\"invocation_id\":\"inv-example-1\",\"state\":\"completed\",\"verified\":true,\"output\":{\"ok\":true},\"metadata\":{\"source\":\"daemon\"}},\"error\":null}");
	return 0;
}
int32_t easynet_receipt_build_fetch_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "inv-example-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"key\":{\"request_id\":\"inv-example-1\"}},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"receipt\",\"system_ability\":\"invocation.history.get\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_receipt_build_list_history_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "history-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"receipt\",\"system_ability\":\"invocation.history.list\",\"carrier_owner\":\"daemon_sdk\",\"timeout_ms\":2500}}");
	return 0;
}
int32_t easynet_receipt_build_get_history_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "inv-example-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"key\":{\"request_id\":\"inv-example-1\"}},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"receipt\",\"system_ability\":\"invocation.history.get\",\"carrier_owner\":\"daemon_sdk\",\"timeout_ms\":2500}}");
	return 0;
}
int32_t easynet_receipt_build_trace_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "trace-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.invocation.trace.get@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"key\":{\"trace_id\":\"trace-1\"}},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"receipt\",\"system_ability\":\"invocation.trace.get\",\"carrier_owner\":\"daemon_sdk\",\"timeout_ms\":2500}}");
	return 0;
}
int32_t easynet_receipt_project(uint64_t handle, const char *receipt_json, char **out_summary_json) {
	(void)handle; (void)receipt_json;
	*out_summary_json = dup_json("{\"receipt_ura\":null,\"invocation_id\":\"inv-example-1\",\"state\":\"completed\",\"verified\":false,\"output\":{\"ok\":true},\"error\":null,\"causal_ref\":null,\"metadata\":{\"source\":\"fake_receipt_project\"}}");
	return 0;
}
int32_t easynet_receipt_verify(uint64_t handle, const char *receipt_json, char **out_verification_json) {
	(void)handle; (void)receipt_json;
	*out_verification_json = dup_json("{\"verified\":false,\"receipt_ura\":\"easynet:///r/example/receipt/receipt-1\",\"invocation_id\":null,\"method\":\"summary_projection\",\"reason\":\"cryptographic verification not available at this projection layer\",\"metadata\":{}}");
	return 0;
}
int32_t easynet_receipt_verify_chain(uint64_t handle, const char *request_json, char **out_verification_json) {
	(void)handle; (void)request_json;
	*out_verification_json = dup_json("{\"verified\":false,\"continuous\":true,\"method\":\"daemon_receipt_chain_continuity\",\"reason\":\"continuity only\",\"requires_full_receipt\":true,\"root_receipt_ura\":null,\"terminal_receipt_ura\":null,\"receipt_count\":0,\"items\":[],\"metadata\":{}}");
	return 0;
}
int32_t easynet_receipt_causal_ref(uint64_t handle, const char *receipt_json, char **out_causal_ref_json) {
	(void)handle; (void)receipt_json;
	*out_causal_ref_json = dup_json("{\"causal_ref\":\"receipt:easynet:///r/example/receipt/receipt-1#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"receipt_ura\":\"easynet:///r/example/receipt/receipt-1\",\"invocation_id\":null,\"form\":\"receipt_ref\",\"metadata\":{}}");
	return 0;
}
`
