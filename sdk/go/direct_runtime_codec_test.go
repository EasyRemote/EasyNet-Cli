//go:build runtime_direct

package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"strings"
	"testing"
	"time"

	axoninv "axon.run/sdk/go/axon/invocation"
	"easynet.run/cli/sdk/go/internal/axonpb"
)

func TestDirectDescriptorBoundCodecPreservesInvocationAcrossCallModes(t *testing.T) {
	codec, err := newDirectDescriptorBoundCodec(1500 * time.Millisecond)
	if err != nil {
		t.Fatalf("newDirectDescriptorBoundCodec: %v", err)
	}
	draft := directRuntimeSignedDraft(t)
	unaryProjection, err := codec.build(context.Background(), draft, axoninv.CallModeRPC)
	if err != nil {
		t.Fatalf("codec.build(rpc): %v", err)
	}
	unary, err := unaryProjection.unary()
	if err != nil {
		t.Fatalf("request.unary: %v", err)
	}
	streamProjection, err := codec.build(context.Background(), draft, axoninv.CallModeStream)
	if err != nil {
		t.Fatalf("codec.build(stream): %v", err)
	}
	stream, err := streamProjection.stream()
	if err != nil {
		t.Fatalf("request.stream: %v", err)
	}
	bidiProjection, err := codec.build(context.Background(), draft, axoninv.CallModeBidi)
	if err != nil {
		t.Fatalf("codec.build(bidi): %v", err)
	}
	bidi, err := bidiProjection.bidi([]*axonpb.StreamDescriptor{{
		StreamId:    7,
		ContentType: "text/plain",
		Ordering:    "STRICT",
	}})
	if err != nil {
		t.Fatalf("request.bidi: %v", err)
	}

	assertDirectDescriptorBoundEnvelope(t, unary.GetEnvelope(), draft)
	assertDirectDescriptorBoundEnvelope(t, stream.GetEnvelope(), draft)
	assertDirectDescriptorBoundEnvelope(t, bidi.GetEnvelopeOpen().GetEnvelope(), draft)
	requestIDs := map[string]struct{}{
		unary.GetEnvelope().GetRequestId():                  {},
		stream.GetEnvelope().GetRequestId():                 {},
		bidi.GetEnvelopeOpen().GetEnvelope().GetRequestId(): {},
	}
	if len(requestIDs) != 3 {
		t.Fatalf("Axon request ids must be unique per dispatch: %#v", requestIDs)
	}
	if got, want := string(unary.GetArguments()), `{"city":"Singapore"}`; got != want {
		t.Fatalf("unary arguments = %q, want %q", got, want)
	}
	if got := string(stream.GetArguments()); got != string(unary.GetArguments()) {
		t.Fatalf("stream arguments = %q, want unary %q", got, unary.GetArguments())
	}
	if got := string(bidi.GetEnvelopeOpen().GetInitialArgs()); got != string(unary.GetArguments()) {
		t.Fatalf("bidi arguments = %q, want unary %q", got, unary.GetArguments())
	}
	if unary.GetTarget().GetAbility().GetFunctionName() != "er.weather" ||
		stream.GetTarget().GetAbility().GetFunctionName() != "er.weather" ||
		bidi.GetEnvelopeOpen().GetTarget().GetAbility().GetFunctionName() != "er.weather" {
		t.Fatalf(
			"public route drift: unary=%q stream=%q bidi=%q",
			unary.GetTarget().GetAbility().GetFunctionName(),
			stream.GetTarget().GetAbility().GetFunctionName(),
			bidi.GetEnvelopeOpen().GetTarget().GetAbility().GetFunctionName(),
		)
	}
	for mode, target := range map[string]*axonpb.InvocationTarget{
		"unary":  unary.GetTarget(),
		"stream": stream.GetTarget(),
		"bidi":   bidi.GetEnvelopeOpen().GetTarget(),
	} {
		if target.GetAbility().GetAbilityName() != draft.DescriptorRef() {
			t.Fatalf("%s descriptor-bound target = %#v", mode, target)
		}
	}
	if unary.GetTimeoutSeconds() != 2 || stream.GetTimeoutSeconds() != 2 {
		t.Fatalf("wire timeout = unary:%d stream:%d, want 2", unary.GetTimeoutSeconds(), stream.GetTimeoutSeconds())
	}
	for mode, metadata := range map[string]map[string]string{
		"unary":  unary.GetMetadata(),
		"stream": stream.GetMetadata(),
		"bidi":   bidi.GetEnvelopeOpen().GetMetadata(),
	} {
		if metadata["timeout_ms"] != "1500" {
			t.Fatalf("%s metadata = %#v", mode, metadata)
		}
	}
	for mode, content := range map[string]*axonpb.ContentEnvelope{
		"unary":  unary.GetContentEnvelope(),
		"stream": stream.GetContentEnvelope(),
		"bidi":   bidi.GetEnvelopeOpen().GetContentEnvelope(),
	} {
		if content.GetContentType() != "application/json" || content.GetEncoding() != "identity" {
			t.Fatalf("%s content envelope = %#v; codec must make the payload encoding explicit", mode, content)
		}
	}
	if bidi.GetEnvelopeOpen().GetSessionExt().GetContractVersion() != directBidiContractVersion {
		t.Fatalf("bidi contract version = %d", bidi.GetEnvelopeOpen().GetSessionExt().GetContractVersion())
	}
	if got, want := bidi.GetMac(), unary.GetEnvelope().GetCallerSignature().GetSignature(); string(got) != string(want) {
		t.Fatalf("bidi caller MAC = %x, want signature %x", got, want)
	}
}

func TestDirectDescriptorBoundCodecRejectsUnsignedDispatch(t *testing.T) {
	codec, err := newDirectDescriptorBoundCodec(time.Second)
	if err != nil {
		t.Fatalf("newDirectDescriptorBoundCodec: %v", err)
	}
	_, err = codec.build(context.Background(), directRuntimeUnsignedDraft(t), axoninv.CallModeRPC)
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("unsigned direct dispatch error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestDirectRuntimeUnaryReceiptFreeOutcomeRequiresTypedPreAdmissionFailure(t *testing.T) {
	draft := directRuntimeDraft(t)
	_, err := directInvokeResponseJSON(draft, &axonpb.InvokeResponse{
		State: axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
	})
	if !IsCode(err, ErrProtocol) {
		t.Fatalf("receipt-free unary terminal error = %v, want %s", err, ErrProtocol)
	}

	allowed := []axonpb.ErrorStage{
		axonpb.ErrorStage_ERROR_STAGE_GLOBAL_ADMISSION,
		axonpb.ErrorStage_ERROR_STAGE_CALLER_AUTHENTICATION,
		axonpb.ErrorStage_ERROR_STAGE_AUTHORITY_VALIDATION,
		axonpb.ErrorStage_ERROR_STAGE_BOOTSTRAP_AUTHORIZATION,
		axonpb.ErrorStage_ERROR_STAGE_QUOTA,
		axonpb.ErrorStage_ERROR_STAGE_ABILITY_RESOLUTION,
		axonpb.ErrorStage_ERROR_STAGE_ABILITY_POLICY,
		axonpb.ErrorStage_ERROR_STAGE_REQUEST_VALIDATION,
	}
	for _, stage := range allowed {
		raw, err := directInvokeResponseJSON(draft, &axonpb.InvokeResponse{
			State: axonpb.InvocationState_INVOCATION_STATE_FAILED,
			Error: &axonpb.Error{
				Code:    string(ErrAdmissionDenied),
				Message: "caller rejected before admission",
				Stage:   stage,
			},
		})
		if err != nil {
			t.Fatalf("pre-admission stage %s: %v", stage, err)
		}
		result, err := NewInvocationResultFromJSON(raw)
		if err != nil {
			t.Fatalf("NewInvocationResultFromJSON(%s): %v; raw=%s", stage, err, raw)
		}
		if result.OK() || result.TerminalState() != "Failed" || result.TerminalReceipt() != nil {
			t.Fatalf(
				"pre-admission stage %s result = ok:%v state:%q receipt:%s",
				stage,
				result.OK(),
				result.TerminalState(),
				result.TerminalReceipt(),
			)
		}
	}

	for _, stage := range []axonpb.ErrorStage{
		axonpb.ErrorStage_ERROR_STAGE_UNSPECIFIED,
		axonpb.ErrorStage_ERROR_STAGE_TRANSPORT,
		axonpb.ErrorStage_ERROR_STAGE_EXECUTION,
	} {
		_, err := directInvokeResponseJSON(draft, &axonpb.InvokeResponse{
			State: axonpb.InvocationState_INVOCATION_STATE_FAILED,
			Error: &axonpb.Error{
				Code:  string(ErrAdmissionDenied),
				Stage: stage,
			},
		})
		if !IsCode(err, ErrProtocol) {
			t.Fatalf("receipt-free stage %s error = %v, want %s", stage, err, ErrProtocol)
		}
	}
}

func TestDirectAxonFailureProjectsMissingErrorCodeToProtocolMismatch(t *testing.T) {
	failure := directAxonFailure(&axonpb.Error{Message: "provider omitted code"}, "direct_runtime.invoke")
	if got := failure["code"]; got != string(ErrProtocolMismatch) {
		t.Fatalf("directAxonFailure missing code = %v, want %s", got, ErrProtocolMismatch)
	}
}

func TestDirectErrorStageUsesCanonicalProviderProjection(t *testing.T) {
	cases := map[axonpb.ErrorStage]string{
		axonpb.ErrorStage_ERROR_STAGE_GLOBAL_ADMISSION:      "global_admission",
		axonpb.ErrorStage_ERROR_STAGE_CALLER_AUTHENTICATION: "caller_authentication",
		axonpb.ErrorStage_ERROR_STAGE_UNSPECIFIED:           "unspecified",
		axonpb.ErrorStage(9999):                             "direct_runtime.invoke",
	}
	for input, want := range cases {
		if got := directErrorStage(input); got != want {
			t.Fatalf("directErrorStage(%v) = %q, want %q", input, got, want)
		}
	}
	if directPreAdmissionErrorStage(axonpb.ErrorStage_ERROR_STAGE_UNSPECIFIED) {
		t.Fatalf("unspecified stage must not be accepted as pre-admission")
	}
}

func TestDirectRuntimeUnaryRejectsReceiptFreeProofFailureAndPartialReceiptPairs(t *testing.T) {
	draft := directRuntimeDraft(t)
	_, err := directInvokeResponseJSON(draft, &axonpb.InvokeResponse{
		State: axonpb.InvocationState_INVOCATION_STATE_FAILED,
		ProofError: &axonpb.Error{
			Code:  string(ErrAdmissionDenied),
			Stage: axonpb.ErrorStage_ERROR_STAGE_AUTHORITY_VALIDATION,
		},
	})
	if !IsCode(err, ErrProtocol) {
		t.Fatalf("receipt-free proof error = %v, want %s", err, ErrProtocol)
	}

	admission := &axonpb.InvocationReceipt{
		Index:        1,
		InvocationId: "inv-partial",
		State:        axonpb.InvocationState_INVOCATION_STATE_ADMITTED,
	}
	terminal := &axonpb.InvocationReceipt{
		Index:        2,
		InvocationId: "inv-partial",
		State:        axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
	}
	for name, response := range map[string]*axonpb.InvokeResponse{
		"admission only": {
			State:            axonpb.InvocationState_INVOCATION_STATE_ADMITTED,
			AdmissionReceipt: admission,
		},
		"terminal only": {
			State:           axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
			TerminalReceipt: terminal,
		},
	} {
		if _, err := directInvokeResponseJSON(draft, response); !IsCode(err, ErrProtocol) {
			t.Fatalf("%s error = %v, want %s", name, err, ErrProtocol)
		}
	}
}

func TestDirectRuntimeStreamSeparatesCanonicalAndTransportTerminality(t *testing.T) {
	_, err := directStreamChunkJSON(&axonpb.InvokeStreamChunk{
		State:    axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		Terminal: true,
	})
	if !IsCode(err, ErrProtocol) {
		t.Fatalf("receipt-free stream terminal error = %v, want %s", err, ErrProtocol)
	}

	raw, err := directStreamChunkJSON(&axonpb.InvokeStreamChunk{
		State:    axonpb.InvocationState_INVOCATION_STATE_FAILED,
		Terminal: true,
		Error: &axonpb.Error{
			Code:    string(ErrRouteUnavailable),
			Message: "transport lost before admission",
		},
	})
	if err != nil {
		t.Fatalf("transport terminal projection: %v", err)
	}
	event, err := NewStreamEventFromJSON(raw)
	if err != nil {
		t.Fatalf("NewStreamEventFromJSON: %v; raw=%s", err, raw)
	}
	if event.Terminal() || !event.TransportTerminal() || event.Kind() != "error" {
		t.Fatalf(
			"transport failure flags = terminal:%v transport:%v kind:%q",
			event.Terminal(),
			event.TransportTerminal(),
			event.Kind(),
		)
	}
}

func TestDirectRuntimeBidiRejectsCallbackCarrierWithoutTerminalClaim(t *testing.T) {
	raw, err := directBidiDownJSON(&axonpb.InvokeBidiDown{
		Sequence: 1,
		Payload: &axonpb.InvokeBidiDown_DispatchCall{
			DispatchCall: &axonpb.DispatchCall{CallId: 1},
		},
	}, nil)
	if !IsCode(err, ErrProtocol) {
		t.Fatalf("callback frame error = %v, want %s", err, ErrProtocol)
	}
	if len(raw) != 0 {
		t.Fatalf("callback frame produced synthetic lifecycle JSON: %s", raw)
	}
}

func TestDirectRuntimeResponsesExposeOnlyCanonicalReceiptCheckpoints(t *testing.T) {
	admission := &axonpb.InvocationReceipt{
		Index:        1,
		InvocationId: "inv-codec",
		State:        axonpb.InvocationState_INVOCATION_STATE_ADMITTED,
	}
	terminal := &axonpb.InvocationReceipt{
		Index:        2,
		InvocationId: "inv-codec",
		State:        axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
	}
	unary, err := directInvokeResponseJSON(directRuntimeDraft(t), &axonpb.InvokeResponse{
		State:            axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		AdmissionReceipt: admission,
		TerminalReceipt:  terminal,
	})
	if err != nil {
		t.Fatalf("unary receipt projection: %v", err)
	}
	stream, err := directStreamChunkJSONWithAdmission(&axonpb.InvokeStreamChunk{
		State:           axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		Terminal:        true,
		TerminalReceipt: terminal,
	}, admission)
	if err != nil {
		t.Fatalf("stream receipt projection: %v", err)
	}
	bidi, err := directBidiDownJSON(&axonpb.InvokeBidiDown{
		Sequence: 1,
		Payload: &axonpb.InvokeBidiDown_Receipt{
			Receipt: terminal,
		},
	}, directReceipt(admission))
	if err != nil {
		t.Fatalf("bidi receipt projection: %v", err)
	}

	for mode, raw := range map[string][]byte{
		"unary":  unary,
		"stream": stream,
		"bidi":   bidi,
	} {
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(raw, &fields); err != nil {
			t.Fatalf("%s response JSON: %v; raw=%s", mode, err, raw)
		}
		if _, exists := fields["receipt"]; exists {
			t.Fatalf("%s response retained competing receipt alias: %s", mode, raw)
		}
		for _, checkpoint := range []string{"admission_receipt", "terminal_receipt"} {
			value, exists := fields[checkpoint]
			if !exists || string(value) == "null" {
				t.Fatalf("%s response omitted %s: %s", mode, checkpoint, raw)
			}
		}
	}
}

func TestDirectRuntimeRequestWireConstructionBelongsOnlyToCodec(t *testing.T) {
	requestTypes := map[string]struct{}{
		"Envelope":                  {},
		"InvokeRequest":             {},
		"InvokeServerStreamRequest": {},
		"EnvelopeOpen":              {},
	}
	for _, path := range []string{"direct_runtime.go", "direct_runtime_codec.go"} {
		file, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if err != nil {
			t.Fatalf("parse %s: %v", path, err)
		}
		ast.Inspect(file, func(node ast.Node) bool {
			literal, ok := node.(*ast.CompositeLit)
			if !ok {
				return true
			}
			selector, ok := literal.Type.(*ast.SelectorExpr)
			if !ok {
				return true
			}
			packageName, ok := selector.X.(*ast.Ident)
			if !ok || packageName.Name != "axonpb" {
				return true
			}
			if _, owned := requestTypes[selector.Sel.Name]; owned && path != "direct_runtime_codec.go" {
				t.Errorf("%s constructs axonpb.%s outside the descriptor-bound codec", path, selector.Sel.Name)
			}
			return true
		})
	}

	for _, path := range []string{"direct_runtime.go", "direct_runtime_codec.go"} {
		source, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("read %s: %v", path, err)
		}
		for _, retired := range []string{
			"directInvokeFields",
			"directInvokeRequestFromDraftJSON",
			"directStreamRequestFromDraftJSON",
			"directPreparedInvocationDraft",
			"directLocalAbilityName",
			"wireAbilityName",
			"bound.Ability()",
			"carrier-v1",
		} {
			if strings.Contains(string(source), retired) {
				t.Errorf("%s retains retired request path %q", path, retired)
			}
		}
	}

	codecSource, err := os.ReadFile("direct_runtime_codec.go")
	if err != nil {
		t.Fatalf("read direct_runtime_codec.go: %v", err)
	}
	for _, required := range []string{
		"NewDescriptorBoundWireProjectionBuilder",
		"BindCallerSignature",
		"projection.RouteName()",
		"projection.Envelope()",
	} {
		if !strings.Contains(string(codecSource), required) {
			t.Errorf("direct runtime codec does not delegate semantic projection through Axon marker %q", required)
		}
	}
	if strings.Contains(string(codecSource), "time.Now().UnixNano()") {
		t.Error("direct runtime codec still owns request-id generation")
	}
}

func directRuntimeSignedDraft(t *testing.T) InvocationDraft {
	t.Helper()
	raw, err := json.Marshal(map[string]any{
		"caller_ura":     "easynet:///r/example/agent/alice",
		"callee_ura":     "easynet:///r/example/device/dev-a",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
		"subject_ura":    "easynet:///r/example/device/dev-a",
		"nonce_base64":   "AQIDBAUGBwgJCgsMDQ4PEA==",
		"causal_context": map[string]any{
			"form":             "scalar",
			"receipt_ura":      "easynet:///r/example/resource/agent.alice/invocation/parent/receipt",
			"receipt_hash_hex": strings.Repeat("ab", 32),
		},
		"args":         map[string]any{"city": "Singapore"},
		"content_type": "application/json",
		"metadata": map[string]any{
			"timeout_ms": int64(1500),
			"trace_id":   "codec-test",
		},
		"caller_signature": map[string]any{
			"algorithm":        "ed25519",
			"signature_base64": base64.StdEncoding.EncodeToString([]byte(strings.Repeat("s", 64))),
			"key_id_hint":      "key-1",
		},
	})
	if err != nil {
		t.Fatalf("marshal signed draft: %v", err)
	}
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON: %v", err)
	}
	return draft
}

func assertDirectDescriptorBoundEnvelope(t *testing.T, envelope *axonpb.Envelope, draft InvocationDraft) {
	t.Helper()
	if envelope == nil {
		t.Fatal("wire envelope is nil")
	}
	if envelope.GetCaller().GetUra() != draft.CallerURA() ||
		envelope.GetCallee().GetUra() != draft.CalleeURA() ||
		envelope.GetSubject().GetUra() != draft.SubjectURA() {
		t.Fatalf(
			"wire tuple identities = caller:%q callee:%q subject:%q",
			envelope.GetCaller().GetUra(),
			envelope.GetCallee().GetUra(),
			envelope.GetSubject().GetUra(),
		)
	}
	nonce, err := base64.StdEncoding.DecodeString(draft.NonceBase64())
	if err != nil {
		t.Fatalf("decode nonce: %v", err)
	}
	if string(envelope.GetInvocationNonce()) != string(nonce) {
		t.Fatalf("wire nonce = %x, want %x", envelope.GetInvocationNonce(), nonce)
	}
	scalar := envelope.GetCausalContext().GetScalar()
	if scalar.GetReceiptUra() != draft.CausalContext()["receipt_ura"] ||
		string(scalar.GetReceiptHash()) != string([]byte(strings.Repeat("\xab", 32))) {
		t.Fatalf("wire causal context = %#v", scalar)
	}
	signature := draft.CallerSignature()
	wireSignature := envelope.GetCallerSignature()
	if signature == nil ||
		wireSignature.GetAlgorithm() != signature.Algorithm ||
		wireSignature.GetKeyIdHint() != signature.KeyIDHint {
		t.Fatalf("wire caller signature = %#v, draft=%#v", wireSignature, signature)
	}
}
