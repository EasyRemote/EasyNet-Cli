package easynet

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"math"
	"strings"
)

var principalPrivateProjectionFieldTokens = []string{
	"seed",
	"private",
	"secret",
	"vault",
	"passphrase",
	"master_key",
	"ciphertext",
	"keyring",
	"storage_path",
}

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

func (b PublicKeyBinding) MarshalJSON() ([]byte, error) {
	type wire struct {
		BindingID     string                `json:"binding_id"`
		PrincipalURA  string                `json:"principal_ura"`
		KeyID         string                `json:"key_id,omitempty"`
		PublicKeyB64  string                `json:"public_key_b64"`
		State         PublicKeyBindingState `json:"state"`
		CreatedUnixMS int64                 `json:"created_unix_ms"`
		ExpiresUnixMS *int64                `json:"expires_unix_ms,omitempty"`
		RotatedUnixMS *int64                `json:"rotated_unix_ms,omitempty"`
		RevokedUnixMS *int64                `json:"revoked_unix_ms,omitempty"`
		RotatedTo     string                `json:"rotated_to,omitempty"`
	}
	return json.Marshal(wire{
		BindingID:     b.BindingID,
		PrincipalURA:  b.PrincipalURA,
		KeyID:         b.KeyID,
		PublicKeyB64:  base64.StdEncoding.EncodeToString(b.PublicKey),
		State:         b.State,
		CreatedUnixMS: b.CreatedUnixMS,
		ExpiresUnixMS: b.ExpiresUnixMS,
		RotatedUnixMS: b.RotatedUnixMS,
		RevokedUnixMS: b.RevokedUnixMS,
		RotatedTo:     b.RotatedTo,
	})
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

// EnrollmentCapability is a durable one-time authority for creating a
// principal through the same PrincipalLifecycle aggregate.
type EnrollmentCapability struct {
	EnrollmentID           string `json:"enrollment_id"`
	IssuerURA              string `json:"issuer_ura"`
	SubjectPrincipalURA    string `json:"subject_principal_ura"`
	CreatedUnixMS          int64  `json:"created_unix_ms"`
	ExpiresUnixMS          *int64 `json:"expires_unix_ms,omitempty"`
	RevokedUnixMS          *int64 `json:"revoked_unix_ms,omitempty"`
	ConsumedByPrincipalURA string `json:"consumed_by_principal_ura,omitempty"`
	ConsumedUnixMS         *int64 `json:"consumed_unix_ms,omitempty"`
}

// PrincipalSnapshot is the versioned public aggregate returned after a
// committed transition or query.
type PrincipalSnapshot struct {
	PrincipalURA    string                 `json:"principal_ura"`
	State           PrincipalState         `json:"state"`
	Version         uint64                 `json:"version"`
	Bindings        []PublicKeyBinding     `json:"bindings"`
	EnrollmentProof *PrincipalProofRef     `json:"enrollment_proof,omitempty"`
	Recovery        *RecoveryPolicy        `json:"recovery,omitempty"`
	Enrollments     []EnrollmentCapability `json:"enrollments"`
	Grants          []AuthorizationGrant   `json:"grants"`
	CreatedUnixMS   int64                  `json:"created_unix_ms"`
	UpdatedUnixMS   int64                  `json:"updated_unix_ms"`
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

type IssueEnrollmentRequest struct {
	Command             PrincipalCommand `json:"command"`
	PrincipalURA        string           `json:"principal_ura"`
	SubjectPrincipalURA string           `json:"subject_principal_ura"`
	ExpiresUnixMS       *int64           `json:"expires_unix_ms,omitempty"`
}

type RevokeEnrollmentRequest struct {
	Command      PrincipalCommand `json:"command"`
	PrincipalURA string           `json:"principal_ura"`
	EnrollmentID string           `json:"enrollment_id"`
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
	IssueEnrollment(context.Context, IssueEnrollmentRequest) (PrincipalSnapshot, error)
	RevokeEnrollment(context.Context, RevokeEnrollmentRequest) (PrincipalSnapshot, error)
	IssueGrant(context.Context, IssueGrantRequest) (PrincipalSnapshot, error)
	RevokeGrant(context.Context, RevokeGrantRequest) (PrincipalSnapshot, error)
	Get(context.Context, string) (PrincipalSnapshot, error)
}

type PrincipalClient struct {
	lifecycle PrincipalLifecycle
}

func NewPrincipalClient(lifecycle PrincipalLifecycle) (*PrincipalClient, error) {
	if lifecycle == nil {
		return nil, invalidPrincipal("Principal lifecycle is required", nil)
	}
	return &PrincipalClient{lifecycle: lifecycle}, nil
}

var _ PrincipalLifecycle = (*PrincipalClient)(nil)

func (c *PrincipalClient) Create(ctx context.Context, request CreatePrincipalRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.Create(ctx, request)
}

func (c *PrincipalClient) BindFirstKey(ctx context.Context, request BindPrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.BindFirstKey(ctx, request)
}

func (c *PrincipalClient) AddKey(ctx context.Context, request BindPrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.AddKey(ctx, request)
}

func (c *PrincipalClient) RotateKey(ctx context.Context, request RotatePrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.RotateKey(ctx, request)
}

func (c *PrincipalClient) RevokeKey(ctx context.Context, request RevokePrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.RevokeKey(ctx, request)
}

func (c *PrincipalClient) ConfigureRecovery(ctx context.Context, request ConfigureRecoveryRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.ConfigureRecovery(ctx, request)
}

func (c *PrincipalClient) Recover(ctx context.Context, request RecoverPrincipalRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.Recover(ctx, request)
}

func (c *PrincipalClient) Suspend(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.Suspend(ctx, request)
}

func (c *PrincipalClient) Reactivate(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.Reactivate(ctx, request)
}

func (c *PrincipalClient) Delete(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.Delete(ctx, request)
}

func (c *PrincipalClient) IssueEnrollment(ctx context.Context, request IssueEnrollmentRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.IssueEnrollment(ctx, request)
}

func (c *PrincipalClient) RevokeEnrollment(ctx context.Context, request RevokeEnrollmentRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.RevokeEnrollment(ctx, request)
}

func (c *PrincipalClient) IssueGrant(ctx context.Context, request IssueGrantRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.IssueGrant(ctx, request)
}

func (c *PrincipalClient) RevokeGrant(ctx context.Context, request RevokeGrantRequest) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.RevokeGrant(ctx, request)
}

func (c *PrincipalClient) Get(ctx context.Context, principalURA string) (PrincipalSnapshot, error) {
	if c == nil || c.lifecycle == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.lifecycle.Get(ctx, principalURA)
}

type principalAbilityInvoker interface {
	Invoke(context.Context, RuntimeCallContext, string, any) (map[string]any, error)
}

type RuntimePrincipalProvider struct {
	ability principalAbilityInvoker
	call    RuntimeCallContext
}

var _ PrincipalLifecycle = (*RuntimePrincipalProvider)(nil)

func NewRuntimePrincipalProvider(ability principalAbilityInvoker, call RuntimeCallContext) (*RuntimePrincipalProvider, error) {
	if ability == nil {
		return nil, invalidPrincipal("runtime ability client is required", nil)
	}
	if strings.TrimSpace(call.CallerURA) == "" || strings.TrimSpace(call.CalleeURA) == "" || strings.TrimSpace(call.SubjectURA) == "" {
		return nil, invalidPrincipal("runtime call context requires caller_ura, callee_ura and subject_ura", nil)
	}
	if err := validateRuntimeCallContext(call); err != nil {
		return nil, err
	}
	return &RuntimePrincipalProvider{ability: ability, call: call}, nil
}

func (p *RuntimePrincipalProvider) Create(ctx context.Context, request CreatePrincipalRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityCreate, map[string]any{"request": principalCreateWire(request)})
}

func (p *RuntimePrincipalProvider) BindFirstKey(ctx context.Context, request BindPrincipalKeyRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityBindFirstKey, map[string]any{"request": principalBindKeyWire(request)})
}

func (p *RuntimePrincipalProvider) AddKey(ctx context.Context, request BindPrincipalKeyRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityAddKey, map[string]any{"request": principalBindKeyWire(request)})
}

func (p *RuntimePrincipalProvider) RotateKey(ctx context.Context, request RotatePrincipalKeyRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityRotateKey, map[string]any{"request": principalRotateKeyWire(request)})
}

func (p *RuntimePrincipalProvider) RevokeKey(ctx context.Context, request RevokePrincipalKeyRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityRevokeKey, map[string]any{"request": principalRevokeKeyWire(request)})
}

func (p *RuntimePrincipalProvider) ConfigureRecovery(ctx context.Context, request ConfigureRecoveryRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityConfigureRecovery, map[string]any{"request": principalConfigureRecoveryWire(request)})
}

func (p *RuntimePrincipalProvider) Recover(ctx context.Context, request RecoverPrincipalRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityRecover, map[string]any{"request": principalRecoverWire(request)})
}

func (p *RuntimePrincipalProvider) Suspend(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilitySuspend, map[string]any{"request": principalChangeStateWire(request)})
}

func (p *RuntimePrincipalProvider) Reactivate(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityReactivate, map[string]any{"request": principalChangeStateWire(request)})
}

func (p *RuntimePrincipalProvider) Delete(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityDelete, map[string]any{"request": principalChangeStateWire(request)})
}

func (p *RuntimePrincipalProvider) IssueEnrollment(ctx context.Context, request IssueEnrollmentRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityIssueEnrollment, map[string]any{"request": principalIssueEnrollmentWire(request)})
}

func (p *RuntimePrincipalProvider) RevokeEnrollment(ctx context.Context, request RevokeEnrollmentRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityRevokeEnrollment, map[string]any{"request": principalRevokeEnrollmentWire(request)})
}

func (p *RuntimePrincipalProvider) IssueGrant(ctx context.Context, request IssueGrantRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityIssueGrant, map[string]any{"request": principalIssueGrantWire(request)})
}

func (p *RuntimePrincipalProvider) RevokeGrant(ctx context.Context, request RevokeGrantRequest) (PrincipalSnapshot, error) {
	return p.invoke(ctx, principalAbilityRevokeGrant, map[string]any{"request": principalRevokeGrantWire(request)})
}

func (p *RuntimePrincipalProvider) Get(ctx context.Context, principalURA string) (PrincipalSnapshot, error) {
	if strings.TrimSpace(principalURA) == "" {
		return PrincipalSnapshot{}, invalidPrincipal("principal_ura is required", nil)
	}
	return p.invoke(ctx, principalAbilityGet, map[string]any{"principal_ura": strings.TrimSpace(principalURA)})
}

func (p *RuntimePrincipalProvider) invoke(ctx context.Context, ability string, args map[string]any) (PrincipalSnapshot, error) {
	if p == nil || p.ability == nil {
		return PrincipalSnapshot{}, invalidPrincipal("runtime Principal provider is not initialized", nil)
	}
	call := p.call
	metadata := clonePrincipalMetadata(call.Metadata)
	metadata["profile"] = principalLifecycleProfile
	metadata["system_ability"] = ability
	call.Metadata = metadata
	output, err := p.ability.Invoke(ctx, call, ability, args)
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	principal, err := requiredPrincipalMap(output, "principal")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	return principalSnapshotFromMap(principal)
}

func principalCreateWire(request CreatePrincipalRequest) map[string]any {
	return map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
	}
}

func principalBindKeyWire(request BindPrincipalKeyRequest) map[string]any {
	wire := map[string]any{
		"command":        principalCommandWire(request.Command),
		"principal_ura":  strings.TrimSpace(request.PrincipalURA),
		"public_key_b64": base64.StdEncoding.EncodeToString(request.PublicKey),
	}
	optionalPrincipalString(wire, "key_id", request.KeyID)
	if request.ExpiresUnixMS != nil {
		wire["expires_unix_ms"] = *request.ExpiresUnixMS
	}
	return wire
}

func principalRotateKeyWire(request RotatePrincipalKeyRequest) map[string]any {
	return map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
		"binding_id":    strings.TrimSpace(request.BindingID),
		"replacement":   principalBindKeyWire(request.Replacement),
	}
}

func principalRevokeKeyWire(request RevokePrincipalKeyRequest) map[string]any {
	return map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
		"binding_id":    strings.TrimSpace(request.BindingID),
	}
}

func principalConfigureRecoveryWire(request ConfigureRecoveryRequest) map[string]any {
	return map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
		"policy_ref":    strings.TrimSpace(request.PolicyRef),
	}
}

func principalRecoverWire(request RecoverPrincipalRequest) map[string]any {
	return map[string]any{
		"command":         principalCommandWire(request.Command),
		"principal_ura":   strings.TrimSpace(request.PrincipalURA),
		"replacement_key": principalBindKeyWire(request.ReplacementKey),
	}
}

func principalChangeStateWire(request ChangePrincipalStateRequest) map[string]any {
	return map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
	}
}

func principalIssueEnrollmentWire(request IssueEnrollmentRequest) map[string]any {
	wire := map[string]any{
		"command":               principalCommandWire(request.Command),
		"principal_ura":         strings.TrimSpace(request.PrincipalURA),
		"subject_principal_ura": strings.TrimSpace(request.SubjectPrincipalURA),
	}
	if request.ExpiresUnixMS != nil {
		wire["expires_unix_ms"] = *request.ExpiresUnixMS
	}
	return wire
}

func principalRevokeEnrollmentWire(request RevokeEnrollmentRequest) map[string]any {
	return map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
		"enrollment_id": strings.TrimSpace(request.EnrollmentID),
	}
}

func principalIssueGrantWire(request IssueGrantRequest) map[string]any {
	wire := map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
		"actions":       request.Actions,
	}
	if request.ExpiresUnixMS != nil {
		wire["expires_unix_ms"] = *request.ExpiresUnixMS
	}
	return wire
}

func principalRevokeGrantWire(request RevokeGrantRequest) map[string]any {
	return map[string]any{
		"command":       principalCommandWire(request.Command),
		"principal_ura": strings.TrimSpace(request.PrincipalURA),
		"grant_id":      strings.TrimSpace(request.GrantID),
	}
}

func principalCommandWire(command PrincipalCommand) map[string]any {
	wire := map[string]any{
		"actor_ura":       strings.TrimSpace(command.ActorURA),
		"idempotency_key": strings.TrimSpace(command.IdempotencyKey),
		"proof": map[string]any{
			"kind":      string(command.Proof.Kind),
			"reference": strings.TrimSpace(command.Proof.Reference),
		},
	}
	if command.ExpectedVersion != nil {
		wire["expected_version"] = *command.ExpectedVersion
	}
	return wire
}

func principalSnapshotFromMap(raw map[string]any) (PrincipalSnapshot, error) {
	if err := rejectPrincipalPrivateProjectionFields(raw, "principal"); err != nil {
		return PrincipalSnapshot{}, err
	}
	principalURA, err := requiredPrincipalString(raw, "principal_ura", "principal.principal_ura")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	state, err := requiredPrincipalState(raw, "state", "principal.state")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	version, err := requiredPrincipalUint64(raw, "version", "principal.version")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	bindings, err := principalBindingsFromMap(raw, "bindings")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	enrollmentProof, err := principalProofRefFromMap(raw, "enrollment_proof")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	recovery, err := principalRecoveryFromMap(raw, "recovery")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	enrollments, err := principalEnrollmentsFromMap(raw, "enrollments")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	grants, err := principalGrantsFromMap(raw, "grants")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	createdUnixMS, err := requiredPrincipalInt64(raw, "created_unix_ms", "principal.created_unix_ms")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	updatedUnixMS, err := requiredPrincipalInt64(raw, "updated_unix_ms", "principal.updated_unix_ms")
	if err != nil {
		return PrincipalSnapshot{}, err
	}
	snapshot := PrincipalSnapshot{
		PrincipalURA:    principalURA,
		State:           state,
		Version:         version,
		Bindings:        bindings,
		EnrollmentProof: enrollmentProof,
		Recovery:        recovery,
		Enrollments:     enrollments,
		Grants:          grants,
		CreatedUnixMS:   createdUnixMS,
		UpdatedUnixMS:   updatedUnixMS,
	}
	return snapshot, nil
}

func rejectPrincipalPrivateProjectionFields(value any, path string) error {
	switch typed := value.(type) {
	case map[string]any:
		for field, nested := range typed {
			normalized := strings.ToLower(field)
			for _, token := range principalPrivateProjectionFieldTokens {
				if strings.Contains(normalized, token) {
					return invalidPrincipal(
						fmt.Sprintf("Principal projection %s contains forbidden private field %q", path, field),
						nil,
					)
				}
			}
			if err := rejectPrincipalPrivateProjectionFields(nested, path+"."+field); err != nil {
				return err
			}
		}
	case []any:
		for index, nested := range typed {
			if err := rejectPrincipalPrivateProjectionFields(nested, fmt.Sprintf("%s[%d]", path, index)); err != nil {
				return err
			}
		}
	}
	return nil
}

func principalProofRefFromMap(raw map[string]any, key string) (*PrincipalProofRef, error) {
	value, err := optionalPrincipalMap(raw, key, "principal."+key)
	if err != nil || value == nil {
		return nil, err
	}
	kind, err := requiredPrincipalProofKind(value, "kind", "principal."+key+".kind")
	if err != nil {
		return nil, err
	}
	reference, err := requiredPrincipalString(value, "reference", "principal."+key+".reference")
	if err != nil {
		return nil, err
	}
	proof := PrincipalProofRef{
		Kind:      kind,
		Reference: reference,
	}
	return &proof, nil
}

func principalBindingsFromMap(raw map[string]any, key string) ([]PublicKeyBinding, error) {
	values, err := optionalPrincipalSequence(raw, key, "principal."+key)
	if err != nil || values == nil {
		return nil, err
	}
	out := make([]PublicKeyBinding, 0, len(values))
	for index, item := range values {
		path := fmt.Sprintf("principal.%s[%d]", key, index)
		mapped, err := requiredPrincipalMapValue(item, path)
		if err != nil {
			return nil, err
		}
		bindingID, err := requiredPrincipalString(mapped, "binding_id", path+".binding_id")
		if err != nil {
			return nil, err
		}
		principalURA, err := requiredPrincipalString(mapped, "principal_ura", path+".principal_ura")
		if err != nil {
			return nil, err
		}
		keyID, err := requiredPrincipalString(mapped, "key_id", path+".key_id")
		if err != nil {
			return nil, err
		}
		publicKey, err := requiredPrincipalPublicKey(mapped, "public_key_b64", path+".public_key_b64")
		if err != nil {
			return nil, err
		}
		state, err := requiredPublicKeyBindingState(mapped, "state", path+".state")
		if err != nil {
			return nil, err
		}
		createdUnixMS, err := requiredPrincipalInt64(mapped, "created_unix_ms", path+".created_unix_ms")
		if err != nil {
			return nil, err
		}
		expiresUnixMS, err := optionalPrincipalInt64(mapped, "expires_unix_ms", path+".expires_unix_ms")
		if err != nil {
			return nil, err
		}
		rotatedUnixMS, err := optionalPrincipalInt64(mapped, "rotated_unix_ms", path+".rotated_unix_ms")
		if err != nil {
			return nil, err
		}
		revokedUnixMS, err := optionalPrincipalInt64(mapped, "revoked_unix_ms", path+".revoked_unix_ms")
		if err != nil {
			return nil, err
		}
		rotatedTo, err := optionalPrincipalProjectionString(mapped, "rotated_to", path+".rotated_to")
		if err != nil {
			return nil, err
		}
		out = append(out, PublicKeyBinding{
			BindingID:     bindingID,
			PrincipalURA:  principalURA,
			KeyID:         keyID,
			PublicKey:     publicKey,
			State:         state,
			CreatedUnixMS: createdUnixMS,
			ExpiresUnixMS: expiresUnixMS,
			RotatedUnixMS: rotatedUnixMS,
			RevokedUnixMS: revokedUnixMS,
			RotatedTo:     rotatedTo,
		})
	}
	return out, nil
}

func principalRecoveryFromMap(raw map[string]any, key string) (*RecoveryPolicy, error) {
	mapped, err := optionalPrincipalMap(raw, key, "principal."+key)
	if err != nil || mapped == nil {
		return nil, err
	}
	policyRef, err := requiredPrincipalString(mapped, "policy_ref", "principal."+key+".policy_ref")
	if err != nil {
		return nil, err
	}
	enabled, err := requiredPrincipalBool(mapped, "enabled", "principal."+key+".enabled")
	if err != nil {
		return nil, err
	}
	updatedUnixMS, err := requiredPrincipalInt64(mapped, "updated_unix_ms", "principal."+key+".updated_unix_ms")
	if err != nil {
		return nil, err
	}
	return &RecoveryPolicy{
		PolicyRef:     policyRef,
		Enabled:       enabled,
		UpdatedUnixMS: updatedUnixMS,
	}, nil
}

func principalEnrollmentsFromMap(raw map[string]any, key string) ([]EnrollmentCapability, error) {
	values, err := optionalPrincipalSequence(raw, key, "principal."+key)
	if err != nil || values == nil {
		return nil, err
	}
	out := make([]EnrollmentCapability, 0, len(values))
	for index, item := range values {
		path := fmt.Sprintf("principal.%s[%d]", key, index)
		mapped, err := requiredPrincipalMapValue(item, path)
		if err != nil {
			return nil, err
		}
		enrollmentID, err := requiredPrincipalString(mapped, "enrollment_id", path+".enrollment_id")
		if err != nil {
			return nil, err
		}
		issuerURA, err := requiredPrincipalString(mapped, "issuer_ura", path+".issuer_ura")
		if err != nil {
			return nil, err
		}
		subjectPrincipalURA, err := requiredPrincipalString(mapped, "subject_principal_ura", path+".subject_principal_ura")
		if err != nil {
			return nil, err
		}
		createdUnixMS, err := requiredPrincipalInt64(mapped, "created_unix_ms", path+".created_unix_ms")
		if err != nil {
			return nil, err
		}
		expiresUnixMS, err := optionalPrincipalInt64(mapped, "expires_unix_ms", path+".expires_unix_ms")
		if err != nil {
			return nil, err
		}
		revokedUnixMS, err := optionalPrincipalInt64(mapped, "revoked_unix_ms", path+".revoked_unix_ms")
		if err != nil {
			return nil, err
		}
		consumedByPrincipalURA, err := optionalPrincipalProjectionString(mapped, "consumed_by_principal_ura", path+".consumed_by_principal_ura")
		if err != nil {
			return nil, err
		}
		consumedUnixMS, err := optionalPrincipalInt64(mapped, "consumed_unix_ms", path+".consumed_unix_ms")
		if err != nil {
			return nil, err
		}
		out = append(out, EnrollmentCapability{
			EnrollmentID:           enrollmentID,
			IssuerURA:              issuerURA,
			SubjectPrincipalURA:    subjectPrincipalURA,
			CreatedUnixMS:          createdUnixMS,
			ExpiresUnixMS:          expiresUnixMS,
			RevokedUnixMS:          revokedUnixMS,
			ConsumedByPrincipalURA: consumedByPrincipalURA,
			ConsumedUnixMS:         consumedUnixMS,
		})
	}
	return out, nil
}

func principalGrantsFromMap(raw map[string]any, key string) ([]AuthorizationGrant, error) {
	values, err := optionalPrincipalSequence(raw, key, "principal."+key)
	if err != nil || values == nil {
		return nil, err
	}
	out := make([]AuthorizationGrant, 0, len(values))
	for index, item := range values {
		path := fmt.Sprintf("principal.%s[%d]", key, index)
		mapped, err := requiredPrincipalMapValue(item, path)
		if err != nil {
			return nil, err
		}
		grantID, err := requiredPrincipalString(mapped, "grant_id", path+".grant_id")
		if err != nil {
			return nil, err
		}
		principalURA, err := requiredPrincipalString(mapped, "principal_ura", path+".principal_ura")
		if err != nil {
			return nil, err
		}
		issuerURA, err := requiredPrincipalString(mapped, "issuer_ura", path+".issuer_ura")
		if err != nil {
			return nil, err
		}
		actions, err := requiredPrincipalStringSlice(mapped, "actions", path+".actions")
		if err != nil {
			return nil, err
		}
		createdUnixMS, err := requiredPrincipalInt64(mapped, "created_unix_ms", path+".created_unix_ms")
		if err != nil {
			return nil, err
		}
		expiresUnixMS, err := optionalPrincipalInt64(mapped, "expires_unix_ms", path+".expires_unix_ms")
		if err != nil {
			return nil, err
		}
		revokedUnixMS, err := optionalPrincipalInt64(mapped, "revoked_unix_ms", path+".revoked_unix_ms")
		if err != nil {
			return nil, err
		}
		out = append(out, AuthorizationGrant{
			GrantID:       grantID,
			PrincipalURA:  principalURA,
			IssuerURA:     issuerURA,
			Actions:       actions,
			CreatedUnixMS: createdUnixMS,
			ExpiresUnixMS: expiresUnixMS,
			RevokedUnixMS: revokedUnixMS,
		})
	}
	return out, nil
}

func requiredPrincipalMap(raw map[string]any, key string) (map[string]any, error) {
	return requiredPrincipalMapValue(raw[key], key)
}

func requiredPrincipalMapValue(value any, path string) (map[string]any, error) {
	if mapped, ok := value.(map[string]any); ok && mapped != nil {
		return mapped, nil
	}
	return nil, invalidPrincipal(path+" projection must be an object", nil)
}

func optionalPrincipalMap(raw map[string]any, key string, path string) (map[string]any, error) {
	value, ok := raw[key]
	if !ok || value == nil {
		return nil, nil
	}
	return requiredPrincipalMapValue(value, path)
}

func requiredPrincipalString(raw map[string]any, key string, path string) (string, error) {
	value, ok := raw[key]
	if !ok {
		return "", invalidPrincipal(path+" is required", nil)
	}
	text, ok := value.(string)
	if !ok || strings.TrimSpace(text) == "" {
		return "", invalidPrincipal(path+" must be a non-empty string", nil)
	}
	return strings.TrimSpace(text), nil
}

func optionalPrincipalProjectionString(raw map[string]any, key string, path string) (string, error) {
	value, ok := raw[key]
	if !ok || value == nil {
		return "", nil
	}
	text, ok := value.(string)
	if !ok {
		return "", invalidPrincipal(path+" must be a string when present", nil)
	}
	return strings.TrimSpace(text), nil
}

func requiredPrincipalProofKind(raw map[string]any, key string, path string) (PrincipalProofKind, error) {
	value, err := requiredPrincipalString(raw, key, path)
	if err != nil {
		return "", err
	}
	switch PrincipalProofKind(value) {
	case PrincipalProofBootstrap, PrincipalProofActiveKey, PrincipalProofGrant, PrincipalProofEnrollment, PrincipalProofRecovery:
		return PrincipalProofKind(value), nil
	default:
		return "", invalidPrincipal(path+" is not a canonical Principal proof kind", nil)
	}
}

func requiredPrincipalState(raw map[string]any, key string, path string) (PrincipalState, error) {
	value, err := requiredPrincipalString(raw, key, path)
	if err != nil {
		return "", err
	}
	switch PrincipalState(value) {
	case PrincipalStatePending, PrincipalStateActive, PrincipalStateSuspended, PrincipalStateDeleted:
		return PrincipalState(value), nil
	default:
		return "", invalidPrincipal(path+" is not a canonical Principal state", nil)
	}
}

func requiredPublicKeyBindingState(raw map[string]any, key string, path string) (PublicKeyBindingState, error) {
	value, err := requiredPrincipalString(raw, key, path)
	if err != nil {
		return "", err
	}
	switch PublicKeyBindingState(value) {
	case PublicKeyBindingStateActive, PublicKeyBindingStateRotated, PublicKeyBindingStateRevoked:
		return PublicKeyBindingState(value), nil
	default:
		return "", invalidPrincipal(path+" is not a canonical public-key binding state", nil)
	}
}

func optionalPrincipalSequence(raw map[string]any, key string, path string) ([]any, error) {
	value, ok := raw[key]
	if !ok || value == nil {
		return nil, nil
	}
	values, ok := value.([]any)
	if !ok {
		return nil, invalidPrincipal(path+" must be an array when present", nil)
	}
	return values, nil
}

func requiredPrincipalStringSlice(raw map[string]any, key string, path string) ([]string, error) {
	value, ok := raw[key]
	if !ok {
		return nil, invalidPrincipal(path+" is required", nil)
	}
	values, ok := value.([]any)
	if !ok {
		return nil, invalidPrincipal(path+" must be an array", nil)
	}
	out := make([]string, 0, len(values))
	for index, item := range values {
		value, ok := item.(string)
		if !ok || strings.TrimSpace(value) == "" {
			return nil, invalidPrincipal(fmt.Sprintf("%s[%d] must be a non-empty string", path, index), nil)
		}
		out = append(out, strings.TrimSpace(value))
	}
	return out, nil
}

func requiredPrincipalPublicKey(raw map[string]any, key string, path string) (ed25519.PublicKey, error) {
	encoded, err := requiredPrincipalString(raw, key, path)
	if err != nil {
		return nil, err
	}
	decoded, err := base64.StdEncoding.Strict().DecodeString(encoded)
	if err != nil {
		return nil, invalidPrincipal(path+" base64 decode failed", err)
	}
	if len(decoded) != ed25519.PublicKeySize {
		return nil, invalidPrincipal(fmt.Sprintf("%s must decode to %d bytes", path, ed25519.PublicKeySize), nil)
	}
	return ed25519.PublicKey(decoded), nil
}

func requiredPrincipalInt64(raw map[string]any, key string, path string) (int64, error) {
	value, ok := raw[key]
	if !ok {
		return 0, invalidPrincipal(path+" is required", nil)
	}
	parsed, ok := principalIntegerInt64(value)
	if !ok {
		return 0, invalidPrincipal(path+" must be an integer", nil)
	}
	return parsed, nil
}

func requiredPrincipalUint64(raw map[string]any, key string, path string) (uint64, error) {
	value, ok := raw[key]
	if !ok {
		return 0, invalidPrincipal(path+" is required", nil)
	}
	parsed, ok := principalIntegerUint64(value)
	if !ok {
		return 0, invalidPrincipal(path+" must be a non-negative integer", nil)
	}
	return parsed, nil
}

func optionalPrincipalInt64(raw map[string]any, key string, path string) (*int64, error) {
	value, ok := raw[key]
	if !ok || value == nil {
		return nil, nil
	}
	parsed, ok := principalIntegerInt64(value)
	if !ok {
		return nil, invalidPrincipal(path+" must be an integer when present", nil)
	}
	return &parsed, nil
}

func principalIntegerInt64(value any) (int64, bool) {
	switch value := value.(type) {
	case int64:
		return value, true
	case int:
		return int64(value), true
	case uint64:
		if value <= math.MaxInt64 {
			return int64(value), true
		}
	case float64:
		if math.IsNaN(value) || math.IsInf(value, 0) || value < math.MinInt64 || value > math.MaxInt64 {
			return 0, false
		}
		parsed := int64(value)
		if value == float64(parsed) {
			return parsed, true
		}
	}
	return 0, false
}

func principalIntegerUint64(value any) (uint64, bool) {
	switch value := value.(type) {
	case uint64:
		return value, true
	case int:
		if value >= 0 {
			return uint64(value), true
		}
	case int64:
		if value >= 0 {
			return uint64(value), true
		}
	case float64:
		if math.IsNaN(value) || math.IsInf(value, 0) || value < 0 || value > math.MaxUint64 {
			return 0, false
		}
		parsed := uint64(value)
		if value >= 0 && value == float64(parsed) {
			return parsed, true
		}
	}
	return 0, false
}

func requiredPrincipalBool(raw map[string]any, key string, path string) (bool, error) {
	if value, ok := raw[key].(bool); ok {
		return value, nil
	}
	return false, invalidPrincipal(path+" must be a boolean", nil)
}

func optionalPrincipalString(args map[string]any, key string, value string) {
	if strings.TrimSpace(value) != "" {
		args[key] = strings.TrimSpace(value)
	}
}

func clonePrincipalMetadata(input map[string]any) map[string]any {
	output := make(map[string]any, len(input)+2)
	for key, value := range input {
		output[key] = value
	}
	return output
}

func invalidPrincipal(message string, cause error) error {
	return invalidProfilePayload(principalLifecycleProfile, message, cause)
}
