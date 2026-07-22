package easynet

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	axoninv "axon.run/sdk/go/axon/invocation"
)

func completeDraftForRuntimeTest(t *testing.T) InvocationDraft {
	t.Helper()
	draft, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef(runtimeTestDescriptorRef).
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		Build()
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	return draft
}

const runtimeTestDescriptorRef = "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke"

func canonicalRuntimeReceiptFixture(
	invocationID string,
	receiptType string,
	state string,
	index uint64,
) map[string]any {
	proofPayload := []byte("canonical-runtime-test-proof")
	proofHash := sha256.Sum256(proofPayload)
	return map[string]any{
		"receipt_ura":             fmt.Sprintf("easynet:///r/example/resource/runtime/invocation/%s/receipt/%d", invocationID, index),
		"invocation_id":           invocationID,
		"receipt_type":            receiptType,
		"state":                   state,
		"index":                   index,
		"timestamp_unix_ms":       1_783_100_000_000 + index,
		"prev_receipt_hash_hex":   strings.Repeat("00", 32),
		"self_hash_hex":           fmt.Sprintf("%064x", index+1),
		"cleanup_complete":        state != "admitted" && state != "Admitted" && state != "ADMITTED",
		"caller_binding":          map[string]any{"ura": "easynet:///r/example/agent/alice.sdk", "profile": "axon-strict-v2"},
		"callee_binding":          map[string]any{"ura": "easynet:///r/example/device/dev-a", "profile": "axon-strict-v2"},
		"subject_binding":         map[string]any{"ura": "easynet:///r/example/device/dev-a", "profile": "axon-strict-v2"},
		"invocation_nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
		"causal_binding_kind":     "none",
		"causal_binding":          map[string]any{"form": "none"},
		"callee_signature": map[string]any{
			"algorithm":        "ed25519",
			"signature_base64": base64.StdEncoding.EncodeToString(bytes.Repeat([]byte{0x71}, 64)),
		},
		"signer_binding":         map[string]any{"ura": "easynet:///r/example/device/dev-a", "profile": "axon-strict-v2"},
		"authority_binding_kind": "self",
		"authority_binding": map[string]any{
			"kind":          "self",
			"principal_ura": "easynet:///r/example/device/dev-a",
		},
		"ability_binding": runtimeTestDescriptorRef,
		"subject_ref": map[string]any{
			"kind":    1,
			"ura":     "easynet:///r/example/device/dev-a",
			"profile": "axon-strict-v2",
		},
		"descriptor_version": "1.0.0",
		"schema_hash_hex":    strings.Repeat("11", 32),
		"impl_hash_hex":      strings.Repeat("22", 32),
		"runtime_env":        "go-test",
		"authority_proof": map[string]any{
			"proof_type":   "self",
			"binding_kind": "self",
			"binding": map[string]any{
				"kind":          "self",
				"principal_ura": "easynet:///r/example/device/dev-a",
			},
			"proof_payload_base64": base64.StdEncoding.EncodeToString(proofPayload),
			"proof_hash_hex":       fmt.Sprintf("%x", proofHash),
			"issuer":               map[string]any{"ura": "easynet:///r/example/device/dev-a", "profile": "axon-strict-v2"},
			"signature": map[string]any{
				"algorithm":        "ed25519",
				"signature_base64": base64.StdEncoding.EncodeToString(bytes.Repeat([]byte{0x72}, 64)),
			},
			"admission_hook": "test.runtime.admission",
		},
		"input_hash_hex":  strings.Repeat("33", 32),
		"output_hash_hex": strings.Repeat("44", 32),
		"parent_receipts": []any{},
	}
}

func canonicalRuntimeReceiptPairFixture(invocationID, terminalState string) (map[string]any, map[string]any) {
	state, err := ParseInvocationLifecycleState(terminalState)
	if err != nil {
		panic(err)
	}
	if !state.IsTerminal() {
		panic("unsupported terminal fixture state " + terminalState)
	}
	terminalType := canonicalReceiptType(state)
	admission := canonicalRuntimeReceiptFixture(invocationID, "admitted", "Admitted", 0)
	terminal := canonicalRuntimeReceiptFixture(invocationID, terminalType, terminalState, 1)
	terminal["prev_receipt_hash_hex"] = admission["self_hash_hex"]
	return admission, terminal
}

func signedForRuntimeTest(t *testing.T) SignedInvocation {
	t.Helper()
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	signed, err := prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:       "ed25519",
		SignatureBase64: "c2lnbmF0dXJl",
		KeyIDHint:       "caller-key",
	})
	if err != nil {
		t.Fatalf("SignWithCallerSignature: %v", err)
	}
	return signed
}

func submittedHandleForRuntimeTest(t *testing.T) InvocationHandle {
	t.Helper()
	handle, err := newRuntimeInvocationHandleFromJSON([]byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}], "result": null}`))
	if err != nil {
		t.Fatalf("newRuntimeInvocationHandleFromJSON: %v", err)
	}
	return handle
}

func runtimeRecoveryRequestForTest() RuntimeRecoveryRequest {
	return RuntimeRecoveryRequest{
		RecoveryID:     "recovery-1",
		DeadlineUnixMS: 1783100009999,
		MaxInvocations: 32,
	}
}

func runtimeRecoveryReportJSON(recoveryID string) []byte {
	return []byte(`{"recovery_id":` + string(mustJSON(recoveryID)) + `,"state":"runtime_started","recovered_invocations":2,"reaped_orphans":1,"replayed_terminal_receipts":1,"bounded_scan":true,"cleanup_complete":true,"events":[{"sequence":1,"kind":"orphan_reaped","invocation_id":"inv-orphan","state":"cancelled","terminal":true,"receipt_ura":"easynet:///r/example/resource/agent.alice/invocation/inv-orphan/receipt","reason":"host restart"}]}`)
}

func TestPublicInvocationHandleJSONDoesNotGrantControlAuthority(t *testing.T) {
	handle, err := NewInvocationHandleFromJSON([]byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [], "result": null}`))
	if err != nil {
		t.Fatalf("NewInvocationHandleFromJSON: %v", err)
	}
	if handle.State() != "Submitted" || handle.Terminal() {
		t.Fatalf("unexpected observation snapshot: %#v", handle)
	}
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		AwaitHandleFunc: func(context.Context, InvocationControlCapability) ([]byte, error) {
			t.Fatalf("forged public snapshot reached transport")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := client.Await(context.Background(), handle); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Await forged public snapshot = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestInvocationControlCapabilityAdapterHandleID(t *testing.T) {
	runtimeHandle := submittedHandleForRuntimeTest(t)
	handleID, err := runtimeHandle.ControlCapability().AdapterHandleID()
	if err != nil {
		t.Fatalf("runtime-bound AdapterHandleID: %v", err)
	}
	if handleID != 7 {
		t.Fatalf("runtime-bound AdapterHandleID = %d, want 7", handleID)
	}

	publicSnapshot, err := NewInvocationHandleFromJSON([]byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [], "result": null}`))
	if err != nil {
		t.Fatalf("NewInvocationHandleFromJSON: %v", err)
	}
	if _, err := publicSnapshot.ControlCapability().AdapterHandleID(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("snapshot AdapterHandleID = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPublicInvocationCancelJSONDoesNotGrantControlAuthority(t *testing.T) {
	cancel, err := NewInvocationCancelFromJSON([]byte(`{"handle_id": 7, "request_accepted": false, "deduplicated": true, "cancelled": false, "state": "Completed", "terminal": true}`))
	if err != nil {
		t.Fatalf("NewInvocationCancelFromJSON: %v", err)
	}
	if cancel.State() != "Completed" || !cancel.Terminal() {
		t.Fatalf("unexpected cancel snapshot: %#v", cancel)
	}
	if cancel.ControlCapability().valid() {
		t.Fatalf("public cancel snapshot created runtime-bound control")
	}
}

func TestRuntimeClientRestartRecoveryProviderContract(t *testing.T) {
	var seenRequest map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		RecoverFunc: func(_ context.Context, raw []byte) ([]byte, error) {
			if err := json.Unmarshal(raw, &seenRequest); err != nil {
				return nil, err
			}
			return runtimeRecoveryReportJSON("recovery-1"), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	report, err := client.Recover(context.Background(), runtimeRecoveryRequestForTest())
	if err != nil {
		t.Fatalf("Recover: %v", err)
	}

	if seenRequest["recovery_id"] != "recovery-1" ||
		seenRequest["deadline_unix_ms"] != float64(1783100009999) ||
		seenRequest["max_invocations"] != float64(32) {
		t.Fatalf("recovery request not sent to provider: %#v", seenRequest)
	}
	if report.State != "runtime_started" || !report.BoundedScan || !report.CleanupComplete {
		t.Fatalf("recovery report did not prove ready state: %#v", report)
	}
	if report.RecoveredInvocations != 2 || report.ReapedOrphans != 1 || report.ReplayedTerminalReceipts != 1 {
		t.Fatalf("recovery counters = %#v", report)
	}
	if len(report.Events) != 1 ||
		report.Events[0].Kind != "orphan_reaped" ||
		report.Events[0].ReceiptURA == "" ||
		!report.Events[0].Terminal {
		t.Fatalf("recovery event not projected: %#v", report.Events)
	}
	if _, err := NewRuntimeRecoveryReportFromJSON([]byte(`{"recovery_id":"recovery-1","state":"recovering","bounded_scan":true,"cleanup_complete":true,"events":[]}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("recovering state error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := NewRuntimeRecoveryReportFromJSON([]byte(`{"recovery_id":"recovery-1","state":"runtime_started","bounded_scan":false,"cleanup_complete":true,"events":[]}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("unbounded scan error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := NewRuntimeRecoveryReportFromJSON([]byte(`{"recovery_id":"recovery-1","state":"runtime_started","bounded_scan":true,"cleanup_complete":false,"events":[]}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete cleanup error = %v, want %s", err, ErrInvalidArgument)
	}

	invalidClient, err := NewRuntimeClient(RuntimeTransportFunc{
		RecoverFunc: func(context.Context, []byte) ([]byte, error) {
			t.Fatalf("invalid request reached provider")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := invalidClient.Recover(context.Background(), RuntimeRecoveryRequest{}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("empty recovery request error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeClientPrepareDelegatesToTransport(t *testing.T) {
	var seenDraft map[string]any
	var seenOptions map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			if err := json.Unmarshal(optionsJSON, &seenOptions); err != nil {
				t.Fatalf("options JSON: %v", err)
			}
			return []byte(preparedFixture), nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			t.Fatalf("SubmitSigned should not be called")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	prepared, material, err := client.Prepare(context.Background(), completeDraftForRuntimeTest(t), PrepareOptions{
		ExpiresInMS:        60000,
		SignerID:           "signer-alice-key-1",
		PolicyRef:          "daemon-key-inventory:sha256:test-policy",
		LocalDaemonSigning: true,
	})
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	if prepared.SubmitReady() {
		t.Fatalf("prepared is submit-ready")
	}
	if material.CanonicalBytesBase64() == "" {
		t.Fatalf("signing material missing")
	}
	if seenDraft["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("draft not sent to transport: %#v", seenDraft)
	}
	if seenOptions["expires_in_ms"].(float64) != 60000 ||
		seenOptions["signer_id"] != "signer-alice-key-1" ||
		seenOptions["policy_ref"] != "daemon-key-inventory:sha256:test-policy" ||
		seenOptions["local_daemon_signing"] != true {
		t.Fatalf("latest prepare options not sent to transport: %#v", seenOptions)
	}
}

func TestRuntimeClientPrepareSigningMaterialUsesStatelessTransportContract(t *testing.T) {
	var seenOptions map[string]any
	statelessFixture := strings.Replace(
		preparedFixture,
		`  "prepared_id": "prepared-example-1",`,
		`  "canonical_hash_hex": "87de60e0170dac6e11364521ccda53e9e2b8deaec4d0fa209b85f7a12c5260af",`,
		1,
	)
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(_ context.Context, _ []byte, optionsJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(optionsJSON, &seenOptions); err != nil {
				t.Fatalf("options JSON: %v", err)
			}
			return []byte(statelessFixture), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	material, err := client.PrepareSigningMaterial(context.Background(), completeDraftForRuntimeTest(t), PrepareOptions{
		ExpiresInMS: 60_000,
		SignerID:    "browser-key-1",
	})
	if err != nil {
		t.Fatalf("PrepareSigningMaterial: %v", err)
	}
	if material.CanonicalBytesBase64() == "" {
		t.Fatal("canonical signing material is missing")
	}
	if material.CanonicalHashHex() != "87de60e0170dac6e11364521ccda53e9e2b8deaec4d0fa209b85f7a12c5260af" {
		t.Fatalf("canonical hash = %q", material.CanonicalHashHex())
	}
	if _, err := NewPreparedInvocationFromJSON([]byte(statelessFixture)); err == nil {
		t.Fatal("retained prepared decoder accepted a material-only response")
	}
	if seenOptions["material_only"] != true ||
		seenOptions["expires_in_ms"] != float64(60_000) ||
		seenOptions["signer_id"] != "browser-key-1" {
		t.Fatalf("stateless prepare options not sent to transport: %#v", seenOptions)
	}
}

func TestRuntimeClientPrepareSigningMaterialRejectsCanonicalCommitmentMismatch(t *testing.T) {
	mismatched := strings.Replace(
		preparedFixture,
		`  "prepared_id": "prepared-example-1",`,
		`  "canonical_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",`,
		1,
	)
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(context.Context, []byte, []byte) ([]byte, error) {
			return []byte(mismatched), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := client.PrepareSigningMaterial(
		context.Background(),
		completeDraftForRuntimeTest(t),
		PrepareOptions{ExpiresInMS: 60_000},
	); !IsCode(err, ErrInvalidArgument) ||
		!strings.Contains(err.Error(), "canonical_hash_hex does not match canonical_bytes_base64") {
		t.Fatalf("canonical commitment mismatch error = %v, want canonical hash mismatch", err)
	}
}

func TestRuntimeReceiptProofFactsRequired(t *testing.T) {
	fixture := canonicalRuntimeReceiptFixture("inv-1", "completed", "Completed", 1)
	fixture["self_hash_hex"] = strings.Repeat("aa", 32)
	fixture["causal_binding_kind"] = "scalar"
	fixture["causal_binding"] = map[string]any{
		"form": "scalar",
		"receipt": map[string]any{
			"receipt_hash_hex": strings.Repeat("bb", 32),
			"receipt_ura":      "easynet:///r/example/resource/agent.alice.sdk/invocation/root/receipt",
		},
	}
	receipt, err := NewRuntimeReceiptFromJSON(mustJSON(fixture))
	if err != nil {
		t.Fatalf("NewRuntimeReceiptFromJSON: %v", err)
	}
	if err := receipt.ValidateSummary(); err != nil {
		t.Fatalf("ValidateSummary: %v", err)
	}
	selfHash, err := receipt.SelfReceiptHash()
	if err != nil {
		t.Fatalf("SelfReceiptHash: %v", err)
	}
	if !bytes.Equal(selfHash, bytes.Repeat([]byte{0xaa}, 32)) {
		t.Fatalf("self hash = %x", selfHash)
	}
	if receipt.CausalBindingKind != "scalar" || receipt.CausalBinding["form"] != "scalar" {
		t.Fatalf("causal binding not decoded: %#v", receipt.CausalBinding)
	}
	if receipt.AuthorityBindingKind != "self" || receipt.AuthorityBinding["kind"] != "self" {
		t.Fatalf("authority binding not decoded: %#v", receipt.AuthorityBinding)
	}

	incomplete := canonicalRuntimeReceiptFixture("inv-1", "completed", "Completed", 1)
	delete(incomplete, "authority_proof")
	if _, err := NewRuntimeReceiptFromJSON(mustJSON(incomplete)); err == nil {
		t.Fatal("NewRuntimeReceiptFromJSON accepted receipt without proof facts")
	}
}

func TestRuntimeReceiptOwnsFailClosedLifecycleProjection(t *testing.T) {
	complete := canonicalRuntimeReceiptFixture("inv-state", "completed", "completed", 1)
	receipt, err := NewRuntimeReceiptFromJSON(mustJSON(complete))
	if err != nil {
		t.Fatalf("NewRuntimeReceiptFromJSON: %v", err)
	}
	state, err := receipt.LifecycleState()
	if err != nil {
		t.Fatalf("LifecycleState: %v", err)
	}
	if state != InvocationLifecycleCompleted || !state.IsTerminal() {
		t.Fatalf("LifecycleState = %q, want terminal %q", state, InvocationLifecycleCompleted)
	}

	for _, invalid := range []any{"invented_state", " completed ", "5", 5, "UNSPECIFIED"} {
		malformed := canonicalRuntimeReceiptFixture("inv-state", "completed", "completed", 1)
		malformed["state"] = invalid
		if _, err := NewRuntimeReceiptFromJSON(mustJSON(malformed)); !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("NewRuntimeReceiptFromJSON(state=%v) = %v, want %s", invalid, err, ErrInvalidArgument)
		}
	}

	for _, invalid := range []string{"terminal", "failed", "Completed"} {
		malformed := canonicalRuntimeReceiptFixture("inv-state", invalid, "completed", 1)
		if _, err := NewRuntimeReceiptFromJSON(mustJSON(malformed)); !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("NewRuntimeReceiptFromJSON(receipt_type=%q) = %v, want %s", invalid, err, ErrInvalidArgument)
		}
	}
}

func TestRuntimeReceiptRejectsMalformedSummaryHash(t *testing.T) {
	fixture := canonicalRuntimeReceiptFixture("inv-1", "completed", "Completed", 1)
	fixture["self_hash_hex"] = "aa"
	if _, err := NewRuntimeReceiptFromJSON(mustJSON(fixture)); err == nil {
		t.Fatal("NewRuntimeReceiptFromJSON accepted short self hash")
	}
}

func TestRuntimeReceiptRejectsMalformedCanonicalProofFacts(t *testing.T) {
	tests := map[string]func(map[string]any){
		"invalid nonce": func(receipt map[string]any) {
			receipt["invocation_nonce_base64"] = "not-base64"
		},
		"missing parent binding": func(receipt map[string]any) {
			receipt["parent_receipts"] = nil
		},
		"malformed parent hash": func(receipt map[string]any) {
			receipt["parent_receipts"] = []any{map[string]any{
				"receipt_hash_hex": "aa",
				"receipt_ura":      "easynet:///r/example/resource/parent",
			}}
		},
		"mismatched proof hash": func(receipt map[string]any) {
			proof := receipt["authority_proof"].(map[string]any)
			proof["proof_hash_hex"] = strings.Repeat("ff", 32)
		},
		"mismatched authority kind": func(receipt map[string]any) {
			proof := receipt["authority_proof"].(map[string]any)
			proof["binding_kind"] = "delegation"
		},
		"missing proof binding": func(receipt map[string]any) {
			proof := receipt["authority_proof"].(map[string]any)
			delete(proof, "binding")
		},
		"mismatched proof binding": func(receipt map[string]any) {
			proof := receipt["authority_proof"].(map[string]any)
			proof["binding"] = map[string]any{
				"kind":          "self",
				"principal_ura": "easynet:///r/example/device/other",
			}
		},
		"missing admission hook": func(receipt map[string]any) {
			proof := receipt["authority_proof"].(map[string]any)
			delete(proof, "admission_hook")
		},
		"issuer does not match callee": func(receipt map[string]any) {
			proof := receipt["authority_proof"].(map[string]any)
			proof["issuer"] = map[string]any{
				"ura":     "easynet:///r/example/device/other",
				"profile": "axon-strict-v2",
			}
		},
		"invalid identity profile": func(receipt map[string]any) {
			receipt["caller_binding"].(map[string]any)["profile"] = "test"
		},
		"hosted signer without attestation": func(receipt map[string]any) {
			receipt["signer_binding"] = map[string]any{
				"ura":     "easynet:///r/example/device/runtime-host",
				"profile": "axon-strict-v2",
			}
		},
		"self signer with attestation": func(receipt map[string]any) {
			receipt["host_attestation_base64"] = base64.StdEncoding.EncodeToString(
				bytes.Repeat([]byte{0x73}, 64),
			)
		},
	}
	for name, mutate := range tests {
		t.Run(name, func(t *testing.T) {
			fixture := canonicalRuntimeReceiptFixture("inv-1", "completed", "Completed", 1)
			mutate(fixture)
			if _, err := NewRuntimeReceiptFromJSON(mustJSON(fixture)); !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
			}
		})
	}
}

func TestRuntimeReceiptRejectsTypedProjectionThatDiffersFromRaw(t *testing.T) {
	receipt, err := NewRuntimeReceiptFromJSON(
		mustJSON(canonicalRuntimeReceiptFixture("inv-raw", "completed", "Completed", 1)),
	)
	if err != nil {
		t.Fatalf("NewRuntimeReceiptFromJSON: %v", err)
	}
	receipt.State = "Failed"

	if err := receipt.ValidateSummary(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ValidateSummary error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeReceiptAcceptsBindingHashProofWithoutPayloadOrSignature(t *testing.T) {
	fixture := canonicalRuntimeReceiptFixture("inv-empty-proof", "completed", "Completed", 1)
	proof := fixture["authority_proof"].(map[string]any)
	proof["proof_payload_base64"] = ""
	proofHash := axoninv.AuthorityBindingProofHash(
		axoninv.SelfAuthority("easynet:///r/example/device/dev-a"),
	)
	proof["proof_hash_hex"] = hex.EncodeToString(proofHash[:])
	delete(proof, "signature")

	receipt, err := NewRuntimeReceiptFromJSON(mustJSON(fixture))
	if err != nil {
		t.Fatalf("NewRuntimeReceiptFromJSON: %v", err)
	}
	if receipt.AuthorityProof == nil || receipt.AuthorityProof.ProofPayloadBase64 != "" {
		t.Fatalf("unexpected authority proof projection: %#v", receipt.AuthorityProof)
	}
	if receipt.AuthorityProof.Signature != nil {
		t.Fatalf("optional authority proof signature was synthesized: %#v", receipt.AuthorityProof.Signature)
	}
}

func TestRuntimeClientPrepareBuilderConsumesOnlyAfterSuccess(t *testing.T) {
	transportPrepareCalls := 0
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			transportPrepareCalls++
			return []byte(preparedFixture), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	builder := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json")

	prepared, material, err := client.PrepareBuilder(context.Background(), builder, PrepareOptions{})
	if err != nil {
		t.Fatalf("PrepareBuilder: %v", err)
	}
	if prepared.PreparedID() == "" || material.CanonicalBytesBase64() == "" || transportPrepareCalls != 1 {
		t.Fatalf("unexpected prepare-builder result: prepared=%#v material=%#v calls=%d", prepared, material, transportPrepareCalls)
	}
	if _, err := builder.Inspect(); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("Inspect after PrepareBuilder = %v, want %s", err, ErrInvalidHandle)
	}
}

func TestRuntimeClientPrepareBuilderKeepsBuilderOnFailure(t *testing.T) {
	down := errors.New("daemon unavailable")
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			return nil, down
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	builder := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json")

	if _, _, err := client.PrepareBuilder(context.Background(), builder, PrepareOptions{}); !IsCode(err, ErrTransport) {
		t.Fatalf("PrepareBuilder failure = %v, want %s", err, ErrTransport)
	}
	if _, err := builder.Inspect(); err != nil {
		t.Fatalf("builder consumed after failed PrepareBuilder: %v", err)
	}
}

func TestRuntimeClientSubmitSignedPreservesSignature(t *testing.T) {
	var seenSigned map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			t.Fatalf("Prepare should not be called")
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(signedJSON, &seenSigned); err != nil {
				t.Fatalf("signed JSON: %v", err)
			}
			return []byte(`{"handle_id": 7, "state": "Submitted", "terminal": false, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}], "result": null}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	handle, err := client.SubmitSigned(context.Background(), signedForRuntimeTest(t))
	if err != nil {
		t.Fatalf("SubmitSigned: %v", err)
	}
	if !handle.ControlCapability().valid() || handle.State() != "Submitted" || handle.Terminal() {
		t.Fatalf("unexpected handle: %#v", handle)
	}
	if len(handle.Events()) != 1 || handle.Events()[0].Sequence() != 1 {
		t.Fatalf("unexpected handle events: %#v", handle.Events())
	}
	signature := seenSigned["signature"].(map[string]any)
	if signature["signature_base64"] != "c2lnbmF0dXJl" {
		t.Fatalf("signature not preserved: %#v", seenSigned)
	}
	prepared := seenSigned["prepared"].(map[string]any)
	tuple := prepared["tuple"].(map[string]any)
	if tuple["caller_ura"] != "easynet:///r/example/agent/alice.sdk" ||
		tuple["callee_ura"] != "easynet:///r/example/device/dev-a" ||
		tuple["descriptor_ref"] != runtimeTestDescriptorRef {
		t.Fatalf("prepared tuple not preserved: %#v", seenSigned)
	}
}

func TestRuntimeClientPrepareWrapsTransportFailure(t *testing.T) {
	down := errors.New("daemon unavailable")
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			return nil, down
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	_, _, err = client.Prepare(context.Background(), completeDraftForRuntimeTest(t), PrepareOptions{})
	if err == nil {
		t.Fatalf("Prepare succeeded, want transport error")
	}
	if !IsCode(err, ErrTransport) {
		t.Fatalf("error code = %v, want %s", err, ErrTransport)
	}
	if !errors.Is(err, down) {
		t.Fatalf("transport cause not preserved")
	}
}

func TestRuntimeClientSubmitRejectsMalformedHandle(t *testing.T) {
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			return []byte(`{"state": "Submitted"}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	_, err = client.SubmitSigned(context.Background(), signedForRuntimeTest(t))
	if err == nil {
		t.Fatalf("SubmitSigned succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeClientInvokeReturnsTypedResult(t *testing.T) {
	var seenDraft map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			admission, terminal := canonicalRuntimeReceiptPairFixture("inv-runtime-1", "Completed")
			admission["receipt_id"] = "receipt-1-admission"
			terminal["receipt_id"] = "receipt-1"
			return mustJSON(map[string]any{
				"ok":                  true,
				"tuple":               seenDraft,
				"invocation_id":       "inv-runtime-1",
				"terminal_state":      "Completed",
				"output_content_type": "application/json",
				"output_base64":       "eyJyZWFkeSI6dHJ1ZX0=",
				"output_json":         map[string]any{"ready": true},
				"elapsed_ms":          12,
				"admission_receipt":   admission,
				"terminal_receipt":    terminal,
				"error":               nil,
			}), nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			t.Fatalf("Prepare should not be called")
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			t.Fatalf("SubmitSigned should not be called")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	result, err := client.Invoke(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if !result.OK() || result.TerminalState() != "Completed" {
		t.Fatalf("unexpected result: %#v", result)
	}
	lifecycleState, err := result.LifecycleState()
	if err != nil {
		t.Fatalf("LifecycleState: %v", err)
	}
	if lifecycleState != InvocationLifecycleCompleted {
		t.Fatalf("LifecycleState = %q, want %q", lifecycleState, InvocationLifecycleCompleted)
	}
	if result.Tuple().CallerURA() != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("tuple not decoded: %#v", result.Tuple())
	}
	if result.InvocationID() != "inv-runtime-1" {
		t.Fatalf("invocation id = %q", result.InvocationID())
	}
	if seenDraft["descriptor_ref"] == "" {
		t.Fatalf("draft not sent to transport: %#v", seenDraft)
	}
	if string(result.OutputJSON()) != `{"ready":true}` {
		t.Fatalf("output JSON not preserved: %s", result.OutputJSON())
	}
}

func TestInvocationResultDerivesJSONOutputFromCanonicalPayload(t *testing.T) {
	admission, terminal := canonicalRuntimeReceiptPairFixture("inv-derived-output", "Completed")
	raw := mustJSON(map[string]any{
		"ok":                  true,
		"tuple":               completeDraftForRuntimeTest(t),
		"invocation_id":       "inv-derived-output",
		"terminal_state":      "Completed",
		"output_content_type": "application/json; charset=utf-8",
		"output_base64":       "eyJyZWFkeSI6dHJ1ZX0=",
		"output_json":         nil,
		"elapsed_ms":          1,
		"admission_receipt":   admission,
		"terminal_receipt":    terminal,
		"error":               nil,
	})
	result, err := NewInvocationResultFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationResultFromJSON: %v", err)
	}
	if got := string(result.OutputJSON()); got != `{"ready":true}` {
		t.Fatalf("derived output JSON = %s", got)
	}
}

func TestInvocationResultSeparatesAdmissionAndTerminalReceipts(t *testing.T) {
	admission, terminal := canonicalRuntimeReceiptPairFixture("inv-1", "Completed")
	raw := mustJSON(map[string]any{
		"ok":                true,
		"tuple":             completeDraftForRuntimeTest(t),
		"invocation_id":     "inv-1",
		"terminal_state":    "Completed",
		"admission_receipt": admission,
		"terminal_receipt":  terminal,
		"error":             nil,
	})
	result, err := NewInvocationResultFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationResultFromJSON: %v", err)
	}
	if !strings.Contains(string(result.AdmissionReceipt()), `"index":0`) {
		t.Fatalf("admission receipt = %s", result.AdmissionReceipt())
	}
	if !strings.Contains(string(result.TerminalReceipt()), `"index":1`) {
		t.Fatalf("terminal receipt = %s", result.TerminalReceipt())
	}
	if summary := result.TerminalReceiptSummary(); summary == nil || summary.Index != 1 || summary.State != "Completed" {
		t.Fatalf("terminal receipt summary = %#v", summary)
	}
	if summary := result.AdmissionReceiptSummary(); summary == nil || summary.Index != 0 || summary.State != "Admitted" {
		t.Fatalf("admission receipt summary = %#v", summary)
	}

	var legacy map[string]any
	if err := json.Unmarshal(raw, &legacy); err != nil {
		t.Fatalf("decode fixture: %v", err)
	}
	legacy["receipt"] = legacy["terminal_receipt"]
	delete(legacy, "terminal_receipt")
	legacyOnly := mustJSON(legacy)
	if _, err := NewInvocationResultFromJSON(legacyOnly); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("legacy receipt-only field error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestInvocationResultAllowsOnlyTypedReceiptFreePreAdmissionFailure(t *testing.T) {
	draftJSON, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("marshal draft: %v", err)
	}
	allowed := []string{
		"global_admission",
		"caller_authentication",
		"authority_validation",
		"bootstrap_authorization",
		"quota",
		"ability_resolution",
		"ability_policy",
		"request_validation",
	}
	for _, stage := range allowed {
		raw := []byte(fmt.Sprintf(`{
			"ok": false,
			"tuple": %s,
			"terminal_state": "Failed",
			"admission_receipt": null,
			"terminal_receipt": null,
			"error": {
				"code": "ADMISSION_DENIED",
				"stage": %q,
				"message": "rejected before admission",
				"retryable": false
			}
		}`, draftJSON, stage))
		result, err := NewInvocationResultFromJSON(raw)
		if err != nil {
			t.Fatalf("stage %q: %v", stage, err)
		}
		if result.OK() || result.TerminalState() != "Failed" ||
			result.AdmissionReceipt() != nil || result.TerminalReceipt() != nil {
			t.Fatalf("stage %q decoded invalid result: %#v", stage, result)
		}
	}
}

func TestInvocationResultRejectsNonCanonicalReceiptTopology(t *testing.T) {
	draftJSON, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("marshal draft: %v", err)
	}
	failure := `"error":{
		"code":"ADMISSION_DENIED",
		"stage":"global_admission",
		"message":"rejected",
		"retryable":false
	}`
	tests := map[string]string{
		"successful receipt free": fmt.Sprintf(`{
			"ok":true,
			"tuple":%s,
			"terminal_state":"Completed",
			"admission_receipt":null,
			"terminal_receipt":null,
			"error":null
		}`, draftJSON),
		"wrong receipt free state": fmt.Sprintf(`{
			"ok":false,
			"tuple":%s,
			"terminal_state":"Cancelled",
			"admission_receipt":null,
			"terminal_receipt":null,
			%s
		}`, draftJSON, failure),
		"normalized receipt free state": fmt.Sprintf(`{
			"ok":false,
			"tuple":%s,
			"terminal_state":" Failed ",
			"admission_receipt":null,
			"terminal_receipt":null,
			%s
		}`, draftJSON, failure),
		"admission only": fmt.Sprintf(`{
			"ok":false,
			"tuple":%s,
			"terminal_state":"Failed",
			"admission_receipt":{"index":1,"state":"Admitted"},
			"terminal_receipt":null,
			%s
		}`, draftJSON, failure),
		"terminal only": fmt.Sprintf(`{
			"ok":true,
			"tuple":%s,
			"terminal_state":"Completed",
			"admission_receipt":null,
			"terminal_receipt":{"index":2,"state":"Completed"},
			"error":null
		}`, draftJSON),
	}
	for _, stage := range []string{"execution", "transport", "unspecified", "Execution", ""} {
		tests["receipt free stage "+stage] = fmt.Sprintf(`{
			"ok":false,
			"tuple":%s,
			"terminal_state":"Failed",
			"admission_receipt":null,
			"terminal_receipt":null,
			"error":{
				"code":"ADMISSION_DENIED",
				"stage":%q,
				"message":"rejected",
				"retryable":false
			}
		}`, draftJSON, stage)
	}
	for name, raw := range tests {
		t.Run(name, func(t *testing.T) {
			if _, err := NewInvocationResultFromJSON([]byte(raw)); !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
			}
		})
	}
}

func TestInvocationResultRejectsConflictingCanonicalReceiptBindings(t *testing.T) {
	tests := map[string]func(map[string]any, map[string]any){
		"admission state": func(admission, _ map[string]any) {
			admission["state"] = "Running"
		},
		"admission receipt type": func(admission, _ map[string]any) {
			admission["receipt_type"] = "completed"
		},
		"admission receipt type case": func(admission, _ map[string]any) {
			admission["receipt_type"] = "Admitted"
		},
		"terminal state": func(_, terminal map[string]any) {
			terminal["state"] = "Failed"
		},
		"terminal receipt type": func(_, terminal map[string]any) {
			terminal["receipt_type"] = "failed"
		},
		"terminal receipt type case": func(_, terminal map[string]any) {
			terminal["receipt_type"] = "Completed"
		},
		"terminal index": func(_, terminal map[string]any) {
			terminal["index"] = 0
		},
		"terminal cleanup": func(_, terminal map[string]any) {
			terminal["cleanup_complete"] = false
		},
		"terminal timestamp": func(_, terminal map[string]any) {
			terminal["timestamp_unix_ms"] = 0
		},
		"invocation binding": func(_, terminal map[string]any) {
			terminal["invocation_id"] = "other"
		},
		"caller binding": func(_, terminal map[string]any) {
			terminal["caller_binding"] = map[string]any{
				"ura": "easynet:///r/example/agent/other",
			}
		},
		"host attestation": func(_, terminal map[string]any) {
			terminal["host_attestation_base64"] = base64.StdEncoding.EncodeToString([]byte("other-host"))
		},
	}
	for name, mutate := range tests {
		t.Run(name, func(t *testing.T) {
			admission, terminal := canonicalRuntimeReceiptPairFixture("inv-1", "Completed")
			mutate(admission, terminal)
			raw := mustJSON(map[string]any{
				"ok":                true,
				"tuple":             completeDraftForRuntimeTest(t),
				"invocation_id":     "inv-1",
				"terminal_state":    "Completed",
				"admission_receipt": admission,
				"terminal_receipt":  terminal,
				"error":             nil,
			})
			if _, err := NewInvocationResultFromJSON(raw); !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
			}
		})
	}
}

func TestInvocationResultAcceptsNonAdjacentFinalizationCheckpoints(t *testing.T) {
	admission, terminal := canonicalRuntimeReceiptPairFixture("inv-checkpoints", "Completed")
	admission["index"] = uint64(1)
	admission["prev_receipt_hash_hex"] = strings.Repeat("aa", 32)
	terminal["index"] = uint64(7)
	terminal["prev_receipt_hash_hex"] = strings.Repeat("bb", 32)

	raw := mustJSON(map[string]any{
		"ok":                true,
		"tuple":             completeDraftForRuntimeTest(t),
		"invocation_id":     "inv-checkpoints",
		"terminal_state":    "Completed",
		"admission_receipt": admission,
		"terminal_receipt":  terminal,
		"error":             nil,
	})
	if _, err := NewInvocationResultFromJSON(raw); err != nil {
		t.Fatalf("non-adjacent canonical finalization checkpoints: %v", err)
	}
}

func TestRuntimeClientInvokeStreamOpensStreamHandle(t *testing.T) {
	var seenDraft map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		OpenStreamFunc: func(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			return &memoryStreamTransport{events: []string{
				`{"sequence":1,"kind":"terminal","state":"Completed","terminal":true}`,
			}}, []byte(`{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	stream, err := client.InvokeStream(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	if stream.StreamID() != "stream-1" || stream.State() != StreamOpening {
		t.Fatalf("unexpected stream: id=%q state=%s", stream.StreamID(), stream.State())
	}
	if seenDraft["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("draft not forwarded: %#v", seenDraft)
	}
}

func TestRuntimeClientOpenBidiOpensSession(t *testing.T) {
	var seenDraft map[string]any
	var seenStreams []map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		OpenBidiFunc: func(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			if err := json.Unmarshal(streamsJSON, &seenStreams); err != nil {
				t.Fatalf("streams JSON: %v", err)
			}
			return &memoryBidiTransport{}, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	session, err := client.OpenBidi(context.Background(), completeDraftForRuntimeTest(t), []BidiStreamDescriptor{
		{StreamID: 1, ContentType: "application/json", Ordering: "ordered"},
	})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	if session.SessionID() != "bidi-1" || session.State() != BidiOpen {
		t.Fatalf("unexpected bidi session: id=%q state=%s", session.SessionID(), session.State())
	}
	if seenDraft["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("draft not forwarded: %#v", seenDraft)
	}
	if len(seenStreams) != 1 || seenStreams[0]["stream_id"] != float64(1) || seenStreams[0]["content_type"] != "application/json" {
		t.Fatalf("streams not forwarded: %#v", seenStreams)
	}
}

func TestInvocationResultRejectsInconsistentFailure(t *testing.T) {
	draftJSON, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Marshal draft: %v", err)
	}

	_, err = NewInvocationResultFromJSON([]byte(fmt.Sprintf(`{
		"ok": false,
		"tuple": %s,
		"terminal_state": "Failed",
		"output_content_type": "application/json",
		"output_base64": "",
		"output_json": null,
		"elapsed_ms": 3,
		"terminal_receipt": null,
		"error": null
	}`, draftJSON)))
	if err == nil {
		t.Fatalf("NewInvocationResultFromJSON succeeded, want invalid result")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeClientHandleObservationDelegatesToTransport(t *testing.T) {
	draftJSON, err := json.Marshal(completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Marshal draft: %v", err)
	}
	var seenAwaitID uint64
	var seenFreeID uint64
	var seenCancelReason string
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called")
			return nil, nil
		},
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			t.Fatalf("Prepare should not be called")
			return nil, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			t.Fatalf("SubmitSigned should not be called")
			return nil, nil
		},
		AwaitHandleFunc: func(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
			seenAwaitID = control.adapterHandleID()
			var draft map[string]any
			if err := json.Unmarshal(draftJSON, &draft); err != nil {
				return nil, err
			}
			admission, terminal := canonicalRuntimeReceiptPairFixture("inv-await-1", "Completed")
			return mustJSON(map[string]any{
				"ok":                  true,
				"tuple":               draft,
				"invocation_id":       "inv-await-1",
				"terminal_state":      "Completed",
				"output_content_type": "application/json",
				"output_base64":       "e30=",
				"output_json":         map[string]any{},
				"elapsed_ms":          8,
				"admission_receipt":   admission,
				"terminal_receipt":    terminal,
				"error":               nil,
			}), nil
		},
		CancelHandleFunc: func(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
			if control.adapterHandleID() != 7 {
				t.Fatalf("control handle = %d, want 7", control.adapterHandleID())
			}
			seenCancelReason = reason
			return []byte(`{"handle_id": 7, "request_accepted": false, "deduplicated": true, "cancelled": false, "state": "Completed", "terminal": true}`), nil
		},
		HandleEventsFunc: func(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
			if control.adapterHandleID() != 7 {
				t.Fatalf("control handle = %d, want 7", control.adapterHandleID())
			}
			return []byte(`{"handle_id": 7, "state": "Cancelled", "terminal": true, "events": [{"sequence": 1, "kind": "submitted", "state": "Submitted", "terminal": false}, {"sequence": 2, "kind": "cancelled", "state": "Cancelled", "terminal": true, "reason": "client stop"}], "result": null}`), nil
		},
		FreeHandleFunc: func(ctx context.Context, control InvocationControlCapability) error {
			seenFreeID = control.adapterHandleID()
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	handle := submittedHandleForRuntimeTest(t)

	result, err := client.Await(context.Background(), handle)
	if err != nil {
		t.Fatalf("Await: %v", err)
	}
	if seenAwaitID != 7 || !result.OK() {
		t.Fatalf("await did not use handle id/result: id=%d result=%#v", seenAwaitID, result)
	}
	cancelled, err := client.Cancel(context.Background(), handle, "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	if cancelled.RequestAccepted() || !cancelled.Deduplicated() || cancelled.Cancelled() || !cancelled.Terminal() || cancelled.State() != "Completed" || seenCancelReason != "client stop" {
		t.Fatalf("unexpected cancellation: %#v reason=%q", cancelled, seenCancelReason)
	}
	mismatchedCancelClient, err := NewRuntimeClient(RuntimeTransportFunc{
		CancelHandleFunc: func(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
			return []byte(`{"handle_id": 8, "request_accepted": true, "deduplicated": false, "cancelled": true, "state": "Cancelled", "terminal": true}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient mismatched cancel: %v", err)
	}
	if _, err := mismatchedCancelClient.Cancel(context.Background(), handle, "client stop"); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Cancel mismatched handle = %v, want %s", err, ErrInvalidArgument)
	}
	events, err := client.Events(context.Background(), handle)
	if err != nil {
		t.Fatalf("Events: %v", err)
	}
	if !events.Terminal() || len(events.Events()) != 2 || events.Events()[1].Reason() != "client stop" {
		t.Fatalf("unexpected events: %#v", events.Events())
	}
	if err := client.CloseHandle(context.Background(), handle); err != nil {
		t.Fatalf("CloseHandle: %v", err)
	}
	if seenFreeID != 7 {
		t.Fatalf("free did not use handle id: id=%d", seenFreeID)
	}
}

func TestRuntimeClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	var closeCalls int
	var invokeCalls int
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			invokeCalls++
			return []byte(`{}`), nil
		},
		CloseFunc: func(ctx context.Context) error {
			closeCalls++
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", closeCalls)
	}
	_, err = client.Invoke(context.Background(), completeDraftForRuntimeTest(t))
	if err == nil {
		t.Fatalf("Invoke after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if invokeCalls != 0 {
		t.Fatalf("invoke reached transport after close: %d calls", invokeCalls)
	}
}

func TestRuntimeClientCloseFailureIsTerminal(t *testing.T) {
	down := errors.New("close failed")
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			t.Fatalf("Invoke should not be called after failed close")
			return nil, nil
		},
		CloseFunc: func(ctx context.Context) error {
			return down
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	err = client.Close(context.Background())
	if err == nil {
		t.Fatalf("Close succeeded, want transport error")
	}
	if !IsCode(err, ErrTransport) || !errors.Is(err, down) {
		t.Fatalf("close error not wrapped as transport cause: %v", err)
	}
	_, err = client.Invoke(context.Background(), completeDraftForRuntimeTest(t))
	if err == nil {
		t.Fatalf("Invoke after failed close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}
