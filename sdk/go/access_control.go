package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

const accessControlProfile = "access_control"

type PrincipalKind string

const (
	PrincipalUser       PrincipalKind = "user"
	PrincipalToken      PrincipalKind = "token"
	PrincipalHub        PrincipalKind = "hub"
	PrincipalDevice     PrincipalKind = "device"
	PrincipalService    PrincipalKind = "service"
	PrincipalAutomation PrincipalKind = "automation"
)

type TokenClass string

const (
	TokenHubLink        TokenClass = "hub_link"
	TokenBrowserSession TokenClass = "browser_session"
	TokenDevicePairing  TokenClass = "device_pairing"
	TokenAutomation     TokenClass = "automation"
	TokenThirdParty     TokenClass = "third_party"
	TokenService        TokenClass = "service"
)

type AccessAction string

const (
	AccessRead   AccessAction = "read"
	AccessInvoke AccessAction = "invoke"
	AccessStream AccessAction = "stream"
	AccessManage AccessAction = "manage"
	AccessGrant  AccessAction = "grant"
)

type PermissionEffect string

const (
	PermissionAllow PermissionEffect = "allow"
	PermissionDeny  PermissionEffect = "deny"
)

type PermissionGrantState string

const (
	PermissionGrantActive  PermissionGrantState = "active"
	PermissionGrantExpired PermissionGrantState = "expired"
	PermissionGrantRevoked PermissionGrantState = "revoked"
)

type PermissionGrant struct {
	GrantID             string               `json:"grant_id"`
	OwnerUserID         string               `json:"owner_user_id"`
	PrincipalKind       PrincipalKind        `json:"principal_kind"`
	PrincipalID         string               `json:"principal_id"`
	TokenID             string               `json:"token_id,omitempty"`
	TokenClass          TokenClass           `json:"token_class,omitempty"`
	CalleeURA           string               `json:"callee_ura,omitempty"`
	SubjectURAPattern   string               `json:"subject_ura_pattern,omitempty"`
	AbilityURAPattern   string               `json:"ability_ura_pattern,omitempty"`
	Actions             []AccessAction       `json:"actions"`
	Constraints         map[string]any       `json:"constraints,omitempty"`
	Effect              PermissionEffect     `json:"effect"`
	Lifetime            string               `json:"lifetime"`
	State               PermissionGrantState `json:"state"`
	ExpiresAt           string               `json:"expires_at,omitempty"`
	ReviewRequiredAfter string               `json:"review_required_after,omitempty"`
	LastReviewedAt      string               `json:"last_reviewed_at,omitempty"`
	LastUsedAt          string               `json:"last_used_at,omitempty"`
	CreatedBy           string               `json:"created_by"`
	CreatedAt           string               `json:"created_at"`
	UpdatedAt           string               `json:"updated_at,omitempty"`
	RevokedAt           string               `json:"revoked_at,omitempty"`
	Reason              string               `json:"reason,omitempty"`
}

type PolicyDecision struct {
	Decision         string        `json:"decision"`
	Reason           string        `json:"reason"`
	OwnerUserID      string        `json:"owner_user_id,omitempty"`
	OwnerSource      string        `json:"owner_source"`
	CallerURA        string        `json:"caller_ura"`
	PrincipalKind    PrincipalKind `json:"principal_kind"`
	PrincipalID      string        `json:"principal_id"`
	TokenID          string        `json:"token_id,omitempty"`
	CalleeURA        string        `json:"callee_ura"`
	SubjectURA       string        `json:"subject_ura"`
	AbilityURA       string        `json:"ability_ura"`
	Action           AccessAction  `json:"action"`
	RejectorURA      string        `json:"rejector_ura,omitempty"`
	PolicyRuleID     string        `json:"policy_rule_id,omitempty"`
	GrantID          string        `json:"grant_id,omitempty"`
	PromptRequestID  string        `json:"prompt_request_id,omitempty"`
	CanonicalHash    string        `json:"canonical_hash,omitempty"`
	SignatureKeyID   string        `json:"signature_key_id,omitempty"`
	AuthorityProofID string        `json:"authority_proof_id,omitempty"`
}

type SignatureDecision struct {
	Decision                   string `json:"decision"`
	Reason                     string `json:"reason"`
	CallerURA                  string `json:"caller_ura"`
	CalleeURA                  string `json:"callee_ura"`
	AbilityURA                 string `json:"ability_ura"`
	SubjectURA                 string `json:"subject_ura"`
	CanonicalHash              string `json:"canonical_hash"`
	SignatureKeyID             string `json:"signature_key_id,omitempty"`
	PresentedPubkeyFingerprint string `json:"presented_pubkey_fingerprint,omitempty"`
	VerifierURA                string `json:"verifier_ura"`
}

type AuthorityProof struct {
	ProofID                  string        `json:"proof_id"`
	GrantID                  string        `json:"grant_id,omitempty"`
	PermissionRequestID      string        `json:"permission_request_id,omitempty"`
	OwnerUserID              string        `json:"owner_user_id"`
	PrincipalKind            PrincipalKind `json:"principal_kind"`
	PrincipalID              string        `json:"principal_id"`
	TokenID                  string        `json:"token_id,omitempty"`
	CalleeURA                string        `json:"callee_ura"`
	SubjectURA               string        `json:"subject_ura"`
	AbilityURA               string        `json:"ability_ura"`
	Action                   AccessAction  `json:"action"`
	Nonce                    string        `json:"nonce,omitempty"`
	CanonicalHash            string        `json:"canonical_hash,omitempty"`
	SessionID                string        `json:"session_id,omitempty"`
	SessionOwnerUserID       string        `json:"session_owner_user_id,omitempty"`
	AllowedFollowupAbilities []string      `json:"allowed_followup_abilities,omitempty"`
	SessionExpiresAt         string        `json:"session_expires_at,omitempty"`
	IssuedAt                 string        `json:"issued_at"`
	ExpiresAt                string        `json:"expires_at"`
	IssuerURA                string        `json:"issuer_ura"`
	AudienceURA              string        `json:"audience_ura"`
	Signature                string        `json:"signature"`
}

type PermissionRequest struct {
	RequestID          string        `json:"request_id"`
	OwnerUserID        string        `json:"owner_user_id"`
	CallerURA          string        `json:"caller_ura"`
	PrincipalKind      PrincipalKind `json:"principal_kind"`
	PrincipalID        string        `json:"principal_id"`
	TokenID            string        `json:"token_id,omitempty"`
	TokenClass         TokenClass    `json:"token_class,omitempty"`
	CalleeURA          string        `json:"callee_ura"`
	SubjectURA         string        `json:"subject_ura"`
	AbilityURA         string        `json:"ability_ura"`
	Action             AccessAction  `json:"action"`
	Nonce              string        `json:"nonce,omitempty"`
	CanonicalHash      string        `json:"canonical_hash,omitempty"`
	RequestedLifetimes []string      `json:"requested_lifetimes"`
	Status             string        `json:"status"`
	CreatedAt          string        `json:"created_at"`
	ExpiresAt          string        `json:"expires_at"`
	ResolverURA        string        `json:"resolver_ura,omitempty"`
	ResolvedLifetime   string        `json:"resolved_lifetime,omitempty"`
	CreatedGrantID     string        `json:"created_grant_id,omitempty"`
	AuthorityProofID   string        `json:"authority_proof_id,omitempty"`
	ResolvedAt         string        `json:"resolved_at,omitempty"`
	DecisionReason     string        `json:"decision_reason,omitempty"`
}

type AbilityCallTrace struct {
	InvocationID       string             `json:"invocation_id"`
	ParentInvocationID string             `json:"parent_invocation_id,omitempty"`
	RootInvocationID   string             `json:"root_invocation_id"`
	CallerURA          string             `json:"caller_ura"`
	CalleeURA          string             `json:"callee_ura"`
	SubjectURA         string             `json:"subject_ura"`
	AbilityURA         string             `json:"ability_ura"`
	Action             AccessAction       `json:"action"`
	RouteRef           string             `json:"route_ref,omitempty"`
	ExecutionHostURA   string             `json:"execution_host_ura,omitempty"`
	RejectorURA        string             `json:"rejector_ura,omitempty"`
	Stage              string             `json:"stage"`
	SignatureDecision  *SignatureDecision `json:"signature_decision,omitempty"`
	PolicyDecision     *PolicyDecision    `json:"policy_decision,omitempty"`
	AuthorityProofID   string             `json:"authority_proof_id,omitempty"`
	Redacted           bool               `json:"redacted,omitempty"`
	ChildFailureClass  string             `json:"child_failure_class,omitempty"`
	RedactionReason    string             `json:"redaction_reason,omitempty"`
	Children           []AbilityCallTrace `json:"children,omitempty"`
}

type AdmissionExplainResult struct {
	ObserverURA       string             `json:"observer_ura"`
	Redacted          bool               `json:"redacted"`
	RootTrace         *AbilityCallTrace  `json:"root_trace,omitempty"`
	SignatureDecision *SignatureDecision `json:"signature_decision,omitempty"`
	PolicyDecision    *PolicyDecision    `json:"policy_decision,omitempty"`
	AuthorityReason   string             `json:"authority_reason,omitempty"`
	RouteRef          string             `json:"route_ref,omitempty"`
	RejectorURA       string             `json:"rejector_ura,omitempty"`
	RedactionReason   string             `json:"redaction_reason,omitempty"`
}

type AuthorityBindingGrantResult struct {
	Grant            PermissionGrant `json:"grant"`
	IdempotentReplay bool            `json:"idempotent_replay"`
	AuditRecordID    string          `json:"audit_record_id"`
}

type PermissionRequestResolutionResult struct {
	Request          PermissionRequest `json:"request"`
	CreatedGrant     *PermissionGrant  `json:"created_grant,omitempty"`
	AuthorityProof   *AuthorityProof   `json:"authority_proof,omitempty"`
	IdempotentReplay bool              `json:"idempotent_replay"`
}

// AccessControlCarrierBase is the complete Runtime Core carrier context shared
// by access-control system abilities. The SDK owns this invocation envelope;
// product code supplies values but never assembles daemon ability payloads.
type AccessControlCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

type AuthorityBindingGrantRequest struct {
	Carrier  AccessControlCarrierBase `json:"carrier"`
	Grant    PermissionGrant          `json:"grant"`
	ActorURA string                   `json:"actor_ura"`
}

type AuthorityBindingRevokeRequest struct {
	Carrier     AccessControlCarrierBase `json:"carrier"`
	OwnerUserID string                   `json:"owner_user_id"`
	GrantID     string                   `json:"grant_id"`
	ActorURA    string                   `json:"actor_ura"`
	Reason      string                   `json:"reason,omitempty"`
}

type AuthorityBindingListRequest struct {
	Carrier       AccessControlCarrierBase `json:"carrier"`
	OwnerUserID   string                   `json:"owner_user_id,omitempty"`
	PrincipalKind PrincipalKind            `json:"principal_kind,omitempty"`
	PrincipalID   string                   `json:"principal_id,omitempty"`
	TokenID       string                   `json:"token_id,omitempty"`
	CalleeURA     string                   `json:"callee_ura,omitempty"`
	SubjectURA    string                   `json:"subject_ura,omitempty"`
	AbilityURA    string                   `json:"ability_ura,omitempty"`
	Action        AccessAction             `json:"action,omitempty"`
	State         string                   `json:"state,omitempty"`
	Limit         int                      `json:"limit,omitempty"`
	Cursor        string                   `json:"cursor,omitempty"`
}

type AuthorityBindingCheckRequest struct {
	Carrier        AccessControlCarrierBase `json:"carrier"`
	CallerURA      string                   `json:"caller_ura"`
	PrincipalKind  PrincipalKind            `json:"principal_kind"`
	PrincipalID    string                   `json:"principal_id"`
	TokenID        string                   `json:"token_id,omitempty"`
	CalleeURA      string                   `json:"callee_ura"`
	SubjectURA     string                   `json:"subject_ura"`
	AbilityURA     string                   `json:"ability_ura"`
	Action         AccessAction             `json:"action"`
	CanonicalHash  string                   `json:"canonical_hash,omitempty"`
	SignatureKeyID string                   `json:"signature_key_id,omitempty"`
}

type PolicyRequestCreateRequest struct {
	Carrier  AccessControlCarrierBase `json:"carrier"`
	Request  PermissionRequest        `json:"request"`
	ActorURA string                   `json:"actor_ura"`
}

type PolicyRequestResolveRequest struct {
	Carrier        AccessControlCarrierBase `json:"carrier"`
	Request        PermissionRequest        `json:"request"`
	CreatedGrant   *PermissionGrant         `json:"created_grant,omitempty"`
	AuthorityProof *AuthorityProof          `json:"authority_proof,omitempty"`
	ActorURA       string                   `json:"actor_ura"`
}

type PolicyRequestListRequest struct {
	Carrier       AccessControlCarrierBase `json:"carrier"`
	OwnerUserID   string                   `json:"owner_user_id,omitempty"`
	PrincipalKind PrincipalKind            `json:"principal_kind,omitempty"`
	PrincipalID   string                   `json:"principal_id,omitempty"`
	TokenID       string                   `json:"token_id,omitempty"`
	Status        string                   `json:"status,omitempty"`
	Limit         int                      `json:"limit,omitempty"`
	Cursor        string                   `json:"cursor,omitempty"`
}

type AdmissionExplainRequest struct {
	Carrier      AccessControlCarrierBase `json:"carrier"`
	ObserverURA  string                   `json:"observer_ura"`
	InvocationID string                   `json:"invocation_id,omitempty"`
	TraceID      string                   `json:"trace_id,omitempty"`
	RootID       string                   `json:"root_id,omitempty"`
}

type AccessControlTransport interface {
	GrantAuthorityBinding(ctx context.Context, requestJSON []byte) ([]byte, error)
	RevokeAuthorityBinding(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAuthorityBindings(ctx context.Context, requestJSON []byte) ([]byte, error)
	CheckAuthorityBinding(ctx context.Context, requestJSON []byte) ([]byte, error)
	CreatePolicyRequest(ctx context.Context, requestJSON []byte) ([]byte, error)
	ResolvePolicyRequest(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListPolicyRequests(ctx context.Context, requestJSON []byte) ([]byte, error)
	ExplainAdmission(ctx context.Context, requestJSON []byte) ([]byte, error)
}

type AccessControlClient struct {
	transport AccessControlTransport
}

func NewAccessControlClient(transport AccessControlTransport) (*AccessControlClient, error) {
	if transport == nil {
		return nil, invalidProfileClient(accessControlProfile, "access-control transport is required")
	}
	return &AccessControlClient{transport: transport}, nil
}

func (c *AccessControlClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(accessControlProfile, "access-control client is not initialized")
	}
	if ctx == nil {
		return invalidProfileClient(accessControlProfile, "context is required")
	}
	if closer, ok := c.transport.(interface{ Close(context.Context) error }); ok {
		return closer.Close(ctx)
	}
	return nil
}

func (c *AccessControlClient) Grant(ctx context.Context, grant PermissionGrant, actorURA string) (AuthorityBindingGrantResult, error) {
	var out AuthorityBindingGrantResult
	err := c.roundTrip(ctx, map[string]any{"grant": grant, "actor_ura": actorURA}, c.transport.GrantAuthorityBinding, &out)
	return out, err
}

func (c *AccessControlClient) GrantWithRequest(ctx context.Context, request AuthorityBindingGrantRequest) (AuthorityBindingGrantResult, error) {
	var out AuthorityBindingGrantResult
	err := c.roundTrip(ctx, request, c.transport.GrantAuthorityBinding, &out)
	return out, err
}

func (c *AccessControlClient) Revoke(ctx context.Context, ownerUserID, grantID, actorURA, reason string) (PermissionGrant, error) {
	var out struct {
		Grant PermissionGrant `json:"grant"`
	}
	err := c.roundTrip(ctx, map[string]any{"owner_user_id": ownerUserID, "grant_id": grantID, "actor_ura": actorURA, "reason": reason}, c.transport.RevokeAuthorityBinding, &out)
	return out.Grant, err
}

func (c *AccessControlClient) RevokeWithRequest(ctx context.Context, request AuthorityBindingRevokeRequest) (PermissionGrant, error) {
	var out struct {
		Grant PermissionGrant `json:"grant"`
	}
	err := c.roundTrip(ctx, request, c.transport.RevokeAuthorityBinding, &out)
	return out.Grant, err
}

func (c *AccessControlClient) ListGrants(ctx context.Context, request map[string]any) ([]PermissionGrant, error) {
	var out struct {
		Grants []PermissionGrant `json:"grants"`
	}
	err := c.roundTrip(ctx, request, c.transport.ListAuthorityBindings, &out)
	return out.Grants, err
}

func (c *AccessControlClient) ListGrantsWithRequest(ctx context.Context, request AuthorityBindingListRequest) ([]PermissionGrant, error) {
	var out struct {
		Grants []PermissionGrant `json:"grants"`
	}
	err := c.roundTrip(ctx, request, c.transport.ListAuthorityBindings, &out)
	return out.Grants, err
}

func (c *AccessControlClient) Check(ctx context.Context, request map[string]any) (PolicyDecision, error) {
	var out struct {
		PolicyDecision PolicyDecision `json:"policy_decision"`
	}
	err := c.roundTrip(ctx, request, c.transport.CheckAuthorityBinding, &out)
	return out.PolicyDecision, err
}

func (c *AccessControlClient) CheckWithRequest(ctx context.Context, request AuthorityBindingCheckRequest) (PolicyDecision, error) {
	var out struct {
		PolicyDecision PolicyDecision `json:"policy_decision"`
	}
	err := c.roundTrip(ctx, request, c.transport.CheckAuthorityBinding, &out)
	return out.PolicyDecision, err
}

func (c *AccessControlClient) CreateRequest(ctx context.Context, request PermissionRequest, actorURA string) (PermissionRequest, error) {
	var out struct {
		Request PermissionRequest `json:"request"`
	}
	err := c.roundTrip(ctx, map[string]any{"request": request, "actor_ura": actorURA}, c.transport.CreatePolicyRequest, &out)
	return out.Request, err
}

func (c *AccessControlClient) CreateRequestWithCarrier(ctx context.Context, request PolicyRequestCreateRequest) (PermissionRequest, error) {
	var out struct {
		Request PermissionRequest `json:"request"`
	}
	err := c.roundTrip(ctx, request, c.transport.CreatePolicyRequest, &out)
	return out.Request, err
}

func (c *AccessControlClient) ResolveRequest(ctx context.Context, request PermissionRequest, actorURA string) (PermissionRequest, error) {
	result, err := c.ResolveRequestResult(ctx, request, actorURA)
	return result.Request, err
}

func (c *AccessControlClient) ResolveRequestResult(ctx context.Context, request PermissionRequest, actorURA string) (PermissionRequestResolutionResult, error) {
	var out PermissionRequestResolutionResult
	err := c.roundTrip(ctx, map[string]any{"request": request, "actor_ura": actorURA}, c.transport.ResolvePolicyRequest, &out)
	return out, err
}

func (c *AccessControlClient) ResolveRequestWithGrant(ctx context.Context, request PermissionRequest, grant PermissionGrant, actorURA string) (PermissionRequestResolutionResult, error) {
	var out PermissionRequestResolutionResult
	err := c.roundTrip(
		ctx,
		map[string]any{"request": request, "created_grant": grant, "actor_ura": actorURA},
		c.transport.ResolvePolicyRequest,
		&out,
	)
	return out, err
}

func (c *AccessControlClient) ResolveRequestWithAuthorityProof(ctx context.Context, request PermissionRequest, proof AuthorityProof, actorURA string) (PermissionRequestResolutionResult, error) {
	var out PermissionRequestResolutionResult
	err := c.roundTrip(
		ctx,
		map[string]any{"request": request, "authority_proof": proof, "actor_ura": actorURA},
		c.transport.ResolvePolicyRequest,
		&out,
	)
	return out, err
}

func (c *AccessControlClient) ResolveRequestWithCarrier(ctx context.Context, request PolicyRequestResolveRequest) (PermissionRequestResolutionResult, error) {
	var out PermissionRequestResolutionResult
	err := c.roundTrip(ctx, request, c.transport.ResolvePolicyRequest, &out)
	return out, err
}

func (c *AccessControlClient) ListRequests(ctx context.Context, request map[string]any) ([]PermissionRequest, error) {
	var out struct {
		Requests []PermissionRequest `json:"requests"`
	}
	err := c.roundTrip(ctx, request, c.transport.ListPolicyRequests, &out)
	return out.Requests, err
}

func (c *AccessControlClient) ListRequestsWithRequest(ctx context.Context, request PolicyRequestListRequest) ([]PermissionRequest, error) {
	var out struct {
		Requests []PermissionRequest `json:"requests"`
	}
	err := c.roundTrip(ctx, request, c.transport.ListPolicyRequests, &out)
	return out.Requests, err
}

func (c *AccessControlClient) Explain(ctx context.Context, request map[string]any) (AdmissionExplainResult, error) {
	var out AdmissionExplainResult
	err := c.roundTrip(ctx, request, c.transport.ExplainAdmission, &out)
	return out, err
}

func (c *AccessControlClient) ExplainWithRequest(ctx context.Context, request AdmissionExplainRequest) (AdmissionExplainResult, error) {
	var out AdmissionExplainResult
	err := c.roundTrip(ctx, request, c.transport.ExplainAdmission, &out)
	return out, err
}

func (c *AccessControlClient) roundTrip(ctx context.Context, input any, call func(context.Context, []byte) ([]byte, error), output any) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(accessControlProfile, "access-control client is not initialized")
	}
	requestJSON, err := json.Marshal(input)
	if err != nil {
		return invalidProfilePayload(accessControlProfile, fmt.Sprintf("encode access-control request: %v", err), err)
	}
	raw, err := call(ctx, requestJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return withProfileErrorDetails(sdkErr, accessControlProfile)
		}
		return transportProfileError(accessControlProfile, "access-control transport failed", err)
	}
	if err := json.Unmarshal(raw, output); err != nil {
		return invalidProfilePayload(accessControlProfile, fmt.Sprintf("decode access-control response: %v", err), err)
	}
	return nil
}
