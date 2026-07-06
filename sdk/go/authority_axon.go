package easynet

import (
	"crypto/ed25519"
	"time"

	axonsdk "easynet.run/axon/sdk/go/easynet"
)

// DelegationProofRaw is Axon's canonical raw delegation-proof encoding.
type DelegationProofRaw = axonsdk.DelegationProofRaw

// SessionAuthorityRaw is Axon's canonical raw session-authority encoding.
type SessionAuthorityRaw = axonsdk.SessionAuthorityRaw

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
// session authority key.
func (a *SessionAuthority) CanonicalPayload() ([]byte, error) {
	if a == nil {
		return nil, invalidInvocation("session authority is required", nil)
	}
	authority := a.toAxonSessionAuthority()
	return (&authority).CanonicalPayload()
}

// Sign signs this session-authority payload with the authority private key.
func (a *SessionAuthority) Sign(privateKey ed25519.PrivateKey) error {
	if a == nil {
		return invalidInvocation("session authority is required", nil)
	}
	authority := a.toAxonSessionAuthority()
	if err := (&authority).Sign(privateKey); err != nil {
		return err
	}
	a.Signature = append(a.Signature[:0], authority.Signature...)
	a.metadataValue = ""
	return nil
}

// Verify verifies this session-authority payload against the backend public key.
func (a *SessionAuthority) Verify(publicKey ed25519.PublicKey) error {
	if a == nil {
		return invalidInvocation("session authority is required", nil)
	}
	authority := a.toAxonSessionAuthority()
	return (&authority).Verify(publicKey)
}

// MarshalRaw returns the daemon authority metadata wire payload.
func (a *SessionAuthority) MarshalRaw() ([]byte, error) {
	if a == nil {
		return nil, invalidInvocation("session authority is required", nil)
	}
	authority := a.toAxonSessionAuthority()
	return (&authority).MarshalRaw()
}

// UnmarshalRawSessionAuthority decodes daemon authority metadata wire payload.
func UnmarshalRawSessionAuthority(data []byte) (*SessionAuthority, error) {
	authority, err := axonsdk.UnmarshalRawSessionAuthority(data)
	if err != nil {
		return nil, err
	}
	out := sessionAuthorityFromAxon(authority)
	if err := validateSessionAuthority(*out); err != nil {
		return nil, err
	}
	return out, nil
}

func (a *SessionAuthority) MatchesScope(ability string) bool {
	if a == nil {
		return false
	}
	authority := a.toAxonSessionAuthority()
	return (&authority).MatchesScope(ability)
}

func (a *SessionAuthority) MatchesAudience(callee string) bool {
	if a == nil {
		return false
	}
	authority := a.toAxonSessionAuthority()
	return (&authority).MatchesAudience(callee)
}

func (a *SessionAuthority) IsExpired(now time.Time) bool {
	if a == nil {
		return true
	}
	authority := a.toAxonSessionAuthority()
	return (&authority).IsExpired(now)
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

func (a SessionAuthority) toAxonSessionAuthority() axonsdk.SessionAuthority {
	return axonsdk.SessionAuthority{
		BackendURA:  a.BackendURA,
		UserURA:     a.UserURA,
		SessionID:   a.SessionID,
		Scopes:      append([]string(nil), a.Scopes...),
		Audiences:   append([]string(nil), a.Audiences...),
		IssuedAtMS:  a.IssuedAtMS,
		ExpiresAtMS: a.ExpiresAtMS,
		Signature:   append([]byte(nil), a.Signature...),
	}
}

func sessionAuthorityFromAxon(authority *axonsdk.SessionAuthority) *SessionAuthority {
	if authority == nil {
		return nil
	}
	return &SessionAuthority{
		BackendURA:  authority.BackendURA,
		UserURA:     authority.UserURA,
		SessionID:   authority.SessionID,
		Scopes:      append([]string(nil), authority.Scopes...),
		Audiences:   append([]string(nil), authority.Audiences...),
		IssuedAtMS:  authority.IssuedAtMS,
		ExpiresAtMS: authority.ExpiresAtMS,
		Signature:   append([]byte(nil), authority.Signature...),
	}
}
