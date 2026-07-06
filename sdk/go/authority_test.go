package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"testing"
)

func TestDelegationProofFromMetadataProjectsTypedAuthority(t *testing.T) {
	value := authorityMetadataFixture(t, map[string]any{
		"issuer_ura":    "easynet:///r/example/user/alice",
		"subject_ura":   "easynet:///r/example/user/alice",
		"caller_ura":    "easynet:///r/example/agent/backend",
		"audience":      "easynet:///r/example/device/dev-a",
		"scopes":        []string{"device.observe.*"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("delegation-signature"))

	proof, err := NewDelegationProofFromMetadata(value)
	if err != nil {
		t.Fatalf("NewDelegationProofFromMetadata: %v", err)
	}
	if proof.IssuerURA != "easynet:///r/example/user/alice" || proof.CallerURA != "easynet:///r/example/agent/backend" {
		t.Fatalf("unexpected delegation projection: %#v", proof)
	}
	if len(proof.Signature) == 0 || proof.ExpiresAtMS != 2000 {
		t.Fatalf("signature/expiry not projected: %#v", proof)
	}
	metadata, err := proof.Metadata()
	if err != nil {
		t.Fatalf("Metadata: %v", err)
	}
	if metadata.Key() != DelegationMetadataKey || metadata.Value() != value {
		t.Fatalf("metadata key/value = %q/%q", metadata.Key(), metadata.Value())
	}
}

func TestSessionAuthorityFromMetadataProjectsTypedAuthority(t *testing.T) {
	value := authorityMetadataFixture(t, map[string]any{
		"backend_ura":   "easynet:///r/example/agent/backend",
		"user_ura":      "easynet:///r/example/user/alice",
		"session_id":    "sa-example",
		"scopes":        []string{"device.observe.*"},
		"audiences":     []string{"easynet:///r/example/device/dev-a"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("session-signature"))

	authority, err := NewSessionAuthorityFromMetadata(value)
	if err != nil {
		t.Fatalf("NewSessionAuthorityFromMetadata: %v", err)
	}
	if authority.BackendURA != "easynet:///r/example/agent/backend" || authority.SessionID != "sa-example" {
		t.Fatalf("unexpected session authority projection: %#v", authority)
	}
	metadata, err := authority.Metadata()
	if err != nil {
		t.Fatalf("Metadata: %v", err)
	}
	if metadata.Key() != SessionAuthorityMetadataKey || metadata.Value() != value {
		t.Fatalf("metadata key/value = %q/%q", metadata.Key(), metadata.Value())
	}
}

func TestInvocationBuilderAttachesOneAuthorityMetadata(t *testing.T) {
	proof, err := NewDelegationProofFromMetadata(authorityMetadataFixture(t, map[string]any{
		"issuer_ura":    "easynet:///r/example/user/alice",
		"subject_ura":   "easynet:///r/example/user/alice",
		"caller_ura":    "easynet:///r/example/agent/backend",
		"audience":      "*",
		"scopes":        []string{"*"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("signature")))
	if err != nil {
		t.Fatalf("NewDelegationProofFromMetadata: %v", err)
	}
	metadata, err := proof.Metadata()
	if err != nil {
		t.Fatalf("Metadata: %v", err)
	}

	draft, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/backend").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/user/alice").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		WithMetadata(map[string]any{"trace": "t-1"}).
		WithAuthorityMetadata(metadata).
		Build()
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	if draft.Metadata()["trace"] != "t-1" || draft.Metadata()[DelegationMetadataKey] != metadata.Value() {
		t.Fatalf("authority metadata not merged: %#v", draft.Metadata())
	}
}

func TestInvocationBuilderRejectsAmbiguousAuthorityMetadata(t *testing.T) {
	_, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/backend").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/user/alice").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		WithMetadata(map[string]any{
			DelegationMetadataKey:       "delegation",
			SessionAuthorityMetadataKey: "session",
		}).
		Build()
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Build error = %v, want invalid argument", err)
	}
}

func TestAuthorityClientMintsDelegationThroughTransport(t *testing.T) {
	value := authorityMetadataFixture(t, map[string]any{
		"issuer_ura":    "easynet:///r/example/user/alice",
		"subject_ura":   "easynet:///r/example/user/alice",
		"caller_ura":    "easynet:///r/example/agent/backend",
		"audience":      "easynet:///r/example/device/dev-a",
		"scopes":        []string{"device.observe.*"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("delegation-signature"))
	transport := &memoryAuthorityTransport{
		delegationJSON: []byte(`{"metadata_value":"` + value + `"}`),
	}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

	proof, err := client.MintDelegationProof(context.Background(), DelegationRequest{
		IssuerURA:   "easynet:///r/example/user/alice",
		SubjectURA:  "easynet:///r/example/user/alice",
		CallerURA:   "easynet:///r/example/agent/backend",
		Audience:    "easynet:///r/example/device/dev-a",
		Scopes:      []string{"device.observe.*"},
		IssuedAtMS:  1000,
		ExpiresAtMS: 2000,
	})
	if err != nil {
		t.Fatalf("MintDelegationProof: %v", err)
	}
	if proof.CallerURA != "easynet:///r/example/agent/backend" || proof.metadataValue != value {
		t.Fatalf("unexpected proof: %#v", proof)
	}
	if transport.seenDelegation["caller_ura"] != "easynet:///r/example/agent/backend" {
		t.Fatalf("transport did not receive delegation request: %#v", transport.seenDelegation)
	}
}

func TestAuthorityClientMintsSessionAuthorityThroughTransport(t *testing.T) {
	value := authorityMetadataFixture(t, map[string]any{
		"backend_ura":   "easynet:///r/example/agent/backend",
		"user_ura":      "easynet:///r/example/user/alice",
		"session_id":    "sa-example",
		"scopes":        []string{"device.observe.*"},
		"audiences":     []string{"easynet:///r/example/device/dev-a"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("session-signature"))
	transport := &memoryAuthorityTransport{
		sessionJSON: []byte(`{"metadata":{"` + SessionAuthorityMetadataKey + `":"` + value + `"}}`),
	}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

	authority, err := client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		BackendURA:  "easynet:///r/example/agent/backend",
		UserURA:     "easynet:///r/example/user/alice",
		SessionID:   "sa-example",
		Scopes:      []string{"device.observe.*"},
		Audiences:   []string{"easynet:///r/example/device/dev-a"},
		IssuedAtMS:  1000,
		ExpiresAtMS: 2000,
	})
	if err != nil {
		t.Fatalf("MintSessionAuthority: %v", err)
	}
	if authority.SessionID != "sa-example" || authority.metadataValue != value {
		t.Fatalf("unexpected authority: %#v", authority)
	}
	if transport.seenSession["session_id"] != "sa-example" {
		t.Fatalf("transport did not receive session request: %#v", transport.seenSession)
	}
}

func TestAuthorityClientRejectsInvalidMintBeforeTransport(t *testing.T) {
	transport := &memoryAuthorityTransport{}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

	_, err = client.MintDelegationProof(context.Background(), DelegationRequest{
		IssuerURA:   "easynet:///r/example/user/alice",
		SubjectURA:  "easynet:///r/example/user/alice",
		CallerURA:   "easynet:///r/example/agent/backend",
		Audience:    "easynet:///r/example/device/dev-a",
		IssuedAtMS:  1000,
		ExpiresAtMS: 2000,
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("MintDelegationProof error = %v, want invalid argument", err)
	}
	if transport.delegationCalls != 0 {
		t.Fatalf("transport called for invalid delegation request")
	}

	_, err = client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		BackendURA:  "easynet:///r/example/agent/backend",
		UserURA:     "easynet:///r/example/user/alice",
		SessionID:   "sa-example",
		Scopes:      []string{"device.observe.*"},
		Audiences:   []string{"easynet:///r/example/device/dev-a"},
		IssuedAtMS:  2000,
		ExpiresAtMS: 1000,
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("MintSessionAuthority error = %v, want invalid argument", err)
	}
	if transport.sessionCalls != 0 {
		t.Fatalf("transport called for invalid session request")
	}
}

func authorityMetadataFixture(t *testing.T, payload map[string]any, signature []byte) string {
	t.Helper()
	wire, err := json.Marshal(map[string]any{
		"payload":   payload,
		"signature": base64.StdEncoding.EncodeToString(signature),
	})
	if err != nil {
		t.Fatalf("marshal authority fixture: %v", err)
	}
	return base64.StdEncoding.EncodeToString(wire)
}

type memoryAuthorityTransport struct {
	delegationJSON []byte
	sessionJSON    []byte

	delegationCalls int
	sessionCalls    int
	seenDelegation  map[string]any
	seenSession     map[string]any
}

func (m *memoryAuthorityTransport) MintDelegationProof(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.delegationCalls++
	if err := json.Unmarshal(requestJSON, &m.seenDelegation); err != nil {
		return nil, err
	}
	return m.delegationJSON, nil
}

func (m *memoryAuthorityTransport) MintSessionAuthority(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.sessionCalls++
	if err := json.Unmarshal(requestJSON, &m.seenSession); err != nil {
		return nil, err
	}
	return m.sessionJSON, nil
}
