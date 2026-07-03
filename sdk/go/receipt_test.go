package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryReceiptTransport struct {
	fetchJSON      string
	projectJSON    string
	verifyJSON     string
	causalRefJSON  string
	seenRequest    map[string]any
	seenReceiptRaw string
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

func (m *memoryReceiptTransport) CausalRef(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	m.seenReceiptRaw = string(receiptJSON)
	return []byte(m.causalRefJSON), nil
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

func TestReceiptSummaryRequiresOutputField(t *testing.T) {
	if _, err := NewReceiptSummaryFromJSON([]byte(`{"state":"completed","verified":false,"metadata":{}}`)); err == nil {
		t.Fatalf("summary without output accepted")
	}
}
