package easynet

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"fmt"
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

type PrincipalProvider interface {
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
	provider PrincipalProvider
}

func NewPrincipalClient(provider PrincipalProvider) (*PrincipalClient, error) {
	if provider == nil {
		return nil, invalidPrincipal("Principal provider is required", nil)
	}
	return &PrincipalClient{provider: provider}, nil
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

var _ PrincipalLifecycle = (*PrincipalClient)(nil)

func (c *PrincipalClient) Create(ctx context.Context, request CreatePrincipalRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.Create(ctx, request)
}

func (c *PrincipalClient) BindFirstKey(ctx context.Context, request BindPrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.BindFirstKey(ctx, request)
}

func (c *PrincipalClient) AddKey(ctx context.Context, request BindPrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.AddKey(ctx, request)
}

func (c *PrincipalClient) RotateKey(ctx context.Context, request RotatePrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.RotateKey(ctx, request)
}

func (c *PrincipalClient) RevokeKey(ctx context.Context, request RevokePrincipalKeyRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.RevokeKey(ctx, request)
}

func (c *PrincipalClient) ConfigureRecovery(ctx context.Context, request ConfigureRecoveryRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.ConfigureRecovery(ctx, request)
}

func (c *PrincipalClient) Recover(ctx context.Context, request RecoverPrincipalRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.Recover(ctx, request)
}

func (c *PrincipalClient) Suspend(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.Suspend(ctx, request)
}

func (c *PrincipalClient) Reactivate(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.Reactivate(ctx, request)
}

func (c *PrincipalClient) Delete(ctx context.Context, request ChangePrincipalStateRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.Delete(ctx, request)
}

func (c *PrincipalClient) IssueEnrollment(ctx context.Context, request IssueEnrollmentRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.IssueEnrollment(ctx, request)
}

func (c *PrincipalClient) RevokeEnrollment(ctx context.Context, request RevokeEnrollmentRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.RevokeEnrollment(ctx, request)
}

func (c *PrincipalClient) IssueGrant(ctx context.Context, request IssueGrantRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.IssueGrant(ctx, request)
}

func (c *PrincipalClient) RevokeGrant(ctx context.Context, request RevokeGrantRequest) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.RevokeGrant(ctx, request)
}

func (c *PrincipalClient) Get(ctx context.Context, principalURA string) (PrincipalSnapshot, error) {
	if c == nil || c.provider == nil {
		return PrincipalSnapshot{}, invalidPrincipal("Principal client is not initialized", nil)
	}
	return c.provider.Get(ctx, principalURA)
}

type principalAbilityInvoker interface {
	Invoke(context.Context, RuntimeCallContext, string, any) (map[string]any, error)
}

type RuntimePrincipalProvider struct {
	ability principalAbilityInvoker
	call    RuntimeCallContext
}

func NewRuntimePrincipalProvider(ability principalAbilityInvoker, call RuntimeCallContext) (*RuntimePrincipalProvider, error) {
	if ability == nil {
		return nil, invalidPrincipal("runtime ability client is required", nil)
	}
	if strings.TrimSpace(call.CallerURA) == "" || strings.TrimSpace(call.CalleeURA) == "" || strings.TrimSpace(call.SubjectURA) == "" {
		return nil, invalidPrincipal("runtime call context requires caller_ura, callee_ura and subject_ura", nil)
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
	return principalSnapshotFromMap(requiredPrincipalMap(output, "principal"))
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
	snapshot := PrincipalSnapshot{
		PrincipalURA:    principalStringFromMap(raw, "principal_ura"),
		State:           PrincipalState(principalStringFromMap(raw, "state")),
		Version:         uint64FromPrincipalMap(raw, "version"),
		Bindings:        principalBindingsFromMap(raw, "bindings"),
		EnrollmentProof: principalProofRefFromMap(raw, "enrollment_proof"),
		Recovery:        principalRecoveryFromMap(raw, "recovery"),
		Enrollments:     principalEnrollmentsFromMap(raw, "enrollments"),
		Grants:          principalGrantsFromMap(raw, "grants"),
		CreatedUnixMS:   int64FromPrincipalMap(raw, "created_unix_ms"),
		UpdatedUnixMS:   int64FromPrincipalMap(raw, "updated_unix_ms"),
	}
	if snapshot.PrincipalURA == "" {
		return PrincipalSnapshot{}, invalidPrincipal("principal_ura is required in Principal snapshot", nil)
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

func principalProofRefFromMap(raw map[string]any, key string) *PrincipalProofRef {
	value, ok := raw[key].(map[string]any)
	if !ok {
		return nil
	}
	proof := PrincipalProofRef{
		Kind:      PrincipalProofKind(principalStringFromMap(value, "kind")),
		Reference: principalStringFromMap(value, "reference"),
	}
	if proof.Kind == "" && proof.Reference == "" {
		return nil
	}
	return &proof
}

func principalBindingsFromMap(raw map[string]any, key string) []PublicKeyBinding {
	values, ok := raw[key].([]any)
	if !ok {
		return nil
	}
	out := make([]PublicKeyBinding, 0, len(values))
	for _, item := range values {
		mapped := requiredPrincipalMapValue(item)
		out = append(out, PublicKeyBinding{
			BindingID:     principalStringFromMap(mapped, "binding_id"),
			PrincipalURA:  principalStringFromMap(mapped, "principal_ura"),
			KeyID:         principalStringFromMap(mapped, "key_id"),
			PublicKey:     principalPublicKeyFromMap(mapped, "public_key_b64"),
			State:         PublicKeyBindingState(principalStringFromMap(mapped, "state")),
			CreatedUnixMS: int64FromPrincipalMap(mapped, "created_unix_ms"),
			ExpiresUnixMS: optionalInt64FromPrincipalMap(mapped, "expires_unix_ms"),
			RotatedUnixMS: optionalInt64FromPrincipalMap(mapped, "rotated_unix_ms"),
			RevokedUnixMS: optionalInt64FromPrincipalMap(mapped, "revoked_unix_ms"),
			RotatedTo:     principalStringFromMap(mapped, "rotated_to"),
		})
	}
	return out
}

func principalRecoveryFromMap(raw map[string]any, key string) *RecoveryPolicy {
	mapped, ok := raw[key].(map[string]any)
	if !ok || mapped == nil {
		return nil
	}
	return &RecoveryPolicy{
		PolicyRef:     principalStringFromMap(mapped, "policy_ref"),
		Enabled:       boolFromPrincipalMap(mapped, "enabled"),
		UpdatedUnixMS: int64FromPrincipalMap(mapped, "updated_unix_ms"),
	}
}

func principalEnrollmentsFromMap(raw map[string]any, key string) []EnrollmentCapability {
	values, ok := raw[key].([]any)
	if !ok {
		return nil
	}
	out := make([]EnrollmentCapability, 0, len(values))
	for _, item := range values {
		mapped := requiredPrincipalMapValue(item)
		out = append(out, EnrollmentCapability{
			EnrollmentID:           principalStringFromMap(mapped, "enrollment_id"),
			IssuerURA:              principalStringFromMap(mapped, "issuer_ura"),
			SubjectPrincipalURA:    principalStringFromMap(mapped, "subject_principal_ura"),
			CreatedUnixMS:          int64FromPrincipalMap(mapped, "created_unix_ms"),
			ExpiresUnixMS:          optionalInt64FromPrincipalMap(mapped, "expires_unix_ms"),
			RevokedUnixMS:          optionalInt64FromPrincipalMap(mapped, "revoked_unix_ms"),
			ConsumedByPrincipalURA: principalStringFromMap(mapped, "consumed_by_principal_ura"),
			ConsumedUnixMS:         optionalInt64FromPrincipalMap(mapped, "consumed_unix_ms"),
		})
	}
	return out
}

func principalGrantsFromMap(raw map[string]any, key string) []AuthorizationGrant {
	values, ok := raw[key].([]any)
	if !ok {
		return nil
	}
	out := make([]AuthorizationGrant, 0, len(values))
	for _, item := range values {
		mapped := requiredPrincipalMapValue(item)
		out = append(out, AuthorizationGrant{
			GrantID:       principalStringFromMap(mapped, "grant_id"),
			PrincipalURA:  principalStringFromMap(mapped, "principal_ura"),
			IssuerURA:     principalStringFromMap(mapped, "issuer_ura"),
			Actions:       principalStringSliceFromMap(mapped, "actions"),
			CreatedUnixMS: int64FromPrincipalMap(mapped, "created_unix_ms"),
			ExpiresUnixMS: optionalInt64FromPrincipalMap(mapped, "expires_unix_ms"),
			RevokedUnixMS: optionalInt64FromPrincipalMap(mapped, "revoked_unix_ms"),
		})
	}
	return out
}

func requiredPrincipalMap(raw map[string]any, key string) map[string]any {
	return requiredPrincipalMapValue(raw[key])
}

func requiredPrincipalMapValue(value any) map[string]any {
	if mapped, ok := value.(map[string]any); ok && mapped != nil {
		return mapped
	}
	return map[string]any{}
}

func principalStringFromMap(raw map[string]any, key string) string {
	if value, ok := raw[key].(string); ok {
		return value
	}
	return ""
}

func principalStringSliceFromMap(raw map[string]any, key string) []string {
	values, ok := raw[key].([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(values))
	for _, item := range values {
		if value, ok := item.(string); ok {
			out = append(out, value)
		}
	}
	return out
}

func principalPublicKeyFromMap(raw map[string]any, key string) ed25519.PublicKey {
	encoded := principalStringFromMap(raw, key)
	if encoded == "" {
		return nil
	}
	decoded, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		return nil
	}
	return ed25519.PublicKey(decoded)
}

func int64FromPrincipalMap(raw map[string]any, key string) int64 {
	switch value := raw[key].(type) {
	case int64:
		return value
	case int:
		return int64(value)
	case float64:
		return int64(value)
	default:
		return 0
	}
}

func uint64FromPrincipalMap(raw map[string]any, key string) uint64 {
	switch value := raw[key].(type) {
	case uint64:
		return value
	case int:
		if value > 0 {
			return uint64(value)
		}
	case float64:
		if value > 0 {
			return uint64(value)
		}
	}
	return 0
}

func optionalInt64FromPrincipalMap(raw map[string]any, key string) *int64 {
	if _, ok := raw[key]; !ok {
		return nil
	}
	value := int64FromPrincipalMap(raw, key)
	return &value
}

func boolFromPrincipalMap(raw map[string]any, key string) bool {
	if value, ok := raw[key].(bool); ok {
		return value
	}
	return false
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
