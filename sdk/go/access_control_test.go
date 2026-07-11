package easynet

import (
	"context"
	"testing"
)

type memoryAccessControlAbility struct {
	ability string
	args    map[string]any
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
	if grant["owner_user_id"] != "alice" || grant["principal_id"] != "bob" {
		t.Fatalf("canonical URAs not lowered to current daemon wire: %#v", grant)
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
	if transport.args["owner_user_id"] != "alice" || transport.args["principal_id"] != "bob" || transport.args["limit"] != uint32(10) || transport.args["cursor"] != "cursor-1" {
		t.Fatalf("list args not canonicalized: %#v", transport.args)
	}
	if list.Grants[0].TokenClass != "service" || list.Grants[0].Lifetime != "session" || list.Grants[0].Reason != "operator-approved" {
		t.Fatalf("grant lifecycle projection lost: %#v", list.Grants[0])
	}

	check, err := provider.Check(context.Background(), AccessControlCheckRequest{
		Call:          accessControlCallFixture(),
		OwnerURA:      "easynet:///r/example/user/alice",
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
	if requestWire["owner_user_id"] != "alice" || requestWire["principal_id"] != "bob" || requestWire["ability_ura"] == "" {
		t.Fatalf("permission request not lowered canonically: %#v", requestWire)
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
		Call:         accessControlCallFixture(),
		Request:      accessControlPermissionRequestFixture(),
		CreatedGrant: &grant,
		ActorURA:     "easynet:///r/example/user/alice",
	})
	if err != nil {
		t.Fatalf("ResolveRequest: %v", err)
	}
	if !resolved.IdempotentReplay || resolved.CreatedGrant == nil || resolved.AuthorityProof == nil {
		t.Fatalf("resolution projection lost: %#v", resolved)
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
		CausalContext: map[string]any{"kind": "none"},
	}
}
