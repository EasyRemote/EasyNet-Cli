//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

var _ DaemonTransport = (*CABIRuntimeLifecycleTransport)(nil)
var _ RuntimeTransport = (*CABIRuntimeTransport)(nil)
var _ HealthTransport = (*CABIRuntimeTransport)(nil)

func TestCABIRuntimeLifecycleTransportReportsMissingLibrary(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "libeasynet_cli_missing.dylib")

	transport, err := OpenCABIRuntimeLifecycleTransport(missing)

	if err == nil {
		_ = transport.Close(context.Background())
		t.Fatal("OpenCABIRuntimeLifecycleTransport succeeded for missing library")
	}
	if !IsCode(err, ErrTransport) {
		t.Fatalf("missing C ABI daemon library error = %v, want %s", err, ErrTransport)
	}
}

func TestCABIBidiFrameJSONProjectsSDKFramesToFFIWire(t *testing.T) {
	binary, err := NewBidiBinaryFrame(1, 7, []byte("hello"), "application/octet-stream")
	if err != nil {
		t.Fatalf("NewBidiBinaryFrame: %v", err)
	}
	raw, err := json.Marshal(binary)
	if err != nil {
		t.Fatalf("Marshal binary frame: %v", err)
	}
	wire, err := cabiBidiFrameJSON(raw)
	if err != nil {
		t.Fatalf("cabiBidiFrameJSON(binary): %v", err)
	}
	var binaryWire map[string]any
	if err := json.Unmarshal(wire, &binaryWire); err != nil {
		t.Fatalf("decode binary wire: %v", err)
	}
	if binaryWire["type"] != "binary_chunk" || binaryWire["data_base64"] != base64.StdEncoding.EncodeToString([]byte("hello")) {
		t.Fatalf("unexpected binary wire: %#v", binaryWire)
	}

	control, err := NewBidiJSONFrame(2, "control", 7, json.RawMessage(`{"pty_resize":{"cols":120,"rows":40}}`))
	if err != nil {
		t.Fatalf("NewBidiJSONFrame: %v", err)
	}
	raw, err = json.Marshal(control)
	if err != nil {
		t.Fatalf("Marshal control frame: %v", err)
	}
	wire, err = cabiBidiFrameJSON(raw)
	if err != nil {
		t.Fatalf("cabiBidiFrameJSON(control): %v", err)
	}
	var controlWire map[string]any
	if err := json.Unmarshal(wire, &controlWire); err != nil {
		t.Fatalf("decode control wire: %v", err)
	}
	if controlWire["type"] != "control" {
		t.Fatalf("unexpected control wire: %#v", controlWire)
	}
	if _, ok := controlWire["pty_resize"].(map[string]any); !ok {
		t.Fatalf("control payload not projected: %#v", controlWire)
	}
}

func TestProjectCABIOrderedEventKeepsCanonicalBidiReceipts(t *testing.T) {
	var next uint64
	raw := []byte(`{
		"ok": true,
		"kind": "receipt",
		"sequence": 17,
		"terminal": true,
		"payload_json": {"sha256":"abc123"},
		"payload_base64": "eyJzaGEyNTYiOiJhYmMxMjMifQ==",
		"payload_content_type": "application/json",
		"terminal_receipt": {
			"state": "Completed",
			"reason": "",
			"cleanup_complete": true
		}
	}`)

	projected, err := projectCABIOrderedEvent(raw, func(observed *uint64) uint64 {
		if observed == nil {
			next++
			return next
		}
		next = *observed
		return *observed
	}, true)
	if err != nil {
		t.Fatalf("projectCABIOrderedEvent: %v", err)
	}
	var projectedJSON map[string]any
	if err := json.Unmarshal(projected, &projectedJSON); err != nil {
		t.Fatalf("decode projected frame: %v", err)
	}
	if projectedJSON["kind"] != "receipt" {
		t.Fatalf("projected kind = %v, want receipt; raw=%s", projectedJSON["kind"], projected)
	}
	if _, ok := projectedJSON["receipt"]; ok {
		t.Fatalf("legacy receipt field projected: %s", projected)
	}
	if _, ok := projectedJSON["terminal_receipt"]; !ok {
		t.Fatalf("terminal_receipt omitted: %s", projected)
	}
	frame, err := NewBidiFrameFromJSON(projected)
	if err != nil {
		t.Fatalf("NewBidiFrameFromJSON: %v; raw=%s", err, projected)
	}
	if frame.Kind() != "receipt" || !frame.Terminal() || frame.Sequence() != 17 {
		t.Fatalf("unexpected canonical receipt frame: kind=%s terminal=%v seq=%d", frame.Kind(), frame.Terminal(), frame.Sequence())
	}
	if frame.PayloadContentType() != "application/json" {
		t.Fatalf("payload content type = %q", frame.PayloadContentType())
	}
	payload, err := base64.StdEncoding.DecodeString(frame.PayloadBase64())
	if err != nil {
		t.Fatalf("decode payload_base64: %v", err)
	}
	if string(payload) != `{"sha256":"abc123"}` {
		t.Fatalf("payload = %s", payload)
	}
	var meta struct {
		SHA256 string `json:"sha256"`
	}
	if err := json.Unmarshal(frame.PayloadJSON(), &meta); err != nil {
		t.Fatalf("decode payload_json: %v; raw=%s", err, frame.PayloadJSON())
	}
	if meta.SHA256 != "abc123" {
		t.Fatalf("payload sha256 = %q", meta.SHA256)
	}
	var terminalReceipt struct {
		State string `json:"state"`
	}
	if err := json.Unmarshal(frame.TerminalReceiptJSON(), &terminalReceipt); err != nil {
		t.Fatalf("decode terminal receipt: %v; raw=%s", err, frame.TerminalReceiptJSON())
	}
	if terminalReceipt.State != "Completed" {
		t.Fatalf("terminal receipt state = %q", terminalReceipt.State)
	}
}

func TestProjectCABIOrderedEventCanonicalizesBidiBinaryChunkAsDataFrame(t *testing.T) {
	var next uint64
	projected, err := projectCABIOrderedEvent([]byte(`{
		"ok": true,
		"kind": "binary_chunk",
		"sequence": 5,
		"stream_id": 1,
		"data_base64": "aGVsbG8=",
		"terminal": false
	}`), func(observed *uint64) uint64 {
		if observed == nil {
			next++
			return next
		}
		next = *observed
		return *observed
	}, true)
	if err != nil {
		t.Fatalf("projectCABIOrderedEvent: %v", err)
	}
	var projectedJSON map[string]any
	if err := json.Unmarshal(projected, &projectedJSON); err != nil {
		t.Fatalf("decode projected frame: %v", err)
	}
	if projectedJSON["kind"] != "data" || projectedJSON["payload_base64"] != "aGVsbG8=" {
		t.Fatalf("projected canonical data frame mismatch: %#v", projectedJSON)
	}
	frame, err := NewBidiFrameFromJSON(projected)
	if err != nil {
		t.Fatalf("NewBidiFrameFromJSON: %v; raw=%s", err, projected)
	}
	if frame.Kind() != "data" || frame.StreamID() != 1 || frame.PayloadBase64() != "aGVsbG8=" {
		t.Fatalf("unexpected data frame: kind=%s stream=%d payload=%q raw=%s", frame.Kind(), frame.StreamID(), frame.PayloadBase64(), projected)
	}
}

func TestProjectCABIOrderedEventDoesNotSynthesizeKindFromLegacyEvent(t *testing.T) {
	projected, err := projectCABIOrderedEvent(
		[]byte(`{"event":"receipt","terminal":false}`),
		func(*uint64) uint64 { return 1 },
		false,
	)
	if err != nil {
		t.Fatalf("projectCABIOrderedEvent: %v", err)
	}
	var projectedJSON map[string]any
	if err := json.Unmarshal(projected, &projectedJSON); err != nil {
		t.Fatalf("decode projected frame: %v", err)
	}
	if _, hasKind := projectedJSON["kind"]; hasKind {
		t.Fatalf("legacy event produced kind: %#v", projectedJSON)
	}
}

func TestCABIDaemonStartConfigProjectsFacadeShape(t *testing.T) {
	raw, err := runtimeHostStartConfigForCABI([]byte(`{
		"mode":"device",
		"realm":"lab",
		"device_id":"device-a",
		"daemon_bin":"/usr/local/bin/easynet",
		"working_dir":"/srv/easynet",
		"log_path":"/tmp/easynet.log",
		"detached":true,
		"env":{"EASYNET_LOG":"debug"}
	}`))
	if err != nil {
		t.Fatalf("runtimeHostStartConfigForCABI failed: %v", err)
	}
	var projected map[string]any
	if err := json.Unmarshal(raw, &projected); err != nil {
		t.Fatalf("decode projected config: %v", err)
	}

	if projected["device_id"] != "device-a" {
		t.Fatalf("device_id = %v, want device-a", projected["device_id"])
	}
	if projected["node_id"] != nil || projected["detach"] != nil {
		t.Fatalf("projected config leaked legacy keys: %v", projected)
	}
	if projected["detached"] != true {
		t.Fatalf("detached = %v, want true", projected["detached"])
	}
	if projected["working_dir"] != "/srv/easynet" {
		t.Fatalf("working_dir = %v, want /srv/easynet", projected["working_dir"])
	}
	if projected["mode"] != "device" || projected["realm"] != "lab" {
		t.Fatalf("projected config lost daemon fields: %v", projected)
	}
}

func TestCABIDaemonStartConfigRejectsUnsupportedTransportFields(t *testing.T) {
	_, err := runtimeHostStartConfigForCABI([]byte(`{
		"mode":"hub",
		"uds_path":"/tmp/easynet.sock",
		"listen_tcp":"127.0.0.1:9000"
	}`))

	if err == nil {
		t.Fatal("runtimeHostStartConfigForCABI accepted unsupported transport fields")
	}
	if !IsCode(err, ErrNotImplemented) {
		t.Fatalf("unsupported config error = %v, want %s", err, ErrNotImplemented)
	}
}

func TestCABIDaemonStatusProjectionFromFlatAndNestedShapes(t *testing.T) {
	status, err := runtimeHostStatusFromCABI("42", []byte(`{
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
		t.Fatalf("runtimeHostStatusFromCABI failed: %v", err)
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

	fields, err := signedInvocationCABIFields([]byte(`{
		"prepared":{"prepared_id":"prep-2","request_id":"req-2"},
		"signature":{"algorithm":"ed25519","signature_base64":"abc"}
	}`))
	if err != nil {
		t.Fatalf("signedInvocationCABIFields failed: %v", err)
	}
	if fields.key != "prep-2" {
		t.Fatalf("signed prepared key = %q, want prep-2", fields.key)
	}
	if string(fields.signatureJSON) != `{"algorithm":"ed25519","signature_base64":"abc"}` {
		t.Fatalf("signature JSON = %s", fields.signatureJSON)
	}
	if fields.localDaemonSigning {
		t.Fatalf("localDaemonSigning = true, want false")
	}

	localFields, err := signedInvocationCABIFields([]byte(`{
		"prepared":{"prepared_id":"prep-local","request_id":"req-local"},
		"policy":{"mode":"local_daemon_signing","signer_id":"signer-alice-key-1"}
	}`))
	if err != nil {
		t.Fatalf("signedInvocationCABIFields local failed: %v", err)
	}
	if localFields.key != "prep-local" || !localFields.localDaemonSigning || len(localFields.signatureJSON) != 0 {
		t.Fatalf("local fields = %#v", localFields)
	}

	if _, err := signedInvocationCABIFields([]byte(`{
		"prepared":{"request_id":"req-only"},
		"signature":{"algorithm":"ed25519","signature_base64":"abc"}
	}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("request_id-only prepared key error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestCABIPreparedHandleRegistryRejectsDuplicatePreparedID(t *testing.T) {
	registry := newCABIPreparedHandleRegistry()
	var freed []uint64
	free := func(id uint64) error {
		freed = append(freed, id)
		return nil
	}

	if err := registry.register("prep-1", 1001, free); err != nil {
		t.Fatalf("register first prepared handle failed: %v", err)
	}
	err := registry.register("prep-1", 1002, free)
	if !IsCode(err, ErrProtocol) {
		t.Fatalf("duplicate prepared handle error = %v, want %s", err, ErrProtocol)
	}
	sdkErr, ok := err.(*SDKError)
	if !ok || sdkErr.Message != "C ABI prepare returned a duplicate prepared handle id" {
		t.Fatalf("duplicate prepared handle message = %#v", err)
	}
	if len(freed) != 1 || freed[0] != 1002 {
		t.Fatalf("freed duplicate handles = %v, want [1002]", freed)
	}
	remaining := registry.drain()
	if len(remaining) != 1 || remaining[0] != 1001 {
		t.Fatalf("remaining handles = %v, want [1001]", remaining)
	}
}

func TestCABIPreparedHandleRegistryClaimLifecycle(t *testing.T) {
	registry := newCABIPreparedHandleRegistry()
	if err := registry.register("prep-1", 1001, func(uint64) error { return nil }); err != nil {
		t.Fatalf("register prepared handle failed: %v", err)
	}

	id, err := registry.claimForSigning("prep-1")
	if err != nil {
		t.Fatalf("claim prepared handle failed: %v", err)
	}
	if id != 1001 {
		t.Fatalf("claimed id = %d, want 1001", id)
	}
	if _, err := registry.claimForSigning("prep-1"); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("duplicate claim error = %v, want %s", err, ErrInvalidHandle)
	}

	registry.releaseSigningClaim("prep-1", id)
	if _, err := registry.claimForSigning("prep-1"); err != nil {
		t.Fatalf("claim after release failed: %v", err)
	}
	registry.consumeSigningClaim("prep-1", id)
	if _, err := registry.claimForSigning("prep-1"); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("claim after consume error = %v, want %s", err, ErrInvalidHandle)
	}
	if remaining := registry.drain(); len(remaining) != 0 {
		t.Fatalf("remaining handles after consume = %v, want empty", remaining)
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
	transport, err := OpenCABIRuntimeLifecycleTransport(libraryPath)
	if err != nil {
		t.Fatalf("OpenCABIRuntimeLifecycleTransport: %v", err)
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
	if first.Sequence() != 1 || first.Terminal() || string(first.PayloadJSON()) != `{"step":1}` {
		t.Fatalf("unexpected first stream event: %#v payload=%s", first, first.PayloadJSON())
	}
	cancel, err := stream.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("stream cancel: %v", err)
	}
	if cancel.State() != StreamCancelRequested || cancel.Terminal() || cancel.Cancelled() || stream.State() != StreamCancelRequested {
		t.Fatalf("stream cancel must be non-terminal request: outcome=%#v state=%s", cancel, stream.State())
	}
	terminal, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("stream terminal event after cancel request: %v", err)
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
	if ack.Sequence() != 1 || ack.Kind() != "data" {
		t.Fatalf("unexpected bidi send ack: %#v", ack)
	}
	bidiCancel, err := session.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("bidi cancel: %v", err)
	}
	if bidiCancel.State() != BidiCancelRequested || bidiCancel.Terminal() || session.State() != BidiCancelRequested {
		t.Fatalf("bidi cancel must be non-terminal request: outcome=%#v state=%s", bidiCancel, session.State())
	}
	received, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("bidi receive after cancel request: %v", err)
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
#include <stdio.h>
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

uint32_t easynet_abi_version(void) { return 5u; }
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
	*out_health_json = dup_json("{\"api_ready\":true,\"invocation_ready\":true,\"directory_ready\":true,\"trust_ready\":true,\"runtime_ready\":true,\"diagnostics\":[]}");
	return 0;
}
int32_t easynet_runtime_diagnostics(uint64_t handle, char **out_diagnostics_json) {
	(void)handle;
	*out_diagnostics_json = dup_json("{\"profile\":\"health\",\"kind\":\"diagnostics_report\",\"state\":\"Running\",\"ready\":true,\"version\":\"0.91.30\",\"abi_version\":5,\"control_endpoint\":\"/tmp/easynet/control.json\",\"invocation_endpoint\":\"/tmp/easynet/daemon.sock\",\"checks\":[{\"name\":\"api\",\"ready\":true,\"message\":null},{\"name\":\"daemon\",\"ready\":true,\"message\":null},{\"name\":\"invocation\",\"ready\":true,\"message\":null},{\"name\":\"directory\",\"ready\":true,\"message\":null},{\"name\":\"trust\",\"ready\":true,\"message\":null},{\"name\":\"runtime\",\"ready\":true,\"message\":null}],\"diagnostics\":[]}");
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
int32_t easynet_invocation_sign_prepared_local(uint64_t prepared_id, uint64_t *out_signed_id, char **out_signed_json) {
	(void)prepared_id; (void)out_signed_id; (void)out_signed_json;
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
