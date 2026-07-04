package easynet

import (
	"context"
	"encoding/json"
	"fmt"
)

// ReceiptRuntimeTransport lowers Receipt profile read requests into daemon
// Runtime Core invocations and projects daemon ledger facts back unchanged.
type ReceiptRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
}

func NewReceiptRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*ReceiptRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(receiptProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(receiptProfile, "identity client is required")
	}
	return &ReceiptRuntimeTransport{runtime: runtime, identity: identity}, nil
}

func NewRuntimeReceiptClient(runtime *RuntimeClient, identity *IdentityClient) (*ReceiptClient, error) {
	transport, err := NewReceiptRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewReceiptClient(transport)
}

func (t *ReceiptRuntimeTransport) Fetch(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var request ReceiptFetchRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt fetch request: %v", err), err)
	}
	draft, err := BuildReceiptFetchInvocation(request)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, receiptInvocationFailureError(result)
	}
	return receiptFetchSummaryJSON(request, result)
}

func (t *ReceiptRuntimeTransport) BuildListHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildHistoryInvocationJSON(ctx, requestJSON, receiptHistoryListAbility)
}

func (t *ReceiptRuntimeTransport) BuildGetHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildHistoryInvocationJSON(ctx, requestJSON, receiptHistoryGetAbility)
}

func (t *ReceiptRuntimeTransport) BuildTraceInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildHistoryInvocationJSON(ctx, requestJSON, receiptTraceGetAbility)
}

func (t *ReceiptRuntimeTransport) ListHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeHistory(ctx, requestJSON, receiptHistoryListAbility)
}

func (t *ReceiptRuntimeTransport) GetHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeHistory(ctx, requestJSON, receiptHistoryGetAbility)
}

func (t *ReceiptRuntimeTransport) GetTrace(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeHistory(ctx, requestJSON, receiptTraceGetAbility)
}

func (t *ReceiptRuntimeTransport) Project(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(receiptProfile, "receipt projection is not implemented by the runtime transport")
}

func (t *ReceiptRuntimeTransport) Verify(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(receiptProfile, "receipt verification is daemon/Axon-owned and not implemented by the runtime transport")
}

func (t *ReceiptRuntimeTransport) VerifyChain(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(receiptProfile, "receipt-chain verification is daemon/Axon-owned and not implemented by the runtime transport")
}

func (t *ReceiptRuntimeTransport) CausalRef(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(receiptProfile, "receipt causal-ref projection is not implemented by the runtime transport")
}

func (t *ReceiptRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *ReceiptRuntimeTransport) buildHistoryInvocationJSON(ctx context.Context, requestJSON []byte, abilityName string) ([]byte, error) {
	draft, err := t.buildHistoryInvocation(ctx, requestJSON, abilityName)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(receiptProfile, fmt.Sprintf("encode receipt invocation: %v", err), err)
	}
	return raw, nil
}

func (t *ReceiptRuntimeTransport) buildHistoryInvocation(ctx context.Context, requestJSON []byte, abilityName string) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(receiptProfile, "receipt runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(receiptProfile, "context is required")
	}
	request, err := decodeReceiptHistoryReadForRuntime(requestJSON)
	if err != nil {
		return InvocationDraft{}, err
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, request.CalleeURA, abilityName, request.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(request.CallerURA).
		WithCalleeURA(request.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(request.SubjectURA).
		WithNonceBase64(request.NonceBase64).
		WithCausalContext(request.CausalContext).
		WithJSONArgs(copyMap(request.Arguments)).
		WithContentType("application/json").
		WithMetadata(receiptRuntimeMetadata(request.ReceiptCarrierBase, abilityName)).
		Build()
}

func (t *ReceiptRuntimeTransport) invokeHistory(ctx context.Context, requestJSON []byte, abilityName string) ([]byte, error) {
	draft, err := t.buildHistoryInvocation(ctx, requestJSON, abilityName)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, receiptInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(receiptProfile, "receipt history output_json is required", nil)
	}
	return outputJSON, nil
}

func decodeReceiptHistoryReadForRuntime(requestJSON []byte) (ReceiptHistoryReadRequest, error) {
	var req ReceiptHistoryReadRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return ReceiptHistoryReadRequest{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt history request: %v", err), err)
	}
	if _, err := marshalReceiptHistoryReadRequest(req); err != nil {
		return ReceiptHistoryReadRequest{}, err
	}
	return req, nil
}

func receiptRuntimeMetadata(base ReceiptCarrierBase, abilityName string) map[string]any {
	metadata := copyMap(base.Metadata)
	if metadata == nil {
		metadata = map[string]any{}
	}
	metadata["profile"] = receiptProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	if base.TimeoutMS > 0 {
		metadata["timeout_ms"] = base.TimeoutMS
	}
	return metadata
}

func receiptFetchSummaryJSON(request ReceiptFetchRequest, result InvocationResult) ([]byte, error) {
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(receiptProfile, "receipt fetch output_json is required", nil)
	}
	var output any
	if err := json.Unmarshal(outputJSON, &output); err != nil {
		return nil, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt fetch output: %v", err), err)
	}
	invocationID := request.RequestID
	if invocationID == "" {
		invocationID = request.InvocationURA
	}
	if invocationID == "" {
		invocationID = request.TraceID
	}
	metadata := map[string]any{
		"summary_source":   "daemon_invocation_history",
		"selected_node_id": result.SelectedNodeID(),
	}
	return json.Marshal(map[string]any{
		"receipt_ura":   nil,
		"invocation_id": invocationID,
		"state":         result.TerminalState(),
		"verified":      false,
		"output":        output,
		"error":         nil,
		"causal_ref":    nil,
		"metadata":      metadata,
	})
}

func receiptInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "receipt invocation failed"
	code := ErrAbilityFailed
	stage := "runtime"
	retry := RetryNever
	details := map[string]any{"terminal_state": result.TerminalState()}
	if failure != nil {
		if failure.Message() != "" {
			message = failure.Message()
		}
		if failure.Code() != "" {
			code = NormalizeErrorCode(failure.Code())
			details["runtime_code"] = failure.Code()
		}
		if failure.Stage() != "" {
			stage = failure.Stage()
		}
		if failure.Retryable() {
			retry = RetryAfterBackoff
		}
	}
	return &SDKError{
		Code:      code,
		Stage:     stage,
		Retry:     retry,
		Retryable: RetryableForHint(retry),
		Message:   message,
		Details:   details,
	}
}
