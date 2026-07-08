package easynet

import (
	"context"
	"encoding/json"
	"fmt"
)

const (
	accessControlAbilityAuthorityBindingGrant  = "authority.binding.grant"
	accessControlAbilityAuthorityBindingRevoke = "authority.binding.revoke"
	accessControlAbilityAuthorityBindingList   = "authority.binding.list"
	accessControlAbilityAuthorityBindingCheck  = "authority.binding.check"
	accessControlAbilityPolicyRequestCreate    = "policy.request.create"
	accessControlAbilityPolicyRequestResolve   = "policy.request.resolve"
	accessControlAbilityPolicyRequestList      = "policy.request.list"
	accessControlAbilityAdmissionExplain       = "admission.explain"
)

// AccessControlRuntimeTransport lowers access-control profile requests into
// daemon Runtime Core invocations. RFC-014 system ability names and invocation
// metadata are owned here so downstream products consume the typed SDK only.
type AccessControlRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
}

func NewAccessControlRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*AccessControlRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(accessControlProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(accessControlProfile, "identity client is required")
	}
	return &AccessControlRuntimeTransport{runtime: runtime, identity: identity}, nil
}

func NewRuntimeAccessControlClient(runtime *RuntimeClient, identity *IdentityClient) (*AccessControlClient, error) {
	transport, err := NewAccessControlRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewAccessControlClient(transport)
}

func (t *AccessControlRuntimeTransport) GrantAuthorityBinding(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityAuthorityBindingGrant)
}

func (t *AccessControlRuntimeTransport) RevokeAuthorityBinding(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityAuthorityBindingRevoke)
}

func (t *AccessControlRuntimeTransport) ListAuthorityBindings(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityAuthorityBindingList)
}

func (t *AccessControlRuntimeTransport) CheckAuthorityBinding(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityAuthorityBindingCheck)
}

func (t *AccessControlRuntimeTransport) CreatePolicyRequest(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityPolicyRequestCreate)
}

func (t *AccessControlRuntimeTransport) ResolvePolicyRequest(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityPolicyRequestResolve)
}

func (t *AccessControlRuntimeTransport) ListPolicyRequests(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityPolicyRequestList)
}

func (t *AccessControlRuntimeTransport) ExplainAdmission(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invoke(ctx, requestJSON, accessControlAbilityAdmissionExplain)
}

func (t *AccessControlRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *AccessControlRuntimeTransport) buildInvocationJSON(ctx context.Context, requestJSON []byte, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, abilityName)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(accessControlProfile, fmt.Sprintf("encode access-control invocation: %v", err), err)
	}
	return raw, nil
}

func (t *AccessControlRuntimeTransport) buildInvocation(ctx context.Context, requestJSON []byte, abilityName string) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(accessControlProfile, "access-control runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(accessControlProfile, "context is required")
	}
	carrier, args, err := accessControlRuntimeArgs(requestJSON)
	if err != nil {
		return InvocationDraft{}, err
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, carrier.CalleeURA, abilityName, carrier.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	subjectURA, err := descriptorBoundSubjectURA(ctx, t.identity, carrier.SubjectURA, abilityName)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(carrier.CallerURA).
		WithCalleeURA(carrier.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(subjectURA).
		WithNonceBase64(carrier.NonceBase64).
		WithCausalContext(carrier.CausalContext).
		WithJSONArgs(args).
		WithContentType("application/json").
		WithMetadata(accessControlRuntimeMetadata(carrier.Metadata, abilityName)).
		Build()
}

func (t *AccessControlRuntimeTransport) invoke(ctx context.Context, requestJSON []byte, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, abilityName)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, accessControlInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(accessControlProfile, "access-control output_json is required", nil)
	}
	return outputJSON, nil
}

func accessControlRuntimeArgs(requestJSON []byte) (AccessControlCarrierBase, map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(requestJSON, &payload); err != nil {
		return AccessControlCarrierBase{}, nil, invalidProfilePayload(accessControlProfile, fmt.Sprintf("decode access-control request: %v", err), err)
	}
	rawCarrier, ok := payload["carrier"]
	if !ok {
		return AccessControlCarrierBase{}, nil, invalidProfilePayload(accessControlProfile, "carrier is required for runtime access-control requests", nil)
	}
	carrierJSON, err := json.Marshal(rawCarrier)
	if err != nil {
		return AccessControlCarrierBase{}, nil, invalidProfilePayload(accessControlProfile, fmt.Sprintf("encode access-control carrier: %v", err), err)
	}
	var carrier AccessControlCarrierBase
	if err := json.Unmarshal(carrierJSON, &carrier); err != nil {
		return AccessControlCarrierBase{}, nil, invalidProfilePayload(accessControlProfile, fmt.Sprintf("decode access-control carrier: %v", err), err)
	}
	if err := validateAccessControlCarrierBase(carrier); err != nil {
		return AccessControlCarrierBase{}, nil, err
	}
	delete(payload, "carrier")
	return carrier, payload, nil
}

func validateAccessControlCarrierBase(base AccessControlCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidProfilePayload(accessControlProfile, "caller_ura, callee_ura, subject_ura, descriptor_version, nonce_base64, and causal_context are required", nil)
	}
	return nil
}

func accessControlRuntimeMetadata(input map[string]any, abilityName string) map[string]any {
	metadata := copyMap(input)
	if metadata == nil {
		metadata = map[string]any{}
	}
	metadata["profile"] = accessControlProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func accessControlInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "access-control invocation failed"
	code := ErrAdmissionDenied
	stage := "runtime"
	retry := RetryNever
	details := map[string]any{"terminal_state": result.TerminalState()}
	if failure != nil {
		if failure.Message() != "" {
			message = failure.Message()
		}
		if failure.Code() != "" {
			code = runtimeFailureCode(failure.Code(), ErrAdmissionDenied)
			details["runtime_code"] = failure.Code()
		}
		if failure.Stage() != "" {
			stage = failure.Stage()
		}
		if failure.Retryable() {
			retry = RetryAfterBackoff
		}
		details["runtime_retryable"] = failure.Retryable()
	}
	return &SDKError{
		Code:      code,
		Stage:     stage,
		Retry:     retry,
		Retryable: RetryableForHint(retry),
		Message:   message,
		Details:   profileErrorDetails(accessControlProfile, details),
	}
}
