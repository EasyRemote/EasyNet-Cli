package easynet

import (
	"context"
	"strings"
)

const (
	accessControlProfile = "access_control"

	accessControlAbilityGrant  = "authority.binding.grant"
	accessControlAbilityRevoke = "authority.binding.revoke"
	accessControlAbilityList   = "authority.binding.list"
	accessControlAbilityCheck  = "authority.binding.check"
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
	AccessControlPrincipalUser  AccessControlPrincipalKind = "user"
	AccessControlPrincipalToken AccessControlPrincipalKind = "token"
	AccessControlPrincipalAgent AccessControlPrincipalKind = "agent"
)

type AccessControlGrant struct {
	GrantID            string                     `json:"grant_id"`
	OwnerURA           string                     `json:"owner_ura,omitempty"`
	PrincipalKind      AccessControlPrincipalKind `json:"principal_kind"`
	PrincipalURA       string                     `json:"principal_ura,omitempty"`
	TokenID            string                     `json:"token_id,omitempty"`
	CalleeURA          string                     `json:"callee_ura,omitempty"`
	AbilityURAPattern  string                     `json:"ability_ura_pattern,omitempty"`
	SubjectURAPattern  string                     `json:"subject_ura_pattern,omitempty"`
	Actions            []string                   `json:"actions"`
	Effect             AccessControlEffect        `json:"effect"`
	State              AccessControlGrantState    `json:"state"`
	CreatedBy          string                     `json:"created_by"`
	CreatedAt          string                     `json:"created_at,omitempty"`
	ExpiresAt          string                     `json:"expires_at,omitempty"`
	RevokedAt          string                     `json:"revoked_at,omitempty"`
	RevokedBy          string                     `json:"revoked_by,omitempty"`
	RevocationReason   string                     `json:"revocation_reason,omitempty"`
	Constraints        map[string]any             `json:"constraints,omitempty"`
	AuthorityProofID   string                     `json:"authority_proof_id,omitempty"`
	SourceRequestID    string                     `json:"source_request_id,omitempty"`
	InvocationTemplate map[string]any             `json:"invocation_template,omitempty"`
}

type AccessControlPolicyDecision struct {
	Decision         string                     `json:"decision"`
	Reason           string                     `json:"reason,omitempty"`
	OwnerURA         string                     `json:"owner_ura,omitempty"`
	OwnerSource      string                     `json:"owner_source,omitempty"`
	PrincipalKind    AccessControlPrincipalKind `json:"principal_kind,omitempty"`
	PrincipalURA     string                     `json:"principal_ura,omitempty"`
	TokenID          string                     `json:"token_id,omitempty"`
	CalleeURA        string                     `json:"callee_ura,omitempty"`
	AbilityURA       string                     `json:"ability_ura,omitempty"`
	SubjectURA       string                     `json:"subject_ura,omitempty"`
	Action           string                     `json:"action,omitempty"`
	GrantID          string                     `json:"grant_id,omitempty"`
	RejectorURA      string                     `json:"rejector_ura,omitempty"`
	AuthorityProofID string                     `json:"authority_proof_id,omitempty"`
	AuditWarnings    []string                   `json:"audit_warnings,omitempty"`
}

type AccessControlSignatureDecision struct {
	Decision       string `json:"decision"`
	Reason         string `json:"reason,omitempty"`
	CallerURA      string `json:"caller_ura,omitempty"`
	CalleeURA      string `json:"callee_ura,omitempty"`
	AbilityURA     string `json:"ability_ura,omitempty"`
	SubjectURA     string `json:"subject_ura,omitempty"`
	CanonicalHash  string `json:"canonical_hash,omitempty"`
	SignatureKeyID string `json:"signature_key_id,omitempty"`
	RejectorURA    string `json:"rejector_ura,omitempty"`
}

type AccessControlAuthorityProof struct {
	ProofID                 string                     `json:"proof_id"`
	GrantID                 string                     `json:"grant_id,omitempty"`
	OwnerURA                string                     `json:"owner_ura,omitempty"`
	PrincipalKind           AccessControlPrincipalKind `json:"principal_kind,omitempty"`
	PrincipalURA            string                     `json:"principal_ura,omitempty"`
	AbilityURA              string                     `json:"ability_ura,omitempty"`
	SubjectURA              string                     `json:"subject_ura,omitempty"`
	Action                  string                     `json:"action,omitempty"`
	IssuerURA               string                     `json:"issuer_ura,omitempty"`
	IssuedAt                string                     `json:"issued_at,omitempty"`
	ExpiresAt               string                     `json:"expires_at,omitempty"`
	CanonicalInvocationHash string                     `json:"canonical_invocation_hash,omitempty"`
	Signature               string                     `json:"signature,omitempty"`
	VerificationKeyID       string                     `json:"verification_key_id,omitempty"`
}

type AccessControlGrantRequest struct {
	Call         RuntimeCallContext `json:"call"`
	Grant        AccessControlGrant `json:"grant"`
	OwnerURA     string             `json:"owner_ura,omitempty"`
	PrincipalURA string             `json:"principal_ura,omitempty"`
	ActorURA     string             `json:"actor_ura,omitempty"`
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
	PrincipalURA      string                     `json:"principal_ura,omitempty"`
	TokenID           string                     `json:"token_id,omitempty"`
	CalleeURA         string                     `json:"callee_ura,omitempty"`
	AbilityURAPattern string                     `json:"ability_ura_pattern,omitempty"`
	SubjectURAPattern string                     `json:"subject_ura_pattern,omitempty"`
	Action            string                     `json:"action,omitempty"`
	Effect            AccessControlEffect        `json:"effect,omitempty"`
	State             AccessControlGrantState    `json:"state,omitempty"`
	Limit             uint32                     `json:"limit,omitempty"`
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
	Grant         AccessControlGrant `json:"grant"`
	AuditRecordID string             `json:"audit_record_id,omitempty"`
}

type AccessControlListResult struct {
	Grants []AccessControlGrant `json:"grants"`
}

type AccessControlCheckResult struct {
	PolicyDecision AccessControlPolicyDecision `json:"policy_decision"`
}

type AccessControlProvider interface {
	Grant(context.Context, AccessControlGrantRequest) (AccessControlGrantResult, error)
	Revoke(context.Context, AccessControlRevokeRequest) (AccessControlGrant, error)
	List(context.Context, AccessControlListRequest) (AccessControlListResult, error)
	Check(context.Context, AccessControlCheckRequest) (AccessControlCheckResult, error)
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
	return AccessControlGrantResult{Grant: grant, AuditRecordID: stringFromMap(output, "audit_record_id")}, nil
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

func accessControlGrantArgs(request AccessControlGrantRequest) (AccessControlGrantRequest, map[string]any, error) {
	grant, err := normalizeAccessControlGrant(request.Grant, request.OwnerURA, request.PrincipalURA)
	if err != nil {
		return AccessControlGrantRequest{}, nil, err
	}
	request.Grant = grant
	ownerUserID, _ := accessControlUserIDFromURA(grant.OwnerURA, "owner_ura")
	principalID, err := accessControlPrincipalID(grant.PrincipalKind, grant.PrincipalURA, "")
	if err != nil {
		return AccessControlGrantRequest{}, nil, err
	}
	wireGrant := accessControlGrantWire(grant, ownerUserID, principalID)
	args := map[string]any{"grant": wireGrant, "owner_ura": grant.OwnerURA}
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
	ownerUserID, err := accessControlUserIDFromURA(ownerURA, "owner_ura")
	if err != nil {
		return AccessControlRevokeRequest{}, nil, err
	}
	grantID := strings.TrimSpace(request.GrantID)
	if grantID == "" {
		return AccessControlRevokeRequest{}, nil, invalidAccessControl("grant_id is required", nil)
	}
	request.OwnerURA = ownerURA
	args := map[string]any{"owner_ura": ownerURA, "owner_user_id": ownerUserID, "grant_id": grantID}
	if actor := strings.TrimSpace(request.ActorURA); actor != "" {
		args["actor_ura"] = actor
	}
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
	ownerUserID, err := accessControlUserIDFromURA(ownerURA, "owner_ura")
	if err != nil {
		return AccessControlListRequest{}, nil, err
	}
	principalID, err := accessControlPrincipalID(request.PrincipalKind, request.PrincipalURA, "")
	if err != nil {
		return AccessControlListRequest{}, nil, err
	}
	args := map[string]any{"owner_ura": ownerURA, "owner_user_id": ownerUserID}
	if request.PrincipalKind != "" {
		args["principal_kind"] = string(request.PrincipalKind)
	}
	if principalID != "" {
		args["principal_id"] = principalID
	}
	if principalURA := strings.TrimSpace(request.PrincipalURA); principalURA != "" {
		args["principal_ura"] = principalURA
	}
	optionalStringArg(args, "token_id", request.TokenID)
	optionalStringArg(args, "callee_ura", request.CalleeURA)
	optionalStringArg(args, "ability_ura_pattern", request.AbilityURAPattern)
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
	request.OwnerURA = ownerURA
	return request, args, nil
}

func accessControlCheckArgs(request AccessControlCheckRequest) (AccessControlCheckRequest, map[string]any, error) {
	ownerURA := strings.TrimSpace(request.OwnerURA)
	if ownerURA == "" {
		return AccessControlCheckRequest{}, nil, invalidAccessControl("owner_ura is required", nil)
	}
	ownerUserID, err := accessControlUserIDFromURA(ownerURA, "owner_ura")
	if err != nil {
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
		"owner_user_id":                 ownerUserID,
		"principal_kind":                string(request.PrincipalKind),
		"principal_id":                  principalID,
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
	if _, err := accessControlPrincipalID(grant.PrincipalKind, grant.PrincipalURA, ""); err != nil {
		return AccessControlGrant{}, err
	}
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

func accessControlGrantWire(grant AccessControlGrant, ownerUserID string, principalID string) map[string]any {
	wire := map[string]any{
		"grant_id":       strings.TrimSpace(grant.GrantID),
		"owner_user_id":  ownerUserID,
		"owner_ura":      strings.TrimSpace(grant.OwnerURA),
		"principal_kind": string(grant.PrincipalKind),
		"principal_id":   principalID,
		"principal_ura":  strings.TrimSpace(grant.PrincipalURA),
		"actions":        grant.Actions,
		"effect":         string(grant.Effect),
		"state":          string(grant.State),
		"created_by":     strings.TrimSpace(grant.CreatedBy),
		"constraints":    grant.Constraints,
	}
	optionalStringArg(wire, "token_id", grant.TokenID)
	optionalStringArg(wire, "callee_ura", grant.CalleeURA)
	optionalStringArg(wire, "ability_ura_pattern", grant.AbilityURAPattern)
	optionalStringArg(wire, "subject_ura_pattern", grant.SubjectURAPattern)
	optionalStringArg(wire, "created_at", grant.CreatedAt)
	optionalStringArg(wire, "expires_at", grant.ExpiresAt)
	optionalStringArg(wire, "revoked_at", grant.RevokedAt)
	optionalStringArg(wire, "revoked_by", grant.RevokedBy)
	optionalStringArg(wire, "revocation_reason", grant.RevocationReason)
	optionalStringArg(wire, "authority_proof_id", grant.AuthorityProofID)
	optionalStringArg(wire, "source_request_id", grant.SourceRequestID)
	if grant.InvocationTemplate != nil {
		wire["invocation_template"] = grant.InvocationTemplate
	}
	return wire
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
		return principalID, nil
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
		GrantID:            stringFromMap(raw, "grant_id"),
		OwnerURA:           stringFromMap(raw, "owner_ura"),
		PrincipalKind:      AccessControlPrincipalKind(stringFromMap(raw, "principal_kind")),
		PrincipalURA:       stringFromMap(raw, "principal_ura"),
		TokenID:            stringFromMap(raw, "token_id"),
		CalleeURA:          stringFromMap(raw, "callee_ura"),
		AbilityURAPattern:  stringFromMap(raw, "ability_ura_pattern"),
		SubjectURAPattern:  stringFromMap(raw, "subject_ura_pattern"),
		Actions:            stringSliceFromMap(raw, "actions"),
		Effect:             AccessControlEffect(stringFromMap(raw, "effect")),
		State:              AccessControlGrantState(stringFromMap(raw, "state")),
		CreatedBy:          stringFromMap(raw, "created_by"),
		CreatedAt:          stringFromMap(raw, "created_at"),
		ExpiresAt:          stringFromMap(raw, "expires_at"),
		RevokedAt:          stringFromMap(raw, "revoked_at"),
		RevokedBy:          stringFromMap(raw, "revoked_by"),
		RevocationReason:   stringFromMap(raw, "revocation_reason"),
		Constraints:        mapFromMap(raw, "constraints"),
		AuthorityProofID:   stringFromMap(raw, "authority_proof_id"),
		SourceRequestID:    stringFromMap(raw, "source_request_id"),
		InvocationTemplate: mapFromMap(raw, "invocation_template"),
	}
	if grant.GrantID == "" {
		return AccessControlGrant{}, invalidAccessControl("grant_id is required in access-control projection", nil)
	}
	return grant, nil
}

func accessControlPolicyDecisionFromMap(raw map[string]any) (AccessControlPolicyDecision, error) {
	decision := AccessControlPolicyDecision{
		Decision:         stringFromMap(raw, "decision"),
		Reason:           stringFromMap(raw, "reason"),
		OwnerURA:         stringFromMap(raw, "owner_ura"),
		OwnerSource:      stringFromMap(raw, "owner_source"),
		PrincipalKind:    AccessControlPrincipalKind(stringFromMap(raw, "principal_kind")),
		PrincipalURA:     stringFromMap(raw, "principal_ura"),
		TokenID:          stringFromMap(raw, "token_id"),
		CalleeURA:        stringFromMap(raw, "callee_ura"),
		AbilityURA:       stringFromMap(raw, "ability_ura"),
		SubjectURA:       stringFromMap(raw, "subject_ura"),
		Action:           stringFromMap(raw, "action"),
		GrantID:          stringFromMap(raw, "grant_id"),
		RejectorURA:      stringFromMap(raw, "rejector_ura"),
		AuthorityProofID: stringFromMap(raw, "authority_proof_id"),
		AuditWarnings:    stringSliceFromMap(raw, "audit_warnings"),
	}
	if decision.Decision == "" {
		return AccessControlPolicyDecision{}, invalidAccessControl("policy decision is required", nil)
	}
	return decision, nil
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

func invalidAccessControl(message string, cause error) error {
	return invalidProfilePayload(accessControlProfile, message, cause)
}
