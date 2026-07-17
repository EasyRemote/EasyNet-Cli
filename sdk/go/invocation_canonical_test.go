package easynet

import (
	"crypto/sha256"
	"fmt"
	"strings"
	"testing"
)

const canonicalTestDescriptorRef = "easynet:///r/acme/ability/user.agent.echo@descriptor.v1#0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef!invoke"

func TestCanonicalInvocationBytesUsesStableAxonEncoding(t *testing.T) {
	canonical, err := CanonicalInvocationBytes(Envelope{
		Caller:        AgentRef{URA: "easynet:///r/acme/agent/user.agent"},
		Callee:        AgentRef{URA: "easynet:///r/acme/agent/device.agent"},
		Subject:       SubjectRef{URA: "easynet:///r/acme/resource/fs/tmp"},
		Nonce:         []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalNullWithReason("root does not enter canonical bytes"),
	}, canonicalTestDescriptorRef, []byte(`{"x":1}`))
	if err != nil {
		t.Fatalf("CanonicalInvocationBytes: %v", err)
	}
	got := sha256.Sum256(canonical)
	want := "47ecd2c82bd6afa612eef1d73a41eb6a75acb21368052a0e8c2277d3c3a1a6af"
	if fmt.Sprintf("%x", got) != want {
		t.Fatalf("canonical sha256 drift: got %x want %s", got, want)
	}
}

func TestCanonicalInvocationBytesRejectsMalformedCausalHash(t *testing.T) {
	_, err := CanonicalInvocationBytes(Envelope{
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
		t.Fatal("CanonicalInvocationBytes accepted malformed causal hash")
	}
	if !strings.Contains(err.Error(), "receipt hash hex must be 64 chars") {
		t.Fatalf("unexpected error: %v", err)
	}
}
