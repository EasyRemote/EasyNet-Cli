package easynet

import (
	"context"
	"encoding/json"
	"fmt"
)

const (
	compatibilityAbilityListModels      = "openai.list_models"
	compatibilityAbilityChatCompletions = "openai.chat_completions"
	compatibilityAbilityFileUpload      = "openai.files.upload"
	compatibilityAbilityFileRetrieve    = "openai.files.retrieve"
	compatibilityAbilityFileDelete      = "openai.files.delete"
)

var compatibilityCarrierArgKeys = map[string]struct{}{
	"caller_ura":         {},
	"callee_ura":         {},
	"subject_ura":        {},
	"descriptor_version": {},
	"nonce_base64":       {},
	"causal_context":     {},
	"metadata":           {},
}

// CompatibilityRuntimeTransport lowers Compatibility requests into daemon Runtime invocations.
type CompatibilityRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
}

func NewCompatibilityRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*CompatibilityRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(compatibilityProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(compatibilityProfile, "identity client is required")
	}
	return &CompatibilityRuntimeTransport{
		runtime:  runtime,
		identity: identity,
	}, nil
}

func NewRuntimeCompatibilityClient(runtime *RuntimeClient, identity *IdentityClient) (*CompatibilityClient, error) {
	transport, err := NewCompatibilityRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewCompatibilityClient(transport)
}

func (t *CompatibilityRuntimeTransport) BuildListModelsInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityListModelsForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, base, compatibilityAbilityListModels)
}

func (t *CompatibilityRuntimeTransport) BuildChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityChatCompletionForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, base, compatibilityAbilityChatCompletions)
}

func (t *CompatibilityRuntimeTransport) BuildStreamChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityStreamChatCompletionForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, base, compatibilityAbilityChatCompletions)
}

func (t *CompatibilityRuntimeTransport) BuildFileUploadInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityFileUploadForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, base, compatibilityAbilityFileUpload)
}

func (t *CompatibilityRuntimeTransport) BuildFileRetrieveInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityFileRetrieveForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, base, compatibilityAbilityFileRetrieve)
}

func (t *CompatibilityRuntimeTransport) BuildFileDeleteInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityFileDeleteForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, base, compatibilityAbilityFileDelete)
}

func (t *CompatibilityRuntimeTransport) ListModels(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityListModelsForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, base, compatibilityAbilityListModels)
}

func (t *CompatibilityRuntimeTransport) ChatCompletions(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityChatCompletionForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, base, compatibilityAbilityChatCompletions)
}

func (t *CompatibilityRuntimeTransport) StreamChatCompletions(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityStreamChatCompletionForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, base, compatibilityAbilityChatCompletions)
}

func (t *CompatibilityRuntimeTransport) UploadFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityFileUploadForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, base, compatibilityAbilityFileUpload)
}

func (t *CompatibilityRuntimeTransport) GetFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityFileRetrieveForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, base, compatibilityAbilityFileRetrieve)
}

func (t *CompatibilityRuntimeTransport) DeleteFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, err := decodeCompatibilityFileDeleteForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, base, compatibilityAbilityFileDelete)
}

func (t *CompatibilityRuntimeTransport) Close(ctx context.Context) error {
	return nil
}

func (t *CompatibilityRuntimeTransport) buildInvocationJSON(ctx context.Context, requestJSON []byte, base CompatibilityCarrierBase, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("encode compatibility invocation: %v", err), err)
	}
	return raw, nil
}

func (t *CompatibilityRuntimeTransport) buildInvocation(ctx context.Context, requestJSON []byte, base CompatibilityCarrierBase, abilityName string) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(compatibilityProfile, "compatibility runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(compatibilityProfile, "context is required")
	}
	payload, err := compatibilityRuntimePayload(requestJSON)
	if err != nil {
		return InvocationDraft{}, err
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, base.CalleeURA, abilityName, base.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(base.CallerURA).
		WithCalleeURA(base.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(base.SubjectURA).
		WithNonceBase64(base.NonceBase64).
		WithCausalContext(base.CausalContext).
		WithJSONArgs(compatibilityRuntimeArgs(payload)).
		WithContentType("application/json").
		WithMetadata(compatibilityRuntimeMetadata(payload, abilityName)).
		Build()
}

func (t *CompatibilityRuntimeTransport) invoke(ctx context.Context, requestJSON []byte, base CompatibilityCarrierBase, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, compatibilityInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(compatibilityProfile, "compatibility invocation output_json is required", nil)
	}
	return outputJSON, nil
}

func compatibilityRuntimePayload(requestJSON []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(requestJSON, &payload); err != nil {
		return nil, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility request: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(compatibilityProfile, "compatibility request must be an object", nil)
	}
	return payload, nil
}

func compatibilityRuntimeArgs(payload map[string]any) map[string]any {
	args := make(map[string]any, len(payload))
	for key, value := range payload {
		if _, carrier := compatibilityCarrierArgKeys[key]; carrier {
			continue
		}
		args[key] = value
	}
	return args
}

func compatibilityRuntimeMetadata(payload map[string]any, abilityName string) map[string]any {
	metadata := map[string]any{}
	if raw, ok := payload["metadata"].(map[string]any); ok {
		for key, value := range raw {
			metadata[key] = value
		}
	}
	metadata["profile"] = compatibilityProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func compatibilityInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "compatibility invocation failed"
	code := ErrAdmissionDenied
	stage := "runtime"
	retry := RetryNever
	details := map[string]any{
		"terminal_state": result.TerminalState(),
	}
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
			retry = RetrySafe
		}
		details["runtime_retryable"] = failure.Retryable()
	}
	return withProfileErrorDetails(&SDKError{
		Code:      code,
		Stage:     stage,
		Retry:     retry,
		Retryable: RetryableForHint(retry),
		Message:   message,
		Details:   details,
	}, compatibilityProfile)
}

func decodeCompatibilityListModelsForRuntime(requestJSON []byte) (CompatibilityCarrierBase, error) {
	var req CompatibilityListModelsRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return CompatibilityCarrierBase{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility list-models request: %v", err), err)
	}
	if err := validateCompatibilityListModelsRequest(req); err != nil {
		return CompatibilityCarrierBase{}, err
	}
	return req.CompatibilityCarrierBase, nil
}

func decodeCompatibilityChatCompletionForRuntime(requestJSON []byte) (CompatibilityCarrierBase, error) {
	var req CompatibilityChatCompletionRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return CompatibilityCarrierBase{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility chat-completion request: %v", err), err)
	}
	if err := validateCompatibilityChatCompletionRequest(req); err != nil {
		return CompatibilityCarrierBase{}, err
	}
	return req.CompatibilityCarrierBase, nil
}

func decodeCompatibilityStreamChatCompletionForRuntime(requestJSON []byte) (CompatibilityCarrierBase, error) {
	var req CompatibilityStreamChatCompletionRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return CompatibilityCarrierBase{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility stream-chat-completion request: %v", err), err)
	}
	if err := validateCompatibilityStreamChatCompletionRequest(req); err != nil {
		return CompatibilityCarrierBase{}, err
	}
	return req.CompatibilityCarrierBase, nil
}

func decodeCompatibilityFileUploadForRuntime(requestJSON []byte) (CompatibilityCarrierBase, error) {
	var req CompatibilityFileUploadRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return CompatibilityCarrierBase{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility file-upload request: %v", err), err)
	}
	if err := validateCompatibilityFileUploadCarrierRequest(req); err != nil {
		return CompatibilityCarrierBase{}, err
	}
	return req.CompatibilityCarrierBase, nil
}

func decodeCompatibilityFileRetrieveForRuntime(requestJSON []byte) (CompatibilityCarrierBase, error) {
	var req CompatibilityFileRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return CompatibilityCarrierBase{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility file-retrieve request: %v", err), err)
	}
	if err := validateCompatibilityFileCarrierRequest(req); err != nil {
		return CompatibilityCarrierBase{}, err
	}
	return req.CompatibilityCarrierBase, nil
}

func decodeCompatibilityFileDeleteForRuntime(requestJSON []byte) (CompatibilityCarrierBase, error) {
	var req CompatibilityFileDeleteRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return CompatibilityCarrierBase{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility file-delete request: %v", err), err)
	}
	if err := validateCompatibilityFileDeleteCarrierRequest(req); err != nil {
		return CompatibilityCarrierBase{}, err
	}
	return req.CompatibilityCarrierBase, nil
}
