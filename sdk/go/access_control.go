package easynet

import (
	"context"
	"strings"
)

type AccessControlGrantState string

const (
	AccessControlGrantActive  AccessControlGrantState = "active"
	AccessControlGrantExpired AccessControlGrantState = "expired"
	AccessControlGrantRevoked AccessControlGrantState = "revoked"
)

type AccessControlEffect string

const (
	AccessControlAllow AccessControlEffect = "allow"
	AccessControlDeny  AccessControlEffect = "deny"
)

type AccessControlPrincipalKind string

const (
	AccessControlPrincipalUser       AccessControlPrincipalKind = "user"
	AccessControlPrincipalToken      AccessControlPrincipalKind = "token"
	AccessControlPrincipalAgent      AccessControlPrincipalKind = "agent"
	AccessControlPrincipalHub        AccessControlPrincipalKind = "hub"
	AccessControlPrincipalDevice     AccessControlPrincipalKind = "device"
	AccessControlPrincipalService    AccessControlPrincipalKind = "service"
	AccessControlPrincipalAutomation AccessControlPrincipalKind = "automation"
)

type AccessControlGrant struct {
	GrantID             string                     `json:"grant_id"`
	OwnerURA            string                     `json:"owner_ura,omitempty"`
	PrincipalKind       AccessControlPrincipalKind `json:"principal_kind"`
	PrincipalID         string                     `json:"principal_id,omitempty"`
	PrincipalURA        string                     `json:"principal_ura,omitempty"`
	TokenID             string                     `json:"token_id,omitempty"`
	TokenClass          string                     `json:"token_class,omitempty"`
	CalleeURA           string                     `json:"callee_ura,omitempty"`
	AbilityURAPattern   string                     `json:"ability_ura_pattern,omitempty"`
	SubjectURAPattern   string                     `json:"subject_ura_pattern,omitempty"`
	Actions             []string                   `json:"actions"`
	Effect              AccessControlEffect        `json:"effect"`
	Lifetime            string                     `json:"lifetime,omitempty"`
	State               AccessControlGrantState    `json:"state"`
	CreatedBy           string                     `json:"created_by"`
	CreatedAt           string                     `json:"created_at,omitempty"`
	UpdatedAt           string                     `json:"updated_at,omitempty"`
	ExpiresAt           string                     `json:"expires_at,omitempty"`
	ReviewRequiredAfter string                     `json:"review_required_after,omitempty"`
	LastReviewedAt      string                     `json:"last_reviewed_at,omitempty"`
	LastUsedAt          string                     `json:"last_used_at,omitempty"`
	RevokedAt           string                     `json:"revoked_at,omitempty"`
	RevokedBy           string                     `json:"revoked_by,omitempty"`
	RevocationReason    string                     `json:"revocation_reason,omitempty"`
	Reason              string                     `json:"reason,omitempty"`
	Constraints         map[string]any             `json:"constraints,omitempty"`
	AuthorityProofID    string                     `json:"authority_proof_id,omitempty"`
	SourceRequestID     string                     `json:"source_request_id,omitempty"`
	InvocationTemplate  map[string]any             `json:"invocation_template,omitempty"`
}

type AccessControlPolicyDecision struct {
	Decision         string                     `json:"decision"`
	Reason           string                     `json:"reason,omitempty"`
	OwnerUserID      string                     `json:"owner_user_id,omitempty"`
	OwnerURA         string                     `json:"owner_ura,omitempty"`
	OwnerSource      string                     `json:"owner_source,omitempty"`
	CallerURA        string                     `json:"caller_ura,omitempty"`
	PrincipalKind    AccessControlPrincipalKind `json:"principal_kind,omitempty"`
	PrincipalID      string                     `json:"principal_id,omitempty"`
	PrincipalURA     string                     `json:"principal_ura,omitempty"`
	TokenID          string                     `json:"token_id,omitempty"`
	CalleeURA        string                     `json:"callee_ura,omitempty"`
	AbilityURA       string                     `json:"ability_ura,omitempty"`
	SubjectURA       string                     `json:"subject_ura,omitempty"`
	Action           string                     `json:"action,omitempty"`
	GrantID          string                     `json:"grant_id,omitempty"`
	PolicyRuleID     string                     `json:"policy_rule_id,omitempty"`
	PromptRequestID  string                     `json:"prompt_request_id,omitempty"`
	CanonicalHash    string                     `json:"canonical_hash,omitempty"`
	SignatureKeyID   string                     `json:"signature_key_id,omitempty"`
	RejectorURA      string                     `json:"rejector_ura,omitempty"`
	AuthorityProofID string                     `json:"authority_proof_id,omitempty"`
	AuditWarnings    []string                   `json:"audit_warnings,omitempty"`
}

type AccessControlSignatureDecision struct {
	Decision                   string `json:"decision"`
	Reason                     string `json:"reason,omitempty"`
	CallerURA                  string `json:"caller_ura,omitempty"`
	CalleeURA                  string `json:"callee_ura,omitempty"`
	AbilityURA                 string `json:"ability_ura,omitempty"`
	SubjectURA                 string `json:"subject_ura,omitempty"`
	CanonicalHash              string `json:"canonical_hash,omitempty"`
	SignatureKeyID             string `json:"signature_key_id,omitempty"`
	PresentedPubkeyFingerprint string `json:"presented_pubkey_fingerprint,omitempty"`
	VerifierURA                string `json:"verifier_ura,omitempty"`
	RejectorURA                string `json:"rejector_ura,omitempty"`
}

type AccessControlAuthorityProof struct {
	ProofID                  string                     `json:"proof_id"`
	GrantID                  string                     `json:"grant_id,omitempty"`
	PermissionRequestID      string                     `json:"permission_request_id,omitempty"`
	OwnerURA                 string                     `json:"owner_ura,omitempty"`
	PrincipalKind            AccessControlPrincipalKind `json:"principal_kind,omitempty"`
	PrincipalID              string                     `json:"principal_id,omitempty"`
	PrincipalURA             string                     `json:"principal_ura,omitempty"`
	TokenID                  string                     `json:"token_id,omitempty"`
	CalleeURA                string                     `json:"callee_ura,omitempty"`
	AbilityURA               string                     `json:"ability_ura,omitempty"`
	SubjectURA               string                     `json:"subject_ura,omitempty"`
	Action                   string                     `json:"action,omitempty"`
	Nonce                    string                     `json:"nonce,omitempty"`
	CanonicalHash            string                     `json:"canonical_hash,omitempty"`
	CanonicalInvocationHash  string                     `json:"canonical_invocation_hash,omitempty"`
	SessionID                string                     `json:"session_id,omitempty"`
	SessionOwnerURA          string                     `json:"session_owner_ura,omitempty"`
	AllowedFollowupAbilities []string                   `json:"allowed_followup_abilities,omitempty"`
	SessionExpiresAt         string                     `json:"session_expires_at,omitempty"`
	IssuerURA                string                     `json:"issuer_ura,omitempty"`
	AudienceURA              string                     `json:"audience_ura,omitempty"`
	IssuedAt                 string                     `json:"issued_at,omitempty"`
	ExpiresAt                string                     `json:"expires_at,omitempty"`
	Signature                string                     `json:"signature,omitempty"`
	VerificationKeyID        string                     `json:"verification_key_id,omitempty"`
}

type AccessControlPermissionRequest struct {
	RequestID          string                     `json:"request_id"`
	OwnerURA           string                     `json:"owner_ura,omitempty"`
	CallerURA          string                     `json:"caller_ura,omitempty"`
	PrincipalKind      AccessControlPrincipalKind `json:"principal_kind,omitempty"`
	PrincipalID        string                     `json:"principal_id,omitempty"`
	PrincipalURA       string                     `json:"principal_ura,omitempty"`
	TokenID            string                     `json:"token_id,omitempty"`
	TokenClass         string                     `json:"token_class,omitempty"`
	CalleeURA          string                     `json:"callee_ura,omitempty"`
	SubjectURA         string                     `json:"subject_ura,omitempty"`
	AbilityURA         string                     `json:"ability_ura,omitempty"`
	Action             string                     `json:"action,omitempty"`
	Nonce              string                     `json:"nonce,omitempty"`
	CanonicalHash      string                     `json:"canonical_hash,omitempty"`
	RequestedLifetimes []string                   `json:"requested_lifetimes,omitempty"`
	Status             string                     `json:"status,omitempty"`
	CreatedAt          string                     `json:"created_at,omitempty"`
	ExpiresAt          string                     `json:"expires_at,omitempty"`
	ResolverURA        string                     `json:"resolver_ura,omitempty"`
	ResolvedLifetime   string                     `json:"resolved_lifetime,omitempty"`
	CreatedGrantID     string                     `json:"created_grant_id,omitempty"`
	AuthorityProofID   string                     `json:"authority_proof_id,omitempty"`
	ResolvedAt         string                     `json:"resolved_at,omitempty"`
	DecisionReason     string                     `json:"decision_reason,omitempty"`
}

type AccessControlAbilityCallTrace struct {
	InvocationID       string                          `json:"invocation_id"`
	ParentInvocationID string                          `json:"parent_invocation_id,omitempty"`
	RootInvocationID   string                          `json:"root_invocation_id,omitempty"`
	CallerURA          string                          `json:"caller_ura,omitempty"`
	CalleeURA          string                          `json:"callee_ura,omitempty"`
	SubjectURA         string                          `json:"subject_ura,omitempty"`
	AbilityURA         string                          `json:"ability_ura,omitempty"`
	Action             string                          `json:"action,omitempty"`
	RouteRef           string                          `json:"route_ref,omitempty"`
	ExecutionHostURA   string                          `json:"execution_host_ura,omitempty"`
	RejectorURA        string                          `json:"rejector_ura,omitempty"`
	Stage              string                          `json:"stage,omitempty"`
	SignatureDecision  *AccessControlSignatureDecision `json:"signature_decision,omitempty"`
	PolicyDecision     *AccessControlPolicyDecision    `json:"policy_decision,omitempty"`
	AuthorityProofID   string                          `json:"authority_proof_id,omitempty"`
	Redacted           bool                            `json:"redacted,omitempty"`
	ChildFailureClass  string                          `json:"child_failure_class,omitempty"`
	RedactionReason    string                          `json:"redaction_reason,omitempty"`
	Children           []AccessControlAbilityCallTrace `json:"children,omitempty"`
}

type AccessControlGrantRequest struct {
	Call         RuntimeCallContext `json:"call"`
	Grant        AccessControlGrant `json:"grant"`
	OwnerURA     string             `json:"owner_ura,omitempty"`
	PrincipalURA string             `json:"principal_ura,omitempty"`
	ActorURA     string             `json:"actor_ura,omitempty"`
}

type AccessControlPermissionRequestCreateRequest struct {
	Call         RuntimeCallContext             `json:"call"`
	Request      AccessControlPermissionRequest `json:"request"`
	OwnerURA     string                         `json:"owner_ura,omitempty"`
	PrincipalURA string                         `json:"principal_ura,omitempty"`
	ActorURA     string                         `json:"actor_ura,omitempty"`
}

type AccessControlPermissionRequestResolveRequest struct {
	Call           RuntimeCallContext             `json:"call"`
	Request        AccessControlPermissionRequest `json:"request"`
	CreatedGrant   *AccessControlGrant            `json:"created_grant,omitempty"`
	AuthorityProof *AccessControlAuthorityProof   `json:"authority_proof,omitempty"`
	OwnerURA       string                         `json:"owner_ura,omitempty"`
	PrincipalURA   string                         `json:"principal_ura,omitempty"`
	ActorURA       string                         `json:"actor_ura,omitempty"`
}

type AccessControlPermissionRequestListRequest struct {
	Call          RuntimeCallContext         `json:"call"`
	OwnerURA      string                     `json:"owner_ura,omitempty"`
	PrincipalKind AccessControlPrincipalKind `json:"principal_kind,omitempty"`
	PrincipalID   string                     `json:"principal_id,omitempty"`
	PrincipalURA  string                     `json:"principal_ura,omitempty"`
	TokenID       string                     `json:"token_id,omitempty"`
	Status        string                     `json:"status,omitempty"`
	Limit         uint32                     `json:"limit,omitempty"`
	Cursor        string                     `json:"cursor,omitempty"`
}

type AccessControlAdmissionExplainRequest struct {
	Call         RuntimeCallContext `json:"call"`
	ObserverURA  string             `json:"observer_ura"`
	InvocationID string             `json:"invocation_id,omitempty"`
	TraceID      string             `json:"trace_id,omitempty"`
	RootID       string             `json:"root_id,omitempty"`
}

type AccessControlRevokeRequest struct {
	Call     RuntimeCallContext `json:"call"`
	OwnerURA string             `json:"owner_ura,omitempty"`
	GrantID  string             `json:"grant_id"`
	ActorURA string             `json:"actor_ura,omitempty"`
	Reason   string             `json:"reason,omitempty"`
}

type AccessControlListRequest struct {
	Call              RuntimeCallContext         `json:"call"`
	OwnerURA          string                     `json:"owner_ura,omitempty"`
	PrincipalKind     AccessControlPrincipalKind `json:"principal_kind,omitempty"`
	PrincipalID       string                     `json:"principal_id,omitempty"`
	PrincipalURA      string                     `json:"principal_ura,omitempty"`
	TokenID           string                     `json:"token_id,omitempty"`
	CalleeURA         string                     `json:"callee_ura,omitempty"`
	AbilityURA        string                     `json:"ability_ura,omitempty"`
	AbilityURAPattern string                     `json:"ability_ura_pattern,omitempty"`
	SubjectURA        string                     `json:"subject_ura,omitempty"`
	SubjectURAPattern string                     `json:"subject_ura_pattern,omitempty"`
	Action            string                     `json:"action,omitempty"`
	Effect            AccessControlEffect        `json:"effect,omitempty"`
	State             AccessControlGrantState    `json:"state,omitempty"`
	Limit             uint32                     `json:"limit,omitempty"`
	Cursor            string                     `json:"cursor,omitempty"`
}

type AccessControlCheckRequest struct {
	Call                        RuntimeCallContext         `json:"call"`
	OwnerURA                    string                     `json:"owner_ura,omitempty"`
	OwnerSource                 string                     `json:"owner_source,omitempty"`
	CallerURA                   string                     `json:"caller_ura,omitempty"`
	PrincipalKind               AccessControlPrincipalKind `json:"principal_kind"`
	PrincipalURA                string                     `json:"principal_ura,omitempty"`
	TokenID                     string                     `json:"token_id,omitempty"`
	TokenClass                  string                     `json:"token_class,omitempty"`
	CalleeURA                   string                     `json:"callee_ura"`
	SubjectURA                  string                     `json:"subject_ura"`
	AbilityURA                  string                     `json:"ability_ura"`
	Action                      string                     `json:"action"`
	SafeRead                    bool                       `json:"safe_read,omitempty"`
	InteractiveContextAvailable bool                       `json:"interactive_context_available,omitempty"`
	CanonicalHash               string                     `json:"canonical_hash,omitempty"`
	SignatureKeyID              string                     `json:"signature_key_id,omitempty"`
	AuthorityProofID            string                     `json:"authority_proof_id,omitempty"`
	RejectorURA                 string                     `json:"rejector_ura,omitempty"`
}

type AccessControlGrantResult struct {
	Grant            AccessControlGrant `json:"grant"`
	IdempotentReplay bool               `json:"idempotent_replay,omitempty"`
	AuditRecordID    string             `json:"audit_record_id,omitempty"`
}

type AccessControlListResult struct {
	Grants []AccessControlGrant `json:"grants"`
}

type AccessControlCheckResult struct {
	PolicyDecision AccessControlPolicyDecision `json:"policy_decision"`
}

type AccessControlPermissionRequestResolutionResult struct {
	Request          AccessControlPermissionRequest `json:"request"`
	CreatedGrant     *AccessControlGrant            `json:"created_grant,omitempty"`
	AuthorityProof   *AccessControlAuthorityProof   `json:"authority_proof,omitempty"`
	IdempotentReplay bool                           `json:"idempotent_replay,omitempty"`
}

type AccessControlPermissionRequestListResult struct {
	Requests []AccessControlPermissionRequest `json:"requests"`
}

type AccessControlAdmissionExplainResult struct {
	ObserverURA       string                          `json:"observer_ura"`
	Redacted          bool                            `json:"redacted"`
	RootTrace         *AccessControlAbilityCallTrace  `json:"root_trace,omitempty"`
	SignatureDecision *AccessControlSignatureDecision `json:"signature_decision,omitempty"`
	PolicyDecision    *AccessControlPolicyDecision    `json:"policy_decision,omitempty"`
	AuthorityReason   string                          `json:"authority_reason,omitempty"`
	RouteRef          string                          `json:"route_ref,omitempty"`
	RejectorURA       string                          `json:"rejector_ura,omitempty"`
	RedactionReason   string                          `json:"redaction_reason,omitempty"`
}

type AccessControlProvider interface {
	Grant(context.Context, AccessControlGrantRequest) (AccessControlGrantResult, error)
	Revoke(context.Context, AccessControlRevokeRequest) (AccessControlGrant, error)
	List(context.Context, AccessControlListRequest) (AccessControlListResult, error)
	Check(context.Context, AccessControlCheckRequest) (AccessControlCheckResult, error)
	CreateRequest(context.Context, AccessControlPermissionRequestCreateRequest) (AccessControlPermissionRequest, error)
	ResolveRequest(context.Context, AccessControlPermissionRequestResolveRequest) (AccessControlPermissionRequestResolutionResult, error)
	ListRequests(context.Context, AccessControlPermissionRequestListRequest) (AccessControlPermissionRequestListResult, error)
	Explain(context.Context, AccessControlAdmissionExplainRequest) (AccessControlAdmissionExplainResult, error)
}

type AccessControlClient struct {
	provider AccessControlProvider
}

func NewAccessControlClient(provider AccessControlProvider) (*AccessControlClient, error) {
	if provider == nil {
		return nil, invalidAccessControl("AccessControl provider is required", nil)
	}
	return &AccessControlClient{provider: provider}, nil
}

func (c *AccessControlClient) Grant(ctx context.Context, request AccessControlGrantRequest) (AccessControlGrantResult, error) {
	if c == nil || c.provider == nil {
		return AccessControlGrantResult{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.Grant(ctx, request)
}

func (c *AccessControlClient) Revoke(ctx context.Context, request AccessControlRevokeRequest) (AccessControlGrant, error) {
	if c == nil || c.provider == nil {
		return AccessControlGrant{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.Revoke(ctx, request)
}

func (c *AccessControlClient) List(ctx context.Context, request AccessControlListRequest) (AccessControlListResult, error) {
	if c == nil || c.provider == nil {
		return AccessControlListResult{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.List(ctx, request)
}

func (c *AccessControlClient) Check(ctx context.Context, request AccessControlCheckRequest) (AccessControlCheckResult, error) {
	if c == nil || c.provider == nil {
		return AccessControlCheckResult{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.Check(ctx, request)
}

func (c *AccessControlClient) CreateRequest(ctx context.Context, request AccessControlPermissionRequestCreateRequest) (AccessControlPermissionRequest, error) {
	if c == nil || c.provider == nil {
		return AccessControlPermissionRequest{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.CreateRequest(ctx, request)
}

func (c *AccessControlClient) ResolveRequest(ctx context.Context, request AccessControlPermissionRequestResolveRequest) (AccessControlPermissionRequestResolutionResult, error) {
	if c == nil || c.provider == nil {
		return AccessControlPermissionRequestResolutionResult{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.ResolveRequest(ctx, request)
}

func (c *AccessControlClient) ListRequests(ctx context.Context, request AccessControlPermissionRequestListRequest) (AccessControlPermissionRequestListResult, error) {
	if c == nil || c.provider == nil {
		return AccessControlPermissionRequestListResult{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.ListRequests(ctx, request)
}

func (c *AccessControlClient) Explain(ctx context.Context, request AccessControlAdmissionExplainRequest) (AccessControlAdmissionExplainResult, error) {
	if c == nil || c.provider == nil {
		return AccessControlAdmissionExplainResult{}, invalidAccessControl("AccessControl client is not initialized", nil)
	}
	return c.provider.Explain(ctx, request)
}

type accessControlAbilityInvoker interface {
	Invoke(context.Context, RuntimeCallContext, string, any) (map[string]any, error)
}

type RuntimeAccessControlProvider struct {
	ability accessControlAbilityInvoker
}

func NewRuntimeAccessControlProvider(ability accessControlAbilityInvoker) (*RuntimeAccessControlProvider, error) {
	if ability == nil {
		return nil, invalidAccessControl("runtime ability client is required", nil)
	}
	return &RuntimeAccessControlProvider{ability: ability}, nil
}

func (p *RuntimeAccessControlProvider) Grant(ctx context.Context, request AccessControlGrantRequest) (AccessControlGrantResult, error) {
	if p == nil || p.ability == nil {
		return AccessControlGrantResult{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlGrantArgs(request)
	if err != nil {
		return AccessControlGrantResult{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityGrant, args)
	if err != nil {
		return AccessControlGrantResult{}, err
	}
	grant, err := accessControlGrantFromMap(requiredMap(output, "grant"))
	if err != nil {
		return AccessControlGrantResult{}, err
	}
	return AccessControlGrantResult{
		Grant:            grant,
		IdempotentReplay: boolFromMap(output, "idempotent_replay"),
		AuditRecordID:    stringFromMap(output, "audit_record_id"),
	}, nil
}

func (p *RuntimeAccessControlProvider) Revoke(ctx context.Context, request AccessControlRevokeRequest) (AccessControlGrant, error) {
	if p == nil || p.ability == nil {
		return AccessControlGrant{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlRevokeArgs(request)
	if err != nil {
		return AccessControlGrant{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityRevoke, args)
	if err != nil {
		return AccessControlGrant{}, err
	}
	return accessControlGrantFromMap(requiredMap(output, "grant"))
}

func (p *RuntimeAccessControlProvider) List(ctx context.Context, request AccessControlListRequest) (AccessControlListResult, error) {
	if p == nil || p.ability == nil {
		return AccessControlListResult{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlListArgs(request)
	if err != nil {
		return AccessControlListResult{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityList, args)
	if err != nil {
		return AccessControlListResult{}, err
	}
	raw, ok := output["grants"].([]any)
	if !ok {
		return AccessControlListResult{}, invalidAccessControl("access-control grants projection is required", nil)
	}
	grants := make([]AccessControlGrant, 0, len(raw))
	for _, item := range raw {
		grant, err := accessControlGrantFromMap(requiredMapValue(item, "grant"))
		if err != nil {
			return AccessControlListResult{}, err
		}
		grants = append(grants, grant)
	}
	return AccessControlListResult{Grants: grants}, nil
}

func (p *RuntimeAccessControlProvider) Check(ctx context.Context, request AccessControlCheckRequest) (AccessControlCheckResult, error) {
	if p == nil || p.ability == nil {
		return AccessControlCheckResult{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlCheckArgs(request)
	if err != nil {
		return AccessControlCheckResult{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityCheck, args)
	if err != nil {
		return AccessControlCheckResult{}, err
	}
	decision, err := accessControlPolicyDecisionFromMap(requiredMap(output, "policy_decision"))
	if err != nil {
		return AccessControlCheckResult{}, err
	}
	return AccessControlCheckResult{PolicyDecision: decision}, nil
}

func (p *RuntimeAccessControlProvider) CreateRequest(ctx context.Context, request AccessControlPermissionRequestCreateRequest) (AccessControlPermissionRequest, error) {
	if p == nil || p.ability == nil {
		return AccessControlPermissionRequest{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlPermissionRequestCreateArgs(request)
	if err != nil {
		return AccessControlPermissionRequest{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityPolicyRequestCreate, args)
	if err != nil {
		return AccessControlPermissionRequest{}, err
	}
	return accessControlPermissionRequestFromMap(requiredMap(output, "request"))
}

func (p *RuntimeAccessControlProvider) ResolveRequest(ctx context.Context, request AccessControlPermissionRequestResolveRequest) (AccessControlPermissionRequestResolutionResult, error) {
	if p == nil || p.ability == nil {
		return AccessControlPermissionRequestResolutionResult{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlPermissionRequestResolveArgs(request)
	if err != nil {
		return AccessControlPermissionRequestResolutionResult{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityPolicyRequestResolve, args)
	if err != nil {
		return AccessControlPermissionRequestResolutionResult{}, err
	}
	resolved, err := accessControlPermissionRequestFromMap(requiredMap(output, "request"))
	if err != nil {
		return AccessControlPermissionRequestResolutionResult{}, err
	}
	result := AccessControlPermissionRequestResolutionResult{
		Request:          resolved,
		IdempotentReplay: boolFromMap(output, "idempotent_replay"),
	}
	if raw := mapFromMap(output, "created_grant"); raw != nil {
		grant, err := accessControlGrantFromMap(raw)
		if err != nil {
			return AccessControlPermissionRequestResolutionResult{}, err
		}
		result.CreatedGrant = &grant
	}
	if raw := mapFromMap(output, "authority_proof"); raw != nil {
		proof, err := accessControlAuthorityProofFromMap(raw)
		if err != nil {
			return AccessControlPermissionRequestResolutionResult{}, err
		}
		result.AuthorityProof = &proof
	}
	return result, nil
}

func (p *RuntimeAccessControlProvider) ListRequests(ctx context.Context, request AccessControlPermissionRequestListRequest) (AccessControlPermissionRequestListResult, error) {
	if p == nil || p.ability == nil {
		return AccessControlPermissionRequestListResult{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlPermissionRequestListArgs(request)
	if err != nil {
		return AccessControlPermissionRequestListResult{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityPolicyRequestList, args)
	if err != nil {
		return AccessControlPermissionRequestListResult{}, err
	}
	raw, ok := output["requests"].([]any)
	if !ok {
		return AccessControlPermissionRequestListResult{}, invalidAccessControl("access-control permission requests projection is required", nil)
	}
	requests := make([]AccessControlPermissionRequest, 0, len(raw))
	for _, item := range raw {
		projected, err := accessControlPermissionRequestFromMap(requiredMapValue(item, "request"))
		if err != nil {
			return AccessControlPermissionRequestListResult{}, err
		}
		requests = append(requests, projected)
	}
	return AccessControlPermissionRequestListResult{Requests: requests}, nil
}

func (p *RuntimeAccessControlProvider) Explain(ctx context.Context, request AccessControlAdmissionExplainRequest) (AccessControlAdmissionExplainResult, error) {
	if p == nil || p.ability == nil {
		return AccessControlAdmissionExplainResult{}, invalidAccessControl("runtime AccessControl provider is not initialized", nil)
	}
	normalized, args, err := accessControlAdmissionExplainArgs(request)
	if err != nil {
		return AccessControlAdmissionExplainResult{}, err
	}
	output, err := p.ability.Invoke(ctx, normalized.Call, accessControlAbilityAdmissionExplain, args)
	if err != nil {
		return AccessControlAdmissionExplainResult{}, err
	}
	return accessControlAdmissionExplainFromMap(output)
}

func accessControlGrantArgs(request AccessControlGrantRequest) (AccessControlGrantRequest, map[string]any, error) {
	grant, err := normalizeAccessControlGrant(request.Grant, request.OwnerURA, request.PrincipalURA)
	if err != nil {
		return AccessControlGrantRequest{}, nil, err
	}
	request.Grant = grant
	args := map[string]any{"grant": accessControlGrantWire(grant), "owner_ura": grant.OwnerURA}
	if grant.PrincipalURA != "" {
		args["principal_ura"] = grant.PrincipalURA
	}
	if actor := strings.TrimSpace(request.ActorURA); actor != "" {
		args["actor_ura"] = actor
	}
	return request, args, nil
}

func accessControlRevokeArgs(request AccessControlRevokeRequest) (AccessControlRevokeRequest, map[string]any, error) {
	ownerURA := strings.TrimSpace(request.OwnerURA)
	if ownerURA == "" {
		return AccessControlRevokeRequest{}, nil, invalidAccessControl("owner_ura is required", nil)
	}
	if _, err := accessControlUserIDFromURA(ownerURA, "owner_ura"); err != nil {
		return AccessControlRevokeRequest{}, nil, err
	}
	grantID := strings.TrimSpace(request.GrantID)
	if grantID == "" {
		return AccessControlRevokeRequest{}, nil, invalidAccessControl("grant_id is required", nil)
	}
	actorURA := strings.TrimSpace(request.ActorURA)
	if actorURA == "" {
		return AccessControlRevokeRequest{}, nil, invalidAccessControl("actor_ura is required", nil)
	}
	if _, err := ParseURAParts(actorURA); err != nil {
		return AccessControlRevokeRequest{}, nil, invalidAccessControl("actor_ura must be canonical", err)
	}
	request.OwnerURA = ownerURA
	request.ActorURA = actorURA
	args := map[string]any{"owner_ura": ownerURA, "grant_id": grantID, "actor_ura": actorURA}
	if reason := strings.TrimSpace(request.Reason); reason != "" {
		args["reason"] = reason
	}
	return request, args, nil
}

func accessControlListArgs(request AccessControlListRequest) (AccessControlListRequest, map[string]any, error) {
	ownerURA := strings.TrimSpace(request.OwnerURA)
	if ownerURA == "" {
		return AccessControlListRequest{}, nil, invalidAccessControl("owner_ura is required", nil)
	}
	if _, err := accessControlUserIDFromURA(ownerURA, "owner_ura"); err != nil {
		return AccessControlListRequest{}, nil, err
	}
	if _, err := accessControlPrincipalID(request.PrincipalKind, request.PrincipalURA, request.PrincipalID); err != nil {
		return AccessControlListRequest{}, nil, err
	}
	args := map[string]any{"owner_ura": ownerURA}
	if request.PrincipalKind != "" {
		args["principal_kind"] = string(request.PrincipalKind)
	}
	if principalURA := strings.TrimSpace(request.PrincipalURA); principalURA != "" {
		args["principal_ura"] = principalURA
	}
	optionalStringArg(args, "token_id", request.TokenID)
	optionalStringArg(args, "callee_ura", request.CalleeURA)
	optionalStringArg(args, "ability_ura", request.AbilityURA)
	optionalStringArg(args, "ability_ura_pattern", request.AbilityURAPattern)
	optionalStringArg(args, "subject_ura", request.SubjectURA)
	optionalStringArg(args, "subject_ura_pattern", request.SubjectURAPattern)
	optionalStringArg(args, "action", request.Action)
	if request.Effect != "" {
		args["effect"] = string(request.Effect)
	}
	if request.State != "" {
		args["state"] = string(request.State)
	}
	if request.Limit != 0 {
		args["limit"] = request.Limit
	}
	optionalStringArg(args, "cursor", request.Cursor)
	request.OwnerURA = ownerURA
	return request, args, nil
}

func accessControlCheckArgs(request AccessControlCheckRequest) (AccessControlCheckRequest, map[string]any, error) {
	ownerURA := strings.TrimSpace(request.OwnerURA)
	if ownerURA == "" {
		return AccessControlCheckRequest{}, nil, invalidAccessControl("owner_ura is required", nil)
	}
	if _, err := accessControlUserIDFromURA(ownerURA, "owner_ura"); err != nil {
		return AccessControlCheckRequest{}, nil, err
	}
	principalID, err := accessControlPrincipalID(request.PrincipalKind, request.PrincipalURA, "")
	if err != nil {
		return AccessControlCheckRequest{}, nil, err
	}
	if principalID == "" {
		return AccessControlCheckRequest{}, nil, invalidAccessControl("principal_ura is required", nil)
	}
	for _, field := range []struct{ name, value string }{
		{"callee_ura", request.CalleeURA},
		{"subject_ura", request.SubjectURA},
		{"ability_ura", request.AbilityURA},
		{"action", request.Action},
	} {
		if strings.TrimSpace(field.value) == "" {
			return AccessControlCheckRequest{}, nil, invalidAccessControl(field.name+" is required", nil)
		}
	}
	args := map[string]any{
		"owner_ura":                     ownerURA,
		"principal_kind":                string(request.PrincipalKind),
		"principal_ura":                 strings.TrimSpace(request.PrincipalURA),
		"callee_ura":                    strings.TrimSpace(request.CalleeURA),
		"subject_ura":                   strings.TrimSpace(request.SubjectURA),
		"ability_ura":                   strings.TrimSpace(request.AbilityURA),
		"action":                        strings.TrimSpace(request.Action),
		"safe_read":                     request.SafeRead,
		"interactive_context_available": request.InteractiveContextAvailable,
	}
	optionalStringArg(args, "owner_source", request.OwnerSource)
	optionalStringArg(args, "caller_ura", request.CallerURA)
	optionalStringArg(args, "token_id", request.TokenID)
	optionalStringArg(args, "token_class", request.TokenClass)
	optionalStringArg(args, "canonical_hash", request.CanonicalHash)
	optionalStringArg(args, "signature_key_id", request.SignatureKeyID)
	optionalStringArg(args, "authority_proof_id", request.AuthorityProofID)
	optionalStringArg(args, "rejector_ura", request.RejectorURA)
	request.OwnerURA = ownerURA
	return request, args, nil
}

func accessControlPermissionRequestCreateArgs(request AccessControlPermissionRequestCreateRequest) (AccessControlPermissionRequestCreateRequest, map[string]any, error) {
	projected, err := normalizeAccessControlPermissionRequest(request.Request, request.OwnerURA, request.PrincipalURA)
	if err != nil {
		return AccessControlPermissionRequestCreateRequest{}, nil, err
	}
	request.Request = projected
	request.OwnerURA = projected.OwnerURA
	request.PrincipalURA = projected.PrincipalURA
	args := map[string]any{
		"request":   accessControlPermissionRequestWire(projected),
		"owner_ura": projected.OwnerURA,
	}
	if projected.PrincipalURA != "" {
		args["principal_ura"] = projected.PrincipalURA
	}
	optionalStringArg(args, "actor_ura", request.ActorURA)
	return request, args, nil
}

func accessControlPermissionRequestResolveArgs(request AccessControlPermissionRequestResolveRequest) (AccessControlPermissionRequestResolveRequest, map[string]any, error) {
	projected, err := normalizeAccessControlPermissionRequest(request.Request, request.OwnerURA, request.PrincipalURA)
	if err != nil {
		return AccessControlPermissionRequestResolveRequest{}, nil, err
	}
	request.Request = projected
	request.OwnerURA = projected.OwnerURA
	request.PrincipalURA = projected.PrincipalURA
	args := map[string]any{
		"request":   accessControlPermissionRequestWire(projected),
		"owner_ura": projected.OwnerURA,
	}
	if projected.PrincipalURA != "" {
		args["principal_ura"] = projected.PrincipalURA
	}
	optionalStringArg(args, "actor_ura", request.ActorURA)
	if request.CreatedGrant != nil {
		grant, err := normalizeAccessControlGrant(*request.CreatedGrant, projected.OwnerURA, projected.PrincipalURA)
		if err != nil {
			return AccessControlPermissionRequestResolveRequest{}, nil, err
		}
		request.CreatedGrant = &grant
		args["created_grant"] = accessControlGrantWire(grant)
	}
	if request.AuthorityProof != nil {
		proof := *request.AuthorityProof
		proof.OwnerURA = firstNonEmpty(proof.OwnerURA, projected.OwnerURA)
		proof.PrincipalKind = firstNonEmptyPrincipalKind(proof.PrincipalKind, projected.PrincipalKind)
		proof.PrincipalURA = firstNonEmpty(proof.PrincipalURA, projected.PrincipalURA)
		wire, err := accessControlAuthorityProofWire(proof)
		if err != nil {
			return AccessControlPermissionRequestResolveRequest{}, nil, err
		}
		request.AuthorityProof = &proof
		args["authority_proof"] = wire
	}
	return request, args, nil
}

func accessControlPermissionRequestListArgs(request AccessControlPermissionRequestListRequest) (AccessControlPermissionRequestListRequest, map[string]any, error) {
	ownerURA := strings.TrimSpace(request.OwnerURA)
	if ownerURA == "" {
		return AccessControlPermissionRequestListRequest{}, nil, invalidAccessControl("owner_ura is required", nil)
	}
	if _, err := accessControlUserIDFromURA(ownerURA, "owner_ura"); err != nil {
		return AccessControlPermissionRequestListRequest{}, nil, err
	}
	if _, err := accessControlPrincipalID(request.PrincipalKind, request.PrincipalURA, request.PrincipalID); err != nil {
		return AccessControlPermissionRequestListRequest{}, nil, err
	}
	args := map[string]any{"owner_ura": ownerURA}
	if request.PrincipalKind != "" {
		args["principal_kind"] = string(request.PrincipalKind)
	}
	optionalStringArg(args, "principal_ura", request.PrincipalURA)
	optionalStringArg(args, "token_id", request.TokenID)
	optionalStringArg(args, "status", request.Status)
	if request.Limit != 0 {
		args["limit"] = request.Limit
	}
	optionalStringArg(args, "cursor", request.Cursor)
	request.OwnerURA = ownerURA
	return request, args, nil
}

func accessControlAdmissionExplainArgs(request AccessControlAdmissionExplainRequest) (AccessControlAdmissionExplainRequest, map[string]any, error) {
	observerURA := strings.TrimSpace(request.ObserverURA)
	if observerURA == "" {
		return AccessControlAdmissionExplainRequest{}, nil, invalidAccessControl("observer_ura is required", nil)
	}
	request.ObserverURA = observerURA
	args := map[string]any{"observer_ura": observerURA}
	optionalStringArg(args, "invocation_id", request.InvocationID)
	optionalStringArg(args, "trace_id", request.TraceID)
	optionalStringArg(args, "root_id", request.RootID)
	return request, args, nil
}

func normalizeAccessControlGrant(grant AccessControlGrant, ownerURA string, principalURA string) (AccessControlGrant, error) {
	grant.OwnerURA = firstNonEmpty(ownerURA, grant.OwnerURA)
	if strings.TrimSpace(grant.OwnerURA) == "" {
		return AccessControlGrant{}, invalidAccessControl("owner_ura is required", nil)
	}
	if _, err := accessControlUserIDFromURA(grant.OwnerURA, "owner_ura"); err != nil {
		return AccessControlGrant{}, err
	}
	grant.PrincipalURA = firstNonEmpty(principalURA, grant.PrincipalURA)
	if grant.PrincipalKind == "" {
		grant.PrincipalKind = AccessControlPrincipalUser
	}
	principalID, err := accessControlPrincipalID(grant.PrincipalKind, grant.PrincipalURA, grant.PrincipalID)
	if err != nil {
		return AccessControlGrant{}, err
	}
	grant.PrincipalID = principalID
	if strings.TrimSpace(grant.GrantID) == "" {
		return AccessControlGrant{}, invalidAccessControl("grant_id is required", nil)
	}
	if len(grant.Actions) == 0 {
		return AccessControlGrant{}, invalidAccessControl("grant actions are required", nil)
	}
	if grant.Effect == "" {
		grant.Effect = AccessControlAllow
	}
	if grant.State == "" {
		grant.State = AccessControlGrantActive
	}
	return grant, nil
}

func accessControlGrantWire(grant AccessControlGrant) map[string]any {
	wire := map[string]any{
		"grant_id":       strings.TrimSpace(grant.GrantID),
		"owner_ura":      strings.TrimSpace(grant.OwnerURA),
		"principal_kind": string(grant.PrincipalKind),
		"principal_ura":  strings.TrimSpace(grant.PrincipalURA),
		"actions":        grant.Actions,
		"effect":         string(grant.Effect),
		"state":          string(grant.State),
		"created_by":     strings.TrimSpace(grant.CreatedBy),
		"constraints":    grant.Constraints,
	}
	optionalStringArg(wire, "token_id", grant.TokenID)
	optionalStringArg(wire, "token_class", grant.TokenClass)
	optionalStringArg(wire, "callee_ura", grant.CalleeURA)
	optionalStringArg(wire, "ability_ura_pattern", grant.AbilityURAPattern)
	optionalStringArg(wire, "subject_ura_pattern", grant.SubjectURAPattern)
	optionalStringArg(wire, "lifetime", grant.Lifetime)
	optionalStringArg(wire, "created_at", grant.CreatedAt)
	optionalStringArg(wire, "updated_at", grant.UpdatedAt)
	optionalStringArg(wire, "expires_at", grant.ExpiresAt)
	optionalStringArg(wire, "review_required_after", grant.ReviewRequiredAfter)
	optionalStringArg(wire, "last_reviewed_at", grant.LastReviewedAt)
	optionalStringArg(wire, "last_used_at", grant.LastUsedAt)
	optionalStringArg(wire, "revoked_at", grant.RevokedAt)
	optionalStringArg(wire, "revoked_by", grant.RevokedBy)
	optionalStringArg(wire, "revocation_reason", grant.RevocationReason)
	optionalStringArg(wire, "reason", grant.Reason)
	optionalStringArg(wire, "authority_proof_id", grant.AuthorityProofID)
	optionalStringArg(wire, "source_request_id", grant.SourceRequestID)
	if grant.InvocationTemplate != nil {
		wire["invocation_template"] = grant.InvocationTemplate
	}
	return wire
}

func normalizeAccessControlPermissionRequest(request AccessControlPermissionRequest, ownerURA string, principalURA string) (AccessControlPermissionRequest, error) {
	request.OwnerURA = firstNonEmpty(ownerURA, request.OwnerURA)
	if strings.TrimSpace(request.OwnerURA) == "" {
		return AccessControlPermissionRequest{}, invalidAccessControl("owner_ura is required", nil)
	}
	if _, err := accessControlUserIDFromURA(request.OwnerURA, "owner_ura"); err != nil {
		return AccessControlPermissionRequest{}, err
	}
	request.PrincipalURA = firstNonEmpty(principalURA, request.PrincipalURA)
	if request.PrincipalKind == "" {
		request.PrincipalKind = AccessControlPrincipalUser
	}
	principalID, err := accessControlPrincipalID(request.PrincipalKind, request.PrincipalURA, request.PrincipalID)
	if err != nil {
		return AccessControlPermissionRequest{}, err
	}
	request.PrincipalID = principalID
	if strings.TrimSpace(request.RequestID) == "" {
		return AccessControlPermissionRequest{}, invalidAccessControl("request_id is required", nil)
	}
	for _, field := range []struct{ name, value string }{
		{"callee_ura", request.CalleeURA},
		{"subject_ura", request.SubjectURA},
		{"ability_ura", request.AbilityURA},
		{"action", request.Action},
	} {
		if strings.TrimSpace(field.value) == "" {
			return AccessControlPermissionRequest{}, invalidAccessControl(field.name+" is required", nil)
		}
	}
	return request, nil
}

func accessControlPermissionRequestWire(request AccessControlPermissionRequest) map[string]any {
	wire := map[string]any{
		"request_id":     strings.TrimSpace(request.RequestID),
		"owner_ura":      strings.TrimSpace(request.OwnerURA),
		"principal_kind": string(request.PrincipalKind),
		"principal_ura":  strings.TrimSpace(request.PrincipalURA),
		"callee_ura":     strings.TrimSpace(request.CalleeURA),
		"subject_ura":    strings.TrimSpace(request.SubjectURA),
		"ability_ura":    strings.TrimSpace(request.AbilityURA),
		"action":         strings.TrimSpace(request.Action),
	}
	optionalStringArg(wire, "caller_ura", request.CallerURA)
	optionalStringArg(wire, "token_id", request.TokenID)
	optionalStringArg(wire, "token_class", request.TokenClass)
	optionalStringArg(wire, "nonce", request.Nonce)
	optionalStringArg(wire, "canonical_hash", request.CanonicalHash)
	if len(request.RequestedLifetimes) != 0 {
		wire["requested_lifetimes"] = request.RequestedLifetimes
	}
	optionalStringArg(wire, "status", request.Status)
	optionalStringArg(wire, "created_at", request.CreatedAt)
	optionalStringArg(wire, "expires_at", request.ExpiresAt)
	optionalStringArg(wire, "resolver_ura", request.ResolverURA)
	optionalStringArg(wire, "resolved_lifetime", request.ResolvedLifetime)
	optionalStringArg(wire, "created_grant_id", request.CreatedGrantID)
	optionalStringArg(wire, "authority_proof_id", request.AuthorityProofID)
	optionalStringArg(wire, "resolved_at", request.ResolvedAt)
	optionalStringArg(wire, "decision_reason", request.DecisionReason)
	return wire
}

func accessControlAuthorityProofWire(proof AccessControlAuthorityProof) (map[string]any, error) {
	if strings.TrimSpace(proof.ProofID) == "" {
		return nil, invalidAccessControl("authority_proof.proof_id is required", nil)
	}
	if _, err := accessControlUserIDFromURA(proof.OwnerURA, "authority_proof.owner_ura"); err != nil {
		return nil, err
	}
	if proof.PrincipalKind == "" {
		proof.PrincipalKind = AccessControlPrincipalUser
	}
	if _, err := accessControlPrincipalID(proof.PrincipalKind, proof.PrincipalURA, proof.PrincipalID); err != nil {
		return nil, err
	}
	wire := map[string]any{
		"proof_id":       strings.TrimSpace(proof.ProofID),
		"owner_ura":      strings.TrimSpace(proof.OwnerURA),
		"principal_kind": string(proof.PrincipalKind),
		"principal_ura":  strings.TrimSpace(proof.PrincipalURA),
	}
	optionalStringArg(wire, "grant_id", proof.GrantID)
	optionalStringArg(wire, "permission_request_id", proof.PermissionRequestID)
	optionalStringArg(wire, "token_id", proof.TokenID)
	optionalStringArg(wire, "callee_ura", proof.CalleeURA)
	optionalStringArg(wire, "subject_ura", proof.SubjectURA)
	optionalStringArg(wire, "ability_ura", proof.AbilityURA)
	optionalStringArg(wire, "action", proof.Action)
	optionalStringArg(wire, "nonce", proof.Nonce)
	optionalStringArg(wire, "canonical_hash", proof.CanonicalHash)
	optionalStringArg(wire, "canonical_invocation_hash", proof.CanonicalInvocationHash)
	optionalStringArg(wire, "session_id", proof.SessionID)
	optionalStringArg(wire, "session_owner_ura", proof.SessionOwnerURA)
	if len(proof.AllowedFollowupAbilities) != 0 {
		wire["allowed_followup_abilities"] = proof.AllowedFollowupAbilities
	}
	optionalStringArg(wire, "session_expires_at", proof.SessionExpiresAt)
	optionalStringArg(wire, "issuer_ura", proof.IssuerURA)
	optionalStringArg(wire, "audience_ura", proof.AudienceURA)
	optionalStringArg(wire, "issued_at", proof.IssuedAt)
	optionalStringArg(wire, "expires_at", proof.ExpiresAt)
	optionalStringArg(wire, "signature", proof.Signature)
	optionalStringArg(wire, "verification_key_id", proof.VerificationKeyID)
	return wire, nil
}

func accessControlUserIDFromURA(raw string, field string) (string, error) {
	parts, err := ParseURAParts(strings.TrimSpace(raw))
	if err != nil {
		return "", invalidAccessControl(field+" must be a canonical User URA", err)
	}
	if parts.Kind != URAKindUser || strings.TrimSpace(parts.UserID) == "" {
		return "", invalidAccessControl(field+" must be a canonical User URA", nil)
	}
	return strings.TrimSpace(parts.UserID), nil
}

func accessControlPrincipalID(kind AccessControlPrincipalKind, principalURA string, principalID string) (string, error) {
	principalURA = strings.TrimSpace(principalURA)
	principalID = strings.TrimSpace(principalID)
	if kind == "" {
		kind = AccessControlPrincipalUser
	}
	if principalURA == "" {
		if principalID != "" {
			return "", invalidAccessControl("principal_ura is required when principal_id is provided", nil)
		}
		return "", nil
	}
	parts, err := ParseURAParts(principalURA)
	if err != nil {
		return "", invalidAccessControl("principal_ura must be canonical", err)
	}
	var canonical string
	switch kind {
	case AccessControlPrincipalUser:
		if parts.Kind != URAKindUser {
			return "", invalidAccessControl("principal_ura for user principal must be a User URA", nil)
		}
		canonical = strings.TrimSpace(parts.UserID)
	case AccessControlPrincipalToken:
		if principalID == "" {
			return "", invalidAccessControl("principal_id is required for token principals", nil)
		}
		canonical = principalID
	default:
		canonical = principalURA
	}
	if principalID != "" && principalID != canonical {
		return "", invalidAccessControl("principal_id must match principal_ura", nil)
	}
	return canonical, nil
}

func accessControlGrantFromMap(raw map[string]any) (AccessControlGrant, error) {
	grant := AccessControlGrant{
		GrantID:             stringFromMap(raw, "grant_id"),
		OwnerURA:            stringFromMap(raw, "owner_ura"),
		PrincipalKind:       AccessControlPrincipalKind(stringFromMap(raw, "principal_kind")),
		PrincipalID:         stringFromMap(raw, "principal_id"),
		PrincipalURA:        stringFromMap(raw, "principal_ura"),
		TokenID:             stringFromMap(raw, "token_id"),
		TokenClass:          stringFromMap(raw, "token_class"),
		CalleeURA:           stringFromMap(raw, "callee_ura"),
		AbilityURAPattern:   stringFromMap(raw, "ability_ura_pattern"),
		SubjectURAPattern:   stringFromMap(raw, "subject_ura_pattern"),
		Actions:             stringSliceFromMap(raw, "actions"),
		Effect:              AccessControlEffect(stringFromMap(raw, "effect")),
		Lifetime:            stringFromMap(raw, "lifetime"),
		State:               AccessControlGrantState(stringFromMap(raw, "state")),
		CreatedBy:           stringFromMap(raw, "created_by"),
		CreatedAt:           stringFromMap(raw, "created_at"),
		UpdatedAt:           stringFromMap(raw, "updated_at"),
		ExpiresAt:           stringFromMap(raw, "expires_at"),
		ReviewRequiredAfter: stringFromMap(raw, "review_required_after"),
		LastReviewedAt:      stringFromMap(raw, "last_reviewed_at"),
		LastUsedAt:          stringFromMap(raw, "last_used_at"),
		RevokedAt:           stringFromMap(raw, "revoked_at"),
		RevokedBy:           stringFromMap(raw, "revoked_by"),
		RevocationReason:    stringFromMap(raw, "revocation_reason"),
		Reason:              stringFromMap(raw, "reason"),
		Constraints:         mapFromMap(raw, "constraints"),
		AuthorityProofID:    stringFromMap(raw, "authority_proof_id"),
		SourceRequestID:     stringFromMap(raw, "source_request_id"),
		InvocationTemplate:  mapFromMap(raw, "invocation_template"),
	}
	if grant.GrantID == "" {
		return AccessControlGrant{}, invalidAccessControl("grant_id is required in access-control projection", nil)
	}
	return grant, nil
}

func accessControlPermissionRequestFromMap(raw map[string]any) (AccessControlPermissionRequest, error) {
	request := AccessControlPermissionRequest{
		RequestID:          stringFromMap(raw, "request_id"),
		OwnerURA:           stringFromMap(raw, "owner_ura"),
		CallerURA:          stringFromMap(raw, "caller_ura"),
		PrincipalKind:      AccessControlPrincipalKind(stringFromMap(raw, "principal_kind")),
		PrincipalID:        stringFromMap(raw, "principal_id"),
		PrincipalURA:       stringFromMap(raw, "principal_ura"),
		TokenID:            stringFromMap(raw, "token_id"),
		TokenClass:         stringFromMap(raw, "token_class"),
		CalleeURA:          stringFromMap(raw, "callee_ura"),
		SubjectURA:         stringFromMap(raw, "subject_ura"),
		AbilityURA:         stringFromMap(raw, "ability_ura"),
		Action:             stringFromMap(raw, "action"),
		Nonce:              stringFromMap(raw, "nonce"),
		CanonicalHash:      stringFromMap(raw, "canonical_hash"),
		RequestedLifetimes: stringSliceFromMap(raw, "requested_lifetimes"),
		Status:             stringFromMap(raw, "status"),
		CreatedAt:          stringFromMap(raw, "created_at"),
		ExpiresAt:          stringFromMap(raw, "expires_at"),
		ResolverURA:        stringFromMap(raw, "resolver_ura"),
		ResolvedLifetime:   stringFromMap(raw, "resolved_lifetime"),
		CreatedGrantID:     stringFromMap(raw, "created_grant_id"),
		AuthorityProofID:   stringFromMap(raw, "authority_proof_id"),
		ResolvedAt:         stringFromMap(raw, "resolved_at"),
		DecisionReason:     stringFromMap(raw, "decision_reason"),
	}
	if request.RequestID == "" {
		return AccessControlPermissionRequest{}, invalidAccessControl("request_id is required in access-control permission request projection", nil)
	}
	return request, nil
}

func accessControlAuthorityProofFromMap(raw map[string]any) (AccessControlAuthorityProof, error) {
	proof := AccessControlAuthorityProof{
		ProofID:                  stringFromMap(raw, "proof_id"),
		GrantID:                  stringFromMap(raw, "grant_id"),
		PermissionRequestID:      stringFromMap(raw, "permission_request_id"),
		OwnerURA:                 stringFromMap(raw, "owner_ura"),
		PrincipalKind:            AccessControlPrincipalKind(stringFromMap(raw, "principal_kind")),
		PrincipalID:              stringFromMap(raw, "principal_id"),
		PrincipalURA:             stringFromMap(raw, "principal_ura"),
		TokenID:                  stringFromMap(raw, "token_id"),
		CalleeURA:                stringFromMap(raw, "callee_ura"),
		AbilityURA:               stringFromMap(raw, "ability_ura"),
		SubjectURA:               stringFromMap(raw, "subject_ura"),
		Action:                   stringFromMap(raw, "action"),
		Nonce:                    stringFromMap(raw, "nonce"),
		CanonicalHash:            stringFromMap(raw, "canonical_hash"),
		CanonicalInvocationHash:  stringFromMap(raw, "canonical_invocation_hash"),
		SessionID:                stringFromMap(raw, "session_id"),
		SessionOwnerURA:          stringFromMap(raw, "session_owner_ura"),
		AllowedFollowupAbilities: stringSliceFromMap(raw, "allowed_followup_abilities"),
		SessionExpiresAt:         stringFromMap(raw, "session_expires_at"),
		IssuerURA:                stringFromMap(raw, "issuer_ura"),
		AudienceURA:              stringFromMap(raw, "audience_ura"),
		IssuedAt:                 stringFromMap(raw, "issued_at"),
		ExpiresAt:                stringFromMap(raw, "expires_at"),
		Signature:                stringFromMap(raw, "signature"),
		VerificationKeyID:        stringFromMap(raw, "verification_key_id"),
	}
	if proof.ProofID == "" {
		return AccessControlAuthorityProof{}, invalidAccessControl("proof_id is required in access-control authority proof projection", nil)
	}
	return proof, nil
}

func accessControlPolicyDecisionFromMap(raw map[string]any) (AccessControlPolicyDecision, error) {
	decision := AccessControlPolicyDecision{
		Decision:         stringFromMap(raw, "decision"),
		Reason:           stringFromMap(raw, "reason"),
		OwnerUserID:      stringFromMap(raw, "owner_user_id"),
		OwnerURA:         stringFromMap(raw, "owner_ura"),
		OwnerSource:      stringFromMap(raw, "owner_source"),
		CallerURA:        stringFromMap(raw, "caller_ura"),
		PrincipalKind:    AccessControlPrincipalKind(stringFromMap(raw, "principal_kind")),
		PrincipalID:      stringFromMap(raw, "principal_id"),
		PrincipalURA:     stringFromMap(raw, "principal_ura"),
		TokenID:          stringFromMap(raw, "token_id"),
		CalleeURA:        stringFromMap(raw, "callee_ura"),
		AbilityURA:       stringFromMap(raw, "ability_ura"),
		SubjectURA:       stringFromMap(raw, "subject_ura"),
		Action:           stringFromMap(raw, "action"),
		GrantID:          stringFromMap(raw, "grant_id"),
		PolicyRuleID:     stringFromMap(raw, "policy_rule_id"),
		PromptRequestID:  stringFromMap(raw, "prompt_request_id"),
		CanonicalHash:    stringFromMap(raw, "canonical_hash"),
		SignatureKeyID:   stringFromMap(raw, "signature_key_id"),
		RejectorURA:      stringFromMap(raw, "rejector_ura"),
		AuthorityProofID: stringFromMap(raw, "authority_proof_id"),
		AuditWarnings:    stringSliceFromMap(raw, "audit_warnings"),
	}
	if decision.Decision == "" {
		return AccessControlPolicyDecision{}, invalidAccessControl("policy decision is required", nil)
	}
	return decision, nil
}

func accessControlSignatureDecisionFromMap(raw map[string]any) (AccessControlSignatureDecision, error) {
	decision := AccessControlSignatureDecision{
		Decision:                   stringFromMap(raw, "decision"),
		Reason:                     stringFromMap(raw, "reason"),
		CallerURA:                  stringFromMap(raw, "caller_ura"),
		CalleeURA:                  stringFromMap(raw, "callee_ura"),
		AbilityURA:                 stringFromMap(raw, "ability_ura"),
		SubjectURA:                 stringFromMap(raw, "subject_ura"),
		CanonicalHash:              stringFromMap(raw, "canonical_hash"),
		SignatureKeyID:             stringFromMap(raw, "signature_key_id"),
		PresentedPubkeyFingerprint: stringFromMap(raw, "presented_pubkey_fingerprint"),
		VerifierURA:                stringFromMap(raw, "verifier_ura"),
		RejectorURA:                stringFromMap(raw, "rejector_ura"),
	}
	if decision.Decision == "" {
		return AccessControlSignatureDecision{}, invalidAccessControl("signature decision is required", nil)
	}
	return decision, nil
}

func accessControlAbilityCallTraceFromMap(raw map[string]any) (AccessControlAbilityCallTrace, error) {
	trace := AccessControlAbilityCallTrace{
		InvocationID:       stringFromMap(raw, "invocation_id"),
		ParentInvocationID: stringFromMap(raw, "parent_invocation_id"),
		RootInvocationID:   stringFromMap(raw, "root_invocation_id"),
		CallerURA:          stringFromMap(raw, "caller_ura"),
		CalleeURA:          stringFromMap(raw, "callee_ura"),
		SubjectURA:         stringFromMap(raw, "subject_ura"),
		AbilityURA:         stringFromMap(raw, "ability_ura"),
		Action:             stringFromMap(raw, "action"),
		RouteRef:           stringFromMap(raw, "route_ref"),
		ExecutionHostURA:   stringFromMap(raw, "execution_host_ura"),
		RejectorURA:        stringFromMap(raw, "rejector_ura"),
		Stage:              stringFromMap(raw, "stage"),
		AuthorityProofID:   stringFromMap(raw, "authority_proof_id"),
		Redacted:           boolFromMap(raw, "redacted"),
		ChildFailureClass:  stringFromMap(raw, "child_failure_class"),
		RedactionReason:    stringFromMap(raw, "redaction_reason"),
	}
	if mapped := mapFromMap(raw, "signature_decision"); mapped != nil {
		decision, err := accessControlSignatureDecisionFromMap(mapped)
		if err != nil {
			return AccessControlAbilityCallTrace{}, err
		}
		trace.SignatureDecision = &decision
	}
	if mapped := mapFromMap(raw, "policy_decision"); mapped != nil {
		decision, err := accessControlPolicyDecisionFromMap(mapped)
		if err != nil {
			return AccessControlAbilityCallTrace{}, err
		}
		trace.PolicyDecision = &decision
	}
	if rawChildren, ok := raw["children"].([]any); ok {
		trace.Children = make([]AccessControlAbilityCallTrace, 0, len(rawChildren))
		for _, item := range rawChildren {
			child, err := accessControlAbilityCallTraceFromMap(requiredMapValue(item, "child_trace"))
			if err != nil {
				return AccessControlAbilityCallTrace{}, err
			}
			trace.Children = append(trace.Children, child)
		}
	}
	return trace, nil
}

func accessControlAdmissionExplainFromMap(raw map[string]any) (AccessControlAdmissionExplainResult, error) {
	result := AccessControlAdmissionExplainResult{
		ObserverURA:     stringFromMap(raw, "observer_ura"),
		Redacted:        boolFromMap(raw, "redacted"),
		AuthorityReason: stringFromMap(raw, "authority_reason"),
		RouteRef:        stringFromMap(raw, "route_ref"),
		RejectorURA:     stringFromMap(raw, "rejector_ura"),
		RedactionReason: stringFromMap(raw, "redaction_reason"),
	}
	if mapped := mapFromMap(raw, "root_trace"); mapped != nil {
		trace, err := accessControlAbilityCallTraceFromMap(mapped)
		if err != nil {
			return AccessControlAdmissionExplainResult{}, err
		}
		result.RootTrace = &trace
	}
	if mapped := mapFromMap(raw, "signature_decision"); mapped != nil {
		decision, err := accessControlSignatureDecisionFromMap(mapped)
		if err != nil {
			return AccessControlAdmissionExplainResult{}, err
		}
		result.SignatureDecision = &decision
	}
	if mapped := mapFromMap(raw, "policy_decision"); mapped != nil {
		decision, err := accessControlPolicyDecisionFromMap(mapped)
		if err != nil {
			return AccessControlAdmissionExplainResult{}, err
		}
		result.PolicyDecision = &decision
	}
	return result, nil
}

func requiredMap(raw map[string]any, key string) map[string]any {
	return requiredMapValue(raw[key], key)
}

func requiredMapValue(value any, key string) map[string]any {
	if mapped, ok := value.(map[string]any); ok && mapped != nil {
		return mapped
	}
	return map[string]any{}
}

func stringFromMap(raw map[string]any, key string) string {
	if value, ok := raw[key].(string); ok {
		return value
	}
	return ""
}

func boolFromMap(raw map[string]any, key string) bool {
	if value, ok := raw[key].(bool); ok {
		return value
	}
	return false
}

func stringSliceFromMap(raw map[string]any, key string) []string {
	values, ok := raw[key].([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(values))
	for _, item := range values {
		if value, ok := item.(string); ok {
			out = append(out, value)
		}
	}
	return out
}

func mapFromMap(raw map[string]any, key string) map[string]any {
	if value, ok := raw[key].(map[string]any); ok {
		return value
	}
	return nil
}

func optionalStringArg(args map[string]any, key string, value string) {
	if strings.TrimSpace(value) != "" {
		args[key] = strings.TrimSpace(value)
	}
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}

func firstNonEmptyPrincipalKind(values ...AccessControlPrincipalKind) AccessControlPrincipalKind {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

func invalidAccessControl(message string, cause error) error {
	return invalidProfilePayload(accessControlProfile, message, cause)
}
