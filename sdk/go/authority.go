package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	DelegationMetadataKey       = "x-easynet-delegation"
	SessionAuthorityMetadataKey = "x-easynet-session-authority"
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

// DelegationProof is a typed projection of daemon/Axon delegated-authority
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

// SessionAuthority is a typed projection of daemon/Axon session-authority
// metadata. It does not own canonical signing or verification.
type SessionAuthority struct {
	IssuerURA                string
	SessionID                string
	SessionOwnerUserID       string
	CreatorPrincipalID       string
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

// AuthorityMetadata is the mutually-exclusive Invocation metadata envelope
// accepted by daemon admission.
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
	lifecycle profileClientLifecycle
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
		CreatorPrincipalID:       payload.CreatorPrincipalID,
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
	if err := validateSessionAuthorityRequest(req); err != nil {
		return nil, err
	}
	raw, err := json.Marshal(req)
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
	authority := SessionAuthority{
		IssuerURA:                req.IssuerURA,
		SessionID:                req.SessionID,
		SessionOwnerUserID:       req.SessionOwnerUserID,
		CreatorPrincipalID:       req.CreatorPrincipalID,
		CalleeURA:                req.CalleeURA,
		SubjectURA:               req.SubjectURA,
		Audience:                 req.Audience,
		Scopes:                   req.Scopes,
		AllowedActions:           req.AllowedActions,
		AllowedFollowupAbilities: req.AllowedFollowupAbilities,
		IssuedAtMS:               req.IssuedAtMS,
		ExpiresAtMS:              req.ExpiresAtMS,
		Signature:                []byte("shape-only"),
	}
	if err := validateSessionAuthority(authority); err != nil {
		return err
	}
	return rejectAuthorityPrivateKeyMetadata(req.Metadata)
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
	if strings.TrimSpace(authority.IssuerURA) == "" ||
		strings.TrimSpace(authority.SessionID) == "" ||
		strings.TrimSpace(authority.SessionOwnerUserID) == "" ||
		strings.TrimSpace(authority.CreatorPrincipalID) == "" ||
		strings.TrimSpace(authority.CalleeURA) == "" ||
		strings.TrimSpace(authority.SubjectURA) == "" ||
		strings.TrimSpace(authority.Audience) == "" {
		return invalidInvocation("session authority must bind issuer, session id, owner, creator principal, callee, subject, and audience", nil)
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
	return nil
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
