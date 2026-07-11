package easynet

import (
	"context"
	"encoding/json"
	"sync"
)

// CanonicalAuthorityTransport mints authority metadata with an opaque
// CanonicalSigner. It owns canonical authority DTO construction and wire
// projection, while the signer remains responsible for key custody.
type CanonicalAuthorityTransport struct {
	mu     sync.RWMutex
	signer CanonicalSigner
	closed bool
}

var _ AuthorityTransport = (*CanonicalAuthorityTransport)(nil)

// NewCanonicalAuthorityTransport creates the generic authority provider for
// callers that hold an opaque canonical signing capability.
func NewCanonicalAuthorityTransport(signer CanonicalSigner) (*CanonicalAuthorityTransport, error) {
	if signer == nil {
		return nil, invalidProfileClient(authorityProfile, "canonical authority signer is required")
	}
	return &CanonicalAuthorityTransport{signer: signer}, nil
}

// NewCanonicalAuthorityClient creates an AuthorityClient over the SDK-owned
// canonical transport. It never accepts or exposes private key material.
func NewCanonicalAuthorityClient(signer CanonicalSigner) (*AuthorityClient, error) {
	transport, err := NewCanonicalAuthorityTransport(signer)
	if err != nil {
		return nil, err
	}
	return NewAuthorityClient(transport)
}

func (t *CanonicalAuthorityTransport) MintDelegationProof(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	var request DelegationRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidProfilePayload(authorityProfile, "decode delegation request", err)
	}
	if err := validateDelegationRequest(request); err != nil {
		return nil, err
	}
	proof := DelegationProof{
		IssuerURA:   request.IssuerURA,
		SubjectURA:  request.SubjectURA,
		CallerURA:   request.CallerURA,
		Audience:    request.Audience,
		Scopes:      append([]string(nil), request.Scopes...),
		IssuedAtMS:  request.IssuedAtMS,
		ExpiresAtMS: request.ExpiresAtMS,
	}
	if err := proof.SignWith(t.signingCapability()); err != nil {
		return nil, err
	}
	metadataValue, err := proof.MarshalMetadataValue()
	if err != nil {
		return nil, err
	}
	return canonicalAuthorityMetadataProjection(DelegationMetadataKey, metadataValue)
}

func (t *CanonicalAuthorityTransport) MintSessionAuthority(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	var request SessionAuthorityRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidProfilePayload(authorityProfile, "decode session authority request", err)
	}
	if err := validateSessionAuthorityRequest(request); err != nil {
		return nil, err
	}
	authority := SessionAuthority{
		IssuerURA:                request.IssuerURA,
		SessionID:                request.SessionID,
		SessionOwnerUserID:       request.SessionOwnerUserID,
		CreatorPrincipalID:       request.CreatorPrincipalID,
		CalleeURA:                request.CalleeURA,
		SubjectURA:               request.SubjectURA,
		Audience:                 request.Audience,
		Scopes:                   append([]string(nil), request.Scopes...),
		AllowedActions:           append([]string(nil), request.AllowedActions...),
		AllowedFollowupAbilities: append([]string(nil), request.AllowedFollowupAbilities...),
		IssuedAtMS:               request.IssuedAtMS,
		ExpiresAtMS:              request.ExpiresAtMS,
	}
	if err := authority.SignWith(t.signingCapability()); err != nil {
		return nil, err
	}
	metadataValue, err := authority.MarshalMetadataValue()
	if err != nil {
		return nil, err
	}
	return canonicalAuthorityMetadataProjection(SessionAuthorityMetadataKey, metadataValue)
}

// Close transitions the transport to its terminal state. The signer is an
// externally owned capability and is deliberately not closed here.
func (t *CanonicalAuthorityTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidProfileClient(authorityProfile, "context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	t.closed = true
	t.signer = nil
	return nil
}

func (t *CanonicalAuthorityTransport) requireOpen(ctx context.Context) error {
	if ctx == nil {
		return invalidProfileClient(authorityProfile, "context is required")
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	t.mu.RLock()
	defer t.mu.RUnlock()
	if t.closed || t.signer == nil {
		return invalidProfileClient(authorityProfile, "canonical authority transport is closed")
	}
	return nil
}

func (t *CanonicalAuthorityTransport) signingCapability() CanonicalSigner {
	t.mu.RLock()
	defer t.mu.RUnlock()
	return t.signer
}

func canonicalAuthorityMetadataProjection(key, value string) ([]byte, error) {
	if key == "" || value == "" {
		return nil, invalidProfilePayload(authorityProfile, "authority metadata projection is empty", nil)
	}
	return json.Marshal(map[string]any{"metadata": map[string]string{key: value}})
}
