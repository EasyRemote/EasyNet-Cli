package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

func TestAccessControlRuntimeTransportBuildsAuthorityGrantInvocation(t *testing.T) {
	identityTransport := newAccessControlRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: accessControlGrantOutputJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAccessControlClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlClient: %v", err)
	}

	result, err := client.GrantWithRequest(context.Background(), AuthorityBindingGrantRequest{
		Carrier: accessControlCarrierForTest(),
		Grant: PermissionGrant{
			GrantID: "grant-1", OwnerUserID: "alice", PrincipalKind: PrincipalToken,
			PrincipalID: "token-principal", Actions: []AccessAction{AccessRead},
			Effect: PermissionAllow, Lifetime: "permanent", State: PermissionGrantActive,
			CreatedBy: "easynet:///r/example/user/alice", CreatedAt: "2026-07-09T00:00:00Z",
		},
		ActorURA: "easynet:///r/example/user/alice",
	})
	if err != nil {
		t.Fatalf("GrantWithRequest: %v", err)
	}
	if result.Grant.GrantID != "grant-1" || result.AuditRecordID != "audit-1" {
		t.Fatalf("unexpected grant result: %#v", result)
	}

	draft := runtimeTransport.seenDraft
	if draft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.authority.binding.grant@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", draft["descriptor_ref"])
	}
	metadata := draft["metadata"].(map[string]any)
	if metadata["profile"] != accessControlProfile ||
		metadata["system_ability"] != accessControlAbilityAuthorityBindingGrant ||
		metadata["carrier_owner"] != "daemon_sdk" {
		t.Fatalf("unexpected metadata: %#v", metadata)
	}
	args := draft["args"].(map[string]any)
	if _, ok := args["carrier"]; ok {
		t.Fatalf("runtime carrier leaked into daemon ability args: %#v", args)
	}
	if args["actor_ura"] != "easynet:///r/example/user/alice" {
		t.Fatalf("actor_ura missing from args: %#v", args)
	}
	if len(identityTransport.seenBuildURA) != 1 ||
		identityTransport.seenBuildURA[0]["ability_name"] != accessControlAbilityAuthorityBindingGrant {
		t.Fatalf("ability name was not delegated through identity client: %#v", identityTransport.seenBuildURA)
	}
}

func TestAccessControlRuntimeTransportDispatchesRFC014AbilityMatrix(t *testing.T) {
	identityTransport := newAccessControlRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: `{"grants":[]}`}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	transport, err := NewAccessControlRuntimeTransport(runtime, identity)
	if err != nil {
		t.Fatalf("NewAccessControlRuntimeTransport: %v", err)
	}
	base := accessControlCarrierForTest()
	cases := []struct {
		name    string
		ability string
		call    func(context.Context, []byte) ([]byte, error)
		raw     []byte
	}{
		{"grant", accessControlAbilityAuthorityBindingGrant, transport.GrantAuthorityBinding, mustAccessControlJSON(t, AuthorityBindingGrantRequest{Carrier: base, Grant: PermissionGrant{GrantID: "grant-1"}, ActorURA: "easynet:///r/example/user/alice"})},
		{"revoke", accessControlAbilityAuthorityBindingRevoke, transport.RevokeAuthorityBinding, mustAccessControlJSON(t, AuthorityBindingRevokeRequest{Carrier: base, OwnerUserID: "alice", GrantID: "grant-1", ActorURA: "easynet:///r/example/user/alice"})},
		{"list grants", accessControlAbilityAuthorityBindingList, transport.ListAuthorityBindings, mustAccessControlJSON(t, AuthorityBindingListRequest{Carrier: base, OwnerUserID: "alice"})},
		{"check", accessControlAbilityAuthorityBindingCheck, transport.CheckAuthorityBinding, mustAccessControlJSON(t, AuthorityBindingCheckRequest{Carrier: base, CallerURA: base.CallerURA, PrincipalKind: PrincipalToken, PrincipalID: "token-principal", CalleeURA: base.CalleeURA, SubjectURA: base.SubjectURA, AbilityURA: "easynet:///r/example/ability/device.dev-a.terminal.create", Action: AccessStream})},
		{"create request", accessControlAbilityPolicyRequestCreate, transport.CreatePolicyRequest, mustAccessControlJSON(t, PolicyRequestCreateRequest{Carrier: base, Request: PermissionRequest{RequestID: "req-1"}, ActorURA: "easynet:///r/example/user/alice"})},
		{"resolve request", accessControlAbilityPolicyRequestResolve, transport.ResolvePolicyRequest, mustAccessControlJSON(t, PolicyRequestResolveRequest{Carrier: base, Request: PermissionRequest{RequestID: "req-1"}, ActorURA: "easynet:///r/example/user/alice"})},
		{"list requests", accessControlAbilityPolicyRequestList, transport.ListPolicyRequests, mustAccessControlJSON(t, PolicyRequestListRequest{Carrier: base, OwnerUserID: "alice"})},
		{"explain", accessControlAbilityAdmissionExplain, transport.ExplainAdmission, mustAccessControlJSON(t, AdmissionExplainRequest{Carrier: base, ObserverURA: "easynet:///r/example/user/alice", TraceID: "trace-1"})},
	}

	for _, tc := range cases {
		runtimeTransport.outputJSON = outputForAccessControlAbility(tc.ability)
		if _, err := tc.call(context.Background(), tc.raw); err != nil {
			t.Fatalf("%s: %v", tc.name, err)
		}
		metadata := runtimeTransport.seenDraft["metadata"].(map[string]any)
		if metadata["system_ability"] != tc.ability {
			t.Fatalf("%s system_ability = %#v, want %s", tc.name, metadata["system_ability"], tc.ability)
		}
	}
}

func TestAccessControlRuntimeTransportRequiresCarrier(t *testing.T) {
	identity, err := NewIdentityClient(newAccessControlRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: `{"grants":[]}`})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAccessControlClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeAccessControlClient: %v", err)
	}
	_, err = client.ListGrants(context.Background(), map[string]any{"owner_user_id": "alice"})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ListGrants without runtime carrier = %v, want %s", err, ErrInvalidArgument)
	}
}

func accessControlCarrierForTest() AccessControlCarrierBase {
	return AccessControlCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "access-control-1"},
	}
}

func newAccessControlRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	abilityByName := map[string]string{
		accessControlAbilityAuthorityBindingGrant:  "easynet:///r/example/ability/device.dev-a.authority.binding.grant",
		accessControlAbilityAuthorityBindingRevoke: "easynet:///r/example/ability/device.dev-a.authority.binding.revoke",
		accessControlAbilityAuthorityBindingList:   "easynet:///r/example/ability/device.dev-a.authority.binding.list",
		accessControlAbilityAuthorityBindingCheck:  "easynet:///r/example/ability/device.dev-a.authority.binding.check",
		accessControlAbilityPolicyRequestCreate:    "easynet:///r/example/ability/device.dev-a.policy.request.create",
		accessControlAbilityPolicyRequestResolve:   "easynet:///r/example/ability/device.dev-a.policy.request.resolve",
		accessControlAbilityPolicyRequestList:      "easynet:///r/example/ability/device.dev-a.policy.request.list",
		accessControlAbilityAdmissionExplain:       "easynet:///r/example/ability/device.dev-a.admission.explain",
	}
	descriptorByAbility := map[string]string{}
	for _, abilityURA := range abilityByName {
		descriptorByAbility[abilityURA] = abilityURA + "@1.0.0"
	}
	return &compatibilityRuntimeIdentityTransport{
		abilityByName:       abilityByName,
		descriptorByAbility: descriptorByAbility,
	}
}

func outputForAccessControlAbility(ability string) string {
	switch ability {
	case accessControlAbilityAuthorityBindingGrant:
		return accessControlGrantOutputJSON
	case accessControlAbilityAuthorityBindingRevoke:
		return `{"grant":{"grant_id":"grant-1","owner_user_id":"alice","principal_kind":"token","principal_id":"token-principal","actions":["read"],"effect":"allow","lifetime":"permanent","state":"revoked","created_by":"easynet:///r/example/user/alice","created_at":"2026-07-09T00:00:00Z"}}`
	case accessControlAbilityAuthorityBindingList:
		return `{"grants":[]}`
	case accessControlAbilityAuthorityBindingCheck:
		return `{"policy_decision":{"decision":"deny","reason":"NON_INTERACTIVE_DENY","owner_source":"subject","caller_ura":"c","principal_kind":"token","principal_id":"p","callee_ura":"d","subject_ura":"s","ability_ura":"a","action":"stream"}}`
	case accessControlAbilityPolicyRequestCreate:
		return `{"request":{"request_id":"req-1","owner_user_id":"alice","caller_ura":"c","principal_kind":"token","principal_id":"p","callee_ura":"d","subject_ura":"s","ability_ura":"a","action":"stream","requested_lifetimes":["session"],"status":"pending","created_at":"t","expires_at":"e"}}`
	case accessControlAbilityPolicyRequestResolve:
		return `{"request":{"request_id":"req-1","owner_user_id":"alice","caller_ura":"c","principal_kind":"token","principal_id":"p","callee_ura":"d","subject_ura":"s","ability_ura":"a","action":"stream","requested_lifetimes":["session"],"status":"approved","created_at":"t","expires_at":"e"}}`
	case accessControlAbilityPolicyRequestList:
		return `{"requests":[]}`
	case accessControlAbilityAdmissionExplain:
		return `{"observer_ura":"easynet:///r/example/user/alice","redacted":true,"authority_reason":"AUTHORITY_PROOF_MISSING"}`
	default:
		return `{}`
	}
}

func mustAccessControlJSON(t *testing.T, value any) []byte {
	t.Helper()
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatalf("marshal access-control test request: %v", err)
	}
	return raw
}

const accessControlGrantOutputJSON = `{"grant":{"grant_id":"grant-1","owner_user_id":"alice","principal_kind":"token","principal_id":"token-principal","actions":["read"],"effect":"allow","lifetime":"permanent","state":"active","created_by":"easynet:///r/example/user/alice","created_at":"2026-07-09T00:00:00Z"},"idempotent_replay":false,"audit_record_id":"audit-1"}`
