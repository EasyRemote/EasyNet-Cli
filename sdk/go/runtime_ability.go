package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

// RuntimeCallContext contains the complete caller-controlled Invocation
// context needed to address one runtime ability. The SDK never manufactures a
// caller, nonce or causal context on behalf of a product.
type RuntimeCallContext struct {
	CallerURA         string `json:"caller_ura"`
	CalleeURA         string `json:"callee_ura"`
	SubjectURA        string `json:"subject_ura"`
	DescriptorVersion string `json:"descriptor_version,omitempty"`
	// SelectedDescriptorRef carries a resolver-selected descriptor identity
	// for callees that are not descriptor-resolvable from this runtime's
	// committed catalog (e.g. Service-owned abilities placed on a remote
	// device, where the product boundary already resolved an executable
	// route). When set, RuntimeAbilityClient binds it verbatim instead of
	// calling ResolveDescriptorRef; canonical projection and authority
	// validation still apply. It cannot override a governance descriptor
	// provider.
	SelectedDescriptorRef string         `json:"selected_descriptor_ref,omitempty"`
	NonceBase64           string         `json:"nonce_base64"`
	CausalContext         map[string]any `json:"causal_context"`
	Metadata              map[string]any `json:"metadata,omitempty"`
	// Authority is the typed authority half of this complete Invocation
	// transaction. RuntimeAbilityClient binds it to the projected tuple and
	// materializes its metadata atomically; products must not lower it by hand.
	Authority RuntimeInvocationAuthority `json:"-"`
}

// RuntimeInvocationAuthority is the closed SDK authority union accepted by a
// complete runtime call. Only canonical DelegationProof and SessionAuthority
// projections implement it.
type RuntimeInvocationAuthority interface {
	Metadata() (AuthorityMetadata, error)
	runtimeInvocationAuthority()
}

// RuntimeAbilityClient is the single generic lowering path from an addressed
// runtime ability call to Runtime Core. Typed capability providers compose it;
// products do not duplicate descriptor or subject projection.
type RuntimeAbilityClient struct {
	runtime    *RuntimeClient
	addressing Addressing
}

type runtimeAbilityDispatchPolicy struct {
	allowGovernanceRead bool
	subjectPolicy       runtimeAbilitySubjectPolicy
	descriptorProvider  string
}

func runtimeAbilityPublicPolicy() runtimeAbilityDispatchPolicy {
	return runtimeAbilityDispatchPolicy{subjectPolicy: runtimeAbilitySubjectDescriptorBound}
}

func runtimeAbilityGovernanceReadPolicy() runtimeAbilityDispatchPolicy {
	return runtimeAbilityDispatchPolicy{
		allowGovernanceRead: true,
		subjectPolicy:       runtimeAbilitySubjectDescriptorBound,
		descriptorProvider:  runtimeReceiptHistoryProvider,
	}
}

func runtimeAbilityCatalogueReadPolicy() runtimeAbilityDispatchPolicy {
	return runtimeAbilityDispatchPolicy{
		allowGovernanceRead: true,
		subjectPolicy:       runtimeAbilitySubjectRuntimeGovernanceRead,
		descriptorProvider:  runtimeAbilityDescriptorProvider,
	}
}

type runtimeAbilitySubjectPolicy int

const (
	runtimeAbilitySubjectDescriptorBound runtimeAbilitySubjectPolicy = iota
	runtimeAbilitySubjectRuntimeOwner
	runtimeAbilitySubjectRuntimeGovernanceRead
)

func (policy runtimeAbilityDispatchPolicy) subjectURA(ctx context.Context, addressing Addressing, call RuntimeCallContext, abilityName string) (string, error) {
	switch policy.subjectPolicy {
	case runtimeAbilitySubjectDescriptorBound:
		return descriptorBoundSubjectURA(ctx, addressing, call.SubjectURA, abilityName)
	case runtimeAbilitySubjectRuntimeOwner:
		return strings.TrimSpace(call.CalleeURA), nil
	case runtimeAbilitySubjectRuntimeGovernanceRead:
		if authority, err := runtimeSessionAuthorityFromCall(call); err != nil {
			return "", err
		} else if authority != nil {
			parts, err := ParseURAParts(strings.TrimSpace(call.SubjectURA))
			if err != nil {
				return "", invalidInvocation("runtime governance read subject_ura must be canonical", err)
			}
			return RuntimeStateReadSubjectURA(parts.Realm, authority.SessionOwnerUserID)
		}
		return runtimeGovernanceReadSubjectURA(call.SubjectURA, call.CalleeURA)
	default:
		return "", invalidRuntimePayload("runtime ability subject policy is unsupported", nil)
	}
}

func runtimeSessionAuthorityFromCall(call RuntimeCallContext) (*SessionAuthority, error) {
	switch typed := call.Authority.(type) {
	case SessionAuthority:
		return &typed, nil
	case *SessionAuthority:
		return typed, nil
	case nil:
	default:
		return nil, nil
	}
	session, err := authorityMetadataValue(cloneAbilityMetadata(call.Metadata), SessionAuthorityMetadataKey)
	if err != nil {
		return nil, err
	}
	if strings.TrimSpace(session) == "" {
		return nil, nil
	}
	authority, err := NewSessionAuthorityFromMetadata(session)
	if err != nil {
		return nil, err
	}
	return &authority, nil
}

func (policy runtimeAbilityDispatchPolicy) descriptorResolutionSubjectURA(call RuntimeCallContext, selectedSubjectURA string) (string, error) {
	if strings.TrimSpace(policy.descriptorProvider) == runtimeAbilityDescriptorProvider {
		if policy.subjectPolicy == runtimeAbilitySubjectRuntimeOwner ||
			policy.subjectPolicy == runtimeAbilitySubjectRuntimeGovernanceRead {
			return strings.TrimSpace(selectedSubjectURA), nil
		}
		return strings.TrimSpace(call.SubjectURA), nil
	}
	if policy.subjectPolicy == runtimeAbilitySubjectRuntimeOwner {
		return strings.TrimSpace(selectedSubjectURA), nil
	}
	return strings.TrimSpace(call.SubjectURA), nil
}

// AbilityChildContext carries the complete child Invocation context derived
// from a parent terminal receipt.
type AbilityChildContext struct {
	client   *RuntimeAbilityClient
	call     RuntimeCallContext
	baseMeta map[string]any
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
	return c.buildWithCallMode(ctx, call, abilityName, args, "rpc")
}

// ChildContext derives a scalar causal child context from a parent terminal
// receipt. The returned context can only dispatch through this client's
// descriptor-bound RuntimeAbilityClient path.
func (c *RuntimeAbilityClient) ChildContext(parent InvocationResult, callerURA string, nonceBase64 string, metadata map[string]any) (AbilityChildContext, error) {
	if c == nil || c.runtime == nil || c.addressing == nil {
		return AbilityChildContext{}, invalidRuntimeClient("runtime ability client is not initialized")
	}
	receipt := parent.TerminalReceiptSummary()
	if receipt == nil || !receipt.HasCausalAnchor() {
		return AbilityChildContext{}, invalidRuntimePayload("parent result is missing a causal receipt anchor", nil)
	}
	reference, err := ReceiptReferenceFromRuntimeReceipt(*receipt)
	if err != nil {
		return AbilityChildContext{}, err
	}
	causalContext, err := reference.CausalContext()
	if err != nil {
		return AbilityChildContext{}, err
	}
	caller := strings.TrimSpace(callerURA)
	if caller == "" {
		return AbilityChildContext{}, invalidRuntimePayload("caller_ura is required", nil)
	}
	nonce := strings.TrimSpace(nonceBase64)
	if nonce == "" {
		return AbilityChildContext{}, invalidRuntimePayload("nonce_base64 is required", nil)
	}
	return AbilityChildContext{
		client: c,
		call: RuntimeCallContext{
			CallerURA:     caller,
			NonceBase64:   nonce,
			CausalContext: causalContext,
		},
		baseMeta: cloneAbilityMetadata(metadata),
	}, nil
}

// Build constructs a child Invocation draft using the inherited scalar causal
// context and the supplied child routing facts.
func (c AbilityChildContext) Build(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (InvocationDraft, error) {
	childCall, err := c.childCall(call)
	if err != nil {
		return InvocationDraft{}, err
	}
	return c.client.Build(ctx, childCall, abilityName, args)
}

// Invoke dispatches one child ability Invocation through Runtime Core.
func (c AbilityChildContext) Invoke(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (map[string]any, error) {
	childCall, err := c.childCall(call)
	if err != nil {
		return nil, err
	}
	return c.client.Invoke(ctx, childCall, abilityName, args)
}

// OpenStream opens one child server-stream Invocation through Runtime Core.
func (c AbilityChildContext) OpenStream(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (*StreamHandle, error) {
	childCall, err := c.childCall(call)
	if err != nil {
		return nil, err
	}
	return c.client.OpenStream(ctx, childCall, abilityName, args)
}

// OpenBidi opens one child bidirectional Invocation through Runtime Core.
func (c AbilityChildContext) OpenBidi(ctx context.Context, call RuntimeCallContext, abilityName string, args any, streams []BidiStreamDescriptor) (*BidiSession, error) {
	childCall, err := c.childCall(call)
	if err != nil {
		return nil, err
	}
	return c.client.OpenBidi(ctx, childCall, abilityName, args, streams)
}

func (c *RuntimeAbilityClient) buildWithCallMode(ctx context.Context, call RuntimeCallContext, abilityName string, args any, callMode string) (InvocationDraft, error) {
	return c.buildWithCallModePolicy(ctx, call, abilityName, args, callMode, runtimeAbilityPublicPolicy())
}

func (c *RuntimeAbilityClient) buildGovernanceRead(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (InvocationDraft, error) {
	return c.buildWithCallModePolicy(ctx, call, abilityName, args, "rpc", runtimeAbilityGovernanceReadPolicy())
}

func (c *RuntimeAbilityClient) buildCatalogueRead(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (InvocationDraft, error) {
	return c.buildWithCallModePolicy(ctx, call, abilityName, args, "rpc", runtimeAbilityCatalogueReadPolicy())
}

func (c *RuntimeAbilityClient) buildWithCallModePolicy(ctx context.Context, call RuntimeCallContext, abilityName string, args any, callMode string, policy runtimeAbilityDispatchPolicy) (InvocationDraft, error) {
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
	if !policy.allowGovernanceRead && isRuntimeGovernanceReadAbility(abilityName) {
		return InvocationDraft{}, invalidRuntimePayload(
			"runtime governance receipt/history/catalogue abilities must use RuntimeReceiptProvider or RuntimeAbilityDescriptorProvider",
			nil,
		)
	}
	mode := strings.TrimSpace(callMode)
	if mode == "" {
		return InvocationDraft{}, invalidRuntimePayload("call_mode is required", nil)
	}
	target, err := newRuntimeCatalogueReadTarget(
		strings.TrimSpace(call.CalleeURA),
		strings.TrimSpace(call.SubjectURA),
		abilityName,
		policy.descriptorProvider,
	)
	if err != nil {
		return InvocationDraft{}, err
	}
	call.CalleeURA = target.calleeURA
	call.SubjectURA = target.subjectURA
	subjectURA, err := policy.subjectURA(ctx, c.addressing, call, abilityName)
	if err != nil {
		return InvocationDraft{}, err
	}
	descriptorSubjectURA, err := policy.descriptorResolutionSubjectURA(call, subjectURA)
	if err != nil {
		return InvocationDraft{}, err
	}
	metadata, authority, err := projectRuntimeCallMetadata(call)
	if err != nil {
		return InvocationDraft{}, err
	}
	descriptorRef := strings.TrimSpace(call.SelectedDescriptorRef)
	if descriptorRef != "" && strings.TrimSpace(policy.descriptorProvider) != "" {
		return InvocationDraft{}, invalidRuntimePayload(
			"selected descriptor_ref cannot override a governance descriptor provider",
			nil,
		)
	}
	if descriptorRef == "" {
		descriptorRef, err = c.runtime.ResolveDescriptorRef(ctx, RuntimeDescriptorRefRequest{
			CalleeURA:         target.calleeURA,
			Ability:           abilityName,
			CallMode:          mode,
			CallerURA:         strings.TrimSpace(call.CallerURA),
			SubjectURA:        descriptorSubjectURA,
			AuthorityMetadata: descriptorResolutionAuthorityMetadata(metadata),
			Provider:          policy.descriptorProvider,
		})
	}
	if err != nil {
		return InvocationDraft{}, err
	}
	ability, err := newRuntimeAbilityProjection(ctx, c.addressing, target.calleeURA, descriptorRef)
	if err != nil {
		return InvocationDraft{}, err
	}
	if authority != nil {
		if err := validateRuntimeAuthorityBinding(authority, call, subjectURA, ability); err != nil {
			return InvocationDraft{}, err
		}
	}
	if err := validateAuthorityMetadata(metadata); err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(strings.TrimSpace(call.CallerURA)).
		WithCalleeURA(target.calleeURA).
		WithDescriptorRef(ability.descriptorRef).
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
	return runtimeAbilityObjectOutput(result)
}

func (c *RuntimeAbilityClient) invokeGovernanceRead(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (map[string]any, error) {
	draft, err := c.buildGovernanceRead(ctx, call, abilityName, args)
	if err != nil {
		return nil, err
	}
	result, err := c.runtime.governanceRead(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, runtimeAbilityFailure(result)
	}
	return runtimeAbilityObjectOutput(result)
}

func (c *RuntimeAbilityClient) invokeCatalogueRead(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (map[string]any, error) {
	draft, err := c.buildCatalogueRead(ctx, call, abilityName, args)
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
	return runtimeAbilityObjectOutput(result)
}

func runtimeAbilityObjectOutput(result InvocationResult) (map[string]any, error) {
	raw := result.OutputJSON()
	if len(raw) == 0 || string(raw) == "null" {
		return nil, invalidRuntimePayload(
			fmt.Sprintf(
				"runtime ability output_json is required (output_content_type=%q, output_base64_len=%d)",
				result.OutputContentType(),
				len(strings.TrimSpace(result.OutputBase64())),
			),
			nil,
		)
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
	draft, err := c.buildWithCallMode(ctx, call, abilityName, args, "stream")
	if err != nil {
		return nil, err
	}
	return c.runtime.InvokeStream(ctx, draft)
}

// OpenLeasedStream opens an ABI v9 payload-lease stream through the same
// canonical draft lowering path used by other ability calls.
func (c *RuntimeAbilityClient) OpenLeasedStream(ctx context.Context, call RuntimeCallContext, abilityName string, args any) (*LeasedStreamHandle, error) {
	draft, err := c.buildWithCallMode(ctx, call, abilityName, args, "stream")
	if err != nil {
		return nil, err
	}
	return c.runtime.InvokeLeasedStream(ctx, draft)
}

// OpenBidi opens one typed provider bidirectional session through the same
// canonical draft lowering path used by unary and stream calls.
func (c *RuntimeAbilityClient) OpenBidi(ctx context.Context, call RuntimeCallContext, abilityName string, args any, streams []BidiStreamDescriptor) (*BidiSession, error) {
	draft, err := c.buildWithCallMode(ctx, call, abilityName, args, "bidi")
	if err != nil {
		return nil, err
	}
	return c.runtime.OpenBidi(ctx, draft, streams)
}

// SubmitSigned submits a signed ability Invocation through Runtime Core.
func (c *RuntimeAbilityClient) SubmitSigned(ctx context.Context, signed SignedInvocation) (InvocationHandle, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationHandle{}, err
	}
	return c.runtime.SubmitSigned(ctx, signed)
}

// Recover delegates restart recovery to Runtime Core.
func (c *RuntimeAbilityClient) Recover(ctx context.Context, request RuntimeRecoveryRequest) (RuntimeRecoveryReport, error) {
	if err := c.requireReady(ctx); err != nil {
		return RuntimeRecoveryReport{}, err
	}
	return c.runtime.Recover(ctx, request)
}

// Await waits for an ability Invocation handle through Runtime Core.
func (c *RuntimeAbilityClient) Await(ctx context.Context, handle InvocationHandle) (InvocationResult, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationResult{}, err
	}
	return c.runtime.Await(ctx, handle)
}

// Cancel requests ability Invocation cancellation through Runtime Core.
func (c *RuntimeAbilityClient) Cancel(ctx context.Context, handle InvocationHandle, reason string) (InvocationCancel, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationCancel{}, err
	}
	return c.runtime.Cancel(ctx, handle, reason)
}

// Events returns the current ability Invocation handle snapshot.
func (c *RuntimeAbilityClient) Events(ctx context.Context, handle InvocationHandle) (InvocationHandle, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationHandle{}, err
	}
	return c.runtime.Events(ctx, handle)
}

// CloseHandle releases an ability Invocation handle.
func (c *RuntimeAbilityClient) CloseHandle(ctx context.Context, handle InvocationHandle) error {
	if err := c.requireReady(ctx); err != nil {
		return err
	}
	return c.runtime.CloseHandle(ctx, handle)
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
		if field.name != "nonce_base64" && containsAllZeroPrincipal(field.value) {
			return invalidRuntimePayload(field.name+" must not be all-zero", nil)
		}
	}
	if err := validateInvocationNonceBase64(call.NonceBase64); err != nil {
		return err
	}
	if call.CausalContext == nil {
		return invalidRuntimePayload("causal_context is required", nil)
	}
	if _, err := CausalContextFromJSON(call.CausalContext); err != nil {
		return err
	}
	return nil
}

type runtimeAbilityProjection struct {
	descriptorRef string
	abilityURA    string
	publicName    string
	action        string
}

func newRuntimeAbilityProjection(
	ctx context.Context,
	addressing Addressing,
	calleeURA string,
	descriptorRef string,
) (runtimeAbilityProjection, error) {
	projection, err := addressing.ProjectDescriptorRef(
		ctx,
		CanonicalDescriptorRefRequest{DescriptorRef: descriptorRef},
	)
	if err != nil {
		return runtimeAbilityProjection{}, invalidRuntimePayload(
			"descriptor_ref must contain a canonical Ability URA",
			err,
		)
	}
	abilityURA := strings.TrimSpace(projection.AbilityURA)
	canonicalRef := strings.TrimSpace(projection.DescriptorRef)
	action := strings.TrimSpace(projection.Action)
	if abilityURA == "" || canonicalRef == "" || action == "" {
		return runtimeAbilityProjection{}, invalidRuntimePayload(
			"descriptor_ref must contain a canonical Ability URA and admission action",
			nil,
		)
	}
	parts, err := ParseURAParts(abilityURA)
	if err != nil || parts.Kind != URAKindAbility {
		return runtimeAbilityProjection{}, invalidRuntimePayload(
			"descriptor_ref must contain a canonical Ability URA",
			err,
		)
	}
	wire := strings.TrimSpace(parts.AbilityID)
	publicName := ""
	if ownerBoundName, ok := PublicAbilityNameFromAbilityURA(strings.TrimSpace(calleeURA), abilityURA); ok {
		publicName = strings.TrimSpace(ownerBoundName)
		if publicName == "" {
			publicName = wire
		}
	}
	return runtimeAbilityProjection{
		descriptorRef: canonicalRef,
		abilityURA:    abilityURA,
		publicName:    strings.TrimSpace(publicName),
		action:        action,
	}, nil
}

func (p runtimeAbilityProjection) matchesScope(match func(string) bool) bool {
	seen := map[string]struct{}{}
	for _, candidate := range []string{p.publicName, p.abilityURA} {
		candidate = strings.TrimSpace(candidate)
		if candidate == "" {
			continue
		}
		if _, ok := seen[candidate]; ok {
			continue
		}
		seen[candidate] = struct{}{}
		if match(candidate) {
			return true
		}
	}
	return false
}

func projectRuntimeCallMetadata(call RuntimeCallContext) (map[string]any, RuntimeInvocationAuthority, error) {
	metadata := cloneAbilityMetadata(call.Metadata)
	if err := validateAuthorityMetadata(metadata); err != nil {
		return nil, nil, err
	}
	if call.Authority != nil {
		if rawRuntimeAuthorityPresent(metadata) {
			return nil, nil, invalidRuntimePayload(
				"runtime call authority must be supplied once as a typed authority or metadata, not both",
				nil,
			)
		}
		projection, err := call.Authority.Metadata()
		if err != nil {
			return nil, nil, err
		}
		metadata, err = projection.MergeInto(metadata)
		if err != nil {
			return nil, nil, err
		}
		return metadata, call.Authority, nil
	}

	authority, err := runtimeAuthorityFromMetadata(metadata)
	if err != nil {
		return nil, nil, err
	}
	return metadata, authority, nil
}

func descriptorResolutionAuthorityMetadata(metadata map[string]any) map[string]string {
	projection := make(map[string]string, 2)
	for _, key := range []string{DelegationMetadataKey, SessionAuthorityMetadataKey} {
		if value, _ := authorityMetadataValue(metadata, key); strings.TrimSpace(value) != "" {
			projection[key] = value
		}
	}
	if len(projection) == 0 {
		return nil
	}
	return projection
}

func rawRuntimeAuthorityPresent(metadata map[string]any) bool {
	for _, key := range []string{DelegationMetadataKey, SessionAuthorityMetadataKey} {
		if value, _ := authorityMetadataValue(metadata, key); value != "" {
			return true
		}
	}
	return false
}

func runtimeAuthorityFromMetadata(
	metadata map[string]any,
) (RuntimeInvocationAuthority, error) {
	delegation, err := authorityMetadataValue(metadata, DelegationMetadataKey)
	if err != nil {
		return nil, err
	}
	if delegation != "" {
		proof, err := NewDelegationProofFromMetadata(delegation)
		if err != nil {
			return nil, err
		}
		return proof, nil
	}
	session, err := authorityMetadataValue(metadata, SessionAuthorityMetadataKey)
	if err != nil {
		return nil, err
	}
	if session != "" {
		authority, err := NewSessionAuthorityFromMetadata(session)
		if err != nil {
			return nil, err
		}
		return authority, nil
	}
	return nil, nil
}

func validateRuntimeAuthorityBinding(
	authority RuntimeInvocationAuthority,
	call RuntimeCallContext,
	envelopeSubjectURA string,
	ability runtimeAbilityProjection,
) error {
	callerURA := strings.TrimSpace(call.CallerURA)
	calleeURA := strings.TrimSpace(call.CalleeURA)
	switch typed := authority.(type) {
	case DelegationProof:
		return validateRuntimeDelegationBinding(&typed, callerURA, calleeURA, envelopeSubjectURA, ability)
	case *DelegationProof:
		return validateRuntimeDelegationBinding(typed, callerURA, calleeURA, envelopeSubjectURA, ability)
	case SessionAuthority:
		return validateRuntimeSessionBinding(&typed, callerURA, calleeURA, envelopeSubjectURA, ability)
	case *SessionAuthority:
		return validateRuntimeSessionBinding(typed, callerURA, calleeURA, envelopeSubjectURA, ability)
	default:
		return invalidRuntimePayload("runtime call authority has an unsupported canonical type", nil)
	}
}

func validateRuntimeDelegationBinding(
	proof *DelegationProof,
	callerURA string,
	calleeURA string,
	subjectURA string,
	ability runtimeAbilityProjection,
) error {
	if proof == nil {
		return invalidRuntimePayload("runtime delegation authority is required", nil)
	}
	if strings.TrimSpace(proof.CallerURA) != callerURA {
		return invalidRuntimePayload("runtime delegation caller does not match caller_ura", nil)
	}
	if strings.TrimSpace(proof.SubjectURA) != subjectURA {
		return invalidRuntimePayload("runtime delegation subject does not match descriptor-bound subject_ura", nil)
	}
	if !proof.MatchesAudience(calleeURA) {
		return invalidRuntimePayload("runtime delegation audience does not admit callee_ura", nil)
	}
	if !ability.matchesScope(proof.MatchesScope) {
		return invalidRuntimePayload("runtime delegation scopes do not admit ability", nil)
	}
	return nil
}

func validateRuntimeSessionBinding(
	authority *SessionAuthority,
	callerURA string,
	calleeURA string,
	subjectURA string,
	ability runtimeAbilityProjection,
) error {
	if authority == nil {
		return invalidRuntimePayload("runtime session authority is required", nil)
	}
	if strings.TrimSpace(authority.IssuerURA) != callerURA {
		return invalidRuntimePayload("runtime session authority issuer does not match caller_ura", nil)
	}
	if strings.TrimSpace(authority.CalleeURA) != calleeURA {
		return invalidRuntimePayload("runtime session authority callee does not match callee_ura", nil)
	}
	if !authority.MatchesAudience(calleeURA) {
		return invalidRuntimePayload("runtime session authority audience does not admit callee_ura", nil)
	}
	if !runtimeSessionAuthorityAdmitsSubject(authority, subjectURA) {
		return invalidRuntimePayload("runtime session authority does not admit descriptor-bound subject_ura", nil)
	}
	if !runtimeAuthorityListAdmits(authority.AllowedActions, ability.action) {
		return invalidRuntimePayload("runtime session authority allowed_actions do not admit "+ability.action, nil)
	}
	if !ability.matchesScope(authority.MatchesScope) {
		return invalidRuntimePayload("runtime session authority scopes do not admit ability", nil)
	}
	return nil
}

func runtimeAuthorityListAdmits(patterns []string, value string) bool {
	value = strings.TrimSpace(value)
	if value == "" {
		return false
	}
	for _, pattern := range patterns {
		if strings.TrimSpace(pattern) == value {
			return true
		}
	}
	return false
}

func (c AbilityChildContext) childCall(call RuntimeCallContext) (RuntimeCallContext, error) {
	if c.client == nil {
		return RuntimeCallContext{}, invalidRuntimeClient("ability child context is not initialized")
	}
	child := call
	child.CallerURA = c.call.CallerURA
	child.NonceBase64 = c.call.NonceBase64
	child.CausalContext = cloneAbilityMetadata(c.call.CausalContext)
	child.Metadata = mergeAbilityMetadata(c.baseMeta, call.Metadata)
	if strings.TrimSpace(child.CalleeURA) == "" {
		return RuntimeCallContext{}, invalidRuntimePayload("callee_ura is required", nil)
	}
	if strings.TrimSpace(child.SubjectURA) == "" {
		return RuntimeCallContext{}, invalidRuntimePayload("subject_ura is required", nil)
	}
	return child, nil
}

func cloneAbilityMetadata(source map[string]any) map[string]any {
	if source == nil {
		return map[string]any{}
	}
	clone := make(map[string]any, len(source))
	for key, value := range source {
		clone[key] = value
	}
	return clone
}

func mergeAbilityMetadata(base map[string]any, overlay map[string]any) map[string]any {
	merged := cloneAbilityMetadata(base)
	for key, value := range overlay {
		merged[key] = value
	}
	return merged
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
