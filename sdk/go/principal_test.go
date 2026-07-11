package easynet

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"reflect"
	"testing"
)

func TestPrincipalLifecycleContractIsComplete(t *testing.T) {
	type required interface {
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

	principalType := reflect.TypeOf((*PrincipalLifecycle)(nil)).Elem()
	requiredType := reflect.TypeOf((*required)(nil)).Elem()
	if !principalType.Implements(requiredType) || !requiredType.Implements(principalType) {
		t.Fatal("PrincipalLifecycle drifted from the canonical transition contract")
	}
}

func TestPrincipalStatesPinTerminalVocabulary(t *testing.T) {
	if PrincipalStatePending != "pending" || PrincipalStateActive != "active" ||
		PrincipalStateSuspended != "suspended" || PrincipalStateDeleted != "deleted" {
		t.Fatal("principal lifecycle state vocabulary changed")
	}
	if PublicKeyBindingStateActive != "active" || PublicKeyBindingStateRotated != "rotated" ||
		PublicKeyBindingStateRevoked != "revoked" {
		t.Fatal("public-key binding state vocabulary changed")
	}
}

type memoryPrincipalAbility struct {
	ability string
	args    map[string]any
}

func (m *memoryPrincipalAbility) Invoke(_ context.Context, call RuntimeCallContext, ability string, args any) (map[string]any, error) {
	if call.CallerURA == "" || call.CalleeURA == "" || call.SubjectURA == "" || call.NonceBase64 == "" || call.CausalContext == nil {
		return nil, invalidRuntimePayload("call context was not preserved", nil)
	}
	m.ability = ability
	m.args = args.(map[string]any)
	return map[string]any{"principal": map[string]any{
		"principal_ura":   "easynet:///r/example/user/alice",
		"state":           "active",
		"version":         float64(2),
		"created_unix_ms": float64(1_700_000_000_000),
		"updated_unix_ms": float64(1_700_000_001_000),
		"bindings": []any{map[string]any{
			"binding_id":      "binding-1",
			"principal_ura":   "easynet:///r/example/user/alice",
			"key_id":          "laptop",
			"public_key_b64":  base64.StdEncoding.EncodeToString(make(ed25519.PublicKey, ed25519.PublicKeySize)),
			"state":           "active",
			"created_unix_ms": float64(1_700_000_000_000),
		}},
		"enrollment_proof": map[string]any{
			"kind":      "bootstrap",
			"reference": "proof-1",
		},
		"recovery": map[string]any{
			"policy_ref":      "recovery-policy-1",
			"enabled":         true,
			"updated_unix_ms": float64(1_700_000_001_000),
		},
		"enrollments": []any{map[string]any{
			"enrollment_id":             "enroll-1",
			"issuer_ura":                "easynet:///r/example/user/alice",
			"subject_principal_ura":     "easynet:///r/example/user/bob",
			"created_unix_ms":           float64(1_700_000_001_000),
			"consumed_by_principal_ura": "easynet:///r/example/user/bob",
			"consumed_unix_ms":          float64(1_700_000_002_000),
		}},
		"grants": []any{map[string]any{
			"grant_id":        "grant-1",
			"principal_ura":   "easynet:///r/example/user/alice",
			"issuer_ura":      "easynet:///r/example/user/admin",
			"actions":         []any{"principal.key.add"},
			"created_unix_ms": float64(1_700_000_001_000),
		}},
	}}, nil
}

func TestRuntimePrincipalProviderLowersLifecycleTransitions(t *testing.T) {
	transport := &memoryPrincipalAbility{}
	provider, err := NewRuntimePrincipalProvider(transport, principalCallFixture())
	if err != nil {
		t.Fatalf("NewRuntimePrincipalProvider: %v", err)
	}
	client, err := NewPrincipalClient(provider)
	if err != nil {
		t.Fatalf("NewPrincipalClient: %v", err)
	}

	result, err := client.BindFirstKey(context.Background(), BindPrincipalKeyRequest{
		Command:      principalCommandFixture(),
		PrincipalURA: "easynet:///r/example/user/alice",
		KeyID:        "laptop",
		PublicKey:    make(ed25519.PublicKey, ed25519.PublicKeySize),
	})
	if err != nil {
		t.Fatalf("BindFirstKey: %v", err)
	}
	if transport.ability != principalAbilityBindFirstKey {
		t.Fatalf("ability = %s", transport.ability)
	}
	request := transport.args["request"].(map[string]any)
	command := request["command"].(map[string]any)
	proof := command["proof"].(map[string]any)
	if request["principal_ura"] != "easynet:///r/example/user/alice" || request["public_key_b64"] == "" {
		t.Fatalf("principal key request not lowered: %#v", request)
	}
	if command["actor_ura"] != "easynet:///r/example/user/admin" || command["idempotency_key"] != "idem-1" || proof["kind"] != "bootstrap" {
		t.Fatalf("principal command not lowered: %#v", command)
	}
	if result.PrincipalURA != "easynet:///r/example/user/alice" || result.State != PrincipalStateActive || len(result.Bindings) != 1 {
		t.Fatalf("snapshot projection lost: %#v", result)
	}
	if result.Bindings[0].PublicKey == nil || result.EnrollmentProof == nil || result.Recovery == nil || len(result.Grants) != 1 {
		t.Fatalf("public aggregate projection incomplete: %#v", result)
	}
	if len(result.Enrollments) != 1 || result.Enrollments[0].EnrollmentID != "enroll-1" || result.Enrollments[0].ConsumedUnixMS == nil {
		t.Fatalf("enrollment capability projection lost: %#v", result.Enrollments)
	}
	if result.EnrollmentProof.Kind != PrincipalProofBootstrap || result.EnrollmentProof.Reference != "proof-1" {
		t.Fatalf("enrollment proof projection lost: %#v", result.EnrollmentProof)
	}
	if _, ok := request["private_key"]; ok {
		t.Fatalf("private key leaked into principal provider args: %#v", request)
	}
}

func TestRuntimePrincipalProviderLowersEnrollmentAuthority(t *testing.T) {
	transport := &memoryPrincipalAbility{}
	provider, err := NewRuntimePrincipalProvider(transport, principalCallFixture())
	if err != nil {
		t.Fatalf("NewRuntimePrincipalProvider: %v", err)
	}

	_, err = provider.IssueEnrollment(context.Background(), IssueEnrollmentRequest{
		Command:             principalCommandFixture(),
		PrincipalURA:        "easynet:///r/example/user/alice",
		SubjectPrincipalURA: "easynet:///r/example/user/bob",
	})
	if err != nil {
		t.Fatalf("IssueEnrollment: %v", err)
	}
	if transport.ability != principalAbilityIssueEnrollment {
		t.Fatalf("ability = %s", transport.ability)
	}
	request := transport.args["request"].(map[string]any)
	if request["principal_ura"] != "easynet:///r/example/user/alice" ||
		request["subject_principal_ura"] != "easynet:///r/example/user/bob" {
		t.Fatalf("issue enrollment request not lowered: %#v", request)
	}

	_, err = provider.RevokeEnrollment(context.Background(), RevokeEnrollmentRequest{
		Command:      principalCommandFixture(),
		PrincipalURA: "easynet:///r/example/user/alice",
		EnrollmentID: "enroll-1",
	})
	if err != nil {
		t.Fatalf("RevokeEnrollment: %v", err)
	}
	if transport.ability != principalAbilityRevokeEnrollment {
		t.Fatalf("ability = %s", transport.ability)
	}
	request = transport.args["request"].(map[string]any)
	if request["enrollment_id"] != "enroll-1" {
		t.Fatalf("revoke enrollment request not lowered: %#v", request)
	}
}

func TestRuntimePrincipalProviderUsesGenericGetAbility(t *testing.T) {
	transport := &memoryPrincipalAbility{}
	provider, err := NewRuntimePrincipalProvider(transport, principalCallFixture())
	if err != nil {
		t.Fatalf("NewRuntimePrincipalProvider: %v", err)
	}

	_, err = provider.Get(context.Background(), "easynet:///r/example/user/alice")
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if transport.ability != principalAbilityGet || transport.args["principal_ura"] != "easynet:///r/example/user/alice" {
		t.Fatalf("get was not lowered through generic ability: ability=%s args=%#v", transport.ability, transport.args)
	}
}

func principalCommandFixture() PrincipalCommand {
	version := uint64(1)
	return PrincipalCommand{
		ActorURA:        "easynet:///r/example/user/admin",
		IdempotencyKey:  "idem-1",
		ExpectedVersion: &version,
		Proof: PrincipalProofRef{
			Kind:      PrincipalProofBootstrap,
			Reference: "proof-1",
		},
	}
}

func principalCallFixture() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:     "easynet:///r/example/user/admin",
		CalleeURA:     "easynet:///r/example/hub",
		SubjectURA:    "easynet:///r/example/user/alice",
		NonceBase64:   "bm9uY2U=",
		CausalContext: map[string]any{"kind": "none"},
	}
}
