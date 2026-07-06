package easynet

import (
	"context"
	"testing"
)

func TestDescriptorBoundResourceSubjectURADelegatesToIdentityTransport(t *testing.T) {
	transport := &memoryIdentityTransport{buildURAJSON: `{
		"kind":"resource",
		"valid":true,
		"ura":"easynet:///r/example/resource/user.alice/invoke/files.read",
		"profile":"easynet-strict-v2",
		"components":{"owner_ura":"easynet:///r/example/user/alice"},
		"metadata":{"grammar_owner":"axon"}
	}`}
	client, err := NewIdentityClient(transport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	got, err := client.DescriptorBoundResourceSubjectURA(
		context.Background(),
		"easynet:///r/example/user/alice",
		"invoke/files.read",
	)
	if err != nil {
		t.Fatalf("DescriptorBoundResourceSubjectURA: %v", err)
	}
	const want = "easynet:///r/example/resource/user.alice/invoke/files.read"
	if got != want {
		t.Fatalf("subject URA = %q, want %q", got, want)
	}
	if transport.seenRequest["kind"] != "resource" {
		t.Fatalf("resource subject kind was not delegated: %#v", transport.seenRequest)
	}
	if transport.seenRequest["owner_ura"] != "easynet:///r/example/user/alice" {
		t.Fatalf("resource subject owner was not delegated: %#v", transport.seenRequest)
	}
	if transport.seenRequest["path"] != "invoke/files.read" {
		t.Fatalf("resource subject path was not delegated: %#v", transport.seenRequest)
	}
}
