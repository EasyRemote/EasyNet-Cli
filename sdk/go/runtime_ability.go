package easynet

import (
	"context"
	"encoding/json"
	"strings"
)

const defaultRuntimeAbilityDescriptorVersion = "1.0.0"

// RuntimeCallContext contains the complete caller-controlled Invocation
// context needed to address one runtime ability. The SDK never manufactures a
// caller, nonce or causal context on behalf of a product.
type RuntimeCallContext struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version,omitempty"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// RuntimeAbilityClient is the single generic lowering path from an addressed
// runtime ability call to Runtime Core. Typed capability providers compose it;
// products do not duplicate descriptor or subject projection.
type RuntimeAbilityClient struct {
	runtime    *RuntimeClient
	addressing Addressing
}

func NewRuntimeAbilityClient(runtime *RuntimeClient, addressing Addressing) (*RuntimeAbilityClient, error) {
	if runtime == nil {
		return nil, invalidRuntimeClient("runtime client is required")
	}
	if addressing == nil {
		return nil, invalidRuntimeClient("Addressing provider is required")
	}
	return &RuntimeAbilityClient{runtime: runtime, addressing: addressing}, nil
}

// Build constructs one complete canonical draft. Ability semantics and
// argument validation remain with the typed capability provider.
func (c *RuntimeAbilityClient) Build(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	if err := validateRuntimeCallContext(call); err != nil {
		return InvocationDraft{}, err
	}
	abilityName = strings.TrimSpace(abilityName)
	if abilityName == "" {
		return InvocationDraft{}, invalidRuntimePayload("ability name is required", nil)
	}
	version := strings.TrimSpace(call.DescriptorVersion)
	if version == "" {
		version = defaultRuntimeAbilityDescriptorVersion
	}
	descriptorRef, err := c.addressing.OwnerAbilityDescriptorRef(ctx, call.CalleeURA, abilityName, version)
	if err != nil {
		return InvocationDraft{}, err
	}
	subjectURA, err := descriptorBoundSubjectURA(ctx, c.addressing, call.SubjectURA, abilityName)
	if err != nil {
		return InvocationDraft{}, err
	}
	metadata := make(map[string]any, len(call.Metadata))
	for key, value := range call.Metadata {
		metadata[key] = value
	}
	return NewInvocationBuilder().
		WithCallerURA(strings.TrimSpace(call.CallerURA)).
		WithCalleeURA(strings.TrimSpace(call.CalleeURA)).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(subjectURA).
		WithNonceBase64(strings.TrimSpace(call.NonceBase64)).
		WithCausalContext(call.CausalContext).
		WithJSONArgs(args).
		WithContentType("application/json").
		WithMetadata(metadata).
		Build()
}

// Invoke executes one addressed ability and returns its object result without
// product-specific envelope or DTO projection.
func (c *RuntimeAbilityClient) Invoke(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (map[string]any, error) {
	draft, err := c.Build(ctx, call, abilityName, args)
	if err != nil {
		return nil, err
	}
	result, err := c.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, runtimeAbilityFailure(result)
	}
	raw := result.OutputJSON()
	if len(raw) == 0 || string(raw) == "null" {
		return nil, invalidRuntimePayload("runtime ability output_json is required", nil)
	}
	var output map[string]any
	if err := json.Unmarshal(raw, &output); err != nil {
		return nil, invalidRuntimePayload("runtime ability output_json must be an object", err)
	}
	if output == nil {
		return nil, invalidRuntimePayload("runtime ability output_json must be an object", nil)
	}
	return output, nil
}

// OpenStream opens one typed provider stream through the same canonical draft
// lowering path used by unary calls.
func (c *RuntimeAbilityClient) OpenStream(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (*StreamHandle, error) {
	draft, err := c.Build(ctx, call, abilityName, args)
	if err != nil {
		return nil, err
	}
	return c.runtime.InvokeStream(ctx, draft)
}

func (c *RuntimeAbilityClient) requireReady(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if c == nil || c.runtime == nil || c.addressing == nil {
		return invalidRuntimeClient("runtime ability client is not initialized")
	}
	return nil
}

func validateRuntimeCallContext(call RuntimeCallContext) error {
	for _, field := range []struct {
		name  string
		value string
	}{
		{name: "caller_ura", value: call.CallerURA},
		{name: "callee_ura", value: call.CalleeURA},
		{name: "subject_ura", value: call.SubjectURA},
		{name: "nonce_base64", value: call.NonceBase64},
	} {
		if strings.TrimSpace(field.value) == "" {
			return invalidRuntimePayload(field.name+" is required", nil)
		}
	}
	if call.CausalContext == nil {
		return invalidRuntimePayload("causal_context is required", nil)
	}
	return nil
}

func runtimeAbilityFailure(result InvocationResult) error {
	message := "runtime ability invocation failed"
	stage := "runtime"
	retry := RetryNever
	code := ErrExecutionFailed
	details := map[string]any{"terminal_state": result.TerminalState()}
	if failure := result.Failure(); failure != nil {
		if failure.Message() != "" {
			message = failure.Message()
		}
		if failure.Stage() != "" {
			stage = failure.Stage()
		}
		if failure.Code() != "" {
			details["runtime_code"] = failure.Code()
			if parsed, err := ParseErrorCode(failure.Code()); err == nil {
				code = parsed
			}
		}
		if failure.Retryable() {
			retry = RetrySafe
		}
	}
	return &SDKError{
		Code:      code,
		Stage:     stage,
		Retry:     retry,
		Retryable: retry == RetrySafe,
		Message:   message,
		Details:   details,
	}
}
