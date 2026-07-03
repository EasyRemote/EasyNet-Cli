package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
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
	handle, err := NewInvocationHandleFromJSON([]byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}], "result": null}`))
	if err != nil {
		t.Fatalf("NewInvocationHandleFromJSON: %v", err)
	}
	return handle
}

func TestRuntimeClientPrepareDelegatesToTransport(t *testing.T) {
	var seenDraft map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
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

	prepared, material, err := client.Prepare(context.Background(), completeDraftForRuntimeTest(t), PrepareOptions{ExpiresInMS: 60000})
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
	if handle.HandleID() != 7 || handle.State() != "Submitted" || handle.Terminal() {
		t.Fatalf("unexpected handle: %#v", handle)
	}
	if len(handle.Events()) != 1 || handle.Events()[0].Sequence() != 1 {
		t.Fatalf("unexpected handle events: %#v", handle.Events())
	}
	signature := seenSigned["signature"].(map[string]any)
	if signature["signature_base64"] != "c2lnbmF0dXJl" {
		t.Fatalf("signature not preserved: %#v", seenSigned)
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
				"terminal_state": "Completed",
				"output_content_type": "application/json",
				"output_base64": "eyJyZWFkeSI6dHJ1ZX0=",
				"output_json": {"ready": true},
				"selected_node_id": "node-a",
				"scheduling_reason": "direct",
				"elapsed_ms": 12,
				"receipt": {"receipt_id": "receipt-1"},
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
	if seenDraft["descriptor_ref"] == "" {
		t.Fatalf("draft not sent to transport: %#v", seenDraft)
	}
	if string(result.OutputJSON()) != `{"ready": true}` {
		t.Fatalf("output JSON not preserved: %s", result.OutputJSON())
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
				`{"sequence":1,"event":"terminal","state":"Completed","terminal":true}`,
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
		AwaitHandleFunc: func(ctx context.Context, handleID uint64) ([]byte, error) {
			seenAwaitID = handleID
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
		CancelHandleFunc: func(ctx context.Context, handleID uint64, reason string) ([]byte, error) {
			if handleID != 7 {
				t.Fatalf("handleID = %d, want 7", handleID)
			}
			seenCancelReason = reason
			return []byte(`{"handle_id": 7, "cancelled": true, "state": "Cancelled", "terminal": true}`), nil
		},
		HandleEventsFunc: func(ctx context.Context, handleID uint64) ([]byte, error) {
			if handleID != 7 {
				t.Fatalf("handleID = %d, want 7", handleID)
			}
			return []byte(`{"handle_id": 7, "state": "Cancelled", "terminal": true, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}, {"sequence": 2, "kind": "cancelled", "state": "Cancelled", "terminal": true, "reason": "client stop"}], "result": null}`), nil
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
	if !cancelled.Cancelled() || !cancelled.Terminal() || seenCancelReason != "client stop" {
		t.Fatalf("unexpected cancellation: %#v reason=%q", cancelled, seenCancelReason)
	}
	events, err := client.Events(context.Background(), handle)
	if err != nil {
		t.Fatalf("Events: %v", err)
	}
	if !events.Terminal() || len(events.Events()) != 2 || events.Events()[1].Reason() != "client stop" {
		t.Fatalf("unexpected events: %#v", events.Events())
	}
}
