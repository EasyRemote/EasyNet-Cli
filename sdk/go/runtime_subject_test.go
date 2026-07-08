package easynet

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestDescriptorBoundSubjectURAProjectsUserSubjectBeforeSigning(t *testing.T) {
	identity := newRuntimeSubjectIdentity(t)

	subjectURA, err := descriptorBoundSubjectURA(
		context.Background(),
		identity,
		"easynet:///r/example/user/alice",
		"namespace.resolve",
	)
	if err != nil {
		t.Fatalf("descriptorBoundSubjectURA: %v", err)
	}
	want := "easynet:///r/example/resource/user.alice/invoke/namespace.resolve"
	if subjectURA != want {
		t.Fatalf("subject = %q, want %q", subjectURA, want)
	}

	_, err = CanonicalInvocationBytes(Envelope{
		Caller:        AgentRef{URA: "easynet:///r/example/agent/backend.sdk"},
		Callee:        AgentRef{URA: "easynet:///r/example/device/dev-a"},
		Subject:       SubjectRef{URA: subjectURA},
		Nonce:         []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalNullWithReason(""),
	}, "easynet:///r/example/ability/device.dev-a.namespace.resolve@1.0.0", []byte(`{}`))
	if err != nil {
		t.Fatalf("CanonicalInvocationBytes(projected subject): %v", err)
	}
}

func TestCanonicalInvocationBytesRejectsUnprojectedUserSubject(t *testing.T) {
	_, err := CanonicalInvocationBytes(Envelope{
		Caller:        AgentRef{URA: "easynet:///r/example/agent/backend.sdk"},
		Callee:        AgentRef{URA: "easynet:///r/example/device/dev-a"},
		Subject:       SubjectRef{URA: "easynet:///r/example/user/alice"},
		Nonce:         []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalNullWithReason(""),
	}, "easynet:///r/example/ability/device.dev-a.namespace.resolve@1.0.0", []byte(`{}`))
	if err == nil {
		t.Fatal("CanonicalInvocationBytes accepted an unprojected user subject")
	}
	if !strings.Contains(err.Error(), "subject_ref_kind_unsupported:user") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func newRuntimeSubjectIdentity(t *testing.T) *IdentityClient {
	t.Helper()
	identity, err := NewIdentityClient(IdentityTransportFunc{
		BuildURAFunc: func(_ context.Context, requestJSON []byte) ([]byte, error) {
			var req URABuildRequest
			if err := json.Unmarshal(requestJSON, &req); err != nil {
				return nil, err
			}
			switch req.Kind {
			case "resource":
				return json.Marshal(map[string]any{
					"kind":       "resource",
					"valid":      true,
					"ura":        ResourceDotURA("example", "user.alice", req.Path),
					"profile":    "directory_identity",
					"components": map[string]any{"owner_ura": req.OwnerURA},
					"metadata":   map[string]any{"source": "runtime_subject_test"},
				})
			case "ability":
				abilityURA := OwnerAbilityURA(req.OwnerURA, req.AbilityName)
				return json.Marshal(map[string]any{
					"kind":       "ability",
					"valid":      true,
					"ura":        abilityURA,
					"profile":    "directory_identity",
					"components": map[string]any{"owner_ura": req.OwnerURA},
					"metadata":   map[string]any{"source": "runtime_subject_test"},
				})
			default:
				t.Fatalf("unexpected URA build kind %q", req.Kind)
				return nil, nil
			}
		},
		BuildDescriptorRefFunc: func(_ context.Context, requestJSON []byte) ([]byte, error) {
			var req DescriptorRefBuildRequest
			if err := json.Unmarshal(requestJSON, &req); err != nil {
				return nil, err
			}
			ref := req.AbilityURA + "@" + req.DescriptorVersion
			return json.Marshal(map[string]any{
				"kind":               "descriptor_ref",
				"valid":              true,
				"descriptor_ref":     ref,
				"ability_ura":        req.AbilityURA,
				"descriptor_version": req.DescriptorVersion,
				"profile":            "directory_identity",
				"components":         map[string]any{"ability_ura": req.AbilityURA},
				"metadata":           map[string]any{"source": "runtime_subject_test"},
			})
		},
	})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	return identity
}
