package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryReceiptTransport struct {
	fetchJSON        string
	projectJSON      string
	verifyJSON       string
	verifyChainJSON  string
	causalRefJSON    string
	seenRequest      map[string]any
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
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		RequestID:         "inv-example-1",
		Metadata:          map[string]any{"request_id": "receipt-fetch-1"},
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
	if transport.seenRequest["request_id"] != "inv-example-1" || transport.seenRequest["caller_ura"] == "" {
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
		causalRefJSON: `{"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1","receipt_ura":"easynet:///r/example/receipt/receipt-1","invocation_id":"inv-example-1","form":"scalar","metadata":{}}`,
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
}

func TestReceiptVerifyChainPreservesReceiptBodiesAndDecodesContinuity(t *testing.T) {
	transport := &memoryReceiptTransport{
		verifyChainJSON: `{"verified":false,"continuous":true,"method":"daemon_receipt_chain_continuity","reason":"continuity only","requires_full_receipt":true,"root_receipt_ura":"easynet:///r/example/receipt/receipt-1","terminal_receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_count":2,"items":[{"index":0,"receipt_ura":"easynet:///r/example/receipt/receipt-1","receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","prev_receipt_hash_hex":null,"continuous":true,"metadata":{}},{"index":1,"receipt_ura":"easynet:///r/example/receipt/receipt-2","receipt_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","continuous":true,"metadata":{}}],"metadata":{"chain_projection":"hash_continuity"}}`,
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

	if result.Verified || !result.Continuous || result.Method != "daemon_receipt_chain_continuity" {
		t.Fatalf("unexpected chain verification: %#v", result)
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

func TestReceiptSummaryDecodesTypedErrorAndNullOutput(t *testing.T) {
	summary, err := NewReceiptSummaryFromJSON([]byte(`{"state":"failed","verified":false,"output":null,"error":{"code":"InvalidArgument","stage":"runtime","message":"bad receipt","retry":"never","details":{"field":"receipt_ura"}},"metadata":{}}`))
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
