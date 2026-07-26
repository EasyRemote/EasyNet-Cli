package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	DelegationMetadataKey       = "x-runtime-delegation"
	SessionAuthorityMetadataKey = "x-runtime-session-authority"
	authorityProfile            = "authority"
)

type AuthorityKind string

const (
	AuthorityKindDelegation       AuthorityKind = "delegation"
	AuthorityKindSessionAuthority AuthorityKind = "session_authority"
)

// AuthoritySigningMaterial is canonical authority material prepared by the
// runtime core for an external signer.
type AuthoritySigningMaterial struct {
	Profile              string         `json:"profile"`
	Kind                 AuthorityKind  `json:"kind"`
	Algorithm            string         `json:"algorithm"`
	MetadataKey          string         `json:"metadata_key"`
	CanonicalBytesBase64 string         `json:"canonical_bytes_base64"`
	CanonicalHashHex     string         `json:"canonical_hash_hex"`
	SignedFields         []string       `json:"signed_fields"`
	Payload              map[string]any `json:"payload"`
}

// AuthoritySignature is the latest-only signature envelope accepted by the
// authority materialization boundary.
type AuthoritySignature struct {
	SignatureBase64 string `json:"signature_base64"`
}

// AuthoritySignatureProvider signs canonical authority material. It owns key
// access; the C ABI transport only prepares and materializes metadata.
type AuthoritySignatureProvider interface {
	SignAuthority(ctx context.Context, material AuthoritySigningMaterial) (AuthoritySignature, error)
}

// AuthoritySignatureProviderFunc adapts a function into an
// AuthoritySignatureProvider.
type AuthoritySignatureProviderFunc func(ctx context.Context, material AuthoritySigningMaterial) (AuthoritySignature, error)

func (f AuthoritySignatureProviderFunc) SignAuthority(ctx context.Context, material AuthoritySigningMaterial) (AuthoritySignature, error) {
	if f == nil {
		return AuthoritySignature{}, invalidProfileClient(authorityProfile, "authority signature provider is required")
	}
	return f(ctx, material)
}

// DelegationProof is a typed projection of runtime delegated-authority
// metadata. It does not own canonical signing or verification.
type DelegationProof struct {
	IssuerURA   string
	SubjectURA  string
	CallerURA   string
	Audience    string
	Scopes      []string
	IssuedAtMS  int64
	ExpiresAtMS int64
	Signature   []byte

	metadataValue string
}

func (DelegationProof) runtimeInvocationAuthority() {}

// SessionAuthority is a typed projection of runtime session-authority
// metadata. It does not own canonical signing or verification.
type SessionAuthority struct {
	IssuerURA                string
	SessionID                string
	SessionOwnerUserID       string
	SessionOwnerURA          string
	CreatorPrincipalID       string
	CreatorPrincipalURA      string
	CalleeURA                string
	SubjectURA               string
	Audience                 string
	Scopes                   []string
	AllowedActions           []string
	AllowedFollowupAbilities []string
	IssuedAtMS               int64
	ExpiresAtMS              int64
	Signature                []byte

	metadataValue string
}

func (SessionAuthority) runtimeInvocationAuthority() {}

// DelegationRequest asks the authority transport to mint delegated-authority
// metadata. The SDK validates shape only; canonical payload creation stays
// below this facade.
type DelegationRequest struct {
	IssuerURA   string         `json:"issuer_ura"`
	SubjectURA  string         `json:"subject_ura"`
	CallerURA   string         `json:"caller_ura"`
	Audience    string         `json:"audience"`
	Scopes      []string       `json:"scopes"`
	IssuedAtMS  int64          `json:"issued_at_ms"`
	ExpiresAtMS int64          `json:"expires_at_ms"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

// SessionAuthorityRequest asks the authority transport to mint session
// authority metadata. It carries generic authority facts, not product session
// or backend auth state.
type SessionAuthorityRequest struct {
	IssuerURA                string         `json:"issuer_ura"`
	SessionID                string         `json:"session_id"`
	SessionOwnerUserID       string         `json:"session_owner_user_id"`
	SessionOwnerURA          string         `json:"session_owner_ura,omitempty"`
	CreatorPrincipalID       string         `json:"creator_principal_id"`
	CreatorPrincipalURA      string         `json:"creator_principal_ura,omitempty"`
	CalleeURA                string         `json:"callee_ura"`
	SubjectURA               string         `json:"subject_ura"`
	Audience                 string         `json:"audience"`
	Scopes                   []string       `json:"scopes"`
	AllowedActions           []string       `json:"allowed_actions"`
	AllowedFollowupAbilities []string       `json:"allowed_followup_abilities"`
	IssuedAtMS               int64          `json:"issued_at_ms"`
	ExpiresAtMS              int64          `json:"expires_at_ms"`
	Metadata                 map[string]any `json:"metadata,omitempty"`
}

// AuthorityMetadata is the mutually-exclusive Invocation metadata envelope
// accepted by runtime admission.
type AuthorityMetadata struct {
	kind  AuthorityKind
	key   string
	value string
}

// AuthorityTransport mints authority metadata behind the SDK facade.
//
// Implementations may call the daemon, C ABI, or an Axon-owned helper. The Go
// SDK contract is that callers never import raw Axon packages to mint authority
// metadata.
type AuthorityTransport interface {
	MintDelegationProof(ctx context.Context, requestJSON []byte) ([]byte, error)
	MintSessionAuthority(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// AuthorityTransportFunc adapts function fields into an AuthorityTransport.
type AuthorityTransportFunc struct {
	MintDelegationProofFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	MintSessionAuthorityFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f AuthorityTransportFunc) MintDelegationProof(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.MintDelegationProofFunc == nil {
		return nil, invalidProfileClient(authorityProfile, "authority delegation mint transport function is required")
	}
	return f.MintDelegationProofFunc(ctx, requestJSON)
}

func (f AuthorityTransportFunc) MintSessionAuthority(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.MintSessionAuthorityFunc == nil {
		return nil, invalidProfileClient(authorityProfile, "authority session mint transport function is required")
	}
	return f.MintSessionAuthorityFunc(ctx, requestJSON)
}

// AuthorityClient is the typed authority metadata minting facade.
type AuthorityClient struct {
	lifecycle runtimeClientLifecycle
	transport AuthorityTransport
}

func NewAuthorityClient(transport AuthorityTransport) (*AuthorityClient, error) {
	if transport == nil {
		return nil, invalidProfileClient(authorityProfile, "authority transport is required")
	}
	return &AuthorityClient{transport: transport}, nil
}

func (c *AuthorityClient) MintDelegationProof(ctx context.Context, req DelegationRequest) (DelegationProof, error) {
	if err := c.requireOpen(ctx); err != nil {
		return DelegationProof{}, err
	}
	requestJSON, err := marshalDelegationRequest(req)
	if err != nil {
		return DelegationProof{}, err
	}
	raw, err := c.transport.MintDelegationProof(ctx, requestJSON)
	if err != nil {
		return DelegationProof{}, transportProfileError(authorityProfile, "authority delegation mint failed", err)
	}
	value, err := decodeAuthorityMetadataProjection(raw, DelegationMetadataKey, "delegation")
	if err != nil {
		return DelegationProof{}, err
	}
	return NewDelegationProofFromMetadata(value)
}

func (c *AuthorityClient) MintSessionAuthority(ctx context.Context, req SessionAuthorityRequest) (SessionAuthority, error) {
	if err := c.requireOpen(ctx); err != nil {
		return SessionAuthority{}, err
	}
	requestJSON, err := marshalSessionAuthorityRequest(req)
	if err != nil {
		return SessionAuthority{}, err
	}
	raw, err := c.transport.MintSessionAuthority(ctx, requestJSON)
	if err != nil {
		return SessionAuthority{}, transportProfileError(authorityProfile, "authority session mint failed", err)
	}
	value, err := decodeAuthorityMetadataProjection(raw, SessionAuthorityMetadataKey, "session authority")
	if err != nil {
		return SessionAuthority{}, err
	}
	return NewSessionAuthorityFromMetadata(value)
}

func (c *AuthorityClient) Close(ctx context.Context) error {
	if c == nil {
		return invalidProfileClient(authorityProfile, "authority client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, authorityProfile)
}

func NewDelegationProofFromMetadata(value string) (DelegationProof, error) {
	var payload delegationAuthorityPayload
	signature, err := decodeAuthorityMetadata(value, &payload, "delegation")
	if err != nil {
		return DelegationProof{}, err
	}
	proof := DelegationProof{
		IssuerURA:     payload.IssuerURA,
		SubjectURA:    payload.SubjectURA,
		CallerURA:     payload.CallerURA,
		Audience:      payload.Audience,
		Scopes:        append([]string(nil), payload.Scopes...),
		IssuedAtMS:    payload.IssuedAtMS,
		ExpiresAtMS:   payload.ExpiresAtMS,
		Signature:     append([]byte(nil), signature...),
		metadataValue: strings.TrimSpace(value),
	}
	if err := validateDelegationProof(proof); err != nil {
		return DelegationProof{}, err
	}
	return proof, nil
}

func NewSessionAuthorityFromMetadata(value string) (SessionAuthority, error) {
	var payload sessionAuthorityPayload
	signature, err := decodeAuthorityMetadata(value, &payload, "session authority")
	if err != nil {
		return SessionAuthority{}, err
	}
	authority := SessionAuthority{
		IssuerURA:                payload.IssuerURA,
		SessionID:                payload.SessionID,
		SessionOwnerUserID:       payload.SessionOwnerUserID,
		SessionOwnerURA:          sessionOwnerURAFromPayload(payload),
		CreatorPrincipalID:       payload.CreatorPrincipalID,
		CreatorPrincipalURA:      canonicalURAOrEmpty(payload.CreatorPrincipalID),
		CalleeURA:                payload.CalleeURA,
		SubjectURA:               payload.SubjectURA,
		Audience:                 payload.Audience,
		Scopes:                   append([]string(nil), payload.Scopes...),
		AllowedActions:           append([]string(nil), payload.AllowedActions...),
		AllowedFollowupAbilities: append([]string(nil), payload.AllowedFollowupAbilities...),
		IssuedAtMS:               payload.IssuedAtMS,
		ExpiresAtMS:              payload.ExpiresAtMS,
		Signature:                append([]byte(nil), signature...),
		metadataValue:            strings.TrimSpace(value),
	}
	if err := validateSessionAuthority(authority); err != nil {
		return SessionAuthority{}, err
	}
	return authority, nil
}

func (p DelegationProof) Metadata() (AuthorityMetadata, error) {
	if err := validateDelegationProof(p); err != nil {
		return AuthorityMetadata{}, err
	}
	if strings.TrimSpace(p.metadataValue) == "" {
		return AuthorityMetadata{}, invalidInvocation("delegation metadata value is required", nil)
	}
	return AuthorityMetadata{
		kind:  AuthorityKindDelegation,
		key:   DelegationMetadataKey,
		value: p.metadataValue,
	}, nil
}

func (a SessionAuthority) Metadata() (AuthorityMetadata, error) {
	if err := validateSessionAuthority(a); err != nil {
		return AuthorityMetadata{}, err
	}
	if strings.TrimSpace(a.metadataValue) == "" {
		return AuthorityMetadata{}, invalidInvocation("session authority metadata value is required", nil)
	}
	return AuthorityMetadata{
		kind:  AuthorityKindSessionAuthority,
		key:   SessionAuthorityMetadataKey,
		value: a.metadataValue,
	}, nil
}

func (a AuthorityMetadata) Kind() AuthorityKind {
	return a.kind
}

func (a AuthorityMetadata) Key() string {
	return a.key
}

func (a AuthorityMetadata) Value() string {
	return a.value
}

func (a AuthorityMetadata) Metadata() map[string]any {
	if strings.TrimSpace(a.key) == "" || strings.TrimSpace(a.value) == "" {
		return map[string]any{}
	}
	return map[string]any{a.key: a.value}
}

func (a AuthorityMetadata) MergeInto(metadata map[string]any) (map[string]any, error) {
	if strings.TrimSpace(a.key) == "" || strings.TrimSpace(a.value) == "" {
		return nil, invalidInvocation("authority metadata is empty", nil)
	}
	next := copyMap(metadata)
	if next == nil {
		next = map[string]any{}
	}
	next[a.key] = a.value
	if err := validateAuthorityMetadata(next); err != nil {
		return nil, err
	}
	return next, nil
}

func validateAuthorityMetadata(metadata map[string]any) error {
	delegation, err := authorityMetadataValue(metadata, DelegationMetadataKey)
	if err != nil {
		return err
	}
	session, err := authorityMetadataValue(metadata, SessionAuthorityMetadataKey)
	if err != nil {
		return err
	}
	if delegation != "" && session != "" {
		return invalidInvocation("invocation authority metadata is ambiguous", nil)
	}
	return nil
}

func (c *AuthorityClient) requireOpen(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(authorityProfile, "authority client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, authorityProfile)
}

func marshalDelegationRequest(req DelegationRequest) ([]byte, error) {
	if err := validateDelegationRequest(req); err != nil {
		return nil, err
	}
	raw, err := json.Marshal(req)
	if err != nil {
		return nil, invalidProfilePayload(authorityProfile, fmt.Sprintf("encode delegation request: %v", err), err)
	}
	return raw, nil
}

func marshalSessionAuthorityRequest(req SessionAuthorityRequest) ([]byte, error) {
	normalized, err := normalizeSessionAuthorityRequest(req)
	if err != nil {
		return nil, err
	}
	if err := validateSessionAuthorityRequest(req); err != nil {
		return nil, err
	}
	wire := sessionAuthorityRequestWire{
		IssuerURA:                normalized.IssuerURA,
		SessionID:                normalized.SessionID,
		SessionOwnerUserID:       normalized.SessionOwnerUserID,
		CreatorPrincipalID:       normalized.CreatorPrincipalID,
		CalleeURA:                normalized.CalleeURA,
		SubjectURA:               normalized.SubjectURA,
		Audience:                 normalized.Audience,
		Scopes:                   normalized.Scopes,
		AllowedActions:           normalized.AllowedActions,
		AllowedFollowupAbilities: normalized.AllowedFollowupAbilities,
		IssuedAtMS:               normalized.IssuedAtMS,
		ExpiresAtMS:              normalized.ExpiresAtMS,
		Metadata:                 normalized.Metadata,
	}
	raw, err := json.Marshal(wire)
	if err != nil {
		return nil, invalidProfilePayload(authorityProfile, fmt.Sprintf("encode session authority request: %v", err), err)
	}
	return raw, nil
}

func validateDelegationRequest(req DelegationRequest) error {
	proof := DelegationProof{
		IssuerURA:   req.IssuerURA,
		SubjectURA:  req.SubjectURA,
		CallerURA:   req.CallerURA,
		Audience:    req.Audience,
		Scopes:      req.Scopes,
		IssuedAtMS:  req.IssuedAtMS,
		ExpiresAtMS: req.ExpiresAtMS,
		Signature:   []byte("shape-only"),
	}
	if err := validateDelegationProof(proof); err != nil {
		return err
	}
	return rejectAuthorityPrivateKeyMetadata(req.Metadata)
}

func validateSessionAuthorityRequest(req SessionAuthorityRequest) error {
	normalized, err := normalizeSessionAuthorityRequest(req)
	if err != nil {
		return err
	}
	authority := SessionAuthority{
		IssuerURA:                normalized.IssuerURA,
		SessionID:                normalized.SessionID,
		SessionOwnerUserID:       normalized.SessionOwnerUserID,
		SessionOwnerURA:          normalized.SessionOwnerURA,
		CreatorPrincipalID:       normalized.CreatorPrincipalID,
		CreatorPrincipalURA:      normalized.CreatorPrincipalURA,
		CalleeURA:                normalized.CalleeURA,
		SubjectURA:               normalized.SubjectURA,
		Audience:                 normalized.Audience,
		Scopes:                   normalized.Scopes,
		AllowedActions:           normalized.AllowedActions,
		AllowedFollowupAbilities: normalized.AllowedFollowupAbilities,
		IssuedAtMS:               normalized.IssuedAtMS,
		ExpiresAtMS:              normalized.ExpiresAtMS,
		Signature:                []byte("shape-only"),
	}
	if err := validateSessionAuthority(authority); err != nil {
		return err
	}
	return rejectAuthorityPrivateKeyMetadata(normalized.Metadata)
}

func newAuthoritySigningMaterial(raw []byte, wantKey string, wantKind AuthorityKind) (AuthoritySigningMaterial, error) {
	var material AuthoritySigningMaterial
	if err := json.Unmarshal(raw, &material); err != nil {
		return AuthoritySigningMaterial{}, invalidProfilePayload(authorityProfile, fmt.Sprintf("decode authority signing material: %v", err), err)
	}
	if material.Profile != authorityProfile ||
		material.Kind != wantKind ||
		material.MetadataKey != wantKey ||
		strings.TrimSpace(material.Algorithm) == "" ||
		strings.TrimSpace(material.CanonicalBytesBase64) == "" ||
		strings.TrimSpace(material.CanonicalHashHex) == "" ||
		len(material.SignedFields) == 0 ||
		material.Payload == nil {
		return AuthoritySigningMaterial{}, invalidProfilePayload(authorityProfile, "invalid authority signing material projection", nil)
	}
	if _, err := base64.StdEncoding.DecodeString(material.CanonicalBytesBase64); err != nil {
		return AuthoritySigningMaterial{}, invalidProfilePayload(authorityProfile, fmt.Sprintf("authority canonical bytes base64 decode failed: %v", err), err)
	}
	return material, nil
}

func authoritySignatureJSON(signature AuthoritySignature) ([]byte, error) {
	if strings.TrimSpace(signature.SignatureBase64) == "" {
		return nil, invalidProfilePayload(authorityProfile, "authority signature_base64 is required", nil)
	}
	if _, err := base64.StdEncoding.DecodeString(signature.SignatureBase64); err != nil {
		return nil, invalidProfilePayload(authorityProfile, fmt.Sprintf("authority signature base64 decode failed: %v", err), err)
	}
	raw, err := json.Marshal(signature)
	if err != nil {
		return nil, invalidProfilePayload(authorityProfile, fmt.Sprintf("encode authority signature: %v", err), err)
	}
	return raw, nil
}

func rejectAuthorityPrivateKeyMetadata(metadata map[string]any) error {
	for key := range metadata {
		switch strings.ToLower(strings.TrimSpace(key)) {
		case "private_key", "private_key_seed", "private_key_seed_base64", "private_key_hex", "signing_key", "ed25519_seed":
			return invalidProfilePayload(authorityProfile, "private key material must not be supplied to authority facade", nil)
		}
	}
	return nil
}

func decodeAuthorityMetadataProjection(raw []byte, metadataKey string, label string) (string, error) {
	trimmed := strings.TrimSpace(string(raw))
	if trimmed == "" {
		return "", invalidProfilePayload(authorityProfile, label+" metadata projection is required", nil)
	}
	if strings.HasPrefix(trimmed, "{") {
		var projection map[string]any
		if err := json.Unmarshal(raw, &projection); err != nil {
			return "", invalidProfilePayload(authorityProfile, fmt.Sprintf("decode %s metadata projection: %v", label, err), err)
		}
		for _, key := range []string{"metadata_value", "value"} {
			if value, ok := projection[key].(string); ok && strings.TrimSpace(value) != "" {
				return strings.TrimSpace(value), nil
			}
		}
		if metadata, ok := projection["metadata"].(map[string]any); ok {
			return authorityMetadataValue(metadata, metadataKey)
		}
		return "", invalidProfilePayload(authorityProfile, label+" metadata projection missing metadata_value", nil)
	}
	if strings.HasPrefix(trimmed, `"`) {
		var value string
		if err := json.Unmarshal(raw, &value); err != nil {
			return "", invalidProfilePayload(authorityProfile, fmt.Sprintf("decode %s metadata value: %v", label, err), err)
		}
		return strings.TrimSpace(value), nil
	}
	return trimmed, nil
}

func authorityMetadataValue(metadata map[string]any, key string) (string, error) {
	if metadata == nil {
		return "", nil
	}
	raw, ok := metadata[key]
	if !ok || raw == nil {
		return "", nil
	}
	value, ok := raw.(string)
	if !ok {
		return "", invalidInvocation(fmt.Sprintf("%s must be a string metadata value", key), nil)
	}
	return strings.TrimSpace(value), nil
}

type authorityWire struct {
	Payload   json.RawMessage `json:"payload"`
	Signature string          `json:"signature"`
}

type delegationAuthorityPayload struct {
	IssuerURA   string   `json:"issuer_ura"`
	SubjectURA  string   `json:"subject_ura"`
	CallerURA   string   `json:"caller_ura"`
	Audience    string   `json:"audience"`
	Scopes      []string `json:"scopes"`
	IssuedAtMS  int64    `json:"issued_at_ms"`
	ExpiresAtMS int64    `json:"expires_at_ms"`
}

type sessionAuthorityRequestWire struct {
	IssuerURA                string         `json:"issuer_ura"`
	SessionID                string         `json:"session_id"`
	SessionOwnerUserID       string         `json:"session_owner_user_id"`
	CreatorPrincipalID       string         `json:"creator_principal_id"`
	CalleeURA                string         `json:"callee_ura"`
	SubjectURA               string         `json:"subject_ura"`
	Audience                 string         `json:"audience"`
	Scopes                   []string       `json:"scopes"`
	AllowedActions           []string       `json:"allowed_actions"`
	AllowedFollowupAbilities []string       `json:"allowed_followup_abilities"`
	IssuedAtMS               int64          `json:"issued_at_ms"`
	ExpiresAtMS              int64          `json:"expires_at_ms"`
	Metadata                 map[string]any `json:"metadata,omitempty"`
}

type sessionAuthorityPayload struct {
	IssuerURA                string   `json:"issuer_ura"`
	SessionID                string   `json:"session_id"`
	SessionOwnerUserID       string   `json:"session_owner_user_id"`
	CreatorPrincipalID       string   `json:"creator_principal_id"`
	CalleeURA                string   `json:"callee_ura"`
	SubjectURA               string   `json:"subject_ura"`
	Audience                 string   `json:"audience"`
	Scopes                   []string `json:"scopes"`
	AllowedActions           []string `json:"allowed_actions"`
	AllowedFollowupAbilities []string `json:"allowed_followup_abilities"`
	IssuedAtMS               int64    `json:"issued_at_ms"`
	ExpiresAtMS              int64    `json:"expires_at_ms"`
}

func normalizeSessionAuthorityRequest(req SessionAuthorityRequest) (SessionAuthorityRequest, error) {
	req.SessionOwnerUserID = strings.TrimSpace(req.SessionOwnerUserID)
	req.SessionOwnerURA = strings.TrimSpace(req.SessionOwnerURA)
	req.CreatorPrincipalID = strings.TrimSpace(req.CreatorPrincipalID)
	req.CreatorPrincipalURA = strings.TrimSpace(req.CreatorPrincipalURA)
	if req.SessionOwnerURA == "" {
		derived := sessionOwnerURAFromUserSubject(req.SubjectURA, req.SessionOwnerUserID)
		if derived != "" {
			req.SessionOwnerURA = derived
		}
	}
	if req.SessionOwnerURA != "" {
		ownerUserID, err := userIDFromUserURA(req.SessionOwnerURA, "session_owner_ura")
		if err != nil {
			return SessionAuthorityRequest{}, err
		}
		if req.SessionOwnerUserID != "" && req.SessionOwnerUserID != ownerUserID {
			return SessionAuthorityRequest{}, invalidProfilePayload(authorityProfile, "session_owner_user_id must match session_owner_ura user id", nil)
		}
		req.SessionOwnerUserID = ownerUserID
	}
	if req.CreatorPrincipalURA != "" {
		if _, err := ParseURAParts(req.CreatorPrincipalURA); err != nil {
			return SessionAuthorityRequest{}, invalidProfilePayload(authorityProfile, "creator_principal_ura must be a canonical URA", err)
		}
		if req.CreatorPrincipalID != "" && req.CreatorPrincipalID != req.CreatorPrincipalURA {
			return SessionAuthorityRequest{}, invalidProfilePayload(authorityProfile, "creator_principal_id must match creator_principal_ura", nil)
		}
		req.CreatorPrincipalID = req.CreatorPrincipalURA
	}
	return req, nil
}

func userIDFromUserURA(raw string, field string) (string, error) {
	parts, err := ParseURAParts(strings.TrimSpace(raw))
	if err != nil {
		return "", invalidProfilePayload(authorityProfile, field+" must be a canonical User URA", err)
	}
	if parts.Kind != URAKindUser || strings.TrimSpace(parts.UserID) == "" {
		return "", invalidProfilePayload(authorityProfile, field+" must be a canonical User URA", nil)
	}
	return strings.TrimSpace(parts.UserID), nil
}

func sessionOwnerURAFromPayload(payload sessionAuthorityPayload) string {
	return sessionOwnerURAFromUserSubject(payload.SubjectURA, payload.SessionOwnerUserID)
}

func sessionOwnerURAFromUserSubject(subjectURA string, ownerUserID string) string {
	ownerUserID = strings.TrimSpace(ownerUserID)
	if ownerUserID == "" {
		return ""
	}
	parts, err := ParseURAParts(strings.TrimSpace(subjectURA))
	if err != nil {
		return ""
	}
	switch parts.Kind {
	case URAKindUser:
		if parts.UserID == ownerUserID {
			return parts.Raw
		}
	case URAKindResource:
		if strings.TrimSpace(parts.OwnerID) == "user."+ownerUserID {
			return UserURA(parts.Realm, ownerUserID)
		}
	}
	return ""
}

func canonicalURAOrEmpty(raw string) string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return ""
	}
	if _, err := ParseURAParts(raw); err != nil {
		return ""
	}
	return raw
}

func decodeAuthorityMetadata(value string, payload any, label string) ([]byte, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil, invalidInvocation(label+" metadata value is required", nil)
	}
	wireJSON, err := base64.StdEncoding.DecodeString(value)
	if err != nil {
		return nil, invalidInvocation(fmt.Sprintf("%s metadata base64 decode failed: %v", label, err), err)
	}
	var wire authorityWire
	if err := json.Unmarshal(wireJSON, &wire); err != nil {
		return nil, invalidInvocation(fmt.Sprintf("%s metadata JSON parse failed: %v", label, err), err)
	}
	if len(wire.Payload) == 0 {
		return nil, invalidInvocation(label+" metadata payload is required", nil)
	}
	if strings.TrimSpace(wire.Signature) == "" {
		return nil, invalidInvocation(label+" metadata signature is required", nil)
	}
	if err := json.Unmarshal(wire.Payload, payload); err != nil {
		return nil, invalidInvocation(fmt.Sprintf("%s metadata payload parse failed: %v", label, err), err)
	}
	signature, err := base64.StdEncoding.DecodeString(wire.Signature)
	if err != nil {
		return nil, invalidInvocation(fmt.Sprintf("%s metadata signature base64 decode failed: %v", label, err), err)
	}
	if len(signature) == 0 {
		return nil, invalidInvocation(label+" metadata signature is required", nil)
	}
	return signature, nil
}

func validateDelegationProof(proof DelegationProof) error {
	if strings.TrimSpace(proof.IssuerURA) == "" ||
		strings.TrimSpace(proof.SubjectURA) == "" ||
		strings.TrimSpace(proof.CallerURA) == "" ||
		strings.TrimSpace(proof.Audience) == "" {
		return invalidInvocation("delegation authority must bind issuer, subject, caller, and audience", nil)
	}
	for field, value := range map[string]string{
		"issuer_ura":  proof.IssuerURA,
		"subject_ura": proof.SubjectURA,
		"caller_ura":  proof.CallerURA,
		"audience":    proof.Audience,
	} {
		if containsAllZeroPrincipal(value) {
			return invalidInvocation(field+" must not be all-zero", nil)
		}
	}
	if containsBlankString(proof.Scopes) {
		return invalidInvocation("delegation authority scopes are required", nil)
	}
	if proof.ExpiresAtMS <= proof.IssuedAtMS {
		return invalidInvocation("delegation authority expires_at_ms must be greater than issued_at_ms", nil)
	}
	if len(proof.Signature) == 0 {
		return invalidInvocation("delegation authority signature is required", nil)
	}
	return nil
}

func validateSessionAuthority(authority SessionAuthority) error {
	var err error
	authority, err = normalizeSessionAuthority(authority)
	if err != nil {
		return err
	}
	if strings.TrimSpace(authority.IssuerURA) == "" ||
		strings.TrimSpace(authority.SessionID) == "" ||
		strings.TrimSpace(authority.SessionOwnerUserID) == "" ||
		strings.TrimSpace(authority.CreatorPrincipalID) == "" ||
		strings.TrimSpace(authority.CalleeURA) == "" ||
		strings.TrimSpace(authority.SubjectURA) == "" ||
		strings.TrimSpace(authority.Audience) == "" {
		return invalidInvocation("session authority must bind issuer, session id, owner, creator principal, callee, subject, and audience", nil)
	}
	for field, value := range map[string]string{
		"issuer_ura":            authority.IssuerURA,
		"session_owner_user_id": authority.SessionOwnerUserID,
		"session_owner_ura":     authority.SessionOwnerURA,
		"creator_principal_id":  authority.CreatorPrincipalID,
		"creator_principal_ura": authority.CreatorPrincipalURA,
		"callee_ura":            authority.CalleeURA,
		"subject_ura":           authority.SubjectURA,
		"audience":              authority.Audience,
	} {
		if containsAllZeroPrincipal(value) {
			return invalidInvocation(field+" must not be all-zero", nil)
		}
	}
	if containsBlankString(authority.Scopes) {
		return invalidInvocation("session authority scopes are required", nil)
	}
	if containsBlankString(authority.AllowedActions) {
		return invalidInvocation("session authority allowed actions are required", nil)
	}
	if containsBlankString(authority.AllowedFollowupAbilities) {
		return invalidInvocation("session authority allowed follow-up abilities are required", nil)
	}
	if authority.ExpiresAtMS <= authority.IssuedAtMS {
		return invalidInvocation("session authority expires_at_ms must be greater than issued_at_ms", nil)
	}
	if len(authority.Signature) == 0 {
		return invalidInvocation("session authority signature is required", nil)
	}
	if err := validateSessionAuthoritySubjectBinding(authority.SubjectURA, authority.SessionOwnerUserID, authority.SessionID); err != nil {
		return err
	}
	return nil
}

type sessionAuthoritySubject struct {
	kind        string
	ownerUserID string
	sessionID   string
}

func validateSessionAuthoritySubjectBinding(subjectURA string, sessionOwnerUserID string, sessionID string) error {
	if isRetiredInvocationHistorySubjectURA(subjectURA) {
		return invalidInvocation("session authority subject_ura uses retired invocation-history subject; use runtime-state/read", nil)
	}
	subject, err := canonicalSessionAuthoritySubject(subjectURA)
	if err != nil {
		return err
	}
	owner := strings.TrimSpace(sessionOwnerUserID)
	if subject.ownerUserID != owner {
		return invalidInvocation("session authority user subject must match session_owner_user_id", nil)
	}
	if subject.kind == "session" && subject.sessionID != strings.TrimSpace(sessionID) {
		return invalidInvocation("session authority subject_ura owner/session must match session_owner_user_id and session_id", nil)
	}
	return nil
}

func canonicalSessionAuthoritySubject(subjectURA string) (sessionAuthoritySubject, error) {
	parts, err := ParseURAParts(strings.TrimSpace(subjectURA))
	if err != nil {
		return sessionAuthoritySubject{}, invalidInvocation("session authority subject_ura must be a canonical user or session subject", err)
	}
	switch parts.Kind {
	case URAKindUser:
		if strings.TrimSpace(parts.UserID) == "" {
			break
		}
		return sessionAuthoritySubject{kind: "user", ownerUserID: strings.TrimSpace(parts.UserID)}, nil
	case URAKindResource:
		ownerUserID := strings.TrimPrefix(strings.TrimSpace(parts.OwnerID), "user.")
		if ownerUserID == strings.TrimSpace(parts.OwnerID) || ownerUserID == "" || strings.Contains(ownerUserID, ".") || strings.Contains(ownerUserID, "/") {
			break
		}
		sessionID, ok := strings.CutPrefix(strings.TrimSpace(parts.Path), "session/")
		if !ok || strings.TrimSpace(sessionID) == "" || strings.Contains(sessionID, "/") {
			break
		}
		return sessionAuthoritySubject{
			kind:        "session",
			ownerUserID: ownerUserID,
			sessionID:   strings.TrimSpace(sessionID),
		}, nil
	}
	return sessionAuthoritySubject{}, invalidInvocation("session authority subject_ura must be a canonical user or session subject", nil)
}

func normalizeSessionAuthority(authority SessionAuthority) (SessionAuthority, error) {
	authority.SessionOwnerUserID = strings.TrimSpace(authority.SessionOwnerUserID)
	authority.SessionOwnerURA = strings.TrimSpace(authority.SessionOwnerURA)
	authority.CreatorPrincipalID = strings.TrimSpace(authority.CreatorPrincipalID)
	authority.CreatorPrincipalURA = strings.TrimSpace(authority.CreatorPrincipalURA)
	if authority.SessionOwnerURA == "" {
		authority.SessionOwnerURA = sessionOwnerURAFromUserSubject(authority.SubjectURA, authority.SessionOwnerUserID)
	}
	if authority.SessionOwnerURA != "" {
		ownerUserID, err := userIDFromUserURA(authority.SessionOwnerURA, "session_owner_ura")
		if err != nil {
			return SessionAuthority{}, err
		}
		if authority.SessionOwnerUserID != "" && authority.SessionOwnerUserID != ownerUserID {
			return SessionAuthority{}, invalidProfilePayload(authorityProfile, "session_owner_user_id must match session_owner_ura user id", nil)
		}
		authority.SessionOwnerUserID = ownerUserID
	}
	if authority.CreatorPrincipalURA != "" {
		if _, err := ParseURAParts(authority.CreatorPrincipalURA); err != nil {
			return SessionAuthority{}, invalidProfilePayload(authorityProfile, "creator_principal_ura must be a canonical URA", err)
		}
		if authority.CreatorPrincipalID != "" && authority.CreatorPrincipalID != authority.CreatorPrincipalURA {
			return SessionAuthority{}, invalidProfilePayload(authorityProfile, "creator_principal_id must match creator_principal_ura", nil)
		}
		authority.CreatorPrincipalID = authority.CreatorPrincipalURA
	}
	if authority.CreatorPrincipalURA == "" && strings.HasPrefix(authority.CreatorPrincipalID, URAScheme) {
		if _, err := ParseURAParts(authority.CreatorPrincipalID); err == nil {
			authority.CreatorPrincipalURA = authority.CreatorPrincipalID
		}
	}
	return authority, nil
}

func containsBlankString(values []string) bool {
	if len(values) == 0 {
		return true
	}
	for _, value := range values {
		if strings.TrimSpace(value) == "" {
			return true
		}
	}
	return false
}
