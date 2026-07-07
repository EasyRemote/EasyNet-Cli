package easynet

import (
	"crypto/ed25519"
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
