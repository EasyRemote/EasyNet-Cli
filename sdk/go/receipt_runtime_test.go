package easynet

import (
	"context"
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
	if !IsCode(err, ErrAbilityFailed) {
		t.Fatalf("error code = %v, want %s", err, ErrAbilityFailed)
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
