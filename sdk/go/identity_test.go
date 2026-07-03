package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryIdentityTransport struct {
	descriptorJSON string
	identityJSON   string
	resourceJSON   string
	seenRequest    map[string]any
}

func (m *memoryIdentityTransport) ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.descriptorJSON), nil
}

func (m *memoryIdentityTransport) ProjectIdentity(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.identityJSON), nil
}

func (m *memoryIdentityTransport) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.resourceJSON), nil
}

func TestIdentityProjectDescriptorRefDelegatesToTransport(t *testing.T) {
	transport := &memoryIdentityTransport{descriptorJSON: `{
		"kind":"descriptor_ref",
		"valid":true,
		"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
		"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",
		"descriptor_version":"1.0.0",
		"profile":"easynet-strict-v2",
		"components":{"owner_ura":"easynet:///r/example/device/dev-a"},
		"metadata":{"grammar_owner":"axon"}
	}`}
	client, err := NewIdentityClient(transport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	projection, err := client.ProjectDescriptorRef(context.Background(), DescriptorRefRequest{
		DescriptorRef: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
	})
	if err != nil {
		t.Fatalf("ProjectDescriptorRef: %v", err)
	}

	if !projection.Valid || projection.AbilityURA == "" || projection.DescriptorVersion != "1.0.0" {
		t.Fatalf("unexpected descriptor projection: %#v", projection)
	}
	if transport.seenRequest["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" {
		t.Fatalf("descriptor request not delegated: %#v", transport.seenRequest)
	}
}

func TestIdentityBuildResourceRefValidatesProjection(t *testing.T) {
	transport := &memoryIdentityTransport{resourceJSON: `{
		"resource_ura":"easynet:///r/example/resource/device.dev-a/fs/tmp/easynet-weather-package",
		"owner_ura":"easynet:///r/example/device/dev-a",
		"namespace":"fs",
		"display_path":"tmp/easynet-weather-package",
		"capability":"read",
		"expires_unix_ms":4102444800000,
		"revision":"fs-local-mapping-v1"
	}`}
	client, err := NewIdentityClient(transport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	ref, err := client.BuildResourceRef(context.Background(), LocalResourceRefRequest{
		Path:       "/tmp/easynet-weather-package",
		Capability: "read",
	})
	if err != nil {
		t.Fatalf("BuildResourceRef: %v", err)
	}

	if ref.ResourceURA == "" || ref.OwnerURA == "" || ref.Revision != "fs-local-mapping-v1" {
		t.Fatalf("unexpected resource ref: %#v", ref)
	}
	if transport.seenRequest["path"] != "/tmp/easynet-weather-package" {
		t.Fatalf("resource request not delegated: %#v", transport.seenRequest)
	}
}

func TestIdentityRejectsMalformedDescriptorProjection(t *testing.T) {
	transport := &memoryIdentityTransport{descriptorJSON: `{
		"kind":"descriptor_ref",
		"valid":true,
		"profile":"easynet-strict-v2",
		"components":{},
		"metadata":{}
	}`}
	client, err := NewIdentityClient(transport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	_, err = client.ProjectDescriptorRef(context.Background(), DescriptorRefRequest{DescriptorRef: "opaque"})
	if err == nil {
		t.Fatalf("ProjectDescriptorRef accepted malformed projection")
	}
}
