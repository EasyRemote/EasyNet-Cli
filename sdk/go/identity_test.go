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
	keyJSON        string
	keyPageJSON    string
	revokeJSON     string
	signerJSON     string
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

func (m *memoryIdentityTransport) RegisterSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.keyJSON), nil
}

func (m *memoryIdentityTransport) ListSigningKeys(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.keyPageJSON), nil
}

func (m *memoryIdentityTransport) RevokeSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.revokeJSON), nil
}

func (m *memoryIdentityTransport) Signer(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.signerJSON), nil
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

func TestIdentitySigningKeyLifecycleAndSignerHandle(t *testing.T) {
	transport := &memoryIdentityTransport{
		keyJSON:     signingKeyRecordJSON,
		keyPageJSON: signingKeyPageJSON,
		revokeJSON:  signingKeyRevokeJSON,
		signerJSON:  signerHandleJSON,
	}
	client, err := NewIdentityClient(transport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	record, err := client.RegisterSigningKey(context.Background(), SigningKeyRegistrationRequest{
		OwnerURA:        "easynet:///r/example/agent/alice.sdk",
		KeyID:           "alice-key-1",
		Algorithm:       "ed25519",
		PublicKeyBase64: "cHVibGljLWtleQ==",
		Usage:           []string{"invocation.sign"},
	})
	if err != nil {
		t.Fatalf("RegisterSigningKey: %v", err)
	}
	if record.KeyID != "alice-key-1" || record.OwnerURA == "" || len(record.Usage) != 1 {
		t.Fatalf("unexpected signing key record: %#v", record)
	}

	page, err := client.ListSigningKeys(context.Background(), SigningKeyListRequest{OwnerURA: "easynet:///r/example/agent/alice.sdk"})
	if err != nil {
		t.Fatalf("ListSigningKeys: %v", err)
	}
	if page.Limit != DefaultSigningKeyPageSize || len(page.Items) != 1 || page.Items[0].KeyID != "alice-key-1" {
		t.Fatalf("unexpected signing key page: %#v", page)
	}
	if transport.seenRequest["limit"].(float64) != DefaultSigningKeyPageSize {
		t.Fatalf("default limit not delegated: %#v", transport.seenRequest)
	}

	revoke, err := client.RevokeSigningKey(context.Background(), SigningKeyRevokeRequest{KeyID: "alice-key-1", Reason: "rotation"})
	if err != nil {
		t.Fatalf("RevokeSigningKey: %v", err)
	}
	if !revoke.Revoked || revoke.State != "revoked" {
		t.Fatalf("unexpected revoke result: %#v", revoke)
	}

	signer, err := client.Signer(context.Background(), SignerRequest{
		OwnerURA: "easynet:///r/example/agent/alice.sdk",
		KeyID:    "alice-key-1",
		Usage:    "invocation.sign",
	})
	if err != nil {
		t.Fatalf("Signer: %v", err)
	}
	if signer.SignerID != "signer-alice-key-1" || signer.Algorithm != "ed25519" {
		t.Fatalf("unexpected signer handle: %#v", signer)
	}
}

func TestIdentitySigningKeyLifecycleRejectsInvalidInputs(t *testing.T) {
	client, err := NewIdentityClient(&memoryIdentityTransport{keyJSON: signingKeyRecordJSON, revokeJSON: signingKeyRevokeJSON})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	if _, err := client.RegisterSigningKey(context.Background(), SigningKeyRegistrationRequest{
		OwnerURA:        "easynet:///r/example/agent/alice.sdk",
		KeyID:           "alice-key-1",
		Algorithm:       "ed25519",
		PublicKeyBase64: "cHVibGljLWtleQ==",
		Usage:           []string{"invocation.sign"},
		Metadata:        map[string]any{"private_key_seed": "must-not-leak"},
	}); err == nil {
		t.Fatal("expected private key material rejection")
	}
	if _, err := client.ListSigningKeys(context.Background(), SigningKeyListRequest{Limit: MaxSigningKeyPageSize + 1}); err == nil {
		t.Fatal("expected signing-key page limit rejection")
	}
	if _, err := client.RevokeSigningKey(context.Background(), SigningKeyRevokeRequest{KeyID: "alice-key-1"}); err == nil {
		t.Fatal("expected revoke reason rejection")
	}
}

const signingKeyRecordJSON = `{
  "profile":"directory_identity",
  "key_id":"alice-key-1",
  "owner_ura":"easynet:///r/example/agent/alice.sdk",
  "algorithm":"ed25519",
  "public_key_base64":"cHVibGljLWtleQ==",
  "state":"active",
  "usage":["invocation.sign"],
  "created_unix_ms":1783100000123,
  "metadata":{"source":"daemon_keyring"}
}`

const signingKeyPageJSON = `{
  "profile":"directory_identity",
  "items":[
    {
      "profile":"directory_identity",
      "key_id":"alice-key-1",
      "owner_ura":"easynet:///r/example/agent/alice.sdk",
      "algorithm":"ed25519",
      "public_key_base64":"cHVibGljLWtleQ==",
      "state":"active",
      "usage":["invocation.sign"],
      "created_unix_ms":1783100000123,
      "metadata":{"source":"daemon_keyring"}
    }
  ],
  "next_cursor":null,
  "limit":50,
  "metadata":{"source":"daemon_keyring"}
}`

const signingKeyRevokeJSON = `{
  "profile":"directory_identity",
  "key_id":"alice-key-1",
  "revoked":true,
  "state":"revoked",
  "metadata":{"reason":"rotation"}
}`

const signerHandleJSON = `{
  "profile":"directory_identity",
  "signer_id":"signer-alice-key-1",
  "owner_ura":"easynet:///r/example/agent/alice.sdk",
  "key_id":"alice-key-1",
  "algorithm":"ed25519",
  "policy":{"mode":"local_daemon_signing","usage":"invocation.sign"},
  "metadata":{"source":"daemon_keyring"}
}`
