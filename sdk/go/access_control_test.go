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
		return map[string]any{"grant": grant, "audit_record_id": "audit-1"}, nil
	case accessControlAbilityList:
		grant := map[string]any{
			"grant_id":       "grant-1",
			"owner_ura":      "easynet:///r/example/user/alice",
			"principal_kind": "user",
			"principal_ura":  "easynet:///r/example/user/bob",
			"actions":        []any{"invoke"},
			"effect":         "allow",
			"state":          "active",
			"created_by":     "easynet:///r/example/user/alice",
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
			AbilityURAPattern: "easynet:///r/example/device/dev-a/ability/device.observe.health",
			Actions:           []string{"invoke"},
			CreatedBy:         "easynet:///r/example/user/alice",
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
	if result.Grant.OwnerURA != "easynet:///r/example/user/alice" || result.Grant.PrincipalURA != "easynet:///r/example/user/bob" {
		t.Fatalf("canonical grant projection lost: %#v", result.Grant)
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
		Action:        "invoke",
		Limit:         10,
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(list.Grants) != 1 || list.Grants[0].GrantID != "grant-1" {
		t.Fatalf("unexpected grants: %#v", list)
	}
	if transport.args["owner_user_id"] != "alice" || transport.args["principal_id"] != "bob" || transport.args["limit"] != uint32(10) {
		t.Fatalf("list args not canonicalized: %#v", transport.args)
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

func accessControlCallFixture() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:     "easynet:///r/example/user/alice",
		CalleeURA:     "easynet:///r/example/device/dev-a",
		SubjectURA:    "easynet:///r/example/resource/user.alice/access-control",
		NonceBase64:   "bm9uY2U=",
		CausalContext: map[string]any{"kind": "none"},
	}
}
