package easynet

import (
	"bytes"
	"crypto/ed25519"
	"encoding/base64"
	"strings"
	"testing"
)

const preparedFixture = `{
  "prepared_id": "prepared-example-1",
  "tuple": {
    "caller_ura": "easynet:///r/example/agent/alice.sdk",
    "callee_ura": "easynet:///r/example/device/dev-a",
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
    "subject_ura": "easynet:///r/example/device/dev-a",
    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
    "causal_context": {"form": "none"},
    "args": {},
    "content_type": "application/json",
    "metadata": {}
  },
  "signing_material": {
    "algorithm": "ed25519",
    "canonical_bytes_base64": "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=",
    "args_digest_hex": "0000000000000000000000000000000000000000000000000000000000000000",
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
    "expires_at_unix_ms": 1783000000000
  },
  "submit_ready": false
}`

func TestPreparedInvocationDecodesSigningMaterialFixture(t *testing.T) {
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	if prepared.SubmitReady() {
		t.Fatalf("prepared invocation is submit-ready")
	}
	if prepared.PreparedID() != "prepared-example-1" {
		t.Fatalf("prepared id = %q", prepared.PreparedID())
	}
	if prepared.SigningMaterial().CanonicalBytesBase64() == "" {
		t.Fatalf("canonical bytes missing")
	}
	if prepared.SigningMaterial().DescriptorRef() != prepared.DescriptorRef() {
		t.Fatalf("descriptor ref mismatch")
	}
}

func TestPreparedInvocationRejectsMissingCanonicalBytes(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"args_digest_hex": "00",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPreparedInvocationRejectsMaterialFieldsInTuple(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json",
			"args_digest_hex": "00"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if !strings.Contains(err.Error(), "args_digest_hex is not an invocation field") {
		t.Fatalf("error = %v, want material tuple field rejection", err)
	}
}

func TestPreparedInvocationDecodesCurrentABIShape(t *testing.T) {
	prepared, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-current-1",
		"request_id": "req-1",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
		"descriptor_hash_hex": "aa",
		"schema_hash_hex": "bb",
		"canonical_hash_hex": "50d858e0985ecc7f60418aaf0cc5ab587f42c2570a884095a9e8ccacd0f6545c",
		"expires_at_unix_ms": 1783000000000,
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"signed_fields": ["caller_ura", "callee_ura"],
			"signer_policy": {
				"mode": "caller_signing",
				"signer_id": "browser-key",
				"policy_ref": "policy/local",
				"expires_at_unix_ms": 1783000000000
			},
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	if prepared.PreparedID() != "prepared-current-1" {
		t.Fatalf("prepared id = %q", prepared.PreparedID())
	}
	if prepared.RequestID() != "req-1" {
		t.Fatalf("request id = %q", prepared.RequestID())
	}
	if policy := prepared.SigningMaterial().SignerPolicy(); policy == nil || policy.SignerID() != "browser-key" {
		t.Fatalf("signer policy not preserved: %#v", policy)
	}
}

func TestPreparedInvocationRejectsRequestIDOnlyPayload(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"request_id": "req-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON accepted request_id-only payload")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if !strings.Contains(err.Error(), "prepared_id is required") {
		t.Fatalf("error = %v, want prepared_id requirement", err)
	}
}

func TestPreparedInvocationRejectsMissingSigningMaterialDescriptorRef(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPreparedInvocationRejectsSigningMaterialDescriptorMismatch(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.status@1.0.0",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPreparedInvocationRejectsCanonicalHashMismatch(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"canonical_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPreparedInvocationRejectsInvalidCanonicalBase64(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "not valid base64",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPreparedInvocationRejectsSubmitReadyPreparedPayload(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"expires_at_unix_ms": 1783000000000
		},
		"submit_ready": true
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestPreparedInvocationSignsIntoSubmitReadyEnvelope(t *testing.T) {
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	signed, err := prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:       "ed25519",
		SignatureBase64: "c2lnbmF0dXJl",
		KeyIDHint:       "caller-key",
	})
	if err != nil {
		t.Fatalf("SignWithCallerSignature: %v", err)
	}
	if !signed.SubmitReady() {
		t.Fatalf("signed invocation is not submit-ready")
	}
	if signed.Prepared().SubmitReady() {
		t.Fatalf("prepared changed into submit-ready")
	}
	if signed.SignerID() != "caller-key" {
		t.Fatalf("signer id = %q", signed.SignerID())
	}
}

func TestPreparedInvocationNormalizesSignerPubkeyIntoKeyHint(t *testing.T) {
	const pubkey = "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	signed, err := prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:             "ed25519",
		SignatureBase64:       "c2lnbmF0dXJl",
		SignerPublicKeyBase64: pubkey,
	})
	if err != nil {
		t.Fatalf("SignWithCallerSignature: %v", err)
	}
	if signed.Signature().KeyIDHint != pubkey {
		t.Fatalf("key_id_hint = %q, want signer pubkey", signed.Signature().KeyIDHint)
	}
	if signed.SignerID() != pubkey {
		t.Fatalf("signer id = %q, want pubkey", signed.SignerID())
	}
}

type memorySignatureProvider struct {
	material SigningMaterial
	handle   SignerHandle
}

func (p *memorySignatureProvider) Sign(material SigningMaterial, handle SignerHandle) (InvocationSignature, error) {
	p.material = material
	p.handle = handle
	return InvocationSignature{SignatureBase64: "c2lnbmF0dXJl"}, nil
}

func TestSignerProviderSignsWithDaemonAuthorizedHandle(t *testing.T) {
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	provider := &memorySignatureProvider{}
	signer, err := NewSigner(signerHandle(""), provider)
	if err != nil {
		t.Fatalf("NewSigner: %v", err)
	}

	signed, err := signer.Sign(prepared)
	if err != nil {
		t.Fatalf("Sign: %v", err)
	}

	if !signed.SubmitReady() {
		t.Fatalf("signed invocation is not submit-ready")
	}
	if signed.SignerID() != signer.Handle().SignerID {
		t.Fatalf("signer id = %q, want %q", signed.SignerID(), signer.Handle().SignerID)
	}
	if signed.Signature().Algorithm != signer.Handle().Algorithm {
		t.Fatalf("algorithm = %q, want %q", signed.Signature().Algorithm, signer.Handle().Algorithm)
	}
	if signed.Signature().KeyIDHint != signer.Handle().SignerID {
		t.Fatalf("key hint = %q, want %q", signed.Signature().KeyIDHint, signer.Handle().SignerID)
	}
	if provider.material.CanonicalBytesBase64() != prepared.SigningMaterial().CanonicalBytesBase64() {
		t.Fatalf("provider did not receive prepared signing material")
	}
	if provider.handle.SignerID != signer.Handle().SignerID {
		t.Fatalf("provider did not receive signer handle")
	}
}

func TestSignerRejectsForgedHandleProvenance(t *testing.T) {
	provider := &memorySignatureProvider{}
	handle := signerHandle("")
	handle.Metadata = map[string]any{"source": "product_local_fixture"}
	if _, err := NewSigner(handle, provider); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewSigner forged source error = %v, want InvalidArgument", err)
	}

	handle = signerHandle("")
	handle.Policy = map[string]any{"mode": "caller_signing", "usage": "invocation.sign"}
	if _, err := NewSigner(handle, provider); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewSigner forged policy error = %v, want InvalidArgument", err)
	}

	handle = signerHandle("")
	handle.Policy = map[string]any{"mode": "local_daemon_signing"}
	if _, err := NewSigner(handle, provider); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewSigner missing usage error = %v, want InvalidArgument", err)
	}

	handle = signerHandle("")
	handle.Policy["signer_id"] = "other-signer"
	if _, err := NewSigner(handle, provider); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewSigner signer_id mismatch error = %v, want InvalidArgument", err)
	}

	handle = signerHandle("")
	handle.Metadata["policy_ref"] = "daemon-key-inventory:sha256:other-policy"
	if _, err := NewSigner(handle, provider); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewSigner policy_ref mismatch error = %v, want InvalidArgument", err)
	}
}

func TestExternalSignatureProviderImplementsGenericSigningSeam(t *testing.T) {
	seed := bytes.Repeat([]byte{0x11}, ed25519.SeedSize)
	publicKeyBase64 := ed25519PublicKeyBase64(seed)
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	provider := newTestEd25519SignatureProvider(seed)
	signer, err := NewSigner(signerHandle(publicKeyBase64), provider)
	if err != nil {
		t.Fatalf("NewSigner: %v", err)
	}

	signed, err := signer.Sign(prepared)
	if err != nil {
		t.Fatalf("Sign: %v", err)
	}

	if signed.Signature().Algorithm != "ed25519" {
		t.Fatalf("algorithm = %q", signed.Signature().Algorithm)
	}
	if signed.Signature().SignerPublicKeyBase64 != publicKeyBase64 {
		t.Fatalf("public key = %q, want %q", signed.Signature().SignerPublicKeyBase64, publicKeyBase64)
	}
	signature, err := base64.StdEncoding.DecodeString(signed.Signature().SignatureBase64)
	if err != nil {
		t.Fatalf("decode signature: %v", err)
	}
	canonicalBytes, err := base64.StdEncoding.DecodeString(prepared.SigningMaterial().CanonicalBytesBase64())
	if err != nil {
		t.Fatalf("decode canonical bytes: %v", err)
	}
	publicKey := ed25519.NewKeyFromSeed(seed).Public().(ed25519.PublicKey)
	if !ed25519.Verify(publicKey, canonicalBytes, signature) {
		t.Fatalf("ed25519 signature did not verify")
	}
}

func TestSignerRejectsPolicyAndKeyHintMismatch(t *testing.T) {
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	prepared.signingMaterial.signerPolicy = &SignerPolicy{
		mode:     "local_daemon_signing",
		signerID: "other-signer",
	}
	signer, err := NewSignerFromSignature(
		signerHandle(""),
		InvocationSignature{Algorithm: "ed25519", SignatureBase64: "c2lnbmF0dXJl"},
	)
	if err != nil {
		t.Fatalf("NewSignerFromSignature: %v", err)
	}
	if _, err := signer.Sign(prepared); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("policy mismatch error = %v, want InvalidArgument", err)
	}

	prepared.signingMaterial.signerPolicy = nil
	if _, err := signer.SignWithSignature(
		prepared,
		InvocationSignature{
			Algorithm:       "ed25519",
			SignatureBase64: "c2lnbmF0dXJl",
			KeyIDHint:       "wrong-key",
		},
	); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("key hint mismatch error = %v, want InvalidArgument", err)
	}
}

func TestPreparedInvocationRejectsEmptySignature(t *testing.T) {
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	_, err = prepared.SignWithCallerSignature(InvocationSignature{Algorithm: "ed25519"})
	if err == nil {
		t.Fatalf("SignWithCallerSignature succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func signerHandle(publicKeyBase64 string) SignerHandle {
	policyRef := "daemon-key-inventory:sha256:test-policy"
	metadata := map[string]any{"source": "daemon_keyring", "policy_ref": policyRef}
	if publicKeyBase64 != "" {
		metadata["public_key_base64"] = publicKeyBase64
	}
	return SignerHandle{
		Profile:   "signing",
		SignerID:  "signer-alice-key-1",
		OwnerURA:  "easynet:///r/example/agent/alice.sdk",
		KeyID:     "alice-key-1",
		Algorithm: "ed25519",
		Policy: map[string]any{
			"mode":                "local_daemon_signing",
			"usage":               "invocation.sign",
			"signer_id":           "signer-alice-key-1",
			"policy_ref":          policyRef,
			"inventory_owner_ura": "easynet:///r/example/agent/alice.sdk",
			"key_state":           "active",
		},
		Metadata: metadata,
	}
}

func ed25519PublicKeyBase64(seed []byte) string {
	privateKey := ed25519.NewKeyFromSeed(seed)
	publicKey := privateKey.Public().(ed25519.PublicKey)
	return base64.StdEncoding.EncodeToString(publicKey)
}

// testEd25519SignatureProvider demonstrates that consumers can implement the
// product-neutral SignatureProvider seam without giving private material to
// the production SDK package.
type testEd25519SignatureProvider struct {
	privateKey ed25519.PrivateKey
}

func newTestEd25519SignatureProvider(seed []byte) testEd25519SignatureProvider {
	return testEd25519SignatureProvider{privateKey: ed25519.NewKeyFromSeed(seed)}
}

func (p testEd25519SignatureProvider) Sign(material SigningMaterial, handle SignerHandle) (InvocationSignature, error) {
	canonicalBytes, err := base64.StdEncoding.DecodeString(material.CanonicalBytesBase64())
	if err != nil {
		return InvocationSignature{}, err
	}
	publicKey := p.privateKey.Public().(ed25519.PublicKey)
	return InvocationSignature{
		Algorithm:             "ed25519",
		SignatureBase64:       base64.StdEncoding.EncodeToString(ed25519.Sign(p.privateKey, canonicalBytes)),
		KeyIDHint:             handle.SignerID,
		SignerPublicKeyBase64: base64.StdEncoding.EncodeToString(publicKey),
	}, nil
}
