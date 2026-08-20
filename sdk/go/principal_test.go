package easynet

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
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

func TestPrincipalLifecycleRoutesGeneratedFromManifest(t *testing.T) {
	_, source, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	manifestPath := filepath.Join(
		filepath.Dir(source),
		"..",
		"..",
		"provider_routes",
		"runtime-principal-lifecycle-routes.v1.json",
	)
	manifest, err := os.ReadFile(manifestPath)
	if err != nil {
		t.Fatalf("read principal route manifest: %v", err)
	}
	digest := sha256.Sum256(manifest)
	if got, want := principalLifecycleRouteManifestSHA256, fmt.Sprintf("%x", digest[:]); got != want {
		t.Fatalf("principal route manifest digest = %s, want %s", got, want)
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

func TestPublicKeyBindingMarshalUsesCanonicalWirePublicKeyB64(t *testing.T) {
	publicKey := make(ed25519.PublicKey, ed25519.PublicKeySize)
	for index := range publicKey {
		publicKey[index] = byte(index)
	}
	raw, err := json.Marshal(PublicKeyBinding{
		BindingID:     "binding-1",
		PrincipalURA:  "easynet:///r/example/user/alice",
		KeyID:         "laptop",
		PublicKey:     publicKey,
		State:         PublicKeyBindingStateActive,
		CreatedUnixMS: 1_700_000_000_000,
	})
	if err != nil {
		t.Fatalf("Marshal: %v", err)
	}
	var wire map[string]any
	if err := json.Unmarshal(raw, &wire); err != nil {
		t.Fatalf("Unmarshal: %v", err)
	}
	if _, ok := wire["public_key"]; ok {
		t.Fatalf("PublicKeyBinding marshaled in-memory public_key field: %s", raw)
	}
	if wire["public_key_b64"] != base64.StdEncoding.EncodeToString(publicKey) {
		t.Fatalf("public_key_b64 mismatch: %#v", wire["public_key_b64"])
	}
}

type memoryPrincipalAbility struct {
	ability string
	args    map[string]any
}

type recordingPrincipalContextFactory struct {
	abilities []string
	subjects  []string
}

func (f *recordingPrincipalContextFactory) ContextForPrincipalAbility(
	_ context.Context,
	ability string,
) (RuntimeCallContext, error) {
	call := principalCallFixture()
	call.SubjectURA = "easynet:///r/example/resource/user.alice/invoke/" + ability
	f.abilities = append(f.abilities, ability)
	f.subjects = append(f.subjects, call.SubjectURA)
	return call, nil
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

func TestRuntimePrincipalProviderMintsContextAfterOperationSelection(t *testing.T) {
	transport := &memoryPrincipalAbility{}
	factory := &recordingPrincipalContextFactory{}
	provider, err := NewRuntimePrincipalProviderWithContextFactory(transport, factory)
	if err != nil {
		t.Fatalf("NewRuntimePrincipalProviderWithContextFactory: %v", err)
	}
	client, err := NewPrincipalClient(provider)
	if err != nil {
		t.Fatalf("NewPrincipalClient: %v", err)
	}
	ctx := context.Background()
	principalURA := "easynet:///r/example/user/alice"
	command := principalCommandFixture()
	if _, err = client.Get(ctx, principalURA); err != nil {
		t.Fatalf("Get: %v", err)
	}
	if _, err = client.Create(ctx, CreatePrincipalRequest{Command: command, PrincipalURA: principalURA}); err != nil {
		t.Fatalf("Create: %v", err)
	}
	bind := BindPrincipalKeyRequest{
		Command:      command,
		PrincipalURA: principalURA,
		KeyID:        "laptop",
		PublicKey:    make(ed25519.PublicKey, ed25519.PublicKeySize),
	}
	if _, err = client.BindFirstKey(ctx, bind); err != nil {
		t.Fatalf("BindFirstKey: %v", err)
	}
	if _, err = client.AddKey(ctx, bind); err != nil {
		t.Fatalf("AddKey: %v", err)
	}
	wantAbilities := []string{
		principalAbilityGet,
		principalAbilityCreate,
		principalAbilityBindFirstKey,
		principalAbilityAddKey,
	}
	if !reflect.DeepEqual(factory.abilities, wantAbilities) {
		t.Fatalf("factory abilities = %#v, want %#v", factory.abilities, wantAbilities)
	}
	for index, ability := range wantAbilities {
		wantSubject := "easynet:///r/example/resource/user.alice/invoke/" + ability
		if factory.subjects[index] != wantSubject {
			t.Fatalf("subject[%d] = %q, want %q", index, factory.subjects[index], wantSubject)
		}
	}
}

func TestRuntimePrincipalProviderRejectsShortNonce(t *testing.T) {
	call := principalCallFixture()
	call.NonceBase64 = "bm9uY2U="
	_, err := NewRuntimePrincipalProvider(&memoryPrincipalAbility{}, call)
	if err == nil || !strings.Contains(err.Error(), "nonce_base64 must decode to 16 bytes") {
		t.Fatalf("expected canonical nonce rejection, got %v", err)
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

func TestRuntimePrincipalProviderRejectsPrivateProjectionFields(t *testing.T) {
	tests := []struct {
		name   string
		mutate func(map[string]any)
	}{
		{
			name: "top-level private seed",
			mutate: func(principal map[string]any) {
				principal["private_key_seed"] = "forbidden"
			},
		},
		{
			name: "binding vault material",
			mutate: func(principal map[string]any) {
				bindings := principal["bindings"].([]any)
				bindings[0].(map[string]any)["vault_ciphertext"] = "forbidden"
			},
		},
		{
			name: "recovery master key",
			mutate: func(principal map[string]any) {
				principal["recovery"].(map[string]any)["master_key"] = "forbidden"
			},
		},
		{
			name: "enrollment keyring path",
			mutate: func(principal map[string]any) {
				enrollments := principal["enrollments"].([]any)
				enrollments[0].(map[string]any)["keyring_storage_path"] = "/tmp/forbidden"
			},
		},
		{
			name: "grant passphrase",
			mutate: func(principal map[string]any) {
				grants := principal["grants"].([]any)
				grants[0].(map[string]any)["passphrase"] = "forbidden"
			},
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			provider, err := NewRuntimePrincipalProvider(&privateProjectionAbility{mutate: test.mutate}, principalCallFixture())
			if err != nil {
				t.Fatalf("NewRuntimePrincipalProvider: %v", err)
			}
			_, err = provider.Get(context.Background(), "easynet:///r/example/user/alice")
			if err == nil || !IsCode(err, ErrInvalidArgument) || !strings.Contains(err.Error(), "forbidden private field") {
				t.Fatalf("private projection error = %v, want INVALID_ARGUMENT forbidden private field", err)
			}
		})
	}
}

func TestRuntimePrincipalProviderRejectsMalformedPrincipalProjection(t *testing.T) {
	tests := []struct {
		name   string
		mutate func(map[string]any) any
		want   string
	}{
		{
			name: "principal root must be object",
			mutate: func(_ map[string]any) any {
				return "not-object"
			},
			want: "principal projection must be an object",
		},
		{
			name: "bindings must be array",
			mutate: func(principal map[string]any) any {
				principal["bindings"] = "not-array"
				return principal
			},
			want: "principal.bindings must be an array",
		},
		{
			name: "binding key id is required",
			mutate: func(principal map[string]any) any {
				bindings := principal["bindings"].([]any)
				delete(bindings[0].(map[string]any), "key_id")
				return principal
			},
			want: "principal.bindings[0].key_id is required",
		},
		{
			name: "binding public key must decode",
			mutate: func(principal map[string]any) any {
				bindings := principal["bindings"].([]any)
				bindings[0].(map[string]any)["public_key_b64"] = "bad-base64"
				return principal
			},
			want: "principal.bindings[0].public_key_b64 base64 decode failed",
		},
		{
			name: "grant actions must be string array",
			mutate: func(principal map[string]any) any {
				grants := principal["grants"].([]any)
				grants[0].(map[string]any)["actions"] = []any{"principal.key.add", 42}
				return principal
			},
			want: "principal.grants[0].actions[1] must be a non-empty string",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			provider, err := NewRuntimePrincipalProvider(&malformedPrincipalProjectionAbility{mutate: test.mutate}, principalCallFixture())
			if err != nil {
				t.Fatalf("NewRuntimePrincipalProvider: %v", err)
			}
			_, err = provider.Get(context.Background(), "easynet:///r/example/user/alice")
			if err == nil || !IsCode(err, ErrInvalidArgument) || !strings.Contains(err.Error(), test.want) {
				t.Fatalf("malformed projection error = %v, want INVALID_ARGUMENT containing %q", err, test.want)
			}
		})
	}
}

type privateProjectionAbility struct {
	mutate func(map[string]any)
}

func (p *privateProjectionAbility) Invoke(ctx context.Context, call RuntimeCallContext, ability string, args any) (map[string]any, error) {
	base := &memoryPrincipalAbility{}
	output, err := base.Invoke(ctx, call, ability, args)
	if err != nil {
		return nil, err
	}
	if p.mutate != nil {
		p.mutate(output["principal"].(map[string]any))
	}
	return output, nil
}

type malformedPrincipalProjectionAbility struct {
	mutate func(map[string]any) any
}

func (m *malformedPrincipalProjectionAbility) Invoke(ctx context.Context, call RuntimeCallContext, ability string, args any) (map[string]any, error) {
	base := &memoryPrincipalAbility{}
	output, err := base.Invoke(ctx, call, ability, args)
	if err != nil {
		return nil, err
	}
	principal := output["principal"].(map[string]any)
	output["principal"] = m.mutate(principal)
	return output, nil
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
		CalleeURA:     "easynet:///r/example/authority",
		SubjectURA:    "easynet:///r/example/user/alice",
		NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext: map[string]any{"form": "none"},
	}
}
