package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

func TestReceiptRuntimeTransportBuildsHistoryInvocationThroughIdentity(t *testing.T) {
	identityTransport := newReceiptRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: receiptHistoryListRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeReceiptClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeReceiptClient: %v", err)
	}

	draft, err := client.BuildListHistoryInvocation(context.Background(), ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"limit": 5, "compact": true},
	})
	if err != nil {
		t.Fatalf("BuildListHistoryInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0" {
		t.Fatalf("descriptor ref = %q", draft.DescriptorRef())
	}
	args := draft.JSONArgs().(map[string]any)
	if args["limit"] != float64(5) || args["compact"] != true {
		t.Fatalf("history args not preserved: %#v", args)
	}
	if _, ok := args["caller_ura"]; ok {
		t.Fatalf("carrier leaked into args: %#v", args)
	}
	metadata := draft.Metadata()
	if metadata["profile"] != receiptProfile ||
		metadata["system_ability"] != receiptHistoryListAbility ||
		metadata["carrier_owner"] != "daemon_sdk" ||
		metadata["timeout_ms"] != float64(2500) {
		t.Fatalf("metadata not normalized: %#v", metadata)
	}
	if len(identityTransport.seenBuildURA) != 1 || identityTransport.seenBuildURA[0]["ability_name"] != receiptHistoryListAbility {
		t.Fatalf("ability descriptor was not delegated through identity client: %#v", identityTransport.seenBuildURA)
	}
}

func TestReceiptRuntimeTransportInvokesHistoryAndTrace(t *testing.T) {
	identity, err := NewIdentityClient(newReceiptRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: receiptHistoryListRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeReceiptClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeReceiptClient: %v", err)
	}

	list, err := client.ListHistory(context.Background(), ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"limit": 1},
	})
	if err != nil {
		t.Fatalf("ListHistory: %v", err)
	}
	records, ok := list["records"].([]any)
	if !ok || len(records) != 1 {
		t.Fatalf("history records not projected: %#v", list)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if args["limit"] != float64(1) {
		t.Fatalf("history args not sent to runtime: %#v", args)
	}

	runtimeTransport.outputJSON = receiptTraceRawJSON
	trace, err := client.GetTrace(context.Background(), ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"key": map[string]any{"trace_id": "trace-1"}},
	})
	if err != nil {
		t.Fatalf("GetTrace: %v", err)
	}
	if trace["trace_id"] != "trace-1" {
		t.Fatalf("trace not projected: %#v", trace)
	}
}

func TestReceiptRuntimeTransportMapsTerminalFailure(t *testing.T) {
	identity, err := NewIdentityClient(newReceiptRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{fail: true})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeReceiptClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeReceiptClient: %v", err)
	}

	_, err = client.ListHistory(context.Background(), ReceiptHistoryReadRequest{ReceiptCarrierBase: receiptHistoryBaseForTest()})
	if err == nil {
		t.Fatal("ListHistory succeeded, want failure")
	}
	if !IsCode(err, ErrAdmissionDenied) {
		t.Fatalf("error code = %v, want %s", err, ErrAdmissionDenied)
	}
}

func TestReceiptRuntimeTransportDelegatesReceiptProjectionsToProvider(t *testing.T) {
	identity, err := NewIdentityClient(newReceiptRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: receiptHistoryListRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	seen := map[string]string{}
	provider := ReceiptTransportFunc{
		ProjectFunc: func(ctx context.Context, receiptJSON []byte) ([]byte, error) {
			seen["project"] = string(receiptJSON)
			return []byte(`{"receipt_ura":null,"invocation_id":"inv-example-1","state":"completed","verified":false,"output":{"ok":true},"error":null,"causal_ref":null,"metadata":{}}`), nil
		},
		VerifyFunc: func(ctx context.Context, receiptJSON []byte) ([]byte, error) {
			seen["verify"] = string(receiptJSON)
			return []byte(`{"verified":false,"receipt_ura":null,"invocation_id":"inv-example-1","method":"daemon_receipt_projection","reason":"projection only","metadata":{}}`), nil
		},
		VerifyChainFunc: func(ctx context.Context, requestJSON []byte) ([]byte, error) {
			seen["chain"] = string(requestJSON)
			return []byte(`{"verified":false,"continuous":true,"method":"daemon_receipt_chain_continuity","reason":"continuity only","requires_full_receipt":true,"root_receipt_ura":"easynet:///r/example/receipt/receipt-1","terminal_receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_count":2,"items":[{"index":0,"receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","prev_receipt_hash_hex":null,"continuous":true,"metadata":{}},{"index":1,"receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","continuous":true,"metadata":{}}],"metadata":{}}`), nil
		},
		CausalRefFunc: func(ctx context.Context, receiptJSON []byte) ([]byte, error) {
			seen["causal"] = string(receiptJSON)
			return []byte(`{"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","invocation_id":"inv-example-1","form":"scalar","metadata":{}}`), nil
		},
	}
	client, err := NewRuntimeReceiptClientWithProjectionProvider(runtime, identity, provider)
	if err != nil {
		t.Fatalf("NewRuntimeReceiptClientWithProjectionProvider: %v", err)
	}

	summary, err := client.Project(context.Background(), []byte(`{"raw":true}`))
	if err != nil {
		t.Fatalf("Project: %v", err)
	}
	verification, err := client.Verify(context.Background(), []byte(`{"raw":"verify"}`))
	if err != nil {
		t.Fatalf("Verify: %v", err)
	}
	chain, err := client.VerifyChain(context.Background(), ReceiptChainVerificationRequest{
		Receipts: []json.RawMessage{
			json.RawMessage(`{"receipt_ura":"easynet:///r/example/receipt/receipt-1","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`),
			json.RawMessage(`{"receipt_ura":"easynet:///r/example/receipt/receipt-2","self_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`),
		},
	})
	if err != nil {
		t.Fatalf("VerifyChain: %v", err)
	}
	causal, err := client.CausalRef(context.Background(), []byte(`{"raw":"causal"}`))
	if err != nil {
		t.Fatalf("CausalRef: %v", err)
	}

	if summary.State != "completed" || verification.Method != "daemon_receipt_projection" ||
		!chain.Continuous || causal.Form != "scalar" {
		t.Fatalf("unexpected projections: summary=%#v verification=%#v chain=%#v causal=%#v", summary, verification, chain, causal)
	}
	if seen["project"] != `{"raw":true}` || seen["verify"] != `{"raw":"verify"}` || seen["causal"] != `{"raw":"causal"}` {
		t.Fatalf("provider did not receive receipt bodies: %#v", seen)
	}
	if seen["chain"] == "" {
		t.Fatalf("provider did not receive chain request")
	}
}

func TestReceiptRuntimeTransportProjectionMethodsFailClosedWithoutProvider(t *testing.T) {
	identity, err := NewIdentityClient(newReceiptRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: receiptHistoryListRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeReceiptClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeReceiptClient: %v", err)
	}

	if _, err := client.Project(context.Background(), []byte(`{"raw":true}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Project error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := client.Verify(context.Background(), []byte(`{"raw":true}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Verify error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := client.VerifyChain(context.Background(), ReceiptChainVerificationRequest{
		Receipts: []json.RawMessage{
			json.RawMessage(`{"receipt_ura":"easynet:///r/example/receipt/receipt-1","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`),
		},
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("VerifyChain error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := client.CausalRef(context.Background(), []byte(`{"raw":true}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("CausalRef error = %v, want %s", err, ErrInvalidArgument)
	}
}

func newReceiptRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			receiptHistoryListAbility: "easynet:///r/example/ability/device.dev-a.invocation.history.list",
			receiptHistoryGetAbility:  "easynet:///r/example/ability/device.dev-a.invocation.history.get",
			receiptTraceGetAbility:    "easynet:///r/example/ability/device.dev-a.invocation.trace.get",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.invocation.history.list": "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.invocation.history.get":  "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
			"easynet:///r/example/ability/device.dev-a.invocation.trace.get":    "easynet:///r/example/ability/device.dev-a.invocation.trace.get@1.0.0",
		},
		descriptorProjection: identityDescriptorProjectionJSON,
	}
}

const receiptHistoryListRawJSON = `{
	"ledger_ura":"easynet:///r/example/resource/invocations",
	"ledger_path":"/tmp/ledger.redb",
	"records":[{"request_id":"req-1","trace_id":"trace-1"}]
}`

const receiptTraceRawJSON = `{
	"ledger_ura":"easynet:///r/example/resource/invocations",
	"ledger_path":"/tmp/ledger.redb",
	"trace_id":"trace-1",
	"nodes":[],
	"edges":[],
	"edge_semantics":"Axon causal links"
}`
