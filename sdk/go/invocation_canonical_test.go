package easynet

import (
	"crypto/sha256"
	"fmt"
	"strings"
	"testing"
)

const canonicalTestDescriptorRef = "easynet:///r/acme/ability/user.agent.echo@descriptor.v1#0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef!invoke"

func TestDescriptorBoundInvocationBytesUseStableAxonEncoding(t *testing.T) {
	canonical, err := canonicalDescriptorBoundInvocationBytes(Envelope{
		Caller:        AgentRef{URA: "easynet:///r/acme/agent/user.agent"},
		Callee:        AgentRef{URA: "easynet:///r/acme/agent/device.agent"},
		Subject:       SubjectRef{URA: "easynet:///r/acme/resource/fs/tmp"},
		Nonce:         []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalNullWithReason("root does not enter canonical bytes"),
	}, canonicalTestDescriptorRef, []byte(`{"x":1}`))
	if err != nil {
		t.Fatalf("canonicalDescriptorBoundInvocationBytes: %v", err)
	}
	got := sha256.Sum256(canonical)
	want := "a532dd2d1987a8b1808ba9c88b92be37d5b06134286b47dfae9e2e860a860795"
	if fmt.Sprintf("%x", got) != want {
		t.Fatalf("canonical sha256 drift: got %x want %s", got, want)
	}
}

func TestDescriptorBoundInvocationBytesRejectMalformedCausalHash(t *testing.T) {
	_, err := canonicalDescriptorBoundInvocationBytes(Envelope{
		Caller:  AgentRef{URA: "easynet:///r/acme/agent/user.agent"},
		Callee:  AgentRef{URA: "easynet:///r/acme/agent/device.agent"},
		Subject: SubjectRef{URA: "easynet:///r/acme/resource/fs/tmp"},
		Nonce:   []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalScalarRef(CausalReceiptRef{
			HashHex: "abc",
			URA:     "easynet:///r/acme/resource/agent.canonical.test/invocation/01R/receipt",
		}),
	}, canonicalTestDescriptorRef, []byte(`{"x":1}`))
	if err == nil {
		t.Fatal("canonicalDescriptorBoundInvocationBytes accepted malformed causal hash")
	}
	if !strings.Contains(err.Error(), "receipt hash hex must be 64 chars") {
		t.Fatalf("unexpected error: %v", err)
	}
}
