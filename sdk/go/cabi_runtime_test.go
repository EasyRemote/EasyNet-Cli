//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/json"
	"path/filepath"
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

func TestCABIRuntimeTransportClosedAndUnsupportedStreamStates(t *testing.T) {
	runtime := newCABIRuntimeTransport(cabiRuntimeSymbols{}, 99, false)

	if _, _, err := runtime.OpenStream(context.Background(), []byte(`{}`)); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("OpenStream error = %v, want %s", err, ErrNotImplemented)
	}
	if _, _, err := runtime.OpenBidi(context.Background(), []byte(`{}`), []byte(`[]`)); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("OpenBidi error = %v, want %s", err, ErrNotImplemented)
	}
	if err := runtime.Close(context.Background()); err != nil {
		t.Fatalf("Close failed: %v", err)
	}
	if _, err := runtime.Invoke(context.Background(), []byte(`{}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Invoke after close error = %v, want %s", err, ErrInvalidArgument)
	}
}
