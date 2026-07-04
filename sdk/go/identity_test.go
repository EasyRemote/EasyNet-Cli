package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryIdentityTransport struct {
	descriptorJSON string
	identityJSON   string
	buildURAJSON   string
	resourceJSON   string
	keyJSON        string
	keyPageJSON    string
	revokeJSON     string
	signerJSON     string
	seenRequest    map[string]any
	closeCalls     int
}

func (m *memoryIdentityTransport) ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.descriptorJSON), nil
}

func (m *memoryIdentityTransport) BuildDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
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

func (m *memoryIdentityTransport) BuildURA(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(requestJSON, &m.seenRequest); err != nil {
		return nil, err
	}
	return []byte(m.buildURAJSON), nil
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

func (m *memoryIdentityTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
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

func TestIdentityAddressingHelpersDelegateToTransport(t *testing.T) {
	transport := &memoryIdentityTransport{
		descriptorJSON: identityDescriptorProjectionJSON,
		identityJSON:   identityAbilityProjectionJSON,
		buildURAJSON:   identityAbilityProjectionJSON,
	}
	client, err := NewIdentityClient(transport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	abilityURA, err := client.OwnerAbilityURA(
		context.Background(),
		"easynet:///r/example/device/dev-a",
		"observe.health",
	)
	if err != nil {
		t.Fatalf("OwnerAbilityURA: %v", err)
	}
	if abilityURA != "easynet:///r/example/ability/device.dev-a.observe.health" ||
		transport.seenRequest["kind"] != "ability" ||
		transport.seenRequest["owner_ura"] != "easynet:///r/example/device/dev-a" ||
		transport.seenRequest["ability_name"] != "observe.health" {
		t.Fatalf("ability URA was not delegated through build_ura: result=%q request=%#v", abilityURA, transport.seenRequest)
	}

	ownerURA, err := client.OwnerURAForAbility(
		context.Background(),
		"easynet:///r/example/ability/device.dev-a.observe.health",
	)
	if err != nil {
		t.Fatalf("OwnerURAForAbility: %v", err)
	}
	if ownerURA != "easynet:///r/example/device/dev-a" ||
		transport.seenRequest["ura"] != "easynet:///r/example/ability/device.dev-a.observe.health" {
		t.Fatalf("owner URA was not projected through identity transport: result=%q request=%#v", ownerURA, transport.seenRequest)
	}

	descriptorRef, err := client.OwnerAbilityDescriptorRef(
		context.Background(),
		"easynet:///r/example/device/dev-a",
		"observe.health",
		"1.0.0",
	)
	if err != nil {
		t.Fatalf("OwnerAbilityDescriptorRef: %v", err)
	}
	if descriptorRef != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" ||
		transport.seenRequest["ability_ura"] != "easynet:///r/example/ability/device.dev-a.observe.health" ||
		transport.seenRequest["descriptor_version"] != "1.0.0" {
		t.Fatalf("descriptor ref was not delegated through build_descriptor_ref: result=%q request=%#v", descriptorRef, transport.seenRequest)
	}

	canonical, err := client.CanonicalAbilityDescriptorRef(
		context.Background(),
		"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
		"",
	)
	if err != nil {
		t.Fatalf("CanonicalAbilityDescriptorRef: %v", err)
	}
	if canonical != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" ||
		transport.seenRequest["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" {
		t.Fatalf("descriptor ref canonicalization was not delegated through project_descriptor_ref: result=%q request=%#v", canonical, transport.seenRequest)
	}

	fromDescriptor, err := client.AbilityURAFromDescriptorRef(
		context.Background(),
		"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
	)
	if err != nil {
		t.Fatalf("AbilityURAFromDescriptorRef: %v", err)
	}
	if fromDescriptor != "easynet:///r/example/ability/device.dev-a.observe.health" {
		t.Fatalf("unexpected ability URI from descriptor ref: %q", fromDescriptor)
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

func TestIdentityClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := &memoryIdentityTransport{descriptorJSON: `{
		"kind":"descriptor_ref",
		"valid":true,
		"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
		"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",
		"descriptor_version":"1.0.0",
		"profile":"easynet-strict-v2",
		"components":{"owner_ura":"easynet:///r/example/device/dev-a"},
		"metadata":{}
	}`}
	client, err := NewIdentityClient(transport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.ProjectDescriptorRef(context.Background(), DescriptorRefRequest{DescriptorRef: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"})
	if err == nil {
		t.Fatalf("ProjectDescriptorRef after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called after close: %#v", transport.seenRequest)
	}
}

const identityDescriptorProjectionJSON = `{
  "kind":"descriptor_ref",
  "valid":true,
  "descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
  "ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",
  "descriptor_version":"1.0.0",
  "profile":"easynet-strict-v2",
  "components":{"owner_ura":"easynet:///r/example/device/dev-a"},
  "metadata":{"grammar_owner":"axon"}
}`

const identityAbilityProjectionJSON = `{
  "kind":"ability",
  "valid":true,
  "ura":"easynet:///r/example/ability/device.dev-a.observe.health",
  "realm":"example",
  "display_id":"device.dev-a.observe.health",
  "profile":"easynet-strict-v2",
  "components":{
    "owner_ura":"easynet:///r/example/device/dev-a",
    "ability_name":"observe.health"
  },
  "metadata":{"grammar_owner":"axon"}
}`

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
