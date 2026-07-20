package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestRuntimeClientResolveDescriptorRefRequiresCallMode(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: func(context.Context, []byte) ([]byte, error) {
			t.Fatal("descriptor resolver transport must not be called for missing call_mode")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	_, err = runtime.ResolveDescriptorRef(context.Background(), RuntimeDescriptorRefRequest{
		CalleeURA: "easynet:///r/example/device/dev-a",
		Ability:   "observe.health",
	})
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Code != ErrInvalidArgument {
		t.Fatalf("ResolveDescriptorRef error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeAbilityClientBuildRequiresExplicitCallMode(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: func(context.Context, []byte) ([]byte, error) {
			t.Fatal("descriptor resolver transport must not be called for missing call_mode")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}

	_, err = client.buildWithCallMode(
		context.Background(),
		runtimeAbilityTestContext(),
		"namespace.resolve",
		map[string]any{"name": "alice"},
		"",
	)
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Code != ErrInvalidArgument || !strings.Contains(err.Error(), "call_mode is required") {
		t.Fatalf("buildWithCallMode error = %v, want explicit call_mode invalid argument", err)
	}
}

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
	if draft.DescriptorRef() != "easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read" {
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
		ResolveDescriptorRefFunc: func(_ context.Context, requestJSON []byte) ([]byte, error) {
			var request map[string]any
			if err := json.Unmarshal(requestJSON, &request); err != nil {
				return nil, err
			}
			descriptorModes = append(descriptorModes, request["call_mode"].(string))
			return []byte(`{"descriptor_ref":"easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"}`), nil
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
	if got := mustJSONString(descriptorModes); got != `["rpc","stream","bidi"]` {
		t.Fatalf("descriptor call modes = %s", got)
	}
}

func TestRuntimeAbilityClientRestartRecoverDelegatesToProvider(t *testing.T) {
	var seenRecovery map[string]any
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		RecoverFunc: func(_ context.Context, raw []byte) ([]byte, error) {
			if err := json.Unmarshal(raw, &seenRecovery); err != nil {
				return nil, err
			}
			return runtimeRecoveryReportJSON("recovery-1"), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}

	recovery, err := client.Recover(context.Background(), runtimeRecoveryRequestForTest())
	if err != nil {
		t.Fatalf("Recover: %v", err)
	}

	if recovery.State != "runtime_started" || seenRecovery["recovery_id"] != "recovery-1" {
		t.Fatalf("ability recovery did not delegate to runtime provider: recovery=%#v request=%#v", recovery, seenRecovery)
	}
}

func TestRuntimeAbilityChildContextDispatchesWithParentReceiptCausality(t *testing.T) {
	var seen map[string]any
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		if err := json.Unmarshal(raw, &seen); err != nil {
			return nil, err
		}
		causalContext := seen["causal_context"].(map[string]any)
		causalBinding := map[string]any{
			"form": "scalar",
			"receipt": map[string]any{
				"receipt_ura":      causalContext["receipt_ura"],
				"receipt_hash_hex": causalContext["receipt_hash_hex"],
			},
		}
		parents := []any{map[string]any{
			"receipt_ura":      causalContext["receipt_ura"],
			"receipt_hash_hex": causalContext["receipt_hash_hex"],
		}}
		admission, terminal := canonicalRuntimeReceiptPairFixture("child-1", "Completed")
		for _, receipt := range []map[string]any{admission, terminal} {
			receipt["caller_binding"] = map[string]any{"ura": "easynet:///r/example/agent/child.client", "profile": "axon-strict-v2"}
			receipt["causal_binding_kind"] = "scalar"
			receipt["causal_binding"] = causalBinding
			receipt["parent_receipts"] = parents
		}
		terminal["receipt_ura"] = "easynet:///r/example/resource/agent.child.client/invocation/child-1/receipt"
		terminal["self_hash_hex"] = strings.Repeat("dd", 32)
		return mustJSON(map[string]any{
			"ok":                  true,
			"tuple":               seen,
			"invocation_id":       "child-1",
			"terminal_state":      "Completed",
			"output_content_type": "application/json",
			"output_json":         map[string]any{"child": true},
			"elapsed_ms":          1,
			"admission_receipt":   admission,
			"terminal_receipt":    terminal,
			"error":               nil,
		}), nil
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

func TestRuntimeAbilityClientMaterializesTypedAuthorityIntoCanonicalDraft(t *testing.T) {
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
	authority := runtimeAbilitySessionAuthority(t, call, "alice")
	call.Authority = &authority

	draft, err := client.Build(context.Background(), call, "namespace.resolve", map[string]any{})
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	metadata := draft.Metadata()
	if metadata[SessionAuthorityMetadataKey] == "" {
		t.Fatalf("typed authority was not materialized: %#v", metadata)
	}
	if _, present := call.Metadata[SessionAuthorityMetadataKey]; present {
		t.Fatalf("typed authority mutated caller metadata: %#v", call.Metadata)
	}
}

func TestRuntimeAbilityClientRejectsAuthoritySubjectMismatchBeforeResolution(t *testing.T) {
	resolverCalls := 0
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: func(_ context.Context, _ []byte) ([]byte, error) {
			resolverCalls++
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	call := runtimeAbilityTestContext()
	call.SubjectURA = "easynet:///r/example/device/device-a"
	authority := runtimeAbilitySessionAuthority(t, call, "bob")
	call.Authority = &authority

	_, err = client.Build(context.Background(), call, "namespace.resolve", map[string]any{})
	if err == nil || !strings.Contains(err.Error(), "does not admit descriptor-bound subject_ura") {
		t.Fatalf("subject mismatch error = %v", err)
	}
	if resolverCalls != 0 {
		t.Fatalf("descriptor resolver called %d times after authority mismatch", resolverCalls)
	}
}

func TestRuntimeAbilityClientValidatesRawAuthorityMetadata(t *testing.T) {
	resolverCalls := 0
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: func(_ context.Context, _ []byte) ([]byte, error) {
			resolverCalls++
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	call := runtimeAbilityTestContext()
	call.SubjectURA = "easynet:///r/example/device/device-a"
	authority := runtimeAbilitySessionAuthority(t, call, "bob")
	projection, err := authority.Metadata()
	if err != nil {
		t.Fatalf("authority metadata: %v", err)
	}
	call.Metadata = projection.Metadata()

	_, err = client.Build(context.Background(), call, "namespace.resolve", map[string]any{})
	if err == nil || !strings.Contains(err.Error(), "does not admit descriptor-bound subject_ura") {
		t.Fatalf("raw authority mismatch error = %v", err)
	}
	if resolverCalls != 0 {
		t.Fatalf("descriptor resolver called %d times after raw authority mismatch", resolverCalls)
	}
}

func TestRuntimeAbilityClientRejectsDuplicateAuthorityRepresentations(t *testing.T) {
	runtime, _ := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: testResolveDescriptorRef(t),
	})
	client, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	call := runtimeAbilityTestContext()
	authority := runtimeAbilitySessionAuthority(t, call, "alice")
	projection, err := authority.Metadata()
	if err != nil {
		t.Fatalf("authority metadata: %v", err)
	}
	call.Authority = &authority
	call.Metadata = projection.Metadata()

	_, err = client.Build(context.Background(), call, "namespace.resolve", map[string]any{})
	if err == nil || !strings.Contains(err.Error(), "must be supplied once") {
		t.Fatalf("duplicate authority error = %v", err)
	}
}

func runtimeAbilitySessionAuthority(t *testing.T, call RuntimeCallContext, ownerUserID string) SessionAuthority {
	t.Helper()
	payload := sessionAuthorityPayloadFixture()
	payload["issuer_ura"] = call.CallerURA
	payload["session_owner_user_id"] = ownerUserID
	payload["creator_principal_id"] = call.CallerURA
	payload["callee_ura"] = call.CalleeURA
	payload["subject_ura"] = "easynet:///r/example/resource/user." + ownerUserID + "/session/session-1"
	payload["audience"] = call.CalleeURA
	payload["scopes"] = []string{"namespace.resolve"}
	payload["allowed_actions"] = []string{"read"}
	payload["allowed_followup_abilities"] = []string{"namespace.resolve"}
	value := authorityMetadataFixture(t, payload, []byte("session-signature"))
	authority, err := NewSessionAuthorityFromMetadata(value)
	if err != nil {
		t.Fatalf("NewSessionAuthorityFromMetadata: %v", err)
	}
	return authority
}

func runtimeAbilityTestContext() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:     "easynet:///r/example/agent/alice.client",
		CalleeURA:     "easynet:///r/example/authority",
		SubjectURA:    "easynet:///r/example/user/alice",
		NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext: map[string]any{"form": "none"},
		Metadata:      map[string]any{"request_id": "call-1"},
	}
}

func runtimeAbilityResultJSON(ok bool, outputJSON string, message string, retryable bool) []byte {
	var output any
	if err := json.Unmarshal([]byte(outputJSON), &output); err != nil {
		panic(err)
	}
	terminalState := "Completed"
	var failure any
	if !ok {
		terminalState = "Failed"
		failure = map[string]any{
			"code":      "DENIED",
			"stage":     "execution",
			"message":   message,
			"retryable": retryable,
		}
	}
	admission, terminal := canonicalRuntimeReceiptPairFixture("inv-1", terminalState)
	var draft any
	if err := json.Unmarshal([]byte(runtimeAbilityDraftJSON()), &draft); err != nil {
		panic(err)
	}
	return mustJSON(map[string]any{
		"ok":                  ok,
		"tuple":               draft,
		"invocation_id":       "inv-1",
		"terminal_state":      terminalState,
		"output_content_type": "application/json",
		"output_json":         output,
		"elapsed_ms":          1,
		"admission_receipt":   admission,
		"terminal_receipt":    terminal,
		"error":               failure,
	})
}

func runtimeAbilityParentResultJSON() []byte {
	admission, terminal := canonicalRuntimeReceiptPairFixture("parent-1", "Completed")
	terminal["receipt_ura"] = "easynet:///r/example/resource/agent.alice.client/invocation/parent-1/receipt"
	terminal["self_hash_hex"] = strings.Repeat("aa", 32)
	var draft any
	if err := json.Unmarshal([]byte(runtimeAbilityDraftJSON()), &draft); err != nil {
		panic(err)
	}
	return mustJSON(map[string]any{
		"ok":                  true,
		"tuple":               draft,
		"invocation_id":       "parent-1",
		"terminal_state":      "Completed",
		"output_content_type": "application/json",
		"output_json":         map[string]any{},
		"elapsed_ms":          1,
		"admission_receipt":   admission,
		"terminal_receipt":    terminal,
		"error":               nil,
	})
}

func runtimeAbilityDraftJSON() string {
	return `{"caller_ura":"easynet:///r/example/agent/alice.client","callee_ura":"easynet:///r/example/authority","descriptor_ref":"easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read","subject_ura":"easynet:///r/example/resource/user.alice/invoke/namespace.resolve","nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==","causal_context":{"form":"none"},"args":{},"content_type":"application/json","metadata":{}}`
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
