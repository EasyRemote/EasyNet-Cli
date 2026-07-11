package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

func TestRuntimeAbilityClientBuildsCompleteCanonicalDraft(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{})
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
	if draft.DescriptorRef() != "easynet:///r/example/ability/hub.namespace.resolve@1.0.0" {
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
	}}
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

func TestRuntimeAbilityClientFailsClosedOnIncompleteContext(t *testing.T) {
	runtime, _ := NewRuntimeClient(RuntimeTransportFunc{})
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
	return []byte(`{"ok":` + string(mustJSON(ok)) + `,"tuple":` + runtimeAbilityDraftJSON() + `,"invocation_id":"inv-1","terminal_state":"Completed","output_content_type":"application/json","output_json":` + outputJSON + `,"elapsed_ms":1,"error":` + errorJSON + `}`)
}

func runtimeAbilityDraftJSON() string {
	return `{"caller_ura":"easynet:///r/example/agent/alice.client","callee_ura":"easynet:///r/example/hub","descriptor_ref":"easynet:///r/example/ability/hub.namespace.resolve@1.0.0","subject_ura":"easynet:///r/example/resource/user.alice/invoke/namespace.resolve","nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==","causal_context":{"form":"none"},"args":{},"content_type":"application/json","metadata":{}}`
}

func mustJSON(value any) []byte {
	raw, err := json.Marshal(value)
	if err != nil {
		panic(err)
	}
	return raw
}
