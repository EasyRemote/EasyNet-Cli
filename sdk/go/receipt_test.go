package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryReceiptTransport struct {
	fetchJSON        string
	listHistoryJSON  string
	getHistoryJSON   string
	traceJSON        string
	projectJSON      string
	verifyJSON       string
	verifyChainJSON  string
	causalRefJSON    string
	seenRequest      map[string]any
	seenHistoryRead  map[string]any
	seenChainRequest map[string]any
	seenReceiptRaw   string
	closeCalls       int
}

func (m *memoryReceiptTransport) Fetch(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.fetchJSON), nil
}

func (m *memoryReceiptTransport) BuildListHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return m.captureHistoryRead(requestJSON, `{"caller_ura":"easynet:///r/example/agent/alice.sdk","callee_ura":"easynet:///r/example/device/dev-a","descriptor_ref":"easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0","subject_ura":"easynet:///r/example/device/dev-a","nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==","causal_context":{"form":"none"},"args":{},"content_type":"application/json","metadata":{}}`)
}

func (m *memoryReceiptTransport) BuildGetHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return m.BuildListHistoryInvocation(ctx, requestJSON)
}

func (m *memoryReceiptTransport) BuildTraceInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return m.BuildListHistoryInvocation(ctx, requestJSON)
}

func (m *memoryReceiptTransport) ListHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if _, err := m.captureHistoryRead(requestJSON, m.listHistoryJSON); err != nil {
		return nil, err
	}
	return []byte(m.listHistoryJSON), nil
}

func (m *memoryReceiptTransport) GetHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if _, err := m.captureHistoryRead(requestJSON, m.getHistoryJSON); err != nil {
		return nil, err
	}
	return []byte(m.getHistoryJSON), nil
}

func (m *memoryReceiptTransport) GetTrace(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if _, err := m.captureHistoryRead(requestJSON, m.traceJSON); err != nil {
		return nil, err
	}
	return []byte(m.traceJSON), nil
}

func (m *memoryReceiptTransport) captureHistoryRead(requestJSON []byte, responseJSON string) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenHistoryRead); err != nil {
		return nil, err
	}
	return []byte(responseJSON), nil
}

func (m *memoryReceiptTransport) Project(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	m.seenReceiptRaw = string(receiptJSON)
	return []byte(m.projectJSON), nil
}

func (m *memoryReceiptTransport) Verify(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	m.seenReceiptRaw = string(receiptJSON)
	return []byte(m.verifyJSON), nil
}

func (m *memoryReceiptTransport) VerifyChain(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenChainRequest); err != nil {
		return nil, err
	}
	return []byte(m.verifyChainJSON), nil
}

func (m *memoryReceiptTransport) CausalRef(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	m.seenReceiptRaw = string(receiptJSON)
	return []byte(m.causalRefJSON), nil
}

func (m *memoryReceiptTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
}

func baseReceiptFetchRequest() ReceiptFetchRequest {
	return ReceiptFetchRequest{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		DescriptorRef:     "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		RequestID:         "inv-example-1",
		Metadata:          map[string]any{"request_id": "receipt-fetch-1"},
	}
}

func receiptHistoryBaseForTest() ReceiptCarrierBase {
	return ReceiptCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		TimeoutMS:         2500,
		Metadata:          map[string]any{"request_id": "history-1"},
	}
}

func TestReceiptFetchPreservesCarrierAndDecodesSummary(t *testing.T) {
	transport := &memoryReceiptTransport{fetchJSON: `{"receipt_ura":null,"invocation_id":"inv-example-1","state":"completed","verified":false,"output":{"ok":true},"error":null,"causal_ref":null,"metadata":{}}`}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}

	summary, err := client.Fetch(context.Background(), baseReceiptFetchRequest())
	if err != nil {
		t.Fatalf("Fetch: %v", err)
	}

	if summary.State != "completed" || summary.Verified || summary.InvocationID == nil || *summary.InvocationID != "inv-example-1" {
		t.Fatalf("unexpected summary: %#v", summary)
	}
	if transport.seenRequest["request_id"] != "inv-example-1" || transport.seenRequest["caller_ura"] == "" || transport.seenRequest["descriptor_ref"] == "" {
		t.Fatalf("fetch request not forwarded: %#v", transport.seenRequest)
	}
}

func TestReceiptBuildFetchInvocationMatchesSharedCarrier(t *testing.T) {
	root := repositoryRoot(t)
	var req ReceiptFetchRequest
	if err := json.Unmarshal(sharedFixture(t, root, "receipt-fetch-request.v4.json"), &req); err != nil {
		t.Fatalf("decode shared receipt fetch request: %v", err)
	}
	draft, err := BuildReceiptFetchInvocation(req)
	if err != nil {
		t.Fatalf("BuildReceiptFetchInvocation: %v", err)
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		t.Fatalf("marshal receipt fetch invocation: %v", err)
	}
	assertJSONEquivalent(t, raw, sharedFixture(t, root, "receipt-fetch-invocation.v4.json"))
}

func TestReceiptClientBuildFetchInvocationHonorsLifecycle(t *testing.T) {
	transport := &memoryReceiptTransport{}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if _, err := client.BuildFetchInvocation(context.Background(), baseReceiptFetchRequest()); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("BuildFetchInvocation after close = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestReceiptHistoryReadPreservesCarrierAndDecodesReadModel(t *testing.T) {
	transport := &memoryReceiptTransport{
		listHistoryJSON: `{"ledger_ura":"easynet:///r/example/resource/invocations","ledger_path":"/tmp/ledger.redb","records":[{"request_id":"req-1"}]}`,
		getHistoryJSON:  `{"ledger_ura":"easynet:///r/example/resource/invocations","ledger_path":"/tmp/ledger.redb","record":{"request_id":"req-1"}}`,
		traceJSON:       `{"ledger_ura":"easynet:///r/example/resource/invocations","ledger_path":"/tmp/ledger.redb","trace_id":"trace-1","nodes":[],"edges":[],"edge_semantics":"Axon causal links"}`,
	}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}
	req := ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"limit": 1, "compact": true},
	}

	list, err := client.ListHistory(context.Background(), req)
	if err != nil {
		t.Fatalf("ListHistory: %v", err)
	}
	if list["ledger_path"] != "/tmp/ledger.redb" {
		t.Fatalf("unexpected list read-model: %#v", list)
	}
	if transport.seenHistoryRead["caller_ura"] == "" || transport.seenHistoryRead["arguments"] == nil {
		t.Fatalf("history request not forwarded: %#v", transport.seenHistoryRead)
	}

	draft, err := client.BuildListHistoryInvocation(context.Background(), req)
	if err != nil {
		t.Fatalf("BuildListHistoryInvocation: %v", err)
	}
	if draft.DescriptorRef() == "" || !draft.HasJSONArgs() {
		t.Fatalf("unexpected history draft: %#v", draft)
	}

	if _, err := client.GetHistory(context.Background(), ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"key": map[string]any{"request_id": "req-1"}},
	}); err != nil {
		t.Fatalf("GetHistory: %v", err)
	}
	if _, err := client.GetTrace(context.Background(), ReceiptHistoryReadRequest{
		ReceiptCarrierBase: receiptHistoryBaseForTest(),
		Arguments:          map[string]any{"key": map[string]any{"trace_id": "trace-1"}},
	}); err != nil {
		t.Fatalf("GetTrace: %v", err)
	}
}

func TestReceiptFetchRejectsMissingOrAmbiguousLookupKey(t *testing.T) {
	transport := &memoryReceiptTransport{}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}
	req := baseReceiptFetchRequest()
	req.RequestID = ""
	if _, err := client.Fetch(context.Background(), req); err == nil {
		t.Fatalf("Fetch succeeded without lookup key")
	}
	req.RequestID = "inv-example-1"
	req.TraceID = "trace-1"
	if _, err := client.Fetch(context.Background(), req); err == nil {
		t.Fatalf("Fetch succeeded with ambiguous lookup keys")
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called despite invalid lookup keys")
	}
}

func TestReceiptProjectDoesNotUpgradeVerification(t *testing.T) {
	transport := &memoryReceiptTransport{projectJSON: `{"receipt_ura":null,"invocation_id":"inv-example-1","state":"completed","verified":false,"output":{"ok":true},"error":null,"causal_ref":null,"metadata":{}}`}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}

	summary, err := client.Project(context.Background(), []byte(`{"raw":true}`))
	if err != nil {
		t.Fatalf("Project: %v", err)
	}

	if summary.Verified {
		t.Fatalf("summary-only projection was upgraded to verified")
	}
	if transport.seenReceiptRaw != `{"raw":true}` {
		t.Fatalf("receipt input not forwarded: %s", transport.seenReceiptRaw)
	}
}

func TestReceiptVerifyAndCausalRefDecodeDaemonProjections(t *testing.T) {
	receiptURA := "easynet:///r/example/receipt/receipt-1"
	transport := &memoryReceiptTransport{
		verifyJSON:    `{"verified":true,"receipt_ura":"easynet:///r/example/receipt/receipt-1","invocation_id":"inv-example-1","method":"axon-full-receipt","metadata":{"source":"axon"}}`,
		causalRefJSON: `{"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","invocation_id":"inv-example-1","form":"scalar","metadata":{}}`,
	}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}

	verification, err := client.Verify(context.Background(), []byte(`{"receipt_ura":"`+receiptURA+`"}`))
	if err != nil {
		t.Fatalf("Verify: %v", err)
	}
	causal, err := client.CausalRef(context.Background(), []byte(`{"receipt_ura":"`+receiptURA+`"}`))
	if err != nil {
		t.Fatalf("CausalRef: %v", err)
	}

	if !verification.Verified || verification.Method != "axon-full-receipt" {
		t.Fatalf("unexpected verification: %#v", verification)
	}
	if causal.CausalRef == "" || causal.Form != "scalar" {
		t.Fatalf("unexpected causal ref: %#v", causal)
	}
	if causal.ReceiptHashHex != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("causal receipt hash = %q", causal.ReceiptHashHex)
	}
	if causal.CausalContext["receipt_hash_hex"] != causal.ReceiptHashHex {
		t.Fatalf("causal context missing hash: %#v", causal.CausalContext)
	}
	context := causal.ToCausalContext()
	context["receipt_hash_hex"] = "mutated"
	if causal.CausalContext["receipt_hash_hex"] != causal.ReceiptHashHex {
		t.Fatalf("ToCausalContext leaked mutable context: %#v", causal.CausalContext)
	}
}

func TestReceiptVerifyChainPreservesReceiptBodiesAndDecodesContinuity(t *testing.T) {
	transport := &memoryReceiptTransport{
		verifyChainJSON: `{"verified":true,"continuous":true,"method":"axon_receipt_chain_signature","reason":"","requires_full_receipt":true,"root_receipt_ura":"easynet:///r/example/receipt/receipt-1","terminal_receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_count":2,"items":[{"index":0,"receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","prev_receipt_hash_hex":null,"continuous":true,"metadata":{"parent_receipt_count":0}},{"index":1,"receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","continuous":true,"metadata":{"parent_receipt_count":1}}],"metadata":{"chain_projection":"cross_invocation_signature_dag_with_parent_closure","parent_dag_closed":true,"assurance":"cryptographic"}}`,
	}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}

	result, err := client.VerifyChain(context.Background(), ReceiptChainVerificationRequest{
		Receipts: []json.RawMessage{
			json.RawMessage(`{"receipt_ura":"easynet:///r/example/receipt/receipt-1","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`),
			json.RawMessage(`{"receipt_ura":"easynet:///r/example/receipt/receipt-2","self_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`),
		},
		Metadata: map[string]any{"request_id": "chain-1"},
	})
	if err != nil {
		t.Fatalf("VerifyChain: %v", err)
	}

	if !result.Verified || !result.Continuous || result.Method != "axon_receipt_chain_signature" {
		t.Fatalf("unexpected chain verification: %#v", result)
	}
	if result.Metadata["chain_projection"] != "cross_invocation_signature_dag_with_parent_closure" || result.Metadata["parent_dag_closed"] != true {
		t.Fatalf("unexpected chain metadata: %#v", result.Metadata)
	}
	receipts, ok := transport.seenChainRequest["receipts"].([]any)
	if !ok || len(receipts) != 2 {
		t.Fatalf("receipt bodies not forwarded: %#v", transport.seenChainRequest)
	}
	first, ok := receipts[0].(map[string]any)
	if !ok || first["receipt_ura"] != "easynet:///r/example/receipt/receipt-1" {
		t.Fatalf("first receipt not preserved: %#v", receipts[0])
	}
}

func TestReceiptVerifyChainRejectsDuplicateReceiptHash(t *testing.T) {
	transport := &memoryReceiptTransport{}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}

	_, err = client.VerifyChain(context.Background(), ReceiptChainVerificationRequest{
		Receipts: []json.RawMessage{
			json.RawMessage(`{"receipt_ura":"easynet:///r/example/receipt/receipt-1","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`),
			json.RawMessage(`{"receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_hash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}`),
		},
	})
	if err == nil {
		t.Fatalf("VerifyChain accepted duplicate receipt hash")
	}
	if transport.seenChainRequest != nil {
		t.Fatalf("transport called despite invalid chain request")
	}
}

func TestReceiptChainReturnsCopySafeReceipts(t *testing.T) {
	chain, err := NewReceiptChain([]ReceiptRef{{
		ReceiptURA:     "easynet:///r/example/receipt/receipt-1",
		ReceiptHashHex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		Metadata:       map[string]any{"source": "daemon"},
	}})
	if err != nil {
		t.Fatalf("NewReceiptChain: %v", err)
	}

	receipts := chain.Receipts()
	receipts[0].ReceiptURA = "mutated"

	again := chain.Receipts()
	if again[0].ReceiptURA != "easynet:///r/example/receipt/receipt-1" {
		t.Fatalf("Receipts leaked mutable slice: %#v", again)
	}
}

func TestReceiptRefRejectsInvalidAnchors(t *testing.T) {
	if _, err := NewReceiptRefFromJSON(nil); err == nil {
		t.Fatalf("NewReceiptRefFromJSON accepted empty input")
	}
	if _, err := NewReceiptRefFromMap(map[string]any{
		"receipt_ura":      "easynet:///r/example/receipt/receipt-1",
		"receipt_hash_hex": "aa",
	}); err == nil {
		t.Fatalf("NewReceiptRefFromMap accepted short hash")
	}
	if _, err := NewReceiptChain(nil); err == nil {
		t.Fatalf("NewReceiptChain accepted empty input")
	}
}

func TestReceiptCausalRefRejectsEmptyProjection(t *testing.T) {
	transport := &memoryReceiptTransport{causalRefJSON: `{"metadata":{}}`}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}

	if _, err := client.CausalRef(context.Background(), []byte(`{"receipt":true}`)); err == nil {
		t.Fatalf("CausalRef accepted empty projection")
	}
}

func TestReceiptCausalRefRejectsProjectionWithoutReceiptHash(t *testing.T) {
	_, err := NewCausalRefFromJSON([]byte(`{"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1","receipt_ura":"easynet:///r/example/receipt/receipt-1","form":"scalar","metadata":{}}`))
	if err == nil {
		t.Fatalf("CausalRef accepted projection without receipt hash")
	}
}

func TestReceiptCausalRefRejectsContextWithoutReceiptHash(t *testing.T) {
	_, err := NewCausalRefFromJSON([]byte(`{"receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","causal_context":{"form":"scalar","receipt_ura":"easynet:///r/example/receipt/receipt-1"},"metadata":{}}`))
	if err == nil {
		t.Fatalf("CausalRef accepted causal_context without receipt hash")
	}
}

func TestReceiptRefDelegatesCausalContextProjection(t *testing.T) {
	transport := &memoryReceiptTransport{
		causalRefJSON: `{"receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","verified":false,"causal_context":{"form":"scalar","receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1","invocation_id":"inv-example-1","form":"scalar","metadata":{}}`,
	}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}
	invocationID := "inv-example-1"
	ref := ReceiptRef{
		ReceiptURA:     " easynet:///r/example/receipt/receipt-1 ",
		ReceiptHashHex: "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
		InvocationID:   &invocationID,
		Metadata:       map[string]any{"source": "runtime"},
	}

	causalContext, err := ref.CausalContext(context.Background(), client)
	if err != nil {
		t.Fatalf("CausalContext: %v", err)
	}

	var forwarded map[string]any
	if err := json.Unmarshal([]byte(transport.seenReceiptRaw), &forwarded); err != nil {
		t.Fatalf("forwarded receipt ref: %v", err)
	}
	if forwarded["receipt_ura"] != "easynet:///r/example/receipt/receipt-1" || forwarded["receipt_hash_hex"] != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("receipt ref not normalized: %#v", forwarded)
	}
	if causalContext["form"] != "scalar" || causalContext["receipt_hash_hex"] != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("unexpected causal context: %#v", causalContext)
	}
}

func TestReceiptRefFromInvocationResultRequiresAnchor(t *testing.T) {
	draft, err := BuildReceiptFetchInvocation(baseReceiptFetchRequest())
	if err != nil {
		t.Fatalf("BuildReceiptFetchInvocation: %v", err)
	}
	tupleJSON, err := json.Marshal(draft)
	if err != nil {
		t.Fatalf("marshal draft: %v", err)
	}
	result, err := NewInvocationResultFromJSON([]byte(`{"ok":true,"tuple":` + string(tupleJSON) + `,"terminal_state":"Completed","output_json":{},"receipt":{"receipt_ura":"easynet:///r/example/receipt/receipt-1","invocation_id":"inv-example-1","self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"error":null}`))
	if err != nil {
		t.Fatalf("NewInvocationResultFromJSON: %v", err)
	}

	ref, err := NewReceiptRefFromInvocationResult(result)
	if err != nil {
		t.Fatalf("NewReceiptRefFromInvocationResult: %v", err)
	}
	if ref.InvocationID == nil || *ref.InvocationID != "inv-example-1" {
		t.Fatalf("invocation id not preserved: %#v", ref)
	}

	unanchored, err := NewInvocationResultFromJSON([]byte(`{"ok":true,"tuple":` + string(tupleJSON) + `,"terminal_state":"Completed","output_json":{},"receipt":{"invocation_id":"inv-example-1"},"error":null}`))
	if err != nil {
		t.Fatalf("NewInvocationResultFromJSON unanchored: %v", err)
	}
	if _, err := NewReceiptRefFromInvocationResult(unanchored); err == nil {
		t.Fatalf("unanchored invocation result accepted")
	}
	if _, err := NewReceiptRefFromMap(map[string]any{
		"receipt_ura":      "easynet:///r/example/receipt/receipt-1",
		"receipt_hash_hex": "aa",
	}); err == nil {
		t.Fatalf("short receipt hash accepted")
	}
}

func TestReceiptChainDelegatesContinuityProjection(t *testing.T) {
	transport := &memoryReceiptTransport{
		verifyChainJSON: `{"verified":true,"continuous":true,"method":"axon_receipt_chain_signature","reason":"","requires_full_receipt":true,"root_receipt_ura":"easynet:///r/example/receipt/receipt-1","terminal_receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_count":2,"items":[{"index":0,"receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","prev_receipt_hash_hex":null,"continuous":true,"metadata":{"parent_receipt_count":0}},{"index":1,"receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","continuous":true,"metadata":{"parent_receipt_count":1}}],"metadata":{"chain_projection":"cross_invocation_signature_dag_with_parent_closure","parent_dag_closed":true,"assurance":"cryptographic"}}`,
	}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}
	firstIndex := 0
	secondIndex := 1
	prevHash := "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	chain, err := NewReceiptChain([]ReceiptRef{
		{
			ReceiptURA:     "easynet:///r/example/receipt/receipt-1",
			ReceiptHashHex: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			Index:          &firstIndex,
		},
		{
			ReceiptURA:         "easynet:///r/example/receipt/receipt-2",
			ReceiptHashHex:     "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
			PrevReceiptHashHex: &prevHash,
			Index:              &secondIndex,
		},
	})
	if err != nil {
		t.Fatalf("NewReceiptChain: %v", err)
	}

	result, err := chain.VerifyContinuity(context.Background(), client, map[string]any{"request_id": "chain-ref-1"})
	if err != nil {
		t.Fatalf("VerifyContinuity: %v", err)
	}

	if !result.Continuous || transport.seenChainRequest["metadata"].(map[string]any)["request_id"] != "chain-ref-1" {
		t.Fatalf("unexpected continuity result: %#v request=%#v", result, transport.seenChainRequest)
	}
	receipts := transport.seenChainRequest["receipts"].([]any)
	first := receipts[0].(map[string]any)
	second := receipts[1].(map[string]any)
	if first["receipt_hash_hex"] != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" || second["prev_receipt_hash_hex"] != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
		t.Fatalf("receipt refs not normalized in chain request: %#v", receipts)
	}
}

func TestReceiptSummaryDecodesTypedErrorAndNullOutput(t *testing.T) {
	summary, err := NewReceiptSummaryFromJSON([]byte(`{"state":"failed","verified":false,"output":null,"error":{"code":"INVALID_ARGUMENT","stage":"runtime","message":"bad receipt","retry":"never","details":{"field":"receipt_ura"}},"metadata":{}}`))
	if err != nil {
		t.Fatalf("NewReceiptSummaryFromJSON: %v", err)
	}

	if summary.Output != nil {
		t.Fatalf("expected null output, got %#v", summary.Output)
	}
	if summary.Error == nil || summary.Error.Code != ErrInvalidArgument || summary.Error.Stage != "runtime" {
		t.Fatalf("typed error not decoded: %#v", summary.Error)
	}
	if summary.Error.Details["field"] != "receipt_ura" {
		t.Fatalf("error details not preserved: %#v", summary.Error.Details)
	}
}

func TestReceiptClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := &memoryReceiptTransport{fetchJSON: `{"receipt_ura":null,"invocation_id":"inv-example-1","state":"completed","verified":false,"output":{},"error":null,"metadata":{}}`}
	client, err := NewReceiptClient(transport)
	if err != nil {
		t.Fatalf("NewReceiptClient: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.Fetch(context.Background(), baseReceiptFetchRequest())
	if err == nil {
		t.Fatalf("Fetch after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called after close: %#v", transport.seenRequest)
	}
}

func TestReceiptSummaryRequiresOutputField(t *testing.T) {
	if _, err := NewReceiptSummaryFromJSON([]byte(`{"state":"completed","verified":false,"metadata":{}}`)); err == nil {
		t.Fatalf("summary without output accepted")
	}
}
