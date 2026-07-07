package easynet

import (
	"crypto/sha256"
	"fmt"
	"strings"
	"testing"
)

func TestCanonicalInvocationBytesUsesStableAxonEncoding(t *testing.T) {
	canonical, err := CanonicalInvocationBytes(Envelope{
		Caller:        AgentRef{URA: "easynet:///r/acme/agent/user.agent"},
		Callee:        AgentRef{URA: "easynet:///r/acme/agent/device.agent"},
		Subject:       SubjectRef{URA: "easynet:///r/acme/resource/fs/tmp"},
		Nonce:         []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
		CausalContext: CausalNullWithReason("root does not enter canonical bytes"),
	}, "demo.echo", []byte(`{"x":1}`))
	if err != nil {
		t.Fatalf("CanonicalInvocationBytes: %v", err)
	}
	got := sha256.Sum256(canonical)
	want := "4c8cc34ec0f5d892c9eef8468d1b579713783bc358c163b5bc15f5d134a63d8a"
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
	}, "demo.echo", []byte(`{"x":1}`))
	if err == nil {
		t.Fatal("CanonicalInvocationBytes accepted malformed causal hash")
	}
	if !strings.Contains(err.Error(), "receipt hash hex must be 64 chars") {
		t.Fatalf("unexpected error: %v", err)
	}
}
