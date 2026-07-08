package easynet

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"time"

	axonsdk "easynet.run/axon/sdk/go/easynet"
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

// Sign signs this delegated-authority payload with the issuer private key.
func (p *DelegationProof) Sign(privateKey ed25519.PrivateKey) error {
	if p == nil {
		return invalidInvocation("delegation authority is required", nil)
	}
	proof := p.toAxonDelegationProof()
	if err := (&proof).Sign(privateKey); err != nil {
		return err
	}
	p.Signature = append(p.Signature[:0], proof.Signature...)
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

// MarshalRaw returns the daemon authority metadata wire payload.
func (p *DelegationProof) MarshalRaw() ([]byte, error) {
	if p == nil {
		return nil, invalidInvocation("delegation authority is required", nil)
	}
	proof := p.toAxonDelegationProof()
	return (&proof).MarshalRaw()
}

// UnmarshalRawDelegationProof decodes daemon authority metadata wire payload.
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
	if err := validateSessionAuthority(shape); err != nil {
		return nil, err
	}
	payload := map[string]any{
		"allowed_actions":            append([]string(nil), a.AllowedActions...),
		"allowed_followup_abilities": append([]string(nil), a.AllowedFollowupAbilities...),
		"audience":                   a.Audience,
		"callee_ura":                 a.CalleeURA,
		"creator_principal_id":       a.CreatorPrincipalID,
		"expires_at_ms":              a.ExpiresAtMS,
		"issued_at_ms":               a.IssuedAtMS,
		"issuer_ura":                 a.IssuerURA,
		"scopes":                     append([]string(nil), a.Scopes...),
		"session_id":                 a.SessionID,
		"session_owner_user_id":      a.SessionOwnerUserID,
		"subject_ura":                a.SubjectURA,
	}
	return json.Marshal(payload)
}

// Sign signs this session-authority payload with the issuer private key.
func (a *SessionAuthority) Sign(privateKey ed25519.PrivateKey) error {
	if a == nil {
		return invalidInvocation("session authority is required", nil)
	}
	if len(privateKey) != ed25519.PrivateKeySize {
		return invalidInvocation("session authority private key has invalid size", nil)
	}
	payload, err := a.CanonicalPayload()
	if err != nil {
		return err
	}
	a.Signature = append(a.Signature[:0], ed25519.Sign(privateKey, payload)...)
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

// MarshalRaw returns the daemon authority metadata wire payload.
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

// UnmarshalRawSessionAuthority decodes daemon authority metadata wire payload.
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
