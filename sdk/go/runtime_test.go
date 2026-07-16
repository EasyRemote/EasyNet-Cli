package easynet

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
)

func completeDraftForRuntimeTest(t *testing.T) InvocationDraft {
	t.Helper()
	draft, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		Build()
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	return draft
}

func signedForRuntimeTest(t *testing.T) SignedInvocation {
	t.Helper()
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	signed, err := prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:       "ed25519",
		SignatureBase64: "c2lnbmF0dXJl",
		KeyIDHint:       "caller-key",
	})
	if err != nil {
		t.Fatalf("SignWithCallerSignature: %v", err)
	}
	return signed
}

func submittedHandleForRuntimeTest(t *testing.T) InvocationHandle {
	t.Helper()
	handle, err := newRuntimeInvocationHandleFromJSON([]byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}], "result": null}`))
	if err != nil {
		t.Fatalf("newRuntimeInvocationHandleFromJSON: %v", err)
	}
	return handle
}

func TestPublicInvocationHandleJSONDoesNotGrantControlAuthority(t *testing.T) {
	handle, err := NewInvocationHandleFromJSON([]byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [], "result": null}`))
	if err != nil {
		t.Fatalf("NewInvocationHandleFromJSON: %v", err)
	}
	if handle.State() != "Submitted" || handle.Terminal() {
		t.Fatalf("unexpected observation snapshot: %#v", handle)
	}
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		AwaitHandleFunc: func(context.Context, InvocationControlCapability) ([]byte, error) {
			t.Fatalf("forged public snapshot reached transport")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := client.Await(context.Background(), handle); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Await forged public snapshot = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestInvocationControlCapabilityAdapterHandleID(t *testing.T) {
	runtimeHandle := submittedHandleForRuntimeTest(t)
	handleID, err := runtimeHandle.ControlCapability().AdapterHandleID()
	if err != nil {
		t.Fatalf("runtime-bound AdapterHandleID: %v", err)
	}
	if handleID != 7 {
		t.Fatalf("runtime-bound AdapterHandleID = %d, want 7", handleID)
	}

	publicSnapshot, err := NewInvocationHandleFromJSON([]byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [], "result": null}`))
	if err != nil {
		t.Fatalf("NewInvocationHandleFromJSON: %v", err)
	}
	if _, err := publicSnapshot.ControlCapability().AdapterHandleID(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("snapshot AdapterHandleID = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPublicInvocationCancelJSONDoesNotGrantControlAuthority(t *testing.T) {
	cancel, err := NewInvocationCancelFromJSON([]byte(`{"handle_id": 7, "request_accepted": false, "deduplicated": true, "cancelled": false, "state": "Completed", "terminal": true}`))
	if err != nil {
		t.Fatalf("NewInvocationCancelFromJSON: %v", err)
	}
	if cancel.State() != "Completed" || !cancel.Terminal() {
		t.Fatalf("unexpected cancel snapshot: %#v", cancel)
	}
	if cancel.ControlCapability().valid() {
		t.Fatalf("public cancel snapshot created runtime-bound control")
	}
}

func TestRuntimeClientPrepareDelegatesToTransport(t *testing.T) {
	var seenDraft map[string]any
	var seenOptions map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			if err := json.Unmarshal(optionsJSON, &seenOptions); err != nil {
				t.Fatalf("options JSON: %v", err)
			}
			return []byte(preparedFixture), nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			t.Fatalf("SubmitSigned should not be called")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	prepared, material, err := client.Prepare(context.Background(), completeDraftForRuntimeTest(t), PrepareOptions{
		ExpiresInMS:        60000,
		SignerID:           "signer-alice-key-1",
		PolicyRef:          "daemon-key-inventory:sha256:test-policy",
		LocalDaemonSigning: true,
	})
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	if prepared.SubmitReady() {
		t.Fatalf("prepared is submit-ready")
	}
	if material.CanonicalBytesBase64() == "" {
		t.Fatalf("signing material missing")
	}
	if seenDraft["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("draft not sent to transport: %#v", seenDraft)
	}
	if seenOptions["expires_in_ms"].(float64) != 60000 ||
		seenOptions["signer_id"] != "signer-alice-key-1" ||
		seenOptions["policy_ref"] != "daemon-key-inventory:sha256:test-policy" ||
		seenOptions["local_daemon_signing"] != true {
		t.Fatalf("latest prepare options not sent to transport: %#v", seenOptions)
	}
}

func TestRuntimeClientPrepareSigningMaterialUsesStatelessTransportContract(t *testing.T) {
	var seenOptions map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(_ context.Context, _ []byte, optionsJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(optionsJSON, &seenOptions); err != nil {
				t.Fatalf("options JSON: %v", err)
			}
			return []byte(strings.Replace(preparedFixture, "  \"prepared_id\": \"prepared-example-1\",\n", "", 1)), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	material, err := client.PrepareSigningMaterial(context.Background(), completeDraftForRuntimeTest(t), PrepareOptions{
		ExpiresInMS: 60_000,
		SignerID:    "browser-key-1",
	})
	if err != nil {
		t.Fatalf("PrepareSigningMaterial: %v", err)
	}
	if material.CanonicalBytesBase64() == "" {
		t.Fatal("canonical signing material is missing")
	}
	if _, err := NewPreparedInvocationFromJSON([]byte(strings.Replace(preparedFixture, "  \"prepared_id\": \"prepared-example-1\",\n", "", 1))); err == nil {
		t.Fatal("retained prepared decoder accepted a material-only response")
	}
	if seenOptions["material_only"] != true ||
		seenOptions["expires_in_ms"] != float64(60_000) ||
		seenOptions["signer_id"] != "browser-key-1" {
		t.Fatalf("stateless prepare options not sent to transport: %#v", seenOptions)
	}
}

func TestRuntimeReceiptValidatesSummaryHashes(t *testing.T) {
	receipt, err := NewRuntimeReceiptFromJSON([]byte(`{
		"index": 1,
		"invocation_id": "inv-1",
		"receipt_type": "completed",
		"state": "completed",
		"timestamp_unix_ms": 1700000000000,
		"prev_receipt_hash_hex": "` + strings.Repeat("00", 32) + `",
		"self_hash_hex": "` + strings.Repeat("aa", 32) + `",
		"causal_binding_kind": "scalar",
		"causal_binding": {
			"form": "scalar",
			"receipt": {
				"receipt_hash_hex": "` + strings.Repeat("bb", 32) + `",
				"receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/root/receipt"
			}
		},
		"authority_binding_kind": "delegation",
		"authority_binding": {
			"kind": "delegation",
			"issuer_ura": "easynet:///r/example/agent/issuer",
			"subject_ura": "easynet:///r/example/resource/subject",
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"audience": "runtime",
			"scopes": ["invoke"]
		}
	}`))
	if err != nil {
		t.Fatalf("NewRuntimeReceiptFromJSON: %v", err)
	}
	if err := receipt.ValidateSummary(); err != nil {
		t.Fatalf("ValidateSummary: %v", err)
	}
	selfHash, err := receipt.SelfReceiptHash()
	if err != nil {
		t.Fatalf("SelfReceiptHash: %v", err)
	}
	if !bytes.Equal(selfHash, bytes.Repeat([]byte{0xaa}, 32)) {
		t.Fatalf("self hash = %x", selfHash)
	}
	if receipt.CausalBindingKind != "scalar" || receipt.CausalBinding["form"] != "scalar" {
		t.Fatalf("causal binding not decoded: %#v", receipt.CausalBinding)
	}
	if receipt.AuthorityBindingKind != "delegation" || receipt.AuthorityBinding["kind"] != "delegation" {
		t.Fatalf("authority binding not decoded: %#v", receipt.AuthorityBinding)
	}
}

func TestRuntimeReceiptRejectsMalformedSummaryHash(t *testing.T) {
	receipt, err := NewRuntimeReceiptFromJSON([]byte(`{
		"index": 1,
		"invocation_id": "inv-1",
		"receipt_type": "completed",
		"timestamp_unix_ms": 1700000000000,
		"prev_receipt_hash_hex": "` + strings.Repeat("00", 32) + `",
		"self_hash_hex": "aa"
	}`))
	if err != nil {
		t.Fatalf("NewRuntimeReceiptFromJSON: %v", err)
	}
	if err := receipt.ValidateSummary(); err == nil {
		t.Fatal("ValidateSummary accepted short self hash")
	}
}

func TestRuntimeClientPrepareBuilderConsumesOnlyAfterSuccess(t *testing.T) {
	transportPrepareCalls := 0
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			transportPrepareCalls++
			return []byte(preparedFixture), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	builder := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json")

	prepared, material, err := client.PrepareBuilder(context.Background(), builder, PrepareOptions{})
	if err != nil {
		t.Fatalf("PrepareBuilder: %v", err)
	}
	if prepared.PreparedID() == "" || material.CanonicalBytesBase64() == "" || transportPrepareCalls != 1 {
		t.Fatalf("unexpected prepare-builder result: prepared=%#v material=%#v calls=%d", prepared, material, transportPrepareCalls)
	}
	if _, err := builder.Inspect(); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("Inspect after PrepareBuilder = %v, want %s", err, ErrInvalidHandle)
	}
}

func TestRuntimeClientPrepareBuilderKeepsBuilderOnFailure(t *testing.T) {
	down := errors.New("daemon unavailable")
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			return nil, down
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	builder := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json")

	if _, _, err := client.PrepareBuilder(context.Background(), builder, PrepareOptions{}); !IsCode(err, ErrTransport) {
		t.Fatalf("PrepareBuilder failure = %v, want %s", err, ErrTransport)
	}
	if _, err := builder.Inspect(); err != nil {
		t.Fatalf("builder consumed after failed PrepareBuilder: %v", err)
	}
}

func TestRuntimeClientSubmitSignedPreservesSignature(t *testing.T) {
	var seenSigned map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			t.Fatalf("Prepare should not be called")
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(signedJSON, &seenSigned); err != nil {
				t.Fatalf("signed JSON: %v", err)
			}
			return []byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}], "result": null}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	handle, err := client.SubmitSigned(context.Background(), signedForRuntimeTest(t))
	if err != nil {
		t.Fatalf("SubmitSigned: %v", err)
	}
	if !handle.ControlCapability().valid() || handle.State() != "Submitted" || handle.Terminal() {
		t.Fatalf("unexpected handle: %#v", handle)
	}
	if len(handle.Events()) != 1 || handle.Events()[0].Sequence() != 1 {
		t.Fatalf("unexpected handle events: %#v", handle.Events())
	}
	signature := seenSigned["signature"].(map[string]any)
	if signature["signature_base64"] != "c2lnbmF0dXJl" {
		t.Fatalf("signature not preserved: %#v", seenSigned)
	}
	prepared := seenSigned["prepared"].(map[string]any)
	tuple := prepared["tuple"].(map[string]any)
	if tuple["caller_ura"] != "easynet:///r/example/agent/alice.sdk" ||
		tuple["callee_ura"] != "easynet:///r/example/device/dev-a" ||
		tuple["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" {
		t.Fatalf("prepared tuple not preserved: %#v", seenSigned)
	}
}

func TestRuntimeClientPrepareWrapsTransportFailure(t *testing.T) {
	down := errors.New("daemon unavailable")
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			return nil, down
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	_, _, err = client.Prepare(context.Background(), completeDraftForRuntimeTest(t), PrepareOptions{})
	if err == nil {
		t.Fatalf("Prepare succeeded, want transport error")
	}
	if !IsCode(err, ErrTransport) {
		t.Fatalf("error code = %v, want %s", err, ErrTransport)
	}
	if !errors.Is(err, down) {
		t.Fatalf("transport cause not preserved")
	}
}

func TestRuntimeClientSubmitRejectsMalformedHandle(t *testing.T) {
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			return []byte(`{"state": "Submitted"}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	_, err = client.SubmitSigned(context.Background(), signedForRuntimeTest(t))
	if err == nil {
		t.Fatalf("SubmitSigned succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeClientInvokeReturnsTypedResult(t *testing.T) {
	var seenDraft map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			return []byte(fmt.Sprintf(`{
				"ok": true,
				"tuple": %s,
				"invocation_id": "inv-runtime-1",
				"terminal_state": "Completed",
				"output_content_type": "application/json",
				"output_base64": "eyJyZWFkeSI6dHJ1ZX0=",
				"output_json": {"ready": true},
					"selected_node_id": "node-a",
					"scheduling_reason": "direct",
					"elapsed_ms": 12,
					"terminal_receipt": {"receipt_id": "receipt-1"},
					"error": null
				}`, draftJSON)), nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			t.Fatalf("Prepare should not be called")
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			t.Fatalf("SubmitSigned should not be called")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	result, err := client.Invoke(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if !result.OK() || result.TerminalState() != "Completed" {
		t.Fatalf("unexpected result: %#v", result)
	}
	if result.Tuple().CallerURA() != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("tuple not decoded: %#v", result.Tuple())
	}
	if result.InvocationID() != "inv-runtime-1" {
		t.Fatalf("invocation id = %q", result.InvocationID())
	}
	if seenDraft["descriptor_ref"] == "" {
		t.Fatalf("draft not sent to transport: %#v", seenDraft)
	}
	if string(result.OutputJSON()) != `{"ready": true}` {
		t.Fatalf("output JSON not preserved: %s", result.OutputJSON())
	}
}

func TestInvocationResultSeparatesAdmissionAndTerminalReceipts(t *testing.T) {
	draftJSON, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("marshal draft: %v", err)
	}
	raw := []byte(fmt.Sprintf(`{
			"ok":true,
			"tuple":%s,
			"terminal_state":"Completed",
			"admission_receipt":{"index":0,"state":"Admitted"},
			"terminal_receipt":{"index":1,"state":"Completed"},
			"error":null
	}`, draftJSON))
	result, err := NewInvocationResultFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationResultFromJSON: %v", err)
	}
	if !strings.Contains(string(result.AdmissionReceipt()), `"index":0`) {
		t.Fatalf("admission receipt = %s", result.AdmissionReceipt())
	}
	if !strings.Contains(string(result.TerminalReceipt()), `"index":1`) {
		t.Fatalf("terminal receipt = %s", result.TerminalReceipt())
	}
	if summary := result.TerminalReceiptSummary(); summary == nil || summary.Index != 1 || summary.State != "Completed" {
		t.Fatalf("terminal receipt summary = %#v", summary)
	}
	if summary := result.AdmissionReceiptSummary(); summary == nil || summary.Index != 0 || summary.State != "Admitted" {
		t.Fatalf("admission receipt summary = %#v", summary)
	}

	legacyOnly := bytes.Replace(raw, []byte(`"terminal_receipt":{"index":1,"state":"Completed"}`), []byte(`"receipt":{"index":1,"state":"Completed"}`), 1)
	legacy, err := NewInvocationResultFromJSON(legacyOnly)
	if err != nil {
		t.Fatalf("legacy receipt-only field should be ignored, not decoded as canonical terminal receipt: %v", err)
	}
	if len(legacy.TerminalReceipt()) != 0 || legacy.TerminalReceiptSummary() != nil {
		t.Fatalf("legacy receipt-only field must not populate canonical terminal receipt")
	}
}

func TestRuntimeClientInvokeStreamOpensStreamHandle(t *testing.T) {
	var seenDraft map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		OpenStreamFunc: func(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			return &memoryStreamTransport{events: []string{
				`{"sequence":1,"kind":"terminal","state":"Completed","terminal":true}`,
			}}, []byte(`{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	stream, err := client.InvokeStream(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	if stream.StreamID() != "stream-1" || stream.State() != StreamOpening {
		t.Fatalf("unexpected stream: id=%q state=%s", stream.StreamID(), stream.State())
	}
	if seenDraft["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("draft not forwarded: %#v", seenDraft)
	}
}

func TestRuntimeClientOpenBidiOpensSession(t *testing.T) {
	var seenDraft map[string]any
	var seenStreams []map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		OpenBidiFunc: func(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			if err := json.Unmarshal(streamsJSON, &seenStreams); err != nil {
				t.Fatalf("streams JSON: %v", err)
			}
			return &memoryBidiTransport{}, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	session, err := client.OpenBidi(context.Background(), completeDraftForRuntimeTest(t), []BidiStreamDescriptor{
		{StreamID: 1, ContentType: "application/json", Ordering: "ordered"},
	})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	if session.SessionID() != "bidi-1" || session.State() != BidiOpen {
		t.Fatalf("unexpected bidi session: id=%q state=%s", session.SessionID(), session.State())
	}
	if seenDraft["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("draft not forwarded: %#v", seenDraft)
	}
	if len(seenStreams) != 1 || seenStreams[0]["stream_id"] != float64(1) || seenStreams[0]["content_type"] != "application/json" {
		t.Fatalf("streams not forwarded: %#v", seenStreams)
	}
}

func TestInvocationResultRejectsInconsistentFailure(t *testing.T) {
	draftJSON, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Marshal draft: %v", err)
	}

	_, err = NewInvocationResultFromJSON([]byte(fmt.Sprintf(`{
		"ok": false,
		"tuple": %s,
		"terminal_state": "Failed",
		"output_content_type": "application/json",
		"output_base64": "",
		"output_json": null,
		"elapsed_ms": 3,
		"receipt": null,
		"error": null
	}`, draftJSON)))
	if err == nil {
		t.Fatalf("NewInvocationResultFromJSON succeeded, want invalid result")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeClientHandleObservationDelegatesToTransport(t *testing.T) {
	draftJSON, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Marshal draft: %v", err)
	}
	var seenAwaitID uint64
	var seenFreeID uint64
	var seenCancelReason string
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			t.Fatalf("Prepare should not be called")
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			t.Fatalf("SubmitSigned should not be called")
			return nil, nil
		},
		AwaitHandleFunc: func(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
			seenAwaitID = control.adapterHandleID()
			return []byte(fmt.Sprintf(`{
				"ok": true,
				"tuple": %s,
				"terminal_state": "Completed",
				"output_content_type": "application/json",
				"output_base64": "e30=",
				"output_json": {},
				"elapsed_ms": 8,
				"receipt": null,
				"error": null
			}`, draftJSON)), nil
		},
		CancelHandleFunc: func(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
			if control.adapterHandleID() != 7 {
				t.Fatalf("control handle = %d, want 7", control.adapterHandleID())
			}
			seenCancelReason = reason
			return []byte(`{"handle_id": 7, "request_accepted": false, "deduplicated": true, "cancelled": false, "state": "Completed", "terminal": true}`), nil
		},
		HandleEventsFunc: func(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
			if control.adapterHandleID() != 7 {
				t.Fatalf("control handle = %d, want 7", control.adapterHandleID())
			}
			return []byte(`{"handle_id": 7, "state": "Cancelled", "terminal": true, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}, {"sequence": 2, "kind": "cancelled", "state": "Cancelled", "terminal": true, "reason": "client stop"}], "result": null}`), nil
		},
		FreeHandleFunc: func(ctx context.Context, control InvocationControlCapability) error {
			seenFreeID = control.adapterHandleID()
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	handle := submittedHandleForRuntimeTest(t)

	result, err := client.Await(context.Background(), handle)
	if err != nil {
		t.Fatalf("Await: %v", err)
	}
	if seenAwaitID != 7 || !result.OK() {
		t.Fatalf("await did not use handle id/result: id=%d result=%#v", seenAwaitID, result)
	}
	cancelled, err := client.Cancel(context.Background(), handle, "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	if cancelled.RequestAccepted() || !cancelled.Deduplicated() || cancelled.Cancelled() || !cancelled.Terminal() || cancelled.State() != "Completed" || seenCancelReason != "client stop" {
		t.Fatalf("unexpected cancellation: %#v reason=%q", cancelled, seenCancelReason)
	}
	mismatchedCancelClient, err := NewRuntimeClient(RuntimeTransportFunc{
		CancelHandleFunc: func(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
			return []byte(`{"handle_id": 8, "request_accepted": true, "deduplicated": false, "cancelled": true, "state": "Cancelled", "terminal": true}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient mismatched cancel: %v", err)
	}
	if _, err := mismatchedCancelClient.Cancel(context.Background(), handle, "client stop"); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Cancel mismatched handle = %v, want %s", err, ErrInvalidArgument)
	}
	events, err := client.Events(context.Background(), handle)
	if err != nil {
		t.Fatalf("Events: %v", err)
	}
	if !events.Terminal() || len(events.Events()) != 2 || events.Events()[1].Reason() != "client stop" {
		t.Fatalf("unexpected events: %#v", events.Events())
	}
	if err := client.CloseHandle(context.Background(), handle); err != nil {
		t.Fatalf("CloseHandle: %v", err)
	}
	if seenFreeID != 7 {
		t.Fatalf("free did not use handle id: id=%d", seenFreeID)
	}
}

func TestRuntimeClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	var closeCalls int
	var invokeCalls int
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			invokeCalls++
			return []byte(`{}`), nil
		},
		CloseFunc: func(ctx context.Context) error {
			closeCalls++
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", closeCalls)
	}
	_, err = client.Invoke(context.Background(), completeDraftForRuntimeTest(t))
	if err == nil {
		t.Fatalf("Invoke after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if invokeCalls != 0 {
		t.Fatalf("invoke reached transport after close: %d calls", invokeCalls)
	}
}

func TestRuntimeClientCloseFailureIsTerminal(t *testing.T) {
	down := errors.New("close failed")
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called after failed close")
			return nil, nil
		},
		CloseFunc: func(ctx context.Context) error {
			return down
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	err = client.Close(context.Background())
	if err == nil {
		t.Fatalf("Close succeeded, want transport error")
	}
	if !IsCode(err, ErrTransport) || !errors.Is(err, down) {
		t.Fatalf("close error not wrapped as transport cause: %v", err)
	}
	_, err = client.Invoke(context.Background(), completeDraftForRuntimeTest(t))
	if err == nil {
		t.Fatalf("Invoke after failed close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}
