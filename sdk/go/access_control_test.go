package easynet

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

type fakeAccessControlTransport struct {
	last map[string]any
}

func (f *fakeAccessControlTransport) decode(raw []byte) {
	_ = json.Unmarshal(raw, &f.last)
}

func (f *fakeAccessControlTransport) GrantAuthorityBinding(_ context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"grant":{"grant_id":"grant-1","owner_user_id":"alice","principal_kind":"token","principal_id":"token-principal","actions":["read"],"effect":"allow","lifetime":"permanent","state":"active","created_by":"easynet:///r/test/user/alice","created_at":"2026-07-09T00:00:00Z"},"idempotent_replay":false,"audit_record_id":"audit-1"}`), nil
}
func (f *fakeAccessControlTransport) RevokeAuthorityBinding(_ context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"grant":{"grant_id":"grant-1","owner_user_id":"alice","principal_kind":"token","principal_id":"token-principal","actions":["read"],"effect":"allow","lifetime":"permanent","state":"revoked","created_by":"easynet:///r/test/user/alice","created_at":"2026-07-09T00:00:00Z"}}`), nil
}
func (f *fakeAccessControlTransport) ListAuthorityBindings(_ context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"grants":[]}`), nil
}
func (f *fakeAccessControlTransport) CheckAuthorityBinding(_ context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"policy_decision":{"decision":"deny","reason":"NON_INTERACTIVE_DENY","owner_source":"subject","caller_ura":"c","principal_kind":"token","principal_id":"p","callee_ura":"d","subject_ura":"s","ability_ura":"a","action":"stream"}}`), nil
}
func (f *fakeAccessControlTransport) CreatePolicyRequest(_ context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"request":{"request_id":"req-1","owner_user_id":"alice","caller_ura":"c","principal_kind":"token","principal_id":"p","callee_ura":"d","subject_ura":"s","ability_ura":"a","action":"stream","requested_lifetimes":["session"],"status":"pending","created_at":"t","expires_at":"e"}}`), nil
}
func (f *fakeAccessControlTransport) ResolvePolicyRequest(ctx context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"request":{"request_id":"req-1","owner_user_id":"alice","caller_ura":"c","principal_kind":"token","principal_id":"p","callee_ura":"d","subject_ura":"s","ability_ura":"terminal.create","action":"stream","requested_lifetimes":["session"],"status":"approved","created_at":"t","expires_at":"e","created_grant_id":"grant-approval-1"},"created_grant":{"grant_id":"grant-approval-1","owner_user_id":"alice","principal_kind":"token","principal_id":"p","callee_ura":"d","subject_ura_pattern":"s","ability_ura_pattern":"terminal.create","actions":["stream"],"effect":"allow","lifetime":"session","state":"active","created_by":"easynet:///r/test/user/alice","created_at":"t"},"idempotent_replay":false}`), nil
}
func (f *fakeAccessControlTransport) ListPolicyRequests(_ context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"requests":[]}`), nil
}
func (f *fakeAccessControlTransport) ExplainAdmission(_ context.Context, raw []byte) ([]byte, error) {
	f.decode(raw)
	return []byte(`{"observer_ura":"easynet:///r/test/user/alice","redacted":true,"authority_reason":"AUTHORITY_PROOF_MISSING"}`), nil
}

func TestAccessControlClientGrantUsesTypedTransport(t *testing.T) {
	transport := &fakeAccessControlTransport{}
	client, err := NewAccessControlClient(transport)
	if err != nil {
		t.Fatal(err)
	}
	result, err := client.Grant(context.Background(), PermissionGrant{
		GrantID: "grant-1", OwnerUserID: "alice", PrincipalKind: PrincipalToken,
		PrincipalID: "token-principal", Actions: []AccessAction{AccessRead},
		Effect: PermissionAllow, Lifetime: "permanent", State: PermissionGrantActive,
		CreatedBy: "easynet:///r/test/user/alice", CreatedAt: "2026-07-09T00:00:00Z",
	}, "easynet:///r/test/user/alice")
	if err != nil {
		t.Fatal(err)
	}
	if result.Grant.GrantID != "grant-1" || result.AuditRecordID != "audit-1" {
		t.Fatalf("unexpected grant result: %#v", result)
	}
	if transport.last["actor_ura"] != "easynet:///r/test/user/alice" {
		t.Fatalf("actor_ura not serialized: %#v", transport.last)
	}
}

func TestAccessControlClientExplainProjectsRFC014DTO(t *testing.T) {
	client, _ := NewAccessControlClient(&fakeAccessControlTransport{})
	result, err := client.Explain(context.Background(), map[string]any{
		"observer_ura": "easynet:///r/test/user/alice",
	})
	if err != nil {
		t.Fatal(err)
	}
	if !result.Redacted || result.AuthorityReason != "AUTHORITY_PROOF_MISSING" {
		t.Fatalf("unexpected explain result: %#v", result)
	}
}

func TestAccessControlClientResolveRequestWithGrantUsesTypedResolution(t *testing.T) {
	transport := &fakeAccessControlTransport{}
	client, err := NewAccessControlClient(transport)
	if err != nil {
		t.Fatal(err)
	}
	request := PermissionRequest{
		RequestID: "req-1", OwnerUserID: "alice", CallerURA: "c",
		PrincipalKind: PrincipalToken, PrincipalID: "p", CalleeURA: "d",
		SubjectURA: "s", AbilityURA: "terminal.create", Action: AccessStream,
		RequestedLifetimes: []string{"session"}, Status: "approved",
		CreatedAt: "t", ExpiresAt: "e",
	}
	grant := PermissionGrant{
		GrantID: "grant-approval-1", OwnerUserID: "alice", PrincipalKind: PrincipalToken,
		PrincipalID: "p", CalleeURA: "d", SubjectURAPattern: "s",
		AbilityURAPattern: "terminal.create", Actions: []AccessAction{AccessStream},
		Effect: PermissionAllow, Lifetime: "session", State: PermissionGrantActive,
		CreatedBy: "easynet:///r/test/user/alice", CreatedAt: "t",
	}
	result, err := client.ResolveRequestWithGrant(
		context.Background(),
		request,
		grant,
		"easynet:///r/test/user/alice",
	)
	if err != nil {
		t.Fatal(err)
	}
	if result.Request.CreatedGrantID != "grant-approval-1" {
		t.Fatalf("request resolution did not decode created grant id: %#v", result)
	}
	if result.CreatedGrant == nil || result.CreatedGrant.GrantID != "grant-approval-1" {
		t.Fatalf("created_grant not decoded: %#v", result)
	}
	if _, ok := transport.last["created_grant"].(map[string]any); !ok {
		t.Fatalf("created_grant not serialized: %#v", transport.last)
	}
}

func TestAccessControlSharedRFC014FixturesDecode(t *testing.T) {
	fixtureRoot := filepath.Join("..", "conformance", "fixtures")
	cases := map[string]any{
		"access-control-permission-grant.v4.json":         &PermissionGrant{},
		"access-control-permission-request.v4.json":       &PermissionRequest{},
		"access-control-authority-proof.v4.json":          &AuthorityProof{},
		"access-control-policy-decision.v4.json":          &PolicyDecision{},
		"access-control-signature-decision.v4.json":       &SignatureDecision{},
		"access-control-ability-call-trace.v4.json":       &AbilityCallTrace{},
		"access-control-admission-explain-result.v4.json": &AdmissionExplainResult{},
	}
	for name, target := range cases {
		raw, err := os.ReadFile(filepath.Join(fixtureRoot, name))
		if err != nil {
			t.Fatalf("read %s: %v", name, err)
		}
		if err := json.Unmarshal(raw, target); err != nil {
			t.Fatalf("decode %s: %v", name, err)
		}
	}
	proofRaw, err := os.ReadFile(filepath.Join(fixtureRoot, "access-control-authority-proof.v4.json"))
	if err != nil {
		t.Fatal(err)
	}
	var proof AuthorityProof
	if err := json.Unmarshal(proofRaw, &proof); err != nil {
		t.Fatal(err)
	}
	if proof.AudienceURA != "easynet:///r/example/device/dev-a" || proof.SessionID != "session-1" {
		t.Fatalf("AuthorityProof RFC-014 fields not decoded: %#v", proof)
	}
}
