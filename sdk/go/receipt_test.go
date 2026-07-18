package easynet

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestReceiptRoutesGeneratedFromManifest(t *testing.T) {
	_, source, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	manifestPath := filepath.Join(
		filepath.Dir(source),
		"..",
		"..",
		"provider_routes",
		"easynet-receipt-routes.v1.json",
	)
	manifest, err := os.ReadFile(manifestPath)
	if err != nil {
		t.Fatalf("read receipt route manifest: %v", err)
	}
	digest := sha256.Sum256(manifest)
	if got, want := receiptRouteManifestSHA256, fmt.Sprintf("%x", digest[:]); got != want {
		t.Fatalf("receipt route manifest digest = %s, want %s", got, want)
	}
}

func TestRuntimeReceiptProviderUsesCanonicalHistoryAndTraceAbilities(t *testing.T) {
	var invocations []map[string]any
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		var draft map[string]any
		if err := json.Unmarshal(raw, &draft); err != nil {
			return nil, err
		}
		invocations = append(invocations, draft)
		descriptor, _ := draft["descriptor_ref"].(string)
		var output map[string]any
		switch {
		case strings.Contains(descriptor, "invocation.history.list"):
			output = map[string]any{
				"ledger_ura":  "easynet:///r/example/resource/device.dev-a/billing/invocations",
				"records":     []any{receiptLedgerRecordFixture()},
				"next_cursor": "receipt-history:v1:cursor-1",
			}
		case strings.Contains(descriptor, "invocation.history.get"):
			output = map[string]any{
				"ledger_ura": "easynet:///r/example/resource/device.dev-a/billing/invocations",
				"record":     receiptLedgerRecordFixture(),
			}
		case strings.Contains(descriptor, "invocation.trace.get"):
			output = map[string]any{
				"ledger_ura": "easynet:///r/example/resource/device.dev-a/billing/invocations",
				"trace_id":   "trace-1",
				"nodes":      []any{receiptLedgerRecordFixture()},
				"edges":      []any{},
			}
		default:
			t.Fatalf("unexpected descriptor_ref: %s", descriptor)
		}
		encoded, err := json.Marshal(output)
		if err != nil {
			return nil, err
		}
		return runtimeAbilityResultJSON(true, string(encoded), "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeReceiptProvider(ability)
	client, _ := NewReceiptClient(provider)

	page, err := client.List(context.Background(), ReceiptListRequest{
		Call:   runtimeAbilityTestContext(),
		Limit:  5,
		Filter: ReceiptFilter{SubjectURAs: []string{"easynet:///r/example/device/dev-a"}},
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(page.Records) != 1 || page.Records[0].RequestID != "req-1" || page.Source.LedgerURA == "" {
		t.Fatalf("unexpected page: %#v", page)
	}
	if page.NextCursor != "receipt-history:v1:cursor-1" {
		t.Fatalf("next cursor not projected: %#v", page)
	}

	lookup := ReceiptLookup{RequestID: "req-1"}
	got, err := client.Get(context.Background(), ReceiptGetRequest{Call: runtimeAbilityTestContext(), Lookup: lookup})
	if err != nil || got.Record == nil || got.Record.InvocationURA == "" {
		t.Fatalf("Get: result=%#v error=%v", got, err)
	}
	trace, err := client.Trace(context.Background(), ReceiptTraceRequest{
		Call:   runtimeAbilityTestContext(),
		Lookup: ReceiptLookup{TraceID: "trace-1"},
	})
	if err != nil || trace.Graph.TraceID != "trace-1" || len(trace.Graph.Records) != 1 {
		t.Fatalf("Trace: result=%#v error=%v", trace, err)
	}
	if len(invocations) != 3 {
		t.Fatalf("invocation count = %d", len(invocations))
	}
	listArgs := invocations[0]["args"].(map[string]any)
	if listArgs["limit"] != float64(5) {
		t.Fatalf("list limit not preserved: %#v", listArgs)
	}
	filter := listArgs["filter"].(map[string]any)
	if len(filter["subject_uras"].([]any)) != 1 {
		t.Fatalf("subject filter not preserved: %#v", filter)
	}
}

func TestReceiptReferenceDelegatesScalarCausalProjectionToAxon(t *testing.T) {
	reference, err := NewReceiptReference(
		"easynet:///r/example/resource/device.dev-a/invocation/req-1/receipt/1",
		bytes.Repeat([]byte{0xab}, 32),
	)
	if err != nil {
		t.Fatalf("NewReceiptReference: %v", err)
	}
	causal, err := reference.CausalContext()
	if err != nil {
		t.Fatalf("CausalContext: %v", err)
	}
	if causal["form"] != "scalar" || causal["receipt_hash_hex"] != strings.Repeat("ab", 32) || causal["receipt_ura"] != reference.ReceiptURA {
		t.Fatalf("unexpected causal projection: %#v", causal)
	}
	if _, err := NewReceiptReference(reference.ReceiptURA, []byte{0xab, 0xcd}); err == nil {
		t.Fatal("short receipt hash was accepted")
	}
}

func TestParseReceiptRecordsAcceptsLedgerRecordWrapper(t *testing.T) {
	records, err := parseReceiptRecords([]any{
		map[string]any{
			"record": receiptLedgerRecordFixture(),
			"source": "invocation.history.list",
		},
	})
	if err != nil {
		t.Fatalf("parseReceiptRecords: %v", err)
	}
	if len(records) != 1 || records[0].RequestID != "req-1" {
		t.Fatalf("records = %#v", records)
	}
}

func TestReceiptReferenceFromRuntimeReceiptUsesSummaryAnchor(t *testing.T) {
	reference, err := ReceiptReferenceFromRuntimeReceipt(RuntimeReceipt{
		ReceiptURA:  "easynet:///r/example/resource/device.dev-a/invocation/req-1/receipt/1",
		SelfHashHex: strings.Repeat("cd", 32),
	})
	if err != nil {
		t.Fatalf("ReceiptReferenceFromRuntimeReceipt: %v", err)
	}
	if reference.ReceiptURA != "easynet:///r/example/resource/device.dev-a/invocation/req-1/receipt/1" ||
		hex.EncodeToString(reference.ReceiptHash[:]) != strings.Repeat("cd", 32) {
		t.Fatalf("unexpected reference: %#v", reference)
	}
	if _, err := ReceiptReferenceFromRuntimeReceipt(RuntimeReceipt{SelfHashHex: strings.Repeat("cd", 32)}); err == nil {
		t.Fatal("runtime receipt summary without receipt_ura was accepted")
	}
	if _, err := ReceiptReferenceFromRuntimeReceipt(RuntimeReceipt{
		ReceiptURA:  reference.ReceiptURA,
		SelfHashHex: "not-hex",
	}); err == nil {
		t.Fatal("runtime receipt summary with malformed hash was accepted")
	}
}

func TestRuntimeReceiptProviderForwardsAndValidatesCursor(t *testing.T) {
	var arguments map[string]any
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		var draft map[string]any
		if err := json.Unmarshal(raw, &draft); err != nil {
			return nil, err
		}
		arguments, _ = draft["args"].(map[string]any)
		output, _ := json.Marshal(map[string]any{
			"ledger_ura":  "easynet:///r/example/resource/device.dev-a/billing/invocations",
			"records":     []any{},
			"next_cursor": "receipt-history:v1:cursor-2",
		})
		return runtimeAbilityResultJSON(true, string(output), "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeReceiptProvider(ability)
	page, err := provider.List(context.Background(), ReceiptListRequest{
		Call:   runtimeAbilityTestContext(),
		Cursor: " receipt-history:v1:cursor-1 ",
		Limit:  2,
	})
	if err != nil {
		t.Fatalf("List with cursor: %v", err)
	}
	if arguments["cursor"] != "receipt-history:v1:cursor-1" {
		t.Fatalf("cursor not forwarded: %#v", arguments)
	}
	if page.NextCursor != "receipt-history:v1:cursor-2" {
		t.Fatalf("next cursor not preserved: %#v", page)
	}

	repeated := runtimeReceiptProviderWithOutput(t, map[string]any{
		"ledger_ura":  "easynet:///r/example/resource/device.dev-a/billing/invocations",
		"records":     []any{},
		"next_cursor": "receipt-history:v1:cursor-1",
	})
	_, err = repeated.List(context.Background(), ReceiptListRequest{
		Call:   runtimeAbilityTestContext(),
		Cursor: "receipt-history:v1:cursor-1",
	})
	if err == nil || !strings.Contains(err.Error(), "repeated cursor") {
		t.Fatalf("repeated cursor error = %v", err)
	}

	if _, err := receiptHistoryCursor(strings.Repeat("x", maxReceiptHistoryCursorLen+1)); err == nil {
		t.Fatal("oversized history cursor was accepted")
	}
	if _, err := receiptHistoryLimit(MaxReceiptHistoryLimit + 1); err == nil {
		t.Fatal("oversized history page was accepted")
	}
	if err := (ReceiptLookup{RequestID: "a", TraceID: "b"}).Validate(); err == nil {
		t.Fatal("ambiguous lookup was accepted")
	}
}

func TestRuntimeReceiptProviderProjectsMultipleAbilityURAsAsOneSet(t *testing.T) {
	var arguments map[string]any
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, raw []byte) ([]byte, error) {
		var draft map[string]any
		if err := json.Unmarshal(raw, &draft); err != nil {
			return nil, err
		}
		arguments, _ = draft["args"].(map[string]any)
		output, _ := json.Marshal(map[string]any{
			"ledger_ura": "easynet:///r/example/resource/device.dev-a/billing/invocations",
			"records":    []any{},
		})
		return runtimeAbilityResultJSON(true, string(output), "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, _ := NewRuntimeClient(transport)
	ability, _ := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	provider, _ := NewRuntimeReceiptProvider(ability)
	_, err := provider.List(context.Background(), ReceiptListRequest{
		Call: runtimeAbilityTestContext(),
		Filter: ReceiptFilter{AbilityURAs: []string{
			"easynet:///r/example/ability/authority.observe.health",
			"easynet:///r/example/ability/authority.observe.metrics",
		}},
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	filter := arguments["filter"].(map[string]any)
	if abilities, ok := filter["ability_uras"].([]any); !ok || len(abilities) != 2 {
		t.Fatalf("ability_uras = %#v", filter["ability_uras"])
	}
}

func TestRuntimeReceiptProviderRejectsMalformedBoundedResults(t *testing.T) {
	provider := runtimeReceiptProviderWithOutput(t, map[string]any{
		"ledger_ura": "easynet:///r/example/resource/device.dev-a/billing/invocations",
		"records":    []any{receiptLedgerRecordFixture(), receiptLedgerRecordFixture()},
	})
	if _, err := provider.List(context.Background(), ReceiptListRequest{Call: runtimeAbilityTestContext(), Limit: 1}); err == nil || !strings.Contains(err.Error(), "exceeds the bounded page") {
		t.Fatalf("bounded list error = %v", err)
	}

	provider = runtimeReceiptProviderWithOutput(t, map[string]any{
		"ledger_ura": "easynet:///r/example/resource/device.dev-a/billing/invocations",
	})
	if _, err := provider.Get(context.Background(), ReceiptGetRequest{Call: runtimeAbilityTestContext(), Lookup: ReceiptLookup{RequestID: "req-1"}}); err == nil || !strings.Contains(err.Error(), "must include record") {
		t.Fatalf("missing record error = %v", err)
	}

	provider = runtimeReceiptProviderWithOutput(t, map[string]any{
		"ledger_ura": "easynet:///r/example/resource/device.dev-a/billing/invocations",
		"trace_id":   "trace-1",
		"nodes":      "not-an-array",
		"edges":      []any{},
	})
	if _, err := provider.Trace(context.Background(), ReceiptTraceRequest{Call: runtimeAbilityTestContext(), Lookup: ReceiptLookup{TraceID: "trace-1"}}); err == nil || !strings.Contains(err.Error(), "nodes must be an array") {
		t.Fatalf("malformed trace error = %v", err)
	}
}

func TestRuntimeReceiptProviderRejectsNonCanonicalURAFilters(t *testing.T) {
	provider := runtimeReceiptProviderWithOutput(t, map[string]any{})
	_, err := provider.List(context.Background(), ReceiptListRequest{
		Call:   runtimeAbilityTestContext(),
		Filter: ReceiptFilter{CallerURA: "https://example.invalid/user/alice"},
	})
	if err == nil || !strings.Contains(err.Error(), "caller_ura must be a canonical URA") {
		t.Fatalf("non-canonical filter error = %v", err)
	}
	_, err = provider.List(context.Background(), ReceiptListRequest{
		Call:               runtimeAbilityTestContext(),
		ExcludeAbilityURAs: []string{"easynet:///r/example/ability/authority.observe.health", "easynet:///r/example/ability/authority.observe.health"},
	})
	if err == nil || !strings.Contains(err.Error(), "must not contain duplicates") {
		t.Fatalf("duplicate ability filter error = %v", err)
	}
}

func TestRuntimeReceiptProviderRequiresCanonicalLedgerSource(t *testing.T) {
	provider := runtimeReceiptProviderWithOutput(t, map[string]any{
		"ledger_ura": "https://example.invalid/invocations",
		"records":    []any{},
	})
	_, err := provider.List(context.Background(), ReceiptListRequest{Call: runtimeAbilityTestContext()})
	if err == nil || !strings.Contains(err.Error(), "ledger_ura must be a canonical URA") {
		t.Fatalf("non-canonical ledger source error = %v", err)
	}
}

func runtimeReceiptProviderWithOutput(t *testing.T, output map[string]any) *RuntimeReceiptProvider {
	t.Helper()
	transport := RuntimeTransportFunc{InvokeFunc: func(_ context.Context, _ []byte) ([]byte, error) {
		encoded, err := json.Marshal(output)
		if err != nil {
			return nil, err
		}
		return runtimeAbilityResultJSON(true, string(encoded), "", false), nil
	}, ResolveDescriptorRefFunc: testResolveDescriptorRef(t)}
	runtime, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	provider, err := NewRuntimeReceiptProvider(ability)
	if err != nil {
		t.Fatalf("NewRuntimeReceiptProvider: %v", err)
	}
	return provider
}

func receiptLedgerRecordFixture() map[string]any {
	return map[string]any{
		"invocation_ura":    "easynet:///r/example/resource/device.dev-a/invocation/req-1",
		"request_id":        "req-1",
		"trace_id":          "trace-1",
		"span_id":           "span-1",
		"caller_ura":        "easynet:///r/example/agent/alice.client",
		"callee_ura":        "easynet:///r/example/authority",
		"subject_ura":       "easynet:///r/example/device/dev-a",
		"ability_ura":       "easynet:///r/example/ability/authority.observe.health",
		"ability_name":      "observe.health",
		"state":             "completed",
		"started_unix_ms":   1,
		"completed_unix_ms": 2,
		"elapsed_ms":        1,
		"args":              map[string]any{"kind": "digest"},
		"result":            map[string]any{"kind": "digest"},
		"error":             nil,
		"diagnostics":       []any{},
		"causal_links":      []any{},
		"receipt_chain": map[string]any{
			"anchors":             []any{},
			"verified":            true,
			"head_receipt_hash":   nil,
			"verification_detail": "empty",
		},
		"visibility":     map[string]any{},
		"authority_form": "self",
		"usage": map[string]any{
			"tokens_in": 0, "tokens_out": 0, "duration_ms": 1, "external_calls": 0,
		},
	}
}
