package easynet

import (
	"context"
	"strings"
	"testing"
)

const runtimeSubjectDescriptorRef = "easynet:///r/example/ability/device.dev-a.namespace.resolve@descriptor.v1#0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef!invoke"

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

	_, err = canonicalDescriptorBoundInvocationBytes(Envelope{
		Caller:        AgentRef{URA: "easynet:///r/example/agent/backend.sdk"},
		Callee:        AgentRef{URA: "easynet:///r/example/device/dev-a"},
		Subject:       SubjectRef{URA: subjectURA},
		Nonce:         []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalRoot(),
	}, runtimeSubjectDescriptorRef, []byte(`{}`))
	if err != nil {
		t.Fatalf("canonicalDescriptorBoundInvocationBytes(projected subject): %v", err)
	}
}

func TestDescriptorBoundSubjectURAProjectsAuthoritySubjectBeforeSigning(t *testing.T) {
	identity := newRuntimeSubjectIdentity(t)

	subjectURA, err := descriptorBoundSubjectURA(
		context.Background(),
		identity,
		"easynet:///r/example/authority",
		"namespace.resolve",
	)
	if err != nil {
		t.Fatalf("descriptorBoundSubjectURA: %v", err)
	}
	want := "easynet:///r/example/resource/authority/invoke/namespace.resolve"
	if subjectURA != want {
		t.Fatalf("subject = %q, want %q", subjectURA, want)
	}
}

func TestDescriptorBoundInvocationBytesRejectUnprojectedUserSubject(t *testing.T) {
	_, err := canonicalDescriptorBoundInvocationBytes(Envelope{
		Caller:        AgentRef{URA: "easynet:///r/example/agent/backend.sdk"},
		Callee:        AgentRef{URA: "easynet:///r/example/device/dev-a"},
		Subject:       SubjectRef{URA: "easynet:///r/example/user/alice"},
		Nonce:         []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalRoot(),
	}, runtimeSubjectDescriptorRef, []byte(`{}`))
	if err == nil {
		t.Fatal("canonicalDescriptorBoundInvocationBytes accepted an unprojected user subject")
	}
	if !strings.Contains(err.Error(), "subject_ref_kind_unsupported:user") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func newRuntimeSubjectIdentity(t *testing.T) Addressing {
	t.Helper()
	return NewCanonicalAddressing()
}
