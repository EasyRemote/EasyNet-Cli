package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

func TestRuntimeAbilityClientBuildsCompleteCanonicalDraft(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: testResolveDescriptorRef(t),
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	call := runtimeAbilityTestContext()
	draft, err := client.Build(context.Background(), call, "namespace.resolve", map[string]any{"ura": "easynet:///r/example/user/alice"})
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	if draft.CallerURA() != call.CallerURA || draft.CalleeURA() != call.CalleeURA || draft.NonceBase64() != call.NonceBase64 {
		t.Fatalf("call context changed: %#v", draft)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/hub.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read" {
		t.Fatalf("descriptor_ref = %q", draft.DescriptorRef())
	}
	if draft.SubjectURA() == call.SubjectURA {
		t.Fatalf("user subject was not descriptor-bound: %q", draft.SubjectURA())
	}
	call.Metadata["request_id"] = "mutated"
	if draft.Metadata()["request_id"] != "call-1" {
		t.Fatal("draft metadata aliases caller-owned state")
	}
}

func TestRuntimeAbilityClientInvokesObjectResult(t *testing.T) {
	var seen map[string]any
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		if err := json.Unmarshal(raw, &seen); err != nil {
			return nil, err
		}
		return runtimeAbilityResultJSON(true, `{"answer_kind":"positive"}`, "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	output, err := client.Invoke(context.Background(), runtimeAbilityTestContext(), "namespace.resolve", map[string]any{"name": "alice"})
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if output["answer_kind"] != "positive" || seen["descriptor_ref"] == "" {
		t.Fatalf("unexpected invocation projection: output=%#v draft=%#v", output, seen)
	}
}

func TestRuntimeAbilityClientDispatchesProviderLifecycleSurfaces(t *testing.T) {
	var seenInvoke map[string]any
	var seenStream map[string]any
	var seenBidi map[string]any
	var seenRecovery map[string]any
	var seenStreams []map[string]any
	var seenCancelReason string
	var descriptorModes []string
	transport := RuntimeTransportFunc{
		InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
			if err := json.Unmarshal(raw, &seenInvoke); err != nil {
				return nil, err
			}
			return runtimeAbilityResultJSON(true, `{"answer_kind":"positive"}`, "", false), nil
		},
		OpenStreamFunc: func(_ context.Context, raw []byte) (StreamTransport, []byte, error) {
			if err := json.Unmarshal(raw, &seenStream); err != nil {
				return nil, nil, err
			}
			return &memoryStreamTransport{events: []string{
				`{"sequence":1,"kind":"terminal","state":"Completed","terminal":true}`,
			}}, []byte(`{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}`), nil
		},
		OpenBidiFunc: func(_ context.Context, raw []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
			if err := json.Unmarshal(raw, &seenBidi); err != nil {
				return nil, nil, err
			}
			if err := json.Unmarshal(streamsJSON, &seenStreams); err != nil {
				return nil, nil, err
			}
			return &memoryBidiTransport{}, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`), nil
		},
		AwaitHandleFunc: func(_ context.Context, _ InvocationControlCapability) ([]byte, error) {
			return runtimeAbilityResultJSON(true, `{"observed":true}`, "", false), nil
		},
		CancelHandleFunc: func(_ context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
			if control.adapterHandleID() != 7 {
				t.Fatalf("cancel handle id = %d, want 7", control.adapterHandleID())
			}
			seenCancelReason = reason
			return []byte(`{"handle_id":7,"request_accepted":true,"deduplicated":false,"cancelled":true,"state":"CancelRequested","terminal":false}`), nil
		},
		RecoverFunc: func(_ context.Context, raw []byte) ([]byte, error) {
			if err := json.Unmarshal(raw, &seenRecovery); err != nil {
				return nil, err
			}
			return runtimeRecoveryReportJSON("recovery-1"), nil
		},
		ResolveDescriptorRefFunc: func(_ context.Context, requestJSON []byte) ([]byte, error) {
			var request map[string]any
			if err := json.Unmarshal(requestJSON, &request); err != nil {
				return nil, err
			}
			descriptorModes = append(descriptorModes, request["call_mode"].(string))
			return []byte(`{"descriptor_ref":"easynet:///r/example/ability/hub.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"}`), nil
		},
	}
	runtime, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}

	output, err := client.Invoke(context.Background(), runtimeAbilityTestContext(), "namespace.resolve", map[string]any{"name": "alice"})
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	stream, err := client.OpenStream(context.Background(), runtimeAbilityTestContext(), "namespace.resolve", map[string]any{"name": "alice"})
	if err != nil {
		t.Fatalf("OpenStream: %v", err)
	}
	session, err := client.OpenBidi(context.Background(), runtimeAbilityTestContext(), "namespace.resolve", map[string]any{"name": "alice"}, []BidiStreamDescriptor{
		{StreamID: 1, ContentType: "application/json", Ordering: "ordered"},
	})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	result, err := client.Await(context.Background(), submittedHandleForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Await: %v", err)
	}
	cancelled, err := client.Cancel(context.Background(), submittedHandleForRuntimeTest(t), "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	recovery, err := client.Recover(context.Background(), runtimeRecoveryRequestForTest())
	if err != nil {
		t.Fatalf("Recover: %v", err)
	}

	if output["answer_kind"] != "positive" || seenInvoke["descriptor_ref"] == "" {
		t.Fatalf("unary provider dispatch missing: output=%#v draft=%#v", output, seenInvoke)
	}
	if stream.StreamID() != "stream-1" || seenStream["descriptor_ref"] == "" {
		t.Fatalf("stream provider dispatch missing: stream=%#v draft=%#v", stream, seenStream)
	}
	if session.SessionID() != "bidi-1" || seenBidi["descriptor_ref"] == "" {
		t.Fatalf("bidi provider dispatch missing: session=%#v draft=%#v", session, seenBidi)
	}
	if len(seenStreams) != 1 || seenStreams[0]["stream_id"] != float64(1) {
		t.Fatalf("bidi streams not forwarded: %#v", seenStreams)
	}
	if summary := result.TerminalReceiptSummary(); summary == nil || summary.State != "Completed" {
		t.Fatalf("ability await terminal receipt = %#v", summary)
	}
	if !cancelled.RequestAccepted() || !cancelled.Cancelled() || cancelled.Terminal() || seenCancelReason != "client stop" {
		t.Fatalf("ability cancel = %#v reason=%q", cancelled, seenCancelReason)
	}
	if recovery.State != "runtime_started" || seenRecovery["recovery_id"] != "recovery-1" {
		t.Fatalf("ability recovery did not delegate to runtime provider: recovery=%#v request=%#v", recovery, seenRecovery)
	}
	if got := mustJSONString(descriptorModes); got != `["rpc","stream","bidi"]` {
		t.Fatalf("descriptor call modes = %s", got)
	}
}

func TestRuntimeAbilityChildContextDispatchesWithParentReceiptCausality(t *testing.T) {
	var seen map[string]any
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		if err := json.Unmarshal(raw, &seen); err != nil {
			return nil, err
		}
		return []byte(`{"ok":true,"tuple":` + string(raw) + `,"invocation_id":"child-1","terminal_state":"Completed","output_content_type":"application/json","output_json":{"child":true},"elapsed_ms":1,"terminal_receipt":{"receipt_ura":"easynet:///r/example/resource/agent.child.client/invocation/child-1/receipt","invocation_id":"child-1","receipt_type":"terminal","state":"completed","index":1,"timestamp_unix_ms":1783100000456,"prev_receipt_hash_hex":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","self_hash_hex":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","causal_binding":` + mustJSONString(seen["causal_context"]) + `,"parent_receipts":[{"receipt_ura":"easynet:///r/example/resource/agent.alice.client/invocation/parent-1/receipt","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"cleanup_complete":true},"error":null}`), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	parent, err := NewInvocationResultFromJSON(runtimeAbilityParentResultJSON())
	if err != nil {
		t.Fatalf("parent result: %v", err)
	}

	child, err := client.ChildContext(
		parent,
		"easynet:///r/example/agent/child.client",
		"AQIDBAUGBwgJCgsMDQ4PEA==",
		map[string]any{"trace_id": "parent-1"},
	)
	if err != nil {
		t.Fatalf("ChildContext: %v", err)
	}
	call := runtimeAbilityTestContext()
	call.CallerURA = "easynet:///r/example/agent/ignored"
	call.NonceBase64 = "AAAAAAAAAAAAAAAAAAAAAA=="
	call.CausalContext = map[string]any{"form": "none"}
	call.Metadata = map[string]any{"attempt": 1}
	draft, err := child.Build(context.Background(), call, "namespace.resolve", map[string]any{"child": true})
	if err != nil {
		t.Fatalf("Build child: %v", err)
	}
	result, err := runtime.Invoke(context.Background(), draft)
	if err != nil {
		t.Fatalf("Invoke child draft: %v", err)
	}

	if string(result.OutputJSON()) != `{"child":true}` {
		t.Fatalf("child output = %s", result.OutputJSON())
	}
	if seen["caller_ura"] != "easynet:///r/example/agent/child.client" || seen["nonce_base64"] != "AQIDBAUGBwgJCgsMDQ4PEA==" {
		t.Fatalf("child caller/nonce not inherited from child context: %#v", seen)
	}
	causal, ok := seen["causal_context"].(map[string]any)
	if !ok {
		t.Fatalf("child causal context missing: %#v", seen["causal_context"])
	}
	if causal["form"] != "scalar" ||
		causal["receipt_ura"] != "easynet:///r/example/resource/agent.alice.client/invocation/parent-1/receipt" ||
		causal["receipt_hash_hex"] != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("child causal context = %#v", causal)
	}
	metadata, ok := seen["metadata"].(map[string]any)
	if !ok || metadata["trace_id"] != "parent-1" || metadata["attempt"].(float64) != 1 {
		t.Fatalf("child metadata = %#v", seen["metadata"])
	}
	receipt := result.TerminalReceiptSummary()
	if receipt == nil || len(receipt.ParentReceipts) != 1 {
		t.Fatalf("child terminal receipt parent links = %#v", receipt)
	}
	if receipt.ParentReceipts[0].ReceiptURA != "easynet:///r/example/resource/agent.alice.client/invocation/parent-1/receipt" ||
		receipt.ParentReceipts[0].ReceiptHashHex != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("child terminal parent receipt = %#v", receipt.ParentReceipts[0])
	}
}

func TestRuntimeAbilityClientFailsClosedOnIncompleteContext(t *testing.T) {
	runtime, _ := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: testResolveDescriptorRef(t),
	})
	client, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	call := runtimeAbilityTestContext()
	call.CausalContext = nil
	if _, err := client.Build(context.Background(), call, "namespace.resolve", map[string]any{}); err == nil {
		t.Fatal("Build accepted missing causal context")
	}
}

func runtimeAbilityTestContext() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:     "easynet:///r/example/agent/alice.client",
		CalleeURA:     "easynet:///r/example/hub",
		SubjectURA:    "easynet:///r/example/user/alice",
		NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext: map[string]any{"form": "none"},
		Metadata:      map[string]any{"request_id": "call-1"},
	}
}

func runtimeAbilityResultJSON(ok bool, outputJSON string, message string, retryable bool) []byte {
	errorJSON := "null"
	if !ok {
		errorJSON = `{"code":"DENIED","stage":"admission","message":` + string(mustJSON(message)) + `,"retryable":` + string(mustJSON(retryable)) + `}`
	}
	return []byte(`{"ok":` + string(mustJSON(ok)) + `,"tuple":` + runtimeAbilityDraftJSON() + `,"invocation_id":"inv-1","terminal_state":"Completed","output_content_type":"application/json","output_json":` + outputJSON + `,"elapsed_ms":1,"terminal_receipt":{"receipt_ura":"easynet:///r/example/resource/agent.alice.client/invocation/inv-1/receipt","invocation_id":"inv-1","receipt_type":"terminal","state":"Completed","index":1,"timestamp_unix_ms":1783100000123,"prev_receipt_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","cleanup_complete":true},"error":` + errorJSON + `}`)
}

func runtimeAbilityParentResultJSON() []byte {
	return []byte(`{"ok":true,"tuple":` + runtimeAbilityDraftJSON() + `,"invocation_id":"parent-1","terminal_state":"Completed","output_content_type":"application/json","output_json":{},"elapsed_ms":1,"terminal_receipt":{"receipt_ura":"easynet:///r/example/resource/agent.alice.client/invocation/parent-1/receipt","invocation_id":"parent-1","receipt_type":"terminal","state":"completed","index":1,"timestamp_unix_ms":1783100000123,"prev_receipt_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","cleanup_complete":true},"error":null}`)
}

func runtimeAbilityDraftJSON() string {
	return `{"caller_ura":"easynet:///r/example/agent/alice.client","callee_ura":"easynet:///r/example/hub","descriptor_ref":"easynet:///r/example/ability/hub.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read","subject_ura":"easynet:///r/example/resource/user.alice/invoke/namespace.resolve","nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==","causal_context":{"form":"none"},"args":{},"content_type":"application/json","metadata":{}}`
}

func mustJSON(value any) []byte {
	raw, err := json.Marshal(value)
	if err != nil {
		panic(err)
	}
	return raw
}

func mustJSONString(value any) string {
	return string(mustJSON(value))
}
