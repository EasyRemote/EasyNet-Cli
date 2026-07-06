package easynet

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	DelegationMetadataKey       = "x-easynet-delegation"
	SessionAuthorityMetadataKey = "x-easynet-session-authority"
)

type AuthorityKind string

const (
	AuthorityKindDelegation       AuthorityKind = "delegation"
	AuthorityKindSessionAuthority AuthorityKind = "session_authority"
)

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
	BackendURA  string
	UserURA     string
	SessionID   string
	Scopes      []string
	Audiences   []string
	IssuedAtMS  int64
	ExpiresAtMS int64
	Signature   []byte

	metadataValue string
}

// AuthorityMetadata is the mutually-exclusive Invocation metadata envelope
// accepted by daemon admission.
type AuthorityMetadata struct {
	kind  AuthorityKind
	key   string
	value string
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
		BackendURA:    payload.BackendURA,
		UserURA:       payload.UserURA,
		SessionID:     payload.SessionID,
		Scopes:        append([]string(nil), payload.Scopes...),
		Audiences:     append([]string(nil), payload.Audiences...),
		IssuedAtMS:    payload.IssuedAtMS,
		ExpiresAtMS:   payload.ExpiresAtMS,
		Signature:     append([]byte(nil), signature...),
		metadataValue: strings.TrimSpace(value),
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
	BackendURA  string   `json:"backend_ura"`
	UserURA     string   `json:"user_ura"`
	SessionID   string   `json:"session_id"`
	Scopes      []string `json:"scopes"`
	Audiences   []string `json:"audiences"`
	IssuedAtMS  int64    `json:"issued_at_ms"`
	ExpiresAtMS int64    `json:"expires_at_ms"`
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
	if len(proof.Scopes) == 0 {
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
	if strings.TrimSpace(authority.BackendURA) == "" ||
		strings.TrimSpace(authority.UserURA) == "" ||
		strings.TrimSpace(authority.SessionID) == "" {
		return invalidInvocation("session authority must bind backend, user, and session_id", nil)
	}
	if len(authority.Scopes) == 0 || len(authority.Audiences) == 0 {
		return invalidInvocation("session authority scopes and audiences are required", nil)
	}
	if authority.ExpiresAtMS <= authority.IssuedAtMS {
		return invalidInvocation("session authority expires_at_ms must be greater than issued_at_ms", nil)
	}
	if len(authority.Signature) == 0 {
		return invalidInvocation("session authority signature is required", nil)
	}
	return nil
}
