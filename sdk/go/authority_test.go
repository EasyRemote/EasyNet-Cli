package easynet

import (
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
