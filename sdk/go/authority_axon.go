package easynet

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"time"

	axonsdk "axon.run/sdk/go/axon"
)

// DelegationProofRaw is the SDK facade for Axon's canonical raw
// delegation-proof encoding.
type DelegationProofRaw struct {
	Payload   json.RawMessage `json:"payload"`
	Signature string          `json:"signature"`
}

// SessionAuthorityRaw is the SDK facade for canonical raw session-authority
// encoding.
type SessionAuthorityRaw struct {
	Payload   json.RawMessage `json:"payload"`
	Signature string          `json:"signature"`
}

// CanonicalPayload returns the canonical authority payload bytes signed by the
// delegated issuer.
func (p *DelegationProof) CanonicalPayload() ([]byte, error) {
	if p == nil {
		return nil, invalidInvocation("delegation authority is required", nil)
	}
	proof := p.toAxonDelegationProof()
	return (&proof).CanonicalPayload()
}

// SignWith signs this delegated authority through an opaque runtime signer.
func (p *DelegationProof) SignWith(signer CanonicalSigner) error {
	if p == nil {
		return invalidInvocation("delegation authority is required", nil)
	}
	if signer == nil {
		return invalidInvocation("delegation signer is required", nil)
	}
	payload, err := p.CanonicalPayload()
	if err != nil {
		return err
	}
	signature, err := signer.SignCanonical(payload)
	if err != nil {
		return fmt.Errorf("sign delegation authority: %w", err)
	}
	if len(signature) != ed25519.SignatureSize {
		return invalidInvocation("delegation signer returned an invalid Ed25519 signature", nil)
	}
	p.Signature = append(p.Signature[:0], signature...)
	p.metadataValue = ""
	return nil
}

// Verify verifies this delegated-authority payload against the issuer public key.
func (p *DelegationProof) Verify(publicKey ed25519.PublicKey) error {
	if p == nil {
		return invalidInvocation("delegation authority is required", nil)
	}
	proof := p.toAxonDelegationProof()
	return (&proof).Verify(publicKey)
}

// MarshalRaw returns the runtime authority metadata wire payload.
func (p *DelegationProof) MarshalRaw() ([]byte, error) {
	if p == nil {
		return nil, invalidInvocation("delegation authority is required", nil)
	}
	proof := p.toAxonDelegationProof()
	return (&proof).MarshalRaw()
}

// MarshalMetadataValue returns the value to put under x-runtime-delegation.
func (p *DelegationProof) MarshalMetadataValue() (string, error) {
	raw, err := p.MarshalRaw()
	if err != nil {
		return "", err
	}
	return base64.StdEncoding.EncodeToString(raw), nil
}

// UnmarshalRawDelegationProof decodes runtime authority metadata wire payload.
func UnmarshalRawDelegationProof(data []byte) (*DelegationProof, error) {
	proof, err := axonsdk.UnmarshalRawDelegationProof(data)
	if err != nil {
		return nil, err
	}
	out := delegationProofFromAxon(proof)
	if err := validateDelegationProof(*out); err != nil {
		return nil, err
	}
	return out, nil
}

func (p *DelegationProof) MatchesScope(ability string) bool {
	if p == nil {
		return false
	}
	proof := p.toAxonDelegationProof()
	return (&proof).MatchesScope(ability)
}

func (p *DelegationProof) MatchesAudience(callee string) bool {
	if p == nil {
		return false
	}
	proof := p.toAxonDelegationProof()
	return (&proof).MatchesAudience(callee)
}

func (p *DelegationProof) IsExpired(now time.Time) bool {
	if p == nil {
		return true
	}
	proof := p.toAxonDelegationProof()
	return (&proof).IsExpired(now)
}

// CanonicalPayload returns the canonical authority payload bytes signed by the
// session authority issuer.
func (a *SessionAuthority) CanonicalPayload() ([]byte, error) {
	if a == nil {
		return nil, invalidInvocation("session authority is required", nil)
	}
	shape := *a
	if len(shape.Signature) == 0 {
		shape.Signature = []byte("shape-only")
	}
	shape, err := normalizeSessionAuthority(shape)
	if err != nil {
		return nil, err
	}
	if err := validateSessionAuthority(shape); err != nil {
		return nil, err
	}
	payload := map[string]any{
		"allowed_actions":            append([]string(nil), a.AllowedActions...),
		"allowed_followup_abilities": append([]string(nil), a.AllowedFollowupAbilities...),
		"audience":                   shape.Audience,
		"callee_ura":                 shape.CalleeURA,
		"creator_principal_id":       shape.CreatorPrincipalID,
		"expires_at_ms":              shape.ExpiresAtMS,
		"issued_at_ms":               shape.IssuedAtMS,
		"issuer_ura":                 shape.IssuerURA,
		"scopes":                     append([]string(nil), shape.Scopes...),
		"session_id":                 shape.SessionID,
		"session_owner_user_id":      shape.SessionOwnerUserID,
		"subject_ura":                shape.SubjectURA,
	}
	return json.Marshal(payload)
}

// SignWith signs this session authority through an opaque runtime signer.
func (a *SessionAuthority) SignWith(signer CanonicalSigner) error {
	if a == nil {
		return invalidInvocation("session authority is required", nil)
	}
	if signer == nil {
		return invalidInvocation("session authority signer is required", nil)
	}
	payload, err := a.CanonicalPayload()
	if err != nil {
		return err
	}
	signature, err := signer.SignCanonical(payload)
	if err != nil {
		return fmt.Errorf("sign session authority: %w", err)
	}
	if len(signature) != ed25519.SignatureSize {
		return invalidInvocation("session authority signer returned an invalid Ed25519 signature", nil)
	}
	a.Signature = append(a.Signature[:0], signature...)
	a.metadataValue = ""
	return nil
}

// Verify verifies this session-authority payload against the issuer public key.
func (a *SessionAuthority) Verify(publicKey ed25519.PublicKey) error {
	if a == nil {
		return invalidInvocation("session authority is required", nil)
	}
	if len(publicKey) != ed25519.PublicKeySize {
		return invalidInvocation("session authority public key has invalid size", nil)
	}
	payload, err := a.CanonicalPayload()
	if err != nil {
		return err
	}
	if !ed25519.Verify(publicKey, payload, a.Signature) {
		return invalidInvocation("session authority signature does not verify", nil)
	}
	return nil
}

// MarshalRaw returns the runtime authority metadata wire payload.
func (a *SessionAuthority) MarshalRaw() ([]byte, error) {
	if a == nil {
		return nil, invalidInvocation("session authority is required", nil)
	}
	payload, err := a.CanonicalPayload()
	if err != nil {
		return nil, err
	}
	if len(a.Signature) == 0 {
		return nil, invalidInvocation("session authority signature is required", nil)
	}
	return json.Marshal(SessionAuthorityRaw{
		Payload:   payload,
		Signature: base64.StdEncoding.EncodeToString(a.Signature),
	})
}

// MarshalMetadataValue returns the value to put under x-runtime-session-authority.
func (a *SessionAuthority) MarshalMetadataValue() (string, error) {
	raw, err := a.MarshalRaw()
	if err != nil {
		return "", err
	}
	return base64.StdEncoding.EncodeToString(raw), nil
}

// UnmarshalRawSessionAuthority decodes runtime authority metadata wire payload.
func UnmarshalRawSessionAuthority(data []byte) (*SessionAuthority, error) {
	var raw SessionAuthorityRaw
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, err
	}
	var payload sessionAuthorityPayload
	if err := json.Unmarshal(raw.Payload, &payload); err != nil {
		return nil, err
	}
	signature, err := base64.StdEncoding.DecodeString(raw.Signature)
	if err != nil {
		return nil, err
	}
	out := &SessionAuthority{
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
		Signature:                signature,
	}
	if err := validateSessionAuthority(*out); err != nil {
		return nil, err
	}
	return out, nil
}

func (a *SessionAuthority) MatchesScope(ability string) bool {
	if a == nil {
		return false
	}
	for _, scope := range a.Scopes {
		if axonsdk.MatchScopePattern(scope, ability) {
			return true
		}
	}
	return false
}

func (a *SessionAuthority) MatchesAudience(callee string) bool {
	if a == nil {
		return false
	}
	return a.Audience == "*" || a.Audience == callee || axonsdk.StrictPrefixMatch(a.Audience, callee)
}

func (a *SessionAuthority) IsExpired(now time.Time) bool {
	if a == nil {
		return true
	}
	return now.UnixMilli() >= a.ExpiresAtMS
}

func (p DelegationProof) toAxonDelegationProof() axonsdk.DelegationProof {
	return axonsdk.DelegationProof{
		IssuerURA:   p.IssuerURA,
		SubjectURA:  p.SubjectURA,
		CallerURA:   p.CallerURA,
		Audience:    p.Audience,
		Scopes:      append([]string(nil), p.Scopes...),
		IssuedAtMS:  p.IssuedAtMS,
		ExpiresAtMS: p.ExpiresAtMS,
		Signature:   append([]byte(nil), p.Signature...),
	}
}

func delegationProofFromAxon(proof *axonsdk.DelegationProof) *DelegationProof {
	if proof == nil {
		return nil
	}
	return &DelegationProof{
		IssuerURA:   proof.IssuerURA,
		SubjectURA:  proof.SubjectURA,
		CallerURA:   proof.CallerURA,
		Audience:    proof.Audience,
		Scopes:      append([]string(nil), proof.Scopes...),
		IssuedAtMS:  proof.IssuedAtMS,
		ExpiresAtMS: proof.ExpiresAtMS,
		Signature:   append([]byte(nil), proof.Signature...),
	}
}
