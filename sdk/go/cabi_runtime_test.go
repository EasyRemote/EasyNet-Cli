//go:build runtime_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"
)

var _ RuntimeLifecycleTransport = (*cabiRuntimeLifecycleTransport)(nil)
var _ RuntimeTransport = (*cabiRuntimeTransport)(nil)
var _ HealthTransport = (*cabiRuntimeTransport)(nil)

func TestCABIProviderReportsMissingLibrary(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "libeasynet_cli_missing.dylib")

	transport, err := openCABIRuntimeLifecycleTransport(missing)

	if err == nil {
		_ = transport.Close(context.Background())
		t.Fatal("openCABIRuntimeLifecycleTransport succeeded for missing library")
	}
	if !IsCode(err, ErrTransport) {
		t.Fatalf("missing C ABI runtime host library error = %v, want %s", err, ErrTransport)
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
	if binaryWire["pts"] != float64(1) {
		t.Fatalf("binary wire did not derive C ABI pts from SDK sequence: %#v", binaryWire)
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

func TestCABIBidiFrameChainMACTagsFFIWireWithoutPollutingSDKProjection(t *testing.T) {
	binary, err := NewBidiBinaryFrame(1, 7, []byte("hello"), "application/octet-stream")
	if err != nil {
		t.Fatalf("NewBidiBinaryFrame: %v", err)
	}
	raw, err := json.Marshal(binary)
	if err != nil {
		t.Fatalf("Marshal binary frame: %v", err)
	}

	chain := newCABIBidiFrameChainMAC(42)
	wire, err := chain.attach(mustCABIBidiFrameJSON(t, raw))
	if err != nil {
		t.Fatalf("attach MAC: %v", err)
	}
	var tagged map[string]any
	if err := json.Unmarshal(wire, &tagged); err != nil {
		t.Fatalf("decode tagged wire: %v", err)
	}
	mac, ok := tagged["mac_base64"].(string)
	if !ok || mac == "" {
		t.Fatalf("tagged wire omitted mac_base64: %#v", tagged)
	}
	decoded, err := base64.StdEncoding.DecodeString(mac)
	if err != nil {
		t.Fatalf("decode mac_base64: %v", err)
	}
	if len(decoded) != 32 {
		t.Fatalf("mac length = %d, want 32", len(decoded))
	}

	projected, err := NewBidiFrameFromJSON(raw)
	if err != nil {
		t.Fatalf("SDK projection polluted by transport MAC: %v", err)
	}
	if projected.Kind() != "data" {
		t.Fatalf("projection kind = %q, want data", projected.Kind())
	}
}

func mustCABIBidiFrameJSON(t *testing.T, raw []byte) []byte {
	t.Helper()
	wire, err := cabiBidiFrameJSON(raw)
	if err != nil {
		t.Fatalf("cabiBidiFrameJSON: %v", err)
	}
	return wire
}

func TestCABIBidiFrameJSONRejectsLegacyBinaryChunkKind(t *testing.T) {
	_, err := cabiBidiFrameJSON([]byte(`{"sequence":1,"kind":"binary_chunk","stream_id":1,"payload_base64":"aGVsbG8="}`))
	if err == nil {
		t.Fatalf("cabiBidiFrameJSON accepted legacy binary_chunk kind")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestProjectCABIOrderedEventKeepsCanonicalBidiReceipts(t *testing.T) {
	var next uint64
	raw := []byte(`{
		"ok": true,
		"kind": "receipt",
		"sequence": 17,
		"mac_base64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
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
	}, true, false)
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
	if _, hasMAC := projectedJSON["mac_base64"]; hasMAC {
		t.Fatalf("transport mac_base64 was preserved in receipt projection: %#v", projectedJSON)
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

func TestProjectCABIOrderedEventKeepsCanonicalDataFrame(t *testing.T) {
	var next uint64
	projected, err := projectCABIOrderedEvent([]byte(`{
		"ok": true,
		"kind": "data",
		"sequence": 5,
		"mac_base64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
		"pts": 99,
		"stream_id": 1,
		"payload_base64": "aGVsbG8=",
		"terminal": false
	}`), func(observed *uint64) uint64 {
		if observed == nil {
			next++
			return next
		}
		next = *observed
		return *observed
	}, true, false)
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
	if _, hasLegacyData := projectedJSON["data_base64"]; hasLegacyData {
		t.Fatalf("legacy data_base64 was preserved in SDK projection: %#v", projectedJSON)
	}
	if _, hasMAC := projectedJSON["mac_base64"]; hasMAC {
		t.Fatalf("transport mac_base64 was preserved in SDK projection: %#v", projectedJSON)
	}
	if _, hasPTS := projectedJSON["pts"]; hasPTS {
		t.Fatalf("transport pts was preserved in SDK projection: %#v", projectedJSON)
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

func TestCABIRuntimeHostStartConfigProjectsFacadeShape(t *testing.T) {
	raw, err := runtimeHostStartConfigForCABI([]byte(`{
		"mode":"edge",
		"realm":"lab",
		"runtime_instance_id":"runtime-a",
		"runtime_bin":"/usr/local/bin/runtime-host",
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

	if projected["runtime_instance_id"] != "runtime-a" {
		t.Fatalf("runtime_instance_id = %v, want runtime-a", projected["runtime_instance_id"])
	}
	if projected["device_id"] != nil || projected["node_id"] != nil || projected["daemon_bin"] != nil || projected["detach"] != nil {
		t.Fatalf("projected config leaked legacy keys: %v", projected)
	}
	if projected["detached"] != true {
		t.Fatalf("detached = %v, want true", projected["detached"])
	}
	if projected["working_dir"] != "/srv/easynet" {
		t.Fatalf("working_dir = %v, want /srv/easynet", projected["working_dir"])
	}
	if projected["mode"] != "edge" || projected["realm"] != "lab" {
		t.Fatalf("projected config lost runtime host fields: %v", projected)
	}
}

func TestCABIRuntimeHostStartConfigRejectsUnsupportedTransportFields(t *testing.T) {
	_, err := runtimeHostStartConfigForCABI([]byte(`{
		"mode":"authority",
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

func TestCABIRuntimeHostStartConfigRejectsRetiredProductModeInput(t *testing.T) {
	for _, mode := range []string{"device", "hub", "both"} {
		_, err := runtimeHostStartConfigForCABI([]byte(fmt.Sprintf(`{"mode":%q}`, mode)))

		if !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("retired mode %q error = %v, want %s", mode, err, ErrInvalidArgument)
		}
		if err == nil || !strings.Contains(err.Error(), "edge, authority, or combined") {
			t.Fatalf("retired mode %q error did not name generic roles: %v", mode, err)
		}
	}
}

func TestCABIRuntimeHostStartConfigRejectsUnsupportedCombinedMode(t *testing.T) {
	_, err := runtimeHostStartConfigForCABI([]byte(`{"mode":"combined"}`))

	if !IsCode(err, ErrNotImplemented) {
		t.Fatalf("combined mode error = %v, want %s", err, ErrNotImplemented)
	}
	if err == nil || !strings.Contains(err.Error(), "does not support combined runtime host mode") {
		t.Fatalf("combined mode error did not name unsupported provider capability: %v", err)
	}
}

func TestCABIRuntimeHostStatusProjectionFromFlatAndNestedShapes(t *testing.T) {
	status, err := runtimeHostStatusFromCABI("42", []byte(`{
		"control_accepting":true,
		"mode":"combined",
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
	if status["state"] != string(RuntimeControlOnly) {
		t.Fatalf("state = %v, want %s", status["state"], RuntimeControlOnly)
	}
	if status["mode"] != "combined" {
		t.Fatalf("mode = %v, want combined", status["mode"])
	}
	endpoints, ok := status["endpoints"].(map[string]any)
	if !ok {
		t.Fatalf("endpoints = %T, want map", status["endpoints"])
	}
	if endpoints["control_endpoint"] != "unix:///tmp/control.sock" || endpoints["public_endpoint"] != "https://hub.example" {
		t.Fatalf("endpoints projection mismatch: %v", endpoints)
	}
}

func TestCABIRuntimeHostStatusRejectsUnknownWireModeWithCanonicalError(t *testing.T) {
	for _, mode := range []string{"daemon", "device", "hub", "both"} {
		_, err := runtimeHostStatusFromCABI("42", []byte(fmt.Sprintf(`{
			"state":"Running",
			"mode":%q,
			"endpoints":{"control_endpoint":"unix:///tmp/control.sock"}
		}`, mode)))

		if !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("invalid status mode %q error = %v, want %s", mode, err, ErrInvalidArgument)
		}
		if err == nil || !strings.Contains(err.Error(), "edge, authority, or combined") {
			t.Fatalf("invalid status mode %q error did not name canonical roles: %v", mode, err)
		}
		if strings.Contains(err.Error(), "device, hub, or both") {
			t.Fatalf("invalid status mode %q error leaked C-ABI wire vocabulary: %v", mode, err)
		}
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
	if fields.providerManagedSigning {
		t.Fatalf("providerManagedSigning = true, want false")
	}

	providerFields, err := signedInvocationCABIFields([]byte(`{
		"prepared":{"prepared_id":"prep-provider","request_id":"req-local"},
		"policy":{
			"mode":"provider_managed_signing",
			"signer_id":"signer-alice-key-1",
			"policy_ref":"provider-key-inventory:alice:signer-alice-key-1"
		}
	}`))
	if err != nil {
		t.Fatalf("signedInvocationCABIFields local failed: %v", err)
	}
	if providerFields.key != "prep-provider" || !providerFields.providerManagedSigning || len(providerFields.signatureJSON) != 0 {
		t.Fatalf("local fields = %#v", providerFields)
	}

	if _, err := signedInvocationCABIFields([]byte(`{
		"prepared":{"request_id":"req-only"},
		"signature":{"algorithm":"ed25519","signature_base64":"abc"}
	}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("request_id-only prepared key error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestCABIProviderManagedSignedInvocationRequiresCustodyFacts(t *testing.T) {
	tests := []struct {
		name    string
		policy  string
		message string
	}{
		{
			name:    "missing signer_id",
			policy:  `{"mode":"provider_managed_signing","policy_ref":"provider-key-inventory:alice:signer-alice-key-1"}`,
			message: "provider-managed signed invocation policy requires signer_id",
		},
		{
			name:    "blank signer_id",
			policy:  `{"mode":"provider_managed_signing","signer_id":"   ","policy_ref":"provider-key-inventory:alice:signer-alice-key-1"}`,
			message: "provider-managed signed invocation policy requires signer_id",
		},
		{
			name:    "missing policy_ref",
			policy:  `{"mode":"provider_managed_signing","signer_id":"signer-alice-key-1"}`,
			message: "provider-managed signed invocation policy requires policy_ref",
		},
		{
			name:    "blank policy_ref",
			policy:  `{"mode":"provider_managed_signing","signer_id":"signer-alice-key-1","policy_ref":"   "}`,
			message: "provider-managed signed invocation policy requires policy_ref",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := signedInvocationCABIFields([]byte(fmt.Sprintf(`{
				"prepared":{"prepared_id":"prep-provider","request_id":"req-local"},
				"policy":%s
			}`, tt.policy)))
			if !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
			}
			if err == nil || !strings.Contains(err.Error(), tt.message) {
				t.Fatalf("error = %v, want message %q", err, tt.message)
			}
		})
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

func TestCABIProviderClosedStateRejectsRuntimeCalls(t *testing.T) {
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

func TestCABIRuntimeProviderRequestsStreamCancelBeforeCanonicalTerminal(t *testing.T) {
	observation := observeCABIStreamLifecycle(t)

	if observation.cancel.State() != StreamCancelRequested ||
		observation.cancel.Terminal() ||
		observation.cancel.Cancelled() {
		t.Fatalf("stream cancel must be a non-terminal request: %#v", observation.cancel)
	}
	if !observation.terminal.Terminal() || len(observation.terminal.TerminalReceiptJSON()) == 0 {
		t.Fatalf("stream cancel did not drain a canonical terminal: %#v", observation.terminal)
	}
}

func TestCABIRuntimeProviderMemoizesConcurrentStreamCancellation(t *testing.T) {
	client := openFakeCABIRuntime(t)
	stream, err := client.InvokeStream(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	if _, err := stream.Next(context.Background()); err != nil {
		t.Fatalf("stream provider frame: %v", err)
	}

	var wait sync.WaitGroup
	errors := make(chan error, 2)
	for range 2 {
		wait.Add(1)
		go func() {
			defer wait.Done()
			_, err := stream.Cancel(context.Background(), "client stop")
			errors <- err
		}()
	}
	wait.Wait()
	close(errors)
	for err := range errors {
		if err != nil {
			t.Fatalf("concurrent stream cancel: %v", err)
		}
	}
	if _, err := stream.Next(context.Background()); err != nil {
		t.Fatalf("stream terminal event after concurrent cancel: %v", err)
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("stream close: %v", err)
	}
}

func TestCABIRuntimeProviderDispatchesStreamBeforeTerminal(t *testing.T) {
	observation := observeCABIStreamLifecycle(t)

	if observation.provider.Sequence() != 1 ||
		observation.provider.Terminal() ||
		string(observation.provider.PayloadJSON()) != `{"step":1}` {
		t.Fatalf("stream provider dispatch was not observable: %#v", observation.provider)
	}
	if observation.terminal.Sequence() <= observation.provider.Sequence() ||
		len(observation.terminal.TerminalReceiptJSON()) == 0 {
		t.Fatalf("stream terminal did not follow provider dispatch: %#v", observation.terminal)
	}
}

func TestCABIRuntimeProviderPreservesStreamOrderAndSingleTerminal(t *testing.T) {
	observation := observeCABIStreamLifecycle(t)

	if observation.provider.Sequence() != 1 ||
		observation.terminal.Sequence() != 2 ||
		observation.eventCount != 2 {
		t.Fatalf("stream ordering observation = %#v", observation)
	}
	if observation.closedState != StreamClosed ||
		observation.runtimeState != StreamTerminalFrameSeen {
		t.Fatalf(
			"stream close/runtime state = %s/%s",
			observation.closedState,
			observation.runtimeState,
		)
	}
}

func TestCABIRuntimeProviderRequestsBidiCancelBeforeCanonicalTerminal(t *testing.T) {
	observation := observeCABIBidiLifecycle(t)

	if observation.cancel.State() != BidiCancelRequested ||
		observation.cancel.Terminal() {
		t.Fatalf("bidi cancel must be a non-terminal request: %#v", observation.cancel)
	}
	if observation.cancel.Reason() != "client stop" {
		t.Fatalf("bidi cancel reason = %q, want caller reason", observation.cancel.Reason())
	}
	if !observation.terminal.Terminal() || len(observation.terminal.TerminalReceiptJSON()) == 0 {
		t.Fatalf("bidi cancel did not drain a canonical terminal: %#v", observation.terminal)
	}
}

func TestCABIRuntimeProviderMemoizesConcurrentBidiCancellation(t *testing.T) {
	client := openFakeCABIRuntime(t)
	session, err := client.OpenBidi(
		context.Background(),
		completeDraftForRuntimeTest(t),
		[]BidiStreamDescriptor{{
			StreamID: 1, ContentType: "application/json", Ordering: "STRICT",
		}},
	)
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	if _, err := session.Receive(context.Background()); err != nil {
		t.Fatalf("bidi provider frame: %v", err)
	}

	var wait sync.WaitGroup
	errors := make(chan error, 2)
	for range 2 {
		wait.Add(1)
		go func() {
			defer wait.Done()
			_, err := session.Cancel(context.Background(), "client stop")
			errors <- err
		}()
	}
	wait.Wait()
	close(errors)
	for err := range errors {
		if err != nil {
			t.Fatalf("concurrent bidi cancel: %v", err)
		}
	}
	if _, err := session.Receive(context.Background()); err != nil {
		t.Fatalf("bidi terminal frame after concurrent cancel: %v", err)
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("bidi close: %v", err)
	}
}

func TestCABIRuntimeProviderDispatchesBidiBeforeTerminal(t *testing.T) {
	observation := observeCABIBidiLifecycle(t)

	if observation.provider.Terminal() ||
		observation.provider.Kind() != "data" ||
		observation.ack.Sequence() != 1 ||
		observation.ack.Kind() != "data" {
		t.Fatalf("bidi provider dispatch observation = %#v", observation)
	}
	if len(observation.terminal.TerminalReceiptJSON()) == 0 {
		t.Fatalf("bidi terminal did not carry provider receipt: %#v", observation.terminal)
	}
	if observation.closedState != BidiClosed ||
		observation.runtimeState != BidiTerminal {
		t.Fatalf(
			"bidi close/runtime state = %s/%s",
			observation.closedState,
			observation.runtimeState,
		)
	}
}

type cabiStreamLifecycleObservation struct {
	provider     StreamEvent
	cancel       StreamCancel
	terminal     StreamEvent
	eventCount   int
	closedState  StreamState
	runtimeState StreamState
}

func observeCABIStreamLifecycle(t *testing.T) cabiStreamLifecycleObservation {
	t.Helper()
	client := openFakeCABIRuntime(t)
	stream, err := client.InvokeStream(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	first, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("stream first event: %v", err)
	}
	cancel, err := stream.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("stream cancel: %v", err)
	}
	terminal, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("stream terminal event after cancel request: %v", err)
	}
	eventCount := len(stream.Events())
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("stream close: %v", err)
	}
	return cabiStreamLifecycleObservation{
		provider:     first,
		cancel:       cancel,
		terminal:     terminal,
		eventCount:   eventCount,
		closedState:  stream.State(),
		runtimeState: stream.RuntimeState(),
	}
}

type cabiBidiLifecycleObservation struct {
	provider     BidiFrame
	ack          BidiFrame
	cancel       BidiOutcome
	terminal     BidiFrame
	closedState  BidiState
	runtimeState BidiState
}

func observeCABIBidiLifecycle(t *testing.T) cabiBidiLifecycleObservation {
	t.Helper()
	client := openFakeCABIRuntime(t)
	session, err := client.OpenBidi(context.Background(), completeDraftForRuntimeTest(t), []BidiStreamDescriptor{
		{StreamID: 1, ContentType: "application/json", Ordering: "STRICT"},
	})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	providerFrame, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("bidi provider frame: %v", err)
	}
	frame, err := NewBidiFrame(1, "data", 1)
	if err != nil {
		t.Fatalf("NewBidiFrame: %v", err)
	}
	ack, err := session.Send(context.Background(), frame)
	if err != nil {
		t.Fatalf("bidi send: %v", err)
	}
	bidiCancel, err := session.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("bidi cancel: %v", err)
	}
	received, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("bidi receive after cancel request: %v", err)
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("bidi close: %v", err)
	}
	return cabiBidiLifecycleObservation{
		provider:     providerFrame,
		ack:          ack,
		cancel:       bidiCancel,
		terminal:     received,
		closedState:  session.State(),
		runtimeState: session.RuntimeState(),
	}
}

func TestCABIRuntimeProviderEnforcesCallbackBackpressure(t *testing.T) {
	client := openFakeCABIRuntime(t)
	draft := cabiDraftWithMetadata(t, map[string]any{"conformance_backpressure": true})

	stream, err := client.InvokeStream(context.Background(), draft)
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	event, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("stream backpressure failure: %v", err)
	}
	assertCABIBackpressureError(t, event.ErrorJSON())
	if event.Terminal() || !event.TransportTerminal() {
		t.Fatalf("stream overflow event = %#v, want non-terminal transport failure", event)
	}
	if stream.State() != StreamFailed || stream.RuntimeState() != StreamOpen {
		t.Fatalf("stream state=%s runtime=%s, want Failed/Open", stream.State(), stream.RuntimeState())
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("close overflowed stream: %v", err)
	}

	session, err := client.OpenBidi(context.Background(), draft, []BidiStreamDescriptor{{
		StreamID: 1, ContentType: "application/json", Ordering: "STRICT",
	}})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	frame, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("bidi backpressure failure: %v", err)
	}
	assertCABIBackpressureError(t, frame.ErrorJSON())
	if frame.Terminal() || !frame.TransportTerminal() {
		t.Fatalf("bidi overflow frame = %#v, want non-terminal transport failure", frame)
	}
	if session.State() != BidiFailed || session.RuntimeState() != BidiOpen {
		t.Fatalf("bidi state=%s runtime=%s, want Failed/Open", session.State(), session.RuntimeState())
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("close overflowed bidi: %v", err)
	}
}

func TestCABIRuntimeProviderOwnsStreamReceiveDeadline(t *testing.T) {
	client := openFakeCABIRuntime(t)
	draft := completeDraftForRuntimeTest(t)

	stream, err := client.InvokeStream(context.Background(), draft)
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	if _, err := stream.Next(context.Background()); err != nil {
		t.Fatalf("stream provider frame: %v", err)
	}
	streamDeadline, cancelStreamDeadline := context.WithTimeout(context.Background(), 10*time.Millisecond)
	defer cancelStreamDeadline()
	if _, err := stream.Next(streamDeadline); !IsCode(err, ErrTimeout) {
		t.Fatalf("stream deadline error = %v, want %s", err, ErrTimeout)
	}
	if stream.RuntimeState() != StreamOpen {
		t.Fatalf("stream timeout changed runtime state to %s", stream.RuntimeState())
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("close timed-out stream: %v", err)
	}
	retryStream, err := client.InvokeStream(context.Background(), draft)
	if err != nil {
		t.Fatalf("retry InvokeStream: %v", err)
	}
	if _, err := retryStream.Next(context.Background()); err != nil {
		t.Fatalf("retry stream provider frame: %v", err)
	}
	if err := retryStream.Close(context.Background()); err != nil {
		t.Fatalf("close retry stream: %v", err)
	}
}

func TestCABIRuntimeProviderOwnsBidiReceiveDeadline(t *testing.T) {
	client := openFakeCABIRuntime(t)
	draft := completeDraftForRuntimeTest(t)
	session, err := client.OpenBidi(context.Background(), draft, []BidiStreamDescriptor{{
		StreamID: 1, ContentType: "application/json", Ordering: "STRICT",
	}})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	if _, err := session.Receive(context.Background()); err != nil {
		t.Fatalf("bidi provider frame: %v", err)
	}
	bidiDeadline, cancelBidiDeadline := context.WithTimeout(context.Background(), 10*time.Millisecond)
	defer cancelBidiDeadline()
	if _, err := session.Receive(bidiDeadline); !IsCode(err, ErrTimeout) {
		t.Fatalf("bidi deadline error = %v, want %s", err, ErrTimeout)
	}
	if session.RuntimeState() != BidiOpen {
		t.Fatalf("bidi timeout changed runtime state to %s", session.RuntimeState())
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("close timed-out bidi: %v", err)
	}
	retryBidi, err := client.OpenBidi(context.Background(), draft, []BidiStreamDescriptor{{
		StreamID: 1, ContentType: "application/json", Ordering: "STRICT",
	}})
	if err != nil {
		t.Fatalf("retry OpenBidi: %v", err)
	}
	if _, err := retryBidi.Receive(context.Background()); err != nil {
		t.Fatalf("retry bidi provider frame: %v", err)
	}
	if err := retryBidi.Close(context.Background()); err != nil {
		t.Fatalf("close retry bidi: %v", err)
	}
}

func TestCABIRuntimeProviderKeepsCloseSendDistinctFromCancel(t *testing.T) {
	client := openFakeCABIRuntime(t)
	session, err := client.OpenBidi(
		context.Background(),
		completeDraftForRuntimeTest(t),
		[]BidiStreamDescriptor{{StreamID: 1, ContentType: "application/json", Ordering: "STRICT"}},
	)
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	outcome, err := session.CloseSend(context.Background())
	if err != nil {
		t.Fatalf("CloseSend: %v", err)
	}
	if outcome.Terminal() || outcome.State() != BidiHalfClosedLocal || session.RuntimeState() != BidiHalfClosedLocal {
		t.Fatalf("close-send outcome=%#v runtime=%s", outcome, session.RuntimeState())
	}
	if _, err := session.Receive(context.Background()); err != nil {
		t.Fatalf("receive after close-send: %v", err)
	}
	frame, err := NewBidiFrame(1, "data", 1)
	if err != nil {
		t.Fatalf("NewBidiFrame: %v", err)
	}
	if _, err := session.Send(context.Background(), frame); !IsCode(err, ErrCancelled) {
		t.Fatalf("send after close-send error = %v, want %s", err, ErrCancelled)
	}
	receiveDeadline, cancelReceive := context.WithTimeout(context.Background(), 10*time.Millisecond)
	defer cancelReceive()
	if _, err := session.Receive(receiveDeadline); !IsCode(err, ErrTimeout) {
		t.Fatalf("close-send emitted cancellation terminal: %v", err)
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if session.RuntimeState() != BidiHalfClosedLocal {
		t.Fatalf("local close changed runtime state to %s", session.RuntimeState())
	}
}

func TestCABIRuntimeProviderRejectsMissingBidiFrameZero(t *testing.T) {
	client := openFakeCABIRuntime(t)

	_, err := client.OpenBidi(context.Background(), completeDraftForRuntimeTest(t), nil)

	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing frame0 error = %v, want %s", err, ErrInvalidArgument)
	}
}

func openFakeCABIRuntime(t *testing.T) *RuntimeClient {
	t.Helper()
	transport, err := openCABIRuntimeLifecycleTransport(buildFakeCABIStreamLibrary(t))
	if err != nil {
		t.Fatalf("openCABIRuntimeLifecycleTransport: %v", err)
	}
	control, err := NewRuntimeHost(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		t.Fatalf("NewRuntimeHost: %v", err)
	}
	handle, err := control.StartRuntime(context.Background(), testRuntimeHostStartRequest{
		payload: map[string]any{
			"mode":                "edge",
			"runtime_instance_id": "dev-a",
		},
	})
	if err != nil {
		_ = transport.Close(context.Background())
		t.Fatalf("Start: %v", err)
	}
	client, err := handle.OpenRuntime(context.Background(), ConnectOptions{})
	if err != nil {
		_ = transport.Close(context.Background())
		t.Fatalf("OpenRuntime: %v", err)
	}
	t.Cleanup(func() {
		if err := client.Close(context.Background()); err != nil {
			t.Errorf("Close runtime client: %v", err)
		}
		if err := transport.Close(context.Background()); err != nil {
			t.Errorf("Close C ABI runtime transport: %v", err)
		}
	})
	return client
}

func TestCABIRuntimeProviderResolvesDescriptorRefThroughNativeProvider(t *testing.T) {
	client := openFakeCABIRuntime(t)

	descriptorRef, err := client.ResolveDescriptorRef(context.Background(), RuntimeDescriptorRefRequest{
		CalleeURA:  "easynet:///r/example/agent/device.dev-a.runtime-introspection",
		Ability:    "meta.list_resources",
		CallMode:   "rpc",
		CallerURA:  "easynet:///r/example/device/caller",
		SubjectURA: "easynet:///r/example/device/dev-a",
		Provider:   "ability_descriptor",
	})

	if err != nil {
		t.Fatalf("ResolveDescriptorRef: %v", err)
	}
	const want = "easynet:///r/example/ability/system-agent.dev-a.runtime-introspection.meta.list_resources@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
	if descriptorRef != want {
		t.Fatalf("descriptor_ref = %q, want %q", descriptorRef, want)
	}
}

func TestCABIRuntimeProviderProjectsDescriptorResolverLastError(t *testing.T) {
	client := openFakeCABIRuntime(t)

	_, err := client.ResolveDescriptorRef(context.Background(), RuntimeDescriptorRefRequest{
		CalleeURA: "easynet:///r/example/device/dev-a",
		Ability:   "missing.descriptor",
		CallMode:  "rpc",
	})

	if err == nil {
		t.Fatal("ResolveDescriptorRef succeeded for missing descriptor")
	}
	if !IsCode(err, ErrDescriptorNotFound) {
		t.Fatalf("descriptor resolver error = %v, want %s", err, ErrDescriptorNotFound)
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) {
		t.Fatalf("descriptor resolver error is not SDKError: %T", err)
	}
	if sdkErr.Stage != "routing" {
		t.Fatalf("descriptor resolver stage = %q, want routing", sdkErr.Stage)
	}
	if sdkErr.Message == "" || strings.Contains(sdkErr.Message, "with code ") {
		t.Fatalf("descriptor resolver kept generic C ABI failure message: %#v", sdkErr)
	}
}

func cabiDraftWithMetadata(t *testing.T, metadata map[string]any) InvocationDraft {
	t.Helper()
	raw, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("marshal complete draft: %v", err)
	}
	var value map[string]any
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatalf("decode complete draft: %v", err)
	}
	value["metadata"] = metadata
	raw, err = json.Marshal(value)
	if err != nil {
		t.Fatalf("marshal draft metadata: %v", err)
	}
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON: %v", err)
	}
	return draft
}

func assertCABIBackpressureError(t *testing.T, raw json.RawMessage) {
	t.Helper()
	var failure struct {
		Code    string `json:"code"`
		Retry   string `json:"retry"`
		Details struct {
			WireCode string `json:"wire_code"`
			Reason   string `json:"reason"`
			Bounded  bool   `json:"bounded_queue"`
		} `json:"details"`
	}
	if err := json.Unmarshal(raw, &failure); err != nil {
		t.Fatalf("decode backpressure error: %v", err)
	}
	if failure.Code != string(ErrAdmissionDenied) ||
		failure.Retry != string(RetryAfterBackoff) ||
		failure.Details.WireCode != "RESOURCE_EXHAUSTED" ||
		failure.Details.Reason != "callback_queue_overflow" ||
		!failure.Details.Bounded {
		t.Fatalf("unexpected backpressure error: %#v", failure)
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

static stream_callback_t active_stream_callback = 0;
static void *active_stream_user_data = 0;
static int active_stream_cancel_calls = 0;
static bidi_callback_t active_bidi_callback = 0;
static void *active_bidi_user_data = 0;
static int active_bidi_cancel_calls = 0;
static const char *last_error_json = "{\"code\":\"INVALID_ARGUMENT\",\"stage\":\"fake\",\"message\":\"invalid fake C ABI request\",\"retry\":\"never\",\"details\":{}}";

static char *dup_json(const char *s) {
	size_t n = strlen(s);
	char *out = (char *)malloc(n + 1);
	if (out == 0) return 0;
	memcpy(out, s, n + 1);
	return out;
}

uint32_t runtime_abi_version(void) { return 7u; }
void runtime_string_free(char *s) { free(s); }
int32_t runtime_last_error_json(char **out_error_json) {
	*out_error_json = dup_json(last_error_json);
	return 0;
}

int32_t runtime_host_start(const char *config_json, uint64_t *out_daemon_handle) {
	(void)config_json;
	*out_daemon_handle = 606;
	return 0;
}
int32_t runtime_host_attach(const char *options_json, uint64_t *out_daemon_handle) {
	(void)options_json;
	*out_daemon_handle = 707;
	return 0;
}
int32_t runtime_host_discover(const char *options_json, char **out_discovery_json) {
	(void)options_json;
	*out_discovery_json = dup_json("{\"control_endpoint\":\"unix:///tmp/control.sock\",\"invocation_endpoint\":\"unix:///tmp/daemon.sock\",\"invocation_accepting\":true,\"diagnostics\":[]}");
	return 0;
}
int32_t runtime_host_stop(uint64_t handle) { (void)handle; return 0; }
int32_t runtime_host_detach(uint64_t handle) { (void)handle; return 0; }
int32_t runtime_host_status(uint64_t handle, char **out_status_json) {
	(void)handle;
	*out_status_json = dup_json("{\"state\":\"Running\",\"mode\":\"edge\",\"endpoints\":{\"control_endpoint\":\"unix:///tmp/control.sock\",\"invocation_endpoint\":\"unix:///tmp/daemon.sock\"},\"diagnostics\":[]}");
	return 0;
}
int32_t runtime_host_open_client(uint64_t daemon_handle, uint64_t *out_handle) {
	(void)daemon_handle;
	*out_handle = 808;
	return 0;
}
int32_t runtime_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t runtime_health(uint64_t handle, char **out_health_json) {
	(void)handle;
	*out_health_json = dup_json("{\"api_ready\":true,\"invocation_ready\":true,\"directory_ready\":true,\"trust_ready\":true,\"runtime_ready\":true,\"diagnostics\":[]}");
	return 0;
}
int32_t runtime_diagnostics(uint64_t handle, char **out_diagnostics_json) {
	(void)handle;
	*out_diagnostics_json = dup_json("{\"profile\":\"health\",\"kind\":\"diagnostics_report\",\"state\":\"Running\",\"ready\":true,\"version\":\"0.91.30\",\"abi_version\":5,\"control_endpoint\":\"/tmp/easynet/control.json\",\"invocation_endpoint\":\"/tmp/easynet/daemon.sock\",\"checks\":[{\"name\":\"api\",\"ready\":true,\"message\":null},{\"name\":\"daemon\",\"ready\":true,\"message\":null},{\"name\":\"invocation\",\"ready\":true,\"message\":null},{\"name\":\"directory\",\"ready\":true,\"message\":null},{\"name\":\"trust\",\"ready\":true,\"message\":null},{\"name\":\"runtime\",\"ready\":true,\"message\":null}],\"diagnostics\":[]}");
	return 0;
}
int32_t runtime_resolve_descriptor_ref(uint64_t handle, const char *request_json, char **out_descriptor_json) {
	(void)handle;
	if (strstr(request_json, "missing.descriptor") != 0) {
		last_error_json = "{\"code\":\"DESCRIPTOR_NOT_FOUND\",\"stage\":\"routing\",\"message\":\"descriptor_ref not found in remote runtime catalog\",\"retry\":\"never\",\"details\":{\"source\":\"fake_native_provider\"}}";
		return 4;
	}
	*out_descriptor_json = dup_json("{\"descriptor_ref\":\"easynet:///r/example/ability/system-agent.dev-a.runtime-introspection.meta.list_resources@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read\",\"ability_ura\":\"easynet:///r/example/ability/system-agent.dev-a.runtime-introspection.meta.list_resources\",\"owner_ura\":\"easynet:///r/example/agent/device.dev-a.runtime-introspection\",\"name\":\"meta.list_resources\",\"call_mode\":\"rpc\",\"source\":\"fake_native_provider\"}");
	return 0;
}
int32_t runtime_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle; (void)invocation_json; (void)out_result_json;
	return 10;
}
int32_t runtime_governance_read(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle; (void)invocation_json; (void)out_result_json;
	return 10;
}
int32_t runtime_invocation_prepare(uint64_t handle, const char *invocation_json, const char *options_json, uint64_t *out_prepared_id, char **out_prepared_json) {
	(void)handle; (void)invocation_json; (void)options_json; (void)out_prepared_id; (void)out_prepared_json;
	return 10;
}
int32_t runtime_invocation_sign_prepared(uint64_t prepared_id, const char *signature_json, uint64_t *out_signed_id, char **out_signed_json) {
	(void)prepared_id; (void)signature_json; (void)out_signed_id; (void)out_signed_json;
	return 10;
}
int32_t runtime_invocation_sign_prepared_local(uint64_t prepared_id, uint64_t *out_signed_id, char **out_signed_json) {
	(void)prepared_id; (void)out_signed_id; (void)out_signed_json;
	return 10;
}
int32_t runtime_invocation_submit_signed_handle(uint64_t handle, uint64_t signed_id, uint64_t *out_invocation_handle_id, char **out_submitted_json) {
	(void)handle; (void)signed_id; (void)out_invocation_handle_id; (void)out_submitted_json;
	return 10;
}
int32_t runtime_invocation_handle_await(uint64_t handle, uint64_t invocation_handle_id, char **out_result_json) {
	(void)handle; (void)invocation_handle_id; (void)out_result_json;
	return 10;
}
int32_t runtime_invocation_handle_cancel(uint64_t handle, uint64_t invocation_handle_id, const char *reason_json, char **out_cancel_json) {
	(void)handle; (void)invocation_handle_id; (void)reason_json; (void)out_cancel_json;
	return 10;
}
int32_t runtime_invocation_handle_events(uint64_t handle, uint64_t invocation_handle_id, char **out_events_json) {
	(void)handle; (void)invocation_handle_id; (void)out_events_json;
	return 10;
}
int32_t runtime_invocation_handle_free(uint64_t handle, uint64_t invocation_handle_id) {
	(void)handle; (void)invocation_handle_id;
	return 0;
}
int32_t runtime_prepared_invocation_free(uint64_t prepared_id) { (void)prepared_id; return 0; }
int32_t runtime_signed_invocation_free(uint64_t signed_id) { (void)signed_id; return 0; }

int32_t runtime_invocation_stream_open(uint64_t handle, const char *invocation_json, stream_callback_t on_chunk, void *user_data, uint64_t *out_stream_id) {
	(void)handle;
	*out_stream_id = 404;
	active_stream_callback = on_chunk;
	active_stream_user_data = user_data;
	active_stream_cancel_calls = 0;
	if (strstr(invocation_json, "conformance_backpressure") != 0) {
		char event[160];
		for (int sequence = 1; sequence <= 1025; sequence++) {
			snprintf(event, sizeof(event), "{\"sequence\":%d,\"kind\":\"data\",\"state\":\"Open\",\"terminal\":false}", sequence);
			on_chunk(user_data, event);
		}
	} else {
		on_chunk(user_data, "{\"sequence\":1,\"kind\":\"data\",\"state\":\"Open\",\"terminal\":false,\"payload_json\":{\"step\":1}}");
	}
	return 0;
}
int32_t runtime_invocation_stream_cancel(uint64_t handle, uint64_t stream_id) {
	(void)handle;
	active_stream_cancel_calls += 1;
	if (active_stream_cancel_calls > 1) return 1;
	if (stream_id == 404 && active_stream_callback != 0) {
		active_stream_callback(active_stream_user_data, "{\"sequence\":2,\"kind\":\"terminal\",\"state\":\"Cancelled\",\"terminal\":true,\"terminal_receipt\":{\"state\":\"Cancelled\",\"cleanup_complete\":true}}");
	}
	return stream_id == 404 ? 0 : 4;
}
int32_t runtime_invocation_stream_close(uint64_t handle, uint64_t stream_id) {
	(void)handle;
	active_stream_callback = 0;
	active_stream_user_data = 0;
	return stream_id == 404 ? 0 : 4;
}
int32_t runtime_invocation_bidi_open(uint64_t handle, const char *invocation_json, bidi_callback_t on_frame, void *user_data, uint64_t *out_bidi_id) {
	(void)handle;
	if (strstr(invocation_json, "\"bidi_streams\":[]") != 0) return 1;
	*out_bidi_id = 505;
	active_bidi_callback = on_frame;
	active_bidi_user_data = user_data;
	active_bidi_cancel_calls = 0;
	if (strstr(invocation_json, "conformance_backpressure") != 0) {
		char frame[160];
		for (int sequence = 1; sequence <= 1025; sequence++) {
			snprintf(frame, sizeof(frame), "{\"sequence\":%d,\"kind\":\"data\",\"stream_id\":1,\"terminal\":false}", sequence);
			on_frame(user_data, frame);
		}
	} else {
		on_frame(user_data, "{\"sequence\":1,\"kind\":\"data\",\"stream_id\":1,\"terminal\":false,\"payload_json\":{\"provider\":\"cabi\"}}");
	}
	return 0;
}
int32_t runtime_invocation_bidi_send(uint64_t handle, uint64_t bidi_id, const char *frame_json) {
	(void)handle; (void)frame_json;
	return bidi_id == 505 ? 0 : 4;
}
int32_t runtime_invocation_bidi_close_send(uint64_t handle, uint64_t bidi_id) {
	(void)handle;
	return bidi_id == 505 ? 0 : 4;
}
int32_t runtime_invocation_bidi_close(uint64_t handle, uint64_t bidi_id) {
	(void)handle;
	active_bidi_callback = 0;
	active_bidi_user_data = 0;
	return bidi_id == 505 ? 0 : 4;
}
int32_t runtime_invocation_bidi_cancel(uint64_t handle, uint64_t bidi_id) {
	(void)handle;
	active_bidi_cancel_calls += 1;
	if (active_bidi_cancel_calls > 1) return 1;
	if (bidi_id == 505 && active_bidi_callback != 0) {
		active_bidi_callback(active_bidi_user_data, "{\"sequence\":2,\"kind\":\"terminal\",\"stream_id\":1,\"terminal\":true,\"terminal_receipt\":{\"state\":\"Cancelled\",\"cleanup_complete\":true}}");
	}
	return bidi_id == 505 ? 0 : 4;
}
`
