package easynet

import "testing"

func TestDecodeDaemonErrorJSONDecodesFixtureShape(t *testing.T) {
	err, decodeErr := DecodeDaemonErrorJSON([]byte(`{
		"code": "InvalidArgument",
		"stage": "prepare",
		"message": "missing caller_ura",
		"retry": "never",
		"source": "sdk",
		"invocation_id": null,
		"receipt_ura": null,
		"details": {}
	}`))
	if decodeErr != nil {
		t.Fatalf("DecodeDaemonErrorJSON: %v", decodeErr)
	}
	if err == nil {
		t.Fatalf("DecodeDaemonErrorJSON returned nil")
	}
	if err.Code != ErrInvalidArgument {
		t.Fatalf("code = %s, want %s", err.Code, ErrInvalidArgument)
	}
	if err.Retryable {
		t.Fatalf("Retryable = true, want false")
	}
	if err.Source != "sdk" {
		t.Fatalf("source = %q, want sdk", err.Source)
	}
}

func TestDecodeDaemonErrorJSONPreservesRuntimeRefsAndRetryability(t *testing.T) {
	err, decodeErr := DecodeDaemonErrorJSON([]byte(`{
		"code": "TIMEOUT",
		"stage": "transport",
		"message": "deadline elapsed",
		"retry": "safe",
		"source": "c_abi",
		"invocation_id": "inv-1",
		"receipt_ura": "easynet:///r/example/receipt/opaque",
		"details": {"abi_symbol": "ERR_TIMEOUT"}
	}`))
	if decodeErr != nil {
		t.Fatalf("DecodeDaemonErrorJSON: %v", decodeErr)
	}
	if err.Code != ErrTimeout {
		t.Fatalf("code = %s, want %s", err.Code, ErrTimeout)
	}
	if !err.Retryable {
		t.Fatalf("Retryable = false, want true")
	}
	if err.InvocationID != "inv-1" {
		t.Fatalf("invocation id = %q, want inv-1", err.InvocationID)
	}
	if err.ReceiptURA != "easynet:///r/example/receipt/opaque" {
		t.Fatalf("receipt URA = %q", err.ReceiptURA)
	}
	if err.Details["abi_symbol"] != "ERR_TIMEOUT" {
		t.Fatalf("details not preserved: %#v", err.Details)
	}
}

func TestDecodeDaemonErrorJSONRejectsInvalidRetryHint(t *testing.T) {
	_, err := DecodeDaemonErrorJSON([]byte(`{
		"code": "TIMEOUT",
		"stage": "transport",
		"message": "deadline elapsed",
		"retry": "maybe",
		"details": {}
	}`))
	if err == nil {
		t.Fatalf("DecodeDaemonErrorJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestDecodeDaemonErrorJSONNullIsNoError(t *testing.T) {
	err, decodeErr := DecodeDaemonErrorJSON([]byte(`null`))
	if decodeErr != nil {
		t.Fatalf("DecodeDaemonErrorJSON: %v", decodeErr)
	}
	if err != nil {
		t.Fatalf("err = %#v, want nil", err)
	}
}
