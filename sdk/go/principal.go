package easynet

import (
	"context"
	"crypto/ed25519"
)

// PrincipalState is the durable lifecycle state of a runtime principal.
// Deleted is terminal; suspension preserves identity and audit history while
// preventing new admissions.
type PrincipalState string

const (
	PrincipalStatePending   PrincipalState = "pending"
	PrincipalStateActive    PrincipalState = "active"
	PrincipalStateSuspended PrincipalState = "suspended"
	PrincipalStateDeleted   PrincipalState = "deleted"
)

// PublicKeyBindingState is the durable admission state of one public key.
// Rotated and revoked bindings remain visible as audit facts and cannot become
// active again.
type PublicKeyBindingState string

const (
	PublicKeyBindingStateActive  PublicKeyBindingState = "active"
	PublicKeyBindingStateRotated PublicKeyBindingState = "rotated"
	PublicKeyBindingStateRevoked PublicKeyBindingState = "revoked"
)

// PrincipalProofKind identifies the authority source for a lifecycle
// transition without exposing provider-specific proof bytes to products.
type PrincipalProofKind string

const (
	PrincipalProofBootstrap  PrincipalProofKind = "bootstrap"
	PrincipalProofActiveKey  PrincipalProofKind = "active_key"
	PrincipalProofGrant      PrincipalProofKind = "grant"
	PrincipalProofEnrollment PrincipalProofKind = "enrollment"
	PrincipalProofRecovery   PrincipalProofKind = "recovery"
)

// PrincipalProofRef references a provider-validated, replay-bounded proof.
// Products may carry this reference but cannot define or verify its wire shape.
type PrincipalProofRef struct {
	Kind      PrincipalProofKind `json:"kind"`
	Reference string             `json:"reference"`
}

// PrincipalCommand identifies the actor, authority and optimistic-concurrency
// boundary shared by every lifecycle mutation.
type PrincipalCommand struct {
	ActorURA        string            `json:"actor_ura"`
	IdempotencyKey  string            `json:"idempotency_key"`
	ExpectedVersion *uint64           `json:"expected_version,omitempty"`
	Proof           PrincipalProofRef `json:"proof"`
}

// PublicKeyBinding is a public admission projection. It never contains seed,
// private-key, vault or key-service persistence material.
type PublicKeyBinding struct {
	BindingID     string                `json:"binding_id"`
	PrincipalURA  string                `json:"principal_ura"`
	KeyID         string                `json:"key_id,omitempty"`
	PublicKey     ed25519.PublicKey     `json:"public_key"`
	State         PublicKeyBindingState `json:"state"`
	CreatedUnixMS int64                 `json:"created_unix_ms"`
	ExpiresUnixMS *int64                `json:"expires_unix_ms,omitempty"`
	RotatedUnixMS *int64                `json:"rotated_unix_ms,omitempty"`
	RevokedUnixMS *int64                `json:"revoked_unix_ms,omitempty"`
	RotatedTo     string                `json:"rotated_to,omitempty"`
}

// RecoveryPolicy is the public projection of a configured recovery policy.
// Provider-owned verifier material is represented by an opaque policy
// reference, never by a secret.
type RecoveryPolicy struct {
	PolicyRef     string `json:"policy_ref"`
	Enabled       bool   `json:"enabled"`
	UpdatedUnixMS int64  `json:"updated_unix_ms"`
}

// AuthorizationGrant is a durable, revocable authority projection.
type AuthorizationGrant struct {
	GrantID       string   `json:"grant_id"`
	PrincipalURA  string   `json:"principal_ura"`
	IssuerURA     string   `json:"issuer_ura"`
	Actions       []string `json:"actions"`
	CreatedUnixMS int64    `json:"created_unix_ms"`
	ExpiresUnixMS *int64   `json:"expires_unix_ms,omitempty"`
	RevokedUnixMS *int64   `json:"revoked_unix_ms,omitempty"`
}

// PrincipalSnapshot is the versioned public aggregate returned after a
// committed transition or query.
type PrincipalSnapshot struct {
	PrincipalURA  string               `json:"principal_ura"`
	State         PrincipalState       `json:"state"`
	Version       uint64               `json:"version"`
	Bindings      []PublicKeyBinding   `json:"bindings"`
	Recovery      *RecoveryPolicy      `json:"recovery,omitempty"`
	Grants        []AuthorizationGrant `json:"grants"`
	CreatedUnixMS int64                `json:"created_unix_ms"`
	UpdatedUnixMS int64                `json:"updated_unix_ms"`
}

type CreatePrincipalRequest struct {
	Command      PrincipalCommand `json:"command"`
	PrincipalURA string           `json:"principal_ura"`
}

type BindPrincipalKeyRequest struct {
	Command       PrincipalCommand  `json:"command"`
	PrincipalURA  string            `json:"principal_ura"`
	KeyID         string            `json:"key_id,omitempty"`
	PublicKey     ed25519.PublicKey `json:"public_key"`
	ExpiresUnixMS *int64            `json:"expires_unix_ms,omitempty"`
}

type RotatePrincipalKeyRequest struct {
	Command      PrincipalCommand        `json:"command"`
	PrincipalURA string                  `json:"principal_ura"`
	BindingID    string                  `json:"binding_id"`
	Replacement  BindPrincipalKeyRequest `json:"replacement"`
}

type RevokePrincipalKeyRequest struct {
	Command      PrincipalCommand `json:"command"`
	PrincipalURA string           `json:"principal_ura"`
	BindingID    string           `json:"binding_id"`
}

type ConfigureRecoveryRequest struct {
	Command      PrincipalCommand `json:"command"`
	PrincipalURA string           `json:"principal_ura"`
	PolicyRef    string           `json:"policy_ref"`
}

type RecoverPrincipalRequest struct {
	Command        PrincipalCommand        `json:"command"`
	PrincipalURA   string                  `json:"principal_ura"`
	ReplacementKey BindPrincipalKeyRequest `json:"replacement_key"`
}

type ChangePrincipalStateRequest struct {
	Command      PrincipalCommand `json:"command"`
	PrincipalURA string           `json:"principal_ura"`
}

type IssueGrantRequest struct {
	Command       PrincipalCommand `json:"command"`
	PrincipalURA  string           `json:"principal_ura"`
	Actions       []string         `json:"actions"`
	ExpiresUnixMS *int64           `json:"expires_unix_ms,omitempty"`
}

type RevokeGrantRequest struct {
	Command      PrincipalCommand `json:"command"`
	PrincipalURA string           `json:"principal_ura"`
	GrantID      string           `json:"grant_id"`
}

// PrincipalLifecycle is the product-neutral seam for the one authoritative
// principal aggregate. Implementations must commit state atomically, enforce
// proof replay protection and emit a receipt only after commit.
type PrincipalLifecycle interface {
	Create(context.Context, CreatePrincipalRequest) (PrincipalSnapshot, error)
	BindFirstKey(context.Context, BindPrincipalKeyRequest) (PrincipalSnapshot, error)
	AddKey(context.Context, BindPrincipalKeyRequest) (PrincipalSnapshot, error)
	RotateKey(context.Context, RotatePrincipalKeyRequest) (PrincipalSnapshot, error)
	RevokeKey(context.Context, RevokePrincipalKeyRequest) (PrincipalSnapshot, error)
	ConfigureRecovery(context.Context, ConfigureRecoveryRequest) (PrincipalSnapshot, error)
	Recover(context.Context, RecoverPrincipalRequest) (PrincipalSnapshot, error)
	Suspend(context.Context, ChangePrincipalStateRequest) (PrincipalSnapshot, error)
	Reactivate(context.Context, ChangePrincipalStateRequest) (PrincipalSnapshot, error)
	Delete(context.Context, ChangePrincipalStateRequest) (PrincipalSnapshot, error)
	IssueGrant(context.Context, IssueGrantRequest) (PrincipalSnapshot, error)
	RevokeGrant(context.Context, RevokeGrantRequest) (PrincipalSnapshot, error)
	Get(context.Context, string) (PrincipalSnapshot, error)
}
