//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

var _ DaemonTransport = (*CABIDaemonTransport)(nil)
var _ RuntimeTransport = (*CABIRuntimeTransport)(nil)
var _ HealthTransport = (*CABIRuntimeTransport)(nil)

func TestCABIDaemonTransportReportsMissingLibrary(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "libeasynet_cli_missing.dylib")

	transport, err := OpenCABIDaemonTransport(missing)

	if err == nil {
		_ = transport.Close(context.Background())
		t.Fatal("OpenCABIDaemonTransport succeeded for missing library")
	}
	if !IsCode(err, ErrTransport) {
		t.Fatalf("missing C ABI daemon library error = %v, want %s", err, ErrTransport)
	}
}

func TestCABIDaemonStartConfigProjectsFacadeShape(t *testing.T) {
	raw, err := daemonStartConfigForCABI([]byte(`{
		"mode":"device",
		"realm":"lab",
		"device_id":"device-a",
		"node_id":"node-a",
		"daemon_bin":"/usr/local/bin/easynet",
		"log_path":"/tmp/easynet.log",
		"detached":true,
		"env":{"EASYNET_LOG":"debug"}
	}`))
	if err != nil {
		t.Fatalf("daemonStartConfigForCABI failed: %v", err)
	}
	var projected map[string]any
	if err := json.Unmarshal(raw, &projected); err != nil {
		t.Fatalf("decode projected config: %v", err)
	}

	if projected["node_id"] != "node-a" {
		t.Fatalf("node_id = %v, want node-a", projected["node_id"])
	}
	if projected["device_id"] != nil || projected["detached"] != nil {
		t.Fatalf("projected config leaked facade-only keys: %v", projected)
	}
	if projected["detach"] != true {
		t.Fatalf("detach = %v, want true", projected["detach"])
	}
	if projected["mode"] != "device" || projected["realm"] != "lab" {
		t.Fatalf("projected config lost daemon fields: %v", projected)
	}
}

func TestCABIDaemonStartConfigRejectsUnsupportedTransportFields(t *testing.T) {
	_, err := daemonStartConfigForCABI([]byte(`{
		"mode":"hub",
		"uds_path":"/tmp/easynet.sock",
		"listen_tcp":"127.0.0.1:9000"
	}`))

	if err == nil {
		t.Fatal("daemonStartConfigForCABI accepted unsupported transport fields")
	}
	if !IsCode(err, ErrNotImplemented) {
		t.Fatalf("unsupported config error = %v, want %s", err, ErrNotImplemented)
	}
}

func TestCABIDaemonStatusProjectionFromFlatAndNestedShapes(t *testing.T) {
	status, err := daemonStatusFromCABI("42", []byte(`{
		"control_accepting":true,
		"pid":123,
		"version":"1.2.3",
		"message":"ready for control",
		"diagnostics":["control-only"],
		"endpoints":{
			"control_endpoint":"unix:///tmp/control.sock",
			"invocation_endpoint":"",
			"public_endpoint":"https://hub.example"
		}
	}`))
	if err != nil {
		t.Fatalf("daemonStatusFromCABI failed: %v", err)
	}

	if status["handle_id"] != "42" {
		t.Fatalf("handle_id = %v, want 42", status["handle_id"])
	}
	if status["state"] != string(DaemonControlOnly) {
		t.Fatalf("state = %v, want %s", status["state"], DaemonControlOnly)
	}
	endpoints, ok := status["endpoints"].(map[string]any)
	if !ok {
		t.Fatalf("endpoints = %T, want map", status["endpoints"])
	}
	if endpoints["control_endpoint"] != "unix:///tmp/control.sock" || endpoints["public_endpoint"] != "https://hub.example" {
		t.Fatalf("endpoints projection mismatch: %v", endpoints)
	}
}

func TestCABIPreparedAndSignedEnvelopeKeys(t *testing.T) {
	key, err := preparedKeyFromJSON([]byte(`{"prepared_id":"prep-1","request_id":"req-1"}`))
	if err != nil {
		t.Fatalf("preparedKeyFromJSON failed: %v", err)
	}
	if key != "prep-1" {
		t.Fatalf("prepared key = %q, want prep-1", key)
	}

	key, signatureJSON, err := signedInvocationCABIFields([]byte(`{
		"prepared":{"request_id":"req-2"},
		"signature":{"algorithm":"ed25519","signature_base64":"abc"}
	}`))
	if err != nil {
		t.Fatalf("signedInvocationCABIFields failed: %v", err)
	}
	if key != "req-2" {
		t.Fatalf("signed prepared key = %q, want req-2", key)
	}
	if string(signatureJSON) != `{"algorithm":"ed25519","signature_base64":"abc"}` {
		t.Fatalf("signature JSON = %s", signatureJSON)
	}
}

func TestCABIRuntimeTransportClosedStateRejectsRuntimeCalls(t *testing.T) {
	runtime := newCABIRuntimeTransport(cabiRuntimeSymbols{}, 99, false)

	if err := runtime.Close(context.Background()); err != nil {
		t.Fatalf("Close failed: %v", err)
	}
	if _, err := runtime.Invoke(context.Background(), []byte(`{}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Invoke after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, _, err := runtime.OpenStream(context.Background(), []byte(`{}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("OpenStream after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, _, err := runtime.OpenBidi(context.Background(), []byte(`{}`), []byte(`[]`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("OpenBidi after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestCABIRuntimeTransportDrivesStreamAndBidiCallbacks(t *testing.T) {
	libraryPath := buildFakeCABIStreamLibrary(t)
	transport, err := OpenCABIDaemonTransport(libraryPath)
	if err != nil {
		t.Fatalf("OpenCABIDaemonTransport: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI daemon transport: %v", err)
		}
	}()
	control, err := NewDaemonControl(transport)
	if err != nil {
		t.Fatalf("NewDaemonControl: %v", err)
	}
	handle, err := control.Start(context.Background(), StartConfig{Mode: ModeDevice, DeviceID: "dev-a"})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	client, err := handle.OpenRuntime(context.Background(), ConnectOptions{})
	if err != nil {
		t.Fatalf("OpenRuntime: %v", err)
	}

	stream, err := client.InvokeStream(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	first, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("stream first event: %v", err)
	}
	terminal, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("stream terminal event: %v", err)
	}
	if first.Sequence() != 1 || first.Terminal() || string(first.PayloadJSON()) != `{"step":1}` {
		t.Fatalf("unexpected first stream event: %#v payload=%s", first, first.PayloadJSON())
	}
	if !terminal.Terminal() || stream.State() != StreamTerminalFrameSeen {
		t.Fatalf("terminal stream event not observed: event=%#v state=%s", terminal, stream.State())
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("stream close: %v", err)
	}
	if stream.State() != StreamClosed {
		t.Fatalf("stream state = %s, want Closed", stream.State())
	}

	session, err := client.OpenBidi(context.Background(), completeDraftForRuntimeTest(t), []BidiStreamDescriptor{
		{StreamID: 1, ContentType: "application/json", Ordering: "STRICT"},
	})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	frame, err := NewBidiFrame(1, "data", 1)
	if err != nil {
		t.Fatalf("NewBidiFrame: %v", err)
	}
	ack, err := session.Send(context.Background(), frame)
	if err != nil {
		t.Fatalf("bidi send: %v", err)
	}
	received, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("bidi receive: %v", err)
	}
	if ack.Sequence() != 1 || ack.Kind() != "data" {
		t.Fatalf("unexpected bidi send ack: %#v", ack)
	}
	if !received.Terminal() || session.State() != BidiTerminal {
		t.Fatalf("bidi terminal frame not observed: frame=%#v state=%s", received, session.State())
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("bidi close: %v", err)
	}
	if session.State() != BidiClosed {
		t.Fatalf("bidi state = %s, want Closed", session.State())
	}
}

func TestCABICallbackInboxOverflowIsBounded(t *testing.T) {
	inbox := newCABICallbackInbox(1)
	inbox.push([]byte(`{"sequence":1,"kind":"chunk","terminal":false}`))
	inbox.push([]byte(`{"sequence":2,"kind":"chunk","terminal":false}`))

	_, err := inbox.recv(context.Background())

	if !IsCode(err, ErrProtocol) {
		t.Fatalf("overflow error = %v, want %s", err, ErrProtocol)
	}
}

func buildFakeCABIStreamLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIStreamSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIStreamSource = `
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef void (*stream_callback_t)(void *user_data, const char *chunk_json);
typedef void (*bidi_callback_t)(void *user_data, const char *frame_json);

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
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}

int32_t easynet_daemon_start(const char *config_json, uint64_t *out_daemon_handle) {
	(void)config_json;
	*out_daemon_handle = 606;
	return 0;
}
int32_t easynet_daemon_attach(const char *options_json, uint64_t *out_daemon_handle) {
	(void)options_json;
	*out_daemon_handle = 707;
	return 0;
}
int32_t easynet_daemon_discover(const char *options_json, char **out_discovery_json) {
	(void)options_json;
	*out_discovery_json = dup_json("{\"control_endpoint\":\"unix:///tmp/control.sock\",\"invocation_endpoint\":\"unix:///tmp/daemon.sock\",\"invocation_accepting\":true,\"diagnostics\":[]}");
	return 0;
}
int32_t easynet_daemon_stop(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_daemon_detach(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_daemon_status(uint64_t handle, char **out_status_json) {
	(void)handle;
	*out_status_json = dup_json("{\"state\":\"Running\",\"mode\":\"device\",\"endpoints\":{\"control_endpoint\":\"unix:///tmp/control.sock\",\"invocation_endpoint\":\"unix:///tmp/daemon.sock\"},\"diagnostics\":[]}");
	return 0;
}
int32_t easynet_daemon_open_client(uint64_t daemon_handle, uint64_t *out_handle) {
	(void)daemon_handle;
	*out_handle = 808;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_runtime_health(uint64_t handle, char **out_health_json) {
	(void)handle;
	*out_health_json = dup_json("{\"api_ready\":true,\"daemon_ready\":true,\"invocation_ready\":true,\"directory_ready\":true,\"trust_ready\":true,\"runtime_ready\":true,\"diagnostics\":[]}");
	return 0;
}
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle; (void)invocation_json; (void)out_result_json;
	return 10;
}
int32_t easynet_invocation_prepare(uint64_t handle, const char *invocation_json, const char *options_json, uint64_t *out_prepared_id, char **out_prepared_json) {
	(void)handle; (void)invocation_json; (void)options_json; (void)out_prepared_id; (void)out_prepared_json;
	return 10;
}
int32_t easynet_invocation_sign_prepared(uint64_t prepared_id, const char *signature_json, uint64_t *out_signed_id, char **out_signed_json) {
	(void)prepared_id; (void)signature_json; (void)out_signed_id; (void)out_signed_json;
	return 10;
}
int32_t easynet_invocation_submit_signed_handle(uint64_t handle, uint64_t signed_id, uint64_t *out_invocation_handle_id, char **out_submitted_json) {
	(void)handle; (void)signed_id; (void)out_invocation_handle_id; (void)out_submitted_json;
	return 10;
}
int32_t easynet_invocation_handle_await(uint64_t handle, uint64_t invocation_handle_id, char **out_result_json) {
	(void)handle; (void)invocation_handle_id; (void)out_result_json;
	return 10;
}
int32_t easynet_invocation_handle_cancel(uint64_t handle, uint64_t invocation_handle_id, const char *reason_json, char **out_cancel_json) {
	(void)handle; (void)invocation_handle_id; (void)reason_json; (void)out_cancel_json;
	return 10;
}
int32_t easynet_invocation_handle_events(uint64_t handle, uint64_t invocation_handle_id, char **out_events_json) {
	(void)handle; (void)invocation_handle_id; (void)out_events_json;
	return 10;
}
int32_t easynet_invocation_handle_free(uint64_t handle, uint64_t invocation_handle_id) {
	(void)handle; (void)invocation_handle_id;
	return 0;
}
int32_t easynet_prepared_invocation_free(uint64_t prepared_id) { (void)prepared_id; return 0; }
int32_t easynet_signed_invocation_free(uint64_t signed_id) { (void)signed_id; return 0; }

int32_t easynet_invocation_stream_open(uint64_t handle, const char *invocation_json, stream_callback_t on_chunk, void *user_data, uint64_t *out_stream_id) {
	(void)handle; (void)invocation_json;
	*out_stream_id = 404;
	on_chunk(user_data, "{\"sequence\":1,\"kind\":\"chunk\",\"state\":\"Open\",\"terminal\":false,\"payload_json\":{\"step\":1}}");
	on_chunk(user_data, "{\"sequence\":2,\"kind\":\"terminal\",\"state\":\"Completed\",\"terminal\":true}");
	return 0;
}
int32_t easynet_invocation_stream_cancel(uint64_t handle, uint64_t stream_id) {
	(void)handle;
	return stream_id == 404 ? 0 : 4;
}
int32_t easynet_invocation_stream_close(uint64_t handle, uint64_t stream_id) {
	(void)handle;
	return stream_id == 404 ? 0 : 4;
}
int32_t easynet_invocation_bidi_open(uint64_t handle, const char *invocation_json, bidi_callback_t on_frame, void *user_data, uint64_t *out_bidi_id) {
	(void)handle;
	if (strstr(invocation_json, "bidi_streams") == 0) return 11;
	*out_bidi_id = 505;
	on_frame(user_data, "{\"sequence\":1,\"kind\":\"terminal\",\"stream_id\":1,\"terminal\":true}");
	return 0;
}
int32_t easynet_invocation_bidi_send(uint64_t handle, uint64_t bidi_id, const char *frame_json) {
	(void)handle; (void)frame_json;
	return bidi_id == 505 ? 0 : 4;
}
int32_t easynet_invocation_bidi_close_send(uint64_t handle, uint64_t bidi_id) {
	(void)handle;
	return bidi_id == 505 ? 0 : 4;
}
int32_t easynet_invocation_bidi_close(uint64_t handle, uint64_t bidi_id) {
	(void)handle;
	return bidi_id == 505 ? 0 : 4;
}
int32_t easynet_invocation_bidi_cancel(uint64_t handle, uint64_t bidi_id) {
	(void)handle;
	return bidi_id == 505 ? 0 : 4;
}
`
