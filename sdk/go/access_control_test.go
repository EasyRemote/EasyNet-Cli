package easynet

import (
	"context"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

type memoryAccessControlAbility struct {
	ability string
	args    map[string]any
}

func TestAccessControlRoutesGeneratedFromManifest(t *testing.T) {
	_, source, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	manifestPath := filepath.Join(
		filepath.Dir(source),
		"..",
		"..",
		"provider_routes",
		"runtime-access-control-routes.v1.json",
	)
	manifest, err := os.ReadFile(manifestPath)
	if err != nil {
		t.Fatalf("read access-control route manifest: %v", err)
	}
	digest := sha256.Sum256(manifest)
	if got, want := accessControlRouteManifestSHA256, fmt.Sprintf("%x", digest[:]); got != want {
		t.Fatalf("access-control route manifest digest = %s, want %s", got, want)
	}
}

func (m *memoryAccessControlAbility) Invoke(_ context.Context, call RuntimeCallContext, ability string, args any) (map[string]any, error) {
	if call.CallerURA == "" || call.CalleeURA == "" || call.SubjectURA == "" || call.NonceBase64 == "" || call.CausalContext == nil {
		return nil, invalidRuntimePayload("call context was not preserved", nil)
	}
	m.ability = ability
	m.args = args.(map[string]any)
	switch ability {
	case accessControlAbilityGrant:
		grant := m.args["grant"].(map[string]any)
		return map[string]any{"grant": grant, "idempotent_replay": true, "audit_record_id": "audit-1"}, nil
	case accessControlAbilityList:
		grant := map[string]any{
			"grant_id":              "grant-1",
			"owner_ura":             "easynet:///r/example/user/alice",
			"principal_kind":        "user",
			"principal_id":          "bob",
			"principal_ura":         "easynet:///r/example/user/bob",
			"token_class":           "service",
			"actions":               []any{"invoke"},
			"effect":                "allow",
			"lifetime":              "session",
			"state":                 "active",
			"created_by":            "easynet:///r/example/user/alice",
			"updated_at":            "2026-07-11T00:00:00Z",
			"review_required_after": "2026-08-11T00:00:00Z",
			"last_reviewed_at":      "2026-07-10T00:00:00Z",
			"last_used_at":          "2026-07-11T01:00:00Z",
			"reason":                "operator-approved",
		}
		return map[string]any{"grants": []any{grant}}, nil
	case accessControlAbilityCheck:
		return map[string]any{"policy_decision": map[string]any{
			"decision":       "allow",
			"owner_ura":      "easynet:///r/example/user/alice",
			"principal_kind": "user",
			"principal_ura":  "easynet:///r/example/user/bob",
			"action":         "invoke",
		}}, nil
	case accessControlAbilityRevoke:
		return map[string]any{"grant": map[string]any{
			"grant_id":          m.args["grant_id"],
			"owner_ura":         m.args["owner_ura"],
			"principal_kind":    "user",
			"principal_ura":     "easynet:///r/example/user/bob",
			"actions":           []any{"invoke"},
			"state":             "revoked",
			"created_by":        m.args["actor_ura"],
			"revoked_by":        m.args["actor_ura"],
			"revocation_reason": m.args["reason"],
		}}, nil
	case accessControlAbilityPolicyRequestCreate:
		request := m.args["request"].(map[string]any)
		return map[string]any{"request": request}, nil
	case accessControlAbilityPolicyRequestResolve:
		request := m.args["request"].(map[string]any)
		createdGrant := m.args["created_grant"].(map[string]any)
		return map[string]any{
			"request":           request,
			"created_grant":     createdGrant,
			"authority_proof":   map[string]any{"proof_id": "proof-1", "owner_ura": "easynet:///r/example/user/alice", "principal_kind": "user", "principal_id": "bob", "principal_ura": "easynet:///r/example/user/bob"},
			"idempotent_replay": true,
		}, nil
	case accessControlAbilityPolicyRequestList:
		return map[string]any{"requests": []any{map[string]any{
			"request_id":     "request-1",
			"owner_ura":      "easynet:///r/example/user/alice",
			"principal_kind": "user",
			"principal_id":   "bob",
			"principal_ura":  "easynet:///r/example/user/bob",
			"callee_ura":     "easynet:///r/example/device/dev-a",
			"subject_ura":    "easynet:///r/example/resource/user.alice/session/session-1",
			"ability_ura":    "easynet:///r/example/device/dev-a/ability/device.observe.health",
			"action":         "invoke",
			"status":         "pending",
		}}}, nil
	case accessControlAbilityAdmissionExplain:
		return map[string]any{
			"observer_ura":       "easynet:///r/example/user/alice",
			"redacted":           true,
			"redaction_reason":   "not_owner",
			"authority_reason":   "observer redacted",
			"root_trace":         map[string]any{"invocation_id": "inv-1", "stage": "admission", "redacted": true},
			"policy_decision":    map[string]any{"decision": "deny", "reason": "not_owner"},
			"signature_decision": map[string]any{"decision": "allow"},
		}, nil
	default:
		return nil, invalidRuntimePayload("unexpected ability", nil)
	}
}

func TestRuntimeAccessControlProviderGrantsWithCanonicalPrincipalURAs(t *testing.T) {
	transport := &memoryAccessControlAbility{}
	provider, err := NewRuntimeAccessControlProvider(transport)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
	}

	result, err := provider.Grant(context.Background(), AccessControlGrantRequest{
		Call: accessControlCallFixture(),
		Grant: AccessControlGrant{
			GrantID:           "grant-1",
			OwnerURA:          "easynet:///r/example/user/alice",
			PrincipalKind:     AccessControlPrincipalUser,
			PrincipalURA:      "easynet:///r/example/user/bob",
			TokenClass:        "service",
			AbilityURAPattern: "easynet:///r/example/device/dev-a/ability/device.observe.health",
			Actions:           []string{"invoke"},
			Lifetime:          "session",
			CreatedBy:         "easynet:///r/example/user/alice",
			UpdatedAt:         "2026-07-11T00:00:00Z",
			LastUsedAt:        "2026-07-11T01:00:00Z",
			Reason:            "operator-approved",
		},
	})
	if err != nil {
		t.Fatalf("Grant: %v", err)
	}
	if transport.ability != accessControlAbilityGrant {
		t.Fatalf("ability = %s", transport.ability)
	}
	grant := transport.args["grant"].(map[string]any)
	if transport.args["owner_ura"] != "easynet:///r/example/user/alice" || transport.args["principal_ura"] != "easynet:///r/example/user/bob" {
		t.Fatalf("canonical boundary URAs missing: %#v", transport.args)
	}
	if _, ok := grant["owner_user_id"]; ok {
		t.Fatalf("owner storage key leaked into grant wire: %#v", grant)
	}
	if _, ok := grant["principal_id"]; ok {
		t.Fatalf("principal storage key leaked into grant wire: %#v", grant)
	}
	if grant["token_class"] != "service" || grant["lifetime"] != "session" || grant["last_used_at"] != "2026-07-11T01:00:00Z" || grant["reason"] != "operator-approved" {
		t.Fatalf("grant lifecycle fields not lowered: %#v", grant)
	}
	if result.Grant.OwnerURA != "easynet:///r/example/user/alice" || result.Grant.PrincipalURA != "easynet:///r/example/user/bob" {
		t.Fatalf("canonical grant projection lost: %#v", result.Grant)
	}
	if !result.IdempotentReplay {
		t.Fatalf("idempotent replay projection lost: %#v", result)
	}
	if _, ok := transport.args["backend_account_id"]; ok {
		t.Fatalf("product account field leaked into SDK args: %#v", transport.args)
	}
}

func TestRuntimeAccessControlProviderListsAndChecksCanonicalPolicies(t *testing.T) {
	transport := &memoryAccessControlAbility{}
	provider, err := NewRuntimeAccessControlProvider(transport)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
	}

	list, err := provider.List(context.Background(), AccessControlListRequest{
		Call:          accessControlCallFixture(),
		OwnerURA:      "easynet:///r/example/user/alice",
		PrincipalKind: AccessControlPrincipalUser,
		PrincipalURA:  "easynet:///r/example/user/bob",
		AbilityURA:    "easynet:///r/example/device/dev-a/ability/device.observe.health",
		SubjectURA:    "easynet:///r/example/resource/user.alice/session/session-1",
		Action:        "invoke",
		Limit:         10,
		Cursor:        "cursor-1",
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(list.Grants) != 1 || list.Grants[0].GrantID != "grant-1" {
		t.Fatalf("unexpected grants: %#v", list)
	}
	if transport.args["owner_ura"] != "easynet:///r/example/user/alice" || transport.args["principal_ura"] != "easynet:///r/example/user/bob" || transport.args["limit"] != uint32(10) || transport.args["cursor"] != "cursor-1" {
		t.Fatalf("list args not canonicalized: %#v", transport.args)
	}
	if _, ok := transport.args["owner_user_id"]; ok {
		t.Fatalf("owner storage key leaked into list args: %#v", transport.args)
	}
	if _, ok := transport.args["principal_id"]; ok {
		t.Fatalf("principal storage key leaked into list args: %#v", transport.args)
	}
	if list.Grants[0].TokenClass != "service" || list.Grants[0].Lifetime != "session" || list.Grants[0].Reason != "operator-approved" {
		t.Fatalf("grant lifecycle projection lost: %#v", list.Grants[0])
	}

	check, err := provider.Check(context.Background(), AccessControlCheckRequest{
		Call:          accessControlCallFixture(),
		OwnerURA:      "easynet:///r/example/user/alice",
		OwnerSource:   "subject",
		PrincipalKind: AccessControlPrincipalUser,
		PrincipalURA:  "easynet:///r/example/user/bob",
		CalleeURA:     "easynet:///r/example/device/dev-a",
		SubjectURA:    "easynet:///r/example/resource/user.alice/session/session-1",
		AbilityURA:    "easynet:///r/example/device/dev-a/ability/device.observe.health",
		Action:        "invoke",
		SafeRead:      true,
	})
	if err != nil {
		t.Fatalf("Check: %v", err)
	}
	if check.PolicyDecision.Decision != "allow" {
		t.Fatalf("unexpected decision: %#v", check)
	}
	if transport.args["owner_source"] != "subject" {
		t.Fatalf("check owner_source was not explicit: %#v", transport.args)
	}
}

func TestRuntimeAccessControlProviderCheckRequiresExplicitOwnerSource(t *testing.T) {
	transport := &memoryAccessControlAbility{}
	provider, err := NewRuntimeAccessControlProvider(transport)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
	}

	_, err = provider.Check(context.Background(), AccessControlCheckRequest{
		Call:          accessControlCallFixture(),
		OwnerURA:      "easynet:///r/example/user/alice",
		PrincipalKind: AccessControlPrincipalUser,
		PrincipalURA:  "easynet:///r/example/user/bob",
		CalleeURA:     "easynet:///r/example/device/dev-a",
		SubjectURA:    "easynet:///r/example/resource/user.alice/session/session-1",
		AbilityURA:    "easynet:///r/example/device/dev-a/ability/device.observe.health",
		Action:        "invoke",
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing owner_source error = %v", err)
	}
	if transport.ability != "" {
		t.Fatalf("missing owner_source should fail before provider invoke, got %s", transport.ability)
	}
}

func TestRuntimeAccessControlProviderRevokeRequiresCanonicalActorURA(t *testing.T) {
	transport := &memoryAccessControlAbility{}
	provider, err := NewRuntimeAccessControlProvider(transport)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
	}

	grant, err := provider.Revoke(context.Background(), AccessControlRevokeRequest{
		Call:     accessControlCallFixture(),
		OwnerURA: "easynet:///r/example/user/alice",
		GrantID:  "grant-1",
		ActorURA: "easynet:///r/example/user/alice",
		Reason:   "operator request",
	})
	if err != nil {
		t.Fatalf("Revoke: %v", err)
	}
	if transport.ability != accessControlAbilityRevoke {
		t.Fatalf("ability = %s", transport.ability)
	}
	if transport.args["actor_ura"] != "easynet:///r/example/user/alice" {
		t.Fatalf("validated actor_ura missing from revoke wire: %#v", transport.args)
	}
	if _, ok := transport.args["owner_user_id"]; ok {
		t.Fatalf("scalar owner leaked into revoke wire: %#v", transport.args)
	}
	if grant.State != AccessControlGrantRevoked || grant.RevokedBy != "easynet:///r/example/user/alice" {
		t.Fatalf("revocation projection lost canonical actor: %#v", grant)
	}

	_, err = provider.Revoke(context.Background(), AccessControlRevokeRequest{
		Call:     accessControlCallFixture(),
		OwnerURA: "easynet:///r/example/user/alice",
		GrantID:  "grant-1",
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing actor_ura error = %v", err)
	}

	_, err = provider.Revoke(context.Background(), AccessControlRevokeRequest{
		Call:     accessControlCallFixture(),
		OwnerURA: "easynet:///r/example/user/alice",
		GrantID:  "grant-1",
		ActorURA: "alice",
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("scalar actor_ura error = %v", err)
	}
}

func TestRuntimeAccessControlProviderRejectsNonUserOwnerURA(t *testing.T) {
	provider, err := NewRuntimeAccessControlProvider(&memoryAccessControlAbility{})
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
	}

	_, err = provider.List(context.Background(), AccessControlListRequest{
		Call:     accessControlCallFixture(),
		OwnerURA: "easynet:///r/example/device/dev-a",
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("List error = %v", err)
	}
}

func TestRuntimeAccessControlProviderRejectsScalarPrincipalMutationInputs(t *testing.T) {
	tests := []struct {
		name string
		run  func(*RuntimeAccessControlProvider) error
	}{
		{
			name: "grant",
			run: func(provider *RuntimeAccessControlProvider) error {
				_, err := provider.Grant(context.Background(), AccessControlGrantRequest{
					Call: accessControlCallFixture(),
					Grant: AccessControlGrant{
						GrantID:       "grant-1",
						OwnerURA:      "easynet:///r/example/user/alice",
						PrincipalKind: AccessControlPrincipalUser,
						PrincipalID:   "bob",
						Actions:       []string{"invoke"},
						CreatedBy:     "easynet:///r/example/user/alice",
					},
				})
				return err
			},
		},
		{
			name: "list",
			run: func(provider *RuntimeAccessControlProvider) error {
				_, err := provider.List(context.Background(), AccessControlListRequest{
					Call:          accessControlCallFixture(),
					OwnerURA:      "easynet:///r/example/user/alice",
					PrincipalKind: AccessControlPrincipalUser,
					PrincipalID:   "bob",
				})
				return err
			},
		},
		{
			name: "permission_request_create",
			run: func(provider *RuntimeAccessControlProvider) error {
				request := accessControlPermissionRequestFixture()
				request.PrincipalURA = ""
				request.PrincipalID = "bob"
				_, err := provider.CreateRequest(context.Background(), AccessControlPermissionRequestCreateRequest{
					Call:    accessControlCallFixture(),
					Request: request,
				})
				return err
			},
		},
		{
			name: "permission_request_list",
			run: func(provider *RuntimeAccessControlProvider) error {
				_, err := provider.ListRequests(context.Background(), AccessControlPermissionRequestListRequest{
					Call:          accessControlCallFixture(),
					OwnerURA:      "easynet:///r/example/user/alice",
					PrincipalKind: AccessControlPrincipalUser,
					PrincipalID:   "bob",
				})
				return err
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			transport := &memoryAccessControlAbility{}
			provider, err := NewRuntimeAccessControlProvider(transport)
			if err != nil {
				t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
			}
			err = tt.run(provider)
			if !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("error = %v", err)
			}
			if transport.ability != "" {
				t.Fatalf("scalar principal reached runtime ability %s with args %#v", transport.ability, transport.args)
			}
		})
	}
}

func TestRuntimeAccessControlProviderManagesPermissionRequests(t *testing.T) {
	transport := &memoryAccessControlAbility{}
	provider, err := NewRuntimeAccessControlProvider(transport)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
	}

	created, err := provider.CreateRequest(context.Background(), AccessControlPermissionRequestCreateRequest{
		Call:     accessControlCallFixture(),
		Request:  accessControlPermissionRequestFixture(),
		ActorURA: "easynet:///r/example/user/alice",
	})
	if err != nil {
		t.Fatalf("CreateRequest: %v", err)
	}
	if created.RequestID != "request-1" || transport.ability != accessControlAbilityPolicyRequestCreate {
		t.Fatalf("unexpected created request: %#v ability=%s", created, transport.ability)
	}
	requestWire := transport.args["request"].(map[string]any)
	if requestWire["ability_ura"] == "" {
		t.Fatalf("permission request not lowered canonically: %#v", requestWire)
	}
	if _, ok := requestWire["owner_user_id"]; ok {
		t.Fatalf("owner storage key leaked into permission request wire: %#v", requestWire)
	}
	if _, ok := requestWire["principal_id"]; ok {
		t.Fatalf("principal storage key leaked into permission request wire: %#v", requestWire)
	}

	grant := AccessControlGrant{
		GrantID:       "grant-1",
		OwnerURA:      "easynet:///r/example/user/alice",
		PrincipalKind: AccessControlPrincipalUser,
		PrincipalURA:  "easynet:///r/example/user/bob",
		Actions:       []string{"invoke"},
		CreatedBy:     "easynet:///r/example/user/alice",
	}
	resolved, err := provider.ResolveRequest(context.Background(), AccessControlPermissionRequestResolveRequest{
		Call:           accessControlCallFixture(),
		Request:        accessControlPermissionRequestFixture(),
		CreatedGrant:   &grant,
		AuthorityProof: &AccessControlAuthorityProof{ProofID: "proof-1"},
		ActorURA:       "easynet:///r/example/user/alice",
	})
	if err != nil {
		t.Fatalf("ResolveRequest: %v", err)
	}
	if !resolved.IdempotentReplay || resolved.CreatedGrant == nil || resolved.AuthorityProof == nil {
		t.Fatalf("resolution projection lost: %#v", resolved)
	}
	authorityProof := transport.args["authority_proof"].(map[string]any)
	if authorityProof["owner_ura"] != "easynet:///r/example/user/alice" || authorityProof["principal_ura"] != "easynet:///r/example/user/bob" {
		t.Fatalf("authority proof did not inherit canonical URAs: %#v", authorityProof)
	}
	if _, ok := authorityProof["owner_user_id"]; ok {
		t.Fatalf("owner storage key leaked into authority proof wire: %#v", authorityProof)
	}
	if _, ok := authorityProof["principal_id"]; ok {
		t.Fatalf("principal storage key leaked into authority proof wire: %#v", authorityProof)
	}

	listed, err := provider.ListRequests(context.Background(), AccessControlPermissionRequestListRequest{
		Call:          accessControlCallFixture(),
		OwnerURA:      "easynet:///r/example/user/alice",
		PrincipalKind: AccessControlPrincipalUser,
		PrincipalURA:  "easynet:///r/example/user/bob",
		Status:        "pending",
		Limit:         10,
		Cursor:        "cursor-1",
	})
	if err != nil {
		t.Fatalf("ListRequests: %v", err)
	}
	if len(listed.Requests) != 1 || listed.Requests[0].RequestID != "request-1" || transport.args["cursor"] != "cursor-1" {
		t.Fatalf("request list projection lost: %#v args=%#v", listed, transport.args)
	}
}

func TestRuntimeAccessControlProviderExplainsAdmission(t *testing.T) {
	transport := &memoryAccessControlAbility{}
	provider, err := NewRuntimeAccessControlProvider(transport)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlProvider: %v", err)
	}

	result, err := provider.Explain(context.Background(), AccessControlAdmissionExplainRequest{
		Call:         accessControlCallFixture(),
		ObserverURA:  "easynet:///r/example/user/alice",
		InvocationID: "inv-1",
	})
	if err != nil {
		t.Fatalf("Explain: %v", err)
	}
	if transport.ability != accessControlAbilityAdmissionExplain || !result.Redacted || result.RootTrace == nil || result.PolicyDecision == nil {
		t.Fatalf("admission explain projection lost: %#v ability=%s", result, transport.ability)
	}
}

func accessControlPermissionRequestFixture() AccessControlPermissionRequest {
	return AccessControlPermissionRequest{
		RequestID:          "request-1",
		OwnerURA:           "easynet:///r/example/user/alice",
		PrincipalKind:      AccessControlPrincipalUser,
		PrincipalURA:       "easynet:///r/example/user/bob",
		CalleeURA:          "easynet:///r/example/device/dev-a",
		SubjectURA:         "easynet:///r/example/resource/user.alice/session/session-1",
		AbilityURA:         "easynet:///r/example/device/dev-a/ability/device.observe.health",
		Action:             "invoke",
		RequestedLifetimes: []string{"session"},
		Status:             "pending",
	}
}

func accessControlCallFixture() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:     "easynet:///r/example/user/alice",
		CalleeURA:     "easynet:///r/example/device/dev-a",
		SubjectURA:    "easynet:///r/example/resource/user.alice/access-control",
		NonceBase64:   "bm9uY2U=",
		CausalContext: map[string]any{"form": "none"},
	}
}
