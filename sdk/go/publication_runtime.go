package easynet

import (
	"context"
	"encoding/json"
	"fmt"
)

const (
	publicationAbilityDeploy      = "ability.deploy"
	publicationAbilityList        = "meta.list_abilities"
	publicationAbilityImplEnable  = "ability.impl.enable"
	publicationAbilityImplDisable = "ability.impl.disable"
	publicationAbilityUnpublish   = "ability.unpublish"
)

// PublicationRuntimeTransport lowers Publication profile carriers into Runtime
// Core invocations and projects daemon publication facts back into Publication
// DTOs.
type PublicationRuntimeTransport struct {
	runtime       *RuntimeClient
	identity      *IdentityClient
	localProvider PublicationLocalProvider
}

// PublicationLocalProvider supplies daemon-local implementation-resource
// operations to the Runtime-backed Publication facade. Runtime Core remains the
// execution path for ability publication carriers.
type PublicationLocalProvider interface {
	ValidatePackage(ctx context.Context, requestJSON []byte) ([]byte, error)
	InstallPlugin(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func NewPublicationRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*PublicationRuntimeTransport, error) {
	return NewPublicationRuntimeTransportWithLocalProvider(runtime, identity, nil)
}

func NewPublicationRuntimeTransportWithLocalProvider(runtime *RuntimeClient, identity *IdentityClient, localProvider PublicationLocalProvider) (*PublicationRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(publicationProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(publicationProfile, "identity client is required")
	}
	return &PublicationRuntimeTransport{
		runtime:       runtime,
		identity:      identity,
		localProvider: localProvider,
	}, nil
}

func NewRuntimePublicationClient(runtime *RuntimeClient, identity *IdentityClient) (*PublicationClient, error) {
	return NewRuntimePublicationClientWithLocalProvider(runtime, identity, nil)
}

func NewRuntimePublicationClientWithLocalProvider(runtime *RuntimeClient, identity *IdentityClient, localProvider PublicationLocalProvider) (*PublicationClient, error) {
	transport, err := NewPublicationRuntimeTransportWithLocalProvider(runtime, identity, localProvider)
	if err != nil {
		return nil, err
	}
	return NewPublicationClient(transport)
}

func (t *PublicationRuntimeTransport) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if t == nil || t.identity == nil {
		return nil, invalidProfileClient(publicationProfile, "publication runtime transport is not initialized")
	}
	var req LocalResourceRefRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication resource-ref request: %v", err), err)
	}
	ref, err := t.identity.BuildResourceRef(ctx, req)
	if err != nil {
		return nil, err
	}
	return json.Marshal(ref)
}

func (t *PublicationRuntimeTransport) ValidatePackage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	provider, err := t.requireLocalProvider(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := provider.ValidatePackage(ctx, requestJSON)
	if err != nil {
		return nil, err
	}
	if _, err := NewPackageValidationFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *PublicationRuntimeTransport) DeployAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationDeployForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, publicationDeployArgs(req), publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityDeploy)
	if err != nil {
		return nil, err
	}
	result, err := NewAbilityDeployResultFromJSON(output)
	if err != nil {
		return nil, err
	}
	return json.Marshal(result)
}

func (t *PublicationRuntimeTransport) BuildDeployInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationDeployForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, publicationDeployArgs(req), publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityDeploy)
}

func (t *PublicationRuntimeTransport) InstallPlugin(ctx context.Context, requestJSON []byte) ([]byte, error) {
	provider, err := t.requireLocalProvider(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := provider.InstallPlugin(ctx, requestJSON)
	if err != nil {
		return nil, err
	}
	if _, err := NewPluginInstallResultFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *PublicationRuntimeTransport) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationListForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, publicationListArgs(req), publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityList)
	if err != nil {
		return nil, err
	}
	page, err := NewPublishedAbilityPageFromJSON(output)
	if err != nil {
		return nil, err
	}
	return json.Marshal(page)
}

func (t *PublicationRuntimeTransport) ShowAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationShowForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	abilityURA, err := t.identity.AbilityURAFromDescriptorRef(ctx, string(req.DescriptorRef))
	if err != nil {
		return nil, err
	}
	args := map[string]any{"subject_ura": abilityURA}
	if req.OwnerURA != "" {
		args["agent_ura"] = req.OwnerURA
	}
	output, err := t.invoke(ctx, args, publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityList)
	if err != nil {
		return nil, err
	}
	ability, err := projectPublishedAbilityForRuntime(req, output)
	if err != nil {
		return nil, err
	}
	return json.Marshal(ability)
}

func (t *PublicationRuntimeTransport) EnableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationImplLifecycleForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, publicationImplLifecycleArgs(req), publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityImplEnable)
	if err != nil {
		return nil, err
	}
	record, err := projectAbilityImplLifecycleForRuntime(req, output, publicationAbilityImplEnable, "enabled")
	if err != nil {
		return nil, err
	}
	return json.Marshal(record)
}

func (t *PublicationRuntimeTransport) DisableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationImplLifecycleForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, publicationImplLifecycleArgs(req), publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityImplDisable)
	if err != nil {
		return nil, err
	}
	record, err := projectAbilityImplLifecycleForRuntime(req, output, publicationAbilityImplDisable, "disabled")
	if err != nil {
		return nil, err
	}
	return json.Marshal(record)
}

func (t *PublicationRuntimeTransport) BuildUnpublishInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationUnpublishForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, map[string]any{"ability_ura": req.AbilityURA}, publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityUnpublish)
}

func (t *PublicationRuntimeTransport) UnpublishAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	req, err := decodePublicationUnpublishForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, map[string]any{"ability_ura": req.AbilityURA}, publicationCarrier{
		CallerURA:         req.CallerURA,
		CalleeURA:         req.CalleeURA,
		SubjectURA:        req.SubjectURA,
		DescriptorVersion: req.DescriptorVersion,
		NonceBase64:       req.NonceBase64,
		CausalContext:     req.CausalContext,
		Metadata:          req.Metadata,
	}, publicationAbilityUnpublish)
	if err != nil {
		return nil, err
	}
	record, err := t.projectUnpublishResultForRuntime(ctx, req, output)
	if err != nil {
		return nil, err
	}
	return json.Marshal(record)
}

func (t *PublicationRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *PublicationRuntimeTransport) requireLocalProvider(ctx context.Context) (PublicationLocalProvider, error) {
	if ctx == nil {
		return nil, invalidProfileClient(publicationProfile, "context is required")
	}
	if t == nil || t.localProvider == nil {
		return nil, invalidProfileClient(publicationProfile, "publication runtime local provider is required")
	}
	return t.localProvider, nil
}

type publicationCarrier struct {
	CallerURA         string
	CalleeURA         string
	SubjectURA        string
	DescriptorVersion string
	NonceBase64       string
	CausalContext     map[string]any
	Metadata          map[string]any
}

func (t *PublicationRuntimeTransport) buildInvocationJSON(ctx context.Context, args map[string]any, carrier publicationCarrier, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, args, carrier, abilityName)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(publicationProfile, fmt.Sprintf("encode publication invocation: %v", err), err)
	}
	return raw, nil
}

func (t *PublicationRuntimeTransport) buildInvocation(ctx context.Context, args map[string]any, carrier publicationCarrier, abilityName string) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(publicationProfile, "publication runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(publicationProfile, "context is required")
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, carrier.CalleeURA, abilityName, carrier.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(carrier.CallerURA).
		WithCalleeURA(carrier.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(carrier.SubjectURA).
		WithNonceBase64(carrier.NonceBase64).
		WithCausalContext(carrier.CausalContext).
		WithJSONArgs(args).
		WithContentType("application/json").
		WithMetadata(publicationRuntimeMetadata(carrier.Metadata, abilityName)).
		Build()
}

func (t *PublicationRuntimeTransport) invoke(ctx context.Context, args map[string]any, carrier publicationCarrier, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, args, carrier, abilityName)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, publicationInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(publicationProfile, "publication invocation output_json is required", nil)
	}
	return outputJSON, nil
}

func decodePublicationDeployForRuntime(requestJSON []byte) (AbilityDeployRequest, error) {
	var req AbilityDeployRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return AbilityDeployRequest{}, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication deploy request: %v", err), err)
	}
	if _, err := marshalAbilityDeployRequest(req); err != nil {
		return AbilityDeployRequest{}, err
	}
	return req, nil
}

func decodePublicationListForRuntime(requestJSON []byte) (PublishedAbilityQuery, error) {
	var req PublishedAbilityQuery
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return PublishedAbilityQuery{}, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication list query: %v", err), err)
	}
	req = normalizePublishedAbilityQuery(req)
	if err := validatePublishedAbilityQuery(req); err != nil {
		return PublishedAbilityQuery{}, err
	}
	return req, nil
}

func decodePublicationShowForRuntime(requestJSON []byte) (ShowAbilityRequest, error) {
	var req ShowAbilityRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return ShowAbilityRequest{}, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication show request: %v", err), err)
	}
	if _, err := marshalShowAbilityRequest(req); err != nil {
		return ShowAbilityRequest{}, err
	}
	return req, nil
}

func decodePublicationUnpublishForRuntime(requestJSON []byte) (UnpublishAbilityRequest, error) {
	var req UnpublishAbilityRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return UnpublishAbilityRequest{}, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication unpublish request: %v", err), err)
	}
	if _, err := marshalUnpublishAbilityRequest(req); err != nil {
		return UnpublishAbilityRequest{}, err
	}
	return req, nil
}

func decodePublicationImplLifecycleForRuntime(requestJSON []byte) (AbilityImplLifecycleRequest, error) {
	var req AbilityImplLifecycleRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return AbilityImplLifecycleRequest{}, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication ability impl lifecycle request: %v", err), err)
	}
	if _, err := marshalAbilityImplLifecycleRequest(req); err != nil {
		return AbilityImplLifecycleRequest{}, err
	}
	return req, nil
}

func projectPublishedAbilityForRuntime(req ShowAbilityRequest, output []byte) (PublishedAbility, error) {
	if ability, err := NewPublishedAbilityFromJSON(output); err == nil {
		if publishedAbilityDescriptorRef(ability) == string(req.DescriptorRef) {
			return ability, nil
		}
		return PublishedAbility{}, invalidProfilePayload(publicationProfile, "publication show ability result does not match descriptor_ref", nil)
	}
	page, err := NewPublishedAbilityPageFromJSON(output)
	if err != nil {
		return PublishedAbility{}, err
	}
	for _, item := range page.Items {
		if publishedAbilityDescriptorRef(item) == string(req.DescriptorRef) {
			return item, nil
		}
	}
	return PublishedAbility{}, invalidProfilePayload(publicationProfile, "published ability was not found", nil)
}

func publishedAbilityDescriptorRef(ability PublishedAbility) string {
	if ability.Descriptor == nil {
		return ""
	}
	return firstStringFromMap(ability.Descriptor, "descriptor_ref", "descriptorRef")
}

func (t *PublicationRuntimeTransport) projectUnpublishResultForRuntime(ctx context.Context, req UnpublishAbilityRequest, output []byte) (PublicationRecord, error) {
	if record, err := newPublicationRecordFromJSON(output, "ability_unpublished"); err == nil {
		return record, nil
	}
	payload, err := publicationRuntimeOutputObject(output, "publication unpublish output")
	if err != nil {
		return PublicationRecord{}, err
	}
	if ok, hasOK := payload["ok"].(bool); hasOK && !ok {
		return PublicationRecord{}, invalidProfilePayload(publicationProfile, "publication unpublish output ok=false", nil)
	}
	abilityURA := firstNonEmpty(firstStringFromMap(payload, "ability_ura", "abilityUra"), req.AbilityURA)
	if abilityURA == "" {
		return PublicationRecord{}, invalidProfilePayload(publicationProfile, "publication unpublish output missing ability_ura", nil)
	}
	descriptorVersion := firstNonEmpty(firstStringFromMap(payload, "descriptor_version", "descriptorVersion"), req.DescriptorVersion)
	descriptorRef, err := t.identity.CanonicalAbilityDescriptorRef(ctx, abilityURA, descriptorVersion)
	if err != nil {
		return PublicationRecord{}, err
	}
	status := "unpublished"
	record := PublicationRecord{
		Profile:       publicationProfile,
		Kind:          "ability_unpublished",
		DescriptorRef: descriptorRef,
		OwnerURA:      firstStringFromMap(payload, "owner_ura", "ownerUra"),
		Status:        &status,
		Metadata: map[string]any{
			"profile":            publicationProfile,
			"source_ability":     publicationAbilityUnpublish,
			"ability_ura":        abilityURA,
			"descriptor_version": descriptorVersion,
			"raw_result":         payload,
		},
	}
	for _, key := range []string{"public_name", "removed_path", "content_hash"} {
		if value := firstStringFromMap(payload, key); value != "" {
			record.Metadata[key] = value
		}
	}
	return record, nil
}

func projectAbilityImplLifecycleForRuntime(req AbilityImplLifecycleRequest, output []byte, sourceAbility string, status string) (PublicationRecord, error) {
	expectedKind := "ability_impl_" + status
	if record, err := newPublicationRecordFromJSON(output, expectedKind); err == nil {
		return record, nil
	}
	payload, err := publicationRuntimeOutputObject(output, "publication ability impl lifecycle output")
	if err != nil {
		return PublicationRecord{}, err
	}
	if ok, hasOK := payload["ok"].(bool); hasOK && !ok {
		return PublicationRecord{}, invalidProfilePayload(publicationProfile, "publication ability impl lifecycle output ok=false", nil)
	}
	abilityURA := firstNonEmpty(firstStringFromMap(payload, "ability_ura", "abilityUra"), req.AbilityURA)
	implID := firstNonEmpty(firstStringFromMap(payload, "impl_id", "implId"), req.ImplID)
	if abilityURA == "" || implID == "" {
		return PublicationRecord{}, invalidProfilePayload(publicationProfile, "publication ability impl lifecycle output missing ability_ura or impl_id", nil)
	}
	record := PublicationRecord{
		Profile:     publicationProfile,
		Kind:        expectedKind,
		OwnerURA:    firstStringFromMap(payload, "owner_ura", "ownerUra"),
		ResourceRef: optionalStringPointer(firstStringFromMap(payload, "resource_ref", "resourceRef")),
		Status:      optionalStringPointer(status),
		Metadata: map[string]any{
			"profile":        publicationProfile,
			"source_ability": sourceAbility,
			"ability_ura":    abilityURA,
			"impl_id":        implID,
			"raw_result":     payload,
		},
	}
	return record, nil
}

func publicationRuntimeOutputObject(raw []byte, label string) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode %s: %v", label, err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(publicationProfile, label+" must be a JSON object", nil)
	}
	return payload, nil
}

func publicationDeployArgs(req AbilityDeployRequest) map[string]any {
	return map[string]any{
		"resource_ref": req.ResourceRef,
		"node_id":      req.NodeID,
	}
}

func publicationListArgs(req PublishedAbilityQuery) map[string]any {
	args := map[string]any{
		"limit": req.Limit,
	}
	if req.Cursor != "" {
		args["cursor"] = req.Cursor
	}
	if req.OwnerURA != "" {
		args["agent_ura"] = req.OwnerURA
	}
	if req.AbilityURA != "" {
		args["subject_ura"] = req.AbilityURA
	}
	return args
}

func publicationImplLifecycleArgs(req AbilityImplLifecycleRequest) map[string]any {
	return map[string]any{
		"impl_id":     req.ImplID,
		"ability_ura": req.AbilityURA,
	}
}

func optionalStringPointer(value string) *string {
	if value == "" {
		return nil
	}
	return &value
}

func publicationRuntimeMetadata(metadata map[string]any, abilityName string) map[string]any {
	out := make(map[string]any, len(metadata)+3)
	for key, value := range metadata {
		out[key] = value
	}
	out["profile"] = publicationProfile
	out["system_ability"] = abilityName
	out["carrier_owner"] = "daemon_sdk"
	return out
}

func publicationInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "publication invocation failed"
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
			code = runtimeFailureCode(failure.Code(), ErrAdmissionDenied)
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
	}, publicationProfile)
}
