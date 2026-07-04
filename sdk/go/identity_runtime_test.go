package easynet

import (
	"context"
	"encoding/base64"
	"testing"
)

func TestIdentityRuntimeTransportBuildsRegisterInvocationThroughResolver(t *testing.T) {
	resolverTransport := newIdentityRuntimeResolverTransport()
	resolver, err := NewIdentityClient(resolverTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: `{"ok":true}`})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeIdentityClient(runtime, resolver)
	if err != nil {
		t.Fatalf("NewRuntimeIdentityClient: %v", err)
	}

	record, err := client.RegisterSigningKey(context.Background(), identityRegisterRequestForTest("user"))
	if err != nil {
		t.Fatalf("RegisterSigningKey: %v", err)
	}
	if record.OwnerURA != "easynet:///r/example/user/alice" || record.KeyID != "alice-key-1" {
		t.Fatalf("record not projected from request: %#v", record)
	}
	if record.Metadata["role"] != "user" || record.Metadata["source"] != identityAbilityRegisterPubkey {
		t.Fatalf("metadata not projected: %#v", record.Metadata)
	}
	if len(resolverTransport.seenBuildURA) != 1 || resolverTransport.seenBuildURA[0]["ability_name"] != identityAbilityRegisterPubkey {
		t.Fatalf("ability descriptor was not delegated through resolver: %#v", resolverTransport.seenBuildURA)
	}
}

func TestIdentityRuntimeTransportInvokesListAndRevoke(t *testing.T) {
	resolver, err := NewIdentityClient(newIdentityRuntimeResolverTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: identityListRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeIdentityClient(runtime, resolver)
	if err != nil {
		t.Fatalf("NewRuntimeIdentityClient: %v", err)
	}

	page, err := client.ListSigningKeys(context.Background(), SigningKeyListRequest{
		IdentityCarrierBase: identityBaseForTest(),
		OwnerURA:            "easynet:///r/example/user/alice",
		Limit:               1,
	})
	if err != nil {
		t.Fatalf("ListSigningKeys: %v", err)
	}
	if len(page.Items) != 1 || page.Items[0].PublicKeyBase64 != identityPublicKeyForTest() {
		t.Fatalf("page not projected: %#v", page)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if args["agent_ura"] != "easynet:///r/example/user/alice" {
		t.Fatalf("list args not normalized: %#v", args)
	}

	runtimeTransport.outputJSON = `{"ok":true,"removed":false}`
	result, err := client.RevokeSigningKey(context.Background(), SigningKeyRevokeRequest{
		IdentityCarrierBase: identityBaseForTest(),
		OwnerURA:            "easynet:///r/example/user/alice",
		KeyID:               "alice-key-1",
		PublicKeyBase64:     identityPublicKeyForTest(),
		Reason:              "rotation",
	})
	if err != nil {
		t.Fatalf("RevokeSigningKey: %v", err)
	}
	if result.State != "not_found" || !result.Revoked {
		t.Fatalf("revoke result not projected: %#v", result)
	}
}

func TestIdentityRuntimeTransportMapsTerminalFailure(t *testing.T) {
	resolver, err := NewIdentityClient(newIdentityRuntimeResolverTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{fail: true})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeIdentityClient(runtime, resolver)
	if err != nil {
		t.Fatalf("NewRuntimeIdentityClient: %v", err)
	}

	_, err = client.RegisterSigningKey(context.Background(), identityRegisterRequestForTest("user"))
	if err == nil {
		t.Fatal("RegisterSigningKey succeeded, want failure")
	}
	if !IsCode(err, ErrAbilityFailed) {
		t.Fatalf("error code = %v, want %s", err, ErrAbilityFailed)
	}
}

func identityRegisterRequestForTest(role string) SigningKeyRegistrationRequest {
	return SigningKeyRegistrationRequest{
		IdentityCarrierBase: identityBaseForTest(),
		OwnerURA:            "easynet:///r/example/user/alice",
		KeyID:               "alice-key-1",
		Algorithm:           "ed25519",
		PublicKeyBase64:     identityPublicKeyForTest(),
		Role:                role,
		Usage:               []string{"invocation.sign"},
	}
}

func identityBaseForTest() IdentityCarrierBase {
	return IdentityCarrierBase{
		CallerURA:         "easynet:///r/example/agent/backend",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/user/alice",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "identity-1"},
	}
}

func identityPublicKeyForTest() string {
	return base64.StdEncoding.EncodeToString([]byte{
		1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
		1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
	})
}

func newIdentityRuntimeResolverTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			identityAbilityRegisterPubkey:  "easynet:///r/example/ability/device.dev-a.identity.register_pubkey",
			identityAbilityListUserPubkeys: "easynet:///r/example/ability/device.dev-a.identity.list_user_pubkeys",
			identityAbilityRevokePubkey:    "easynet:///r/example/ability/device.dev-a.identity.revoke_user_pubkey",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.identity.register_pubkey":    "easynet:///r/example/ability/device.dev-a.identity.register_pubkey@1.0.0",
			"easynet:///r/example/ability/device.dev-a.identity.list_user_pubkeys":  "easynet:///r/example/ability/device.dev-a.identity.list_user_pubkeys@1.0.0",
			"easynet:///r/example/ability/device.dev-a.identity.revoke_user_pubkey": "easynet:///r/example/ability/device.dev-a.identity.revoke_user_pubkey@1.0.0",
		},
		descriptorProjection: identityDescriptorProjectionJSON,
	}
}

const identityListRawJSON = `{
	"agent_ura":"easynet:///r/example/user/alice",
	"keys":[
		{"public_key_b64":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=","added_at_unix_ms":1783100000123},
		{"public_key_b64":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=","added_at_unix_ms":1783100000456}
	],
	"rotation_epoch":3,
	"revoked_key_count":1
}`
