package easynet

import "testing"

const preparedFixture = `{
  "prepared_id": "prepared-example-1",
  "tuple": {
    "caller_ura": "easynet:///r/example/agent/alice.sdk",
    "callee_ura": "easynet:///r/example/device/dev-a",
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
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
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
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
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
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

func TestPreparedInvocationDecodesCurrentABIShape(t *testing.T) {
	prepared, err := NewPreparedInvocationFromJSON([]byte(`{
		"request_id": "req-1",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
		"descriptor_hash_hex": "aa",
		"schema_hash_hex": "bb",
		"canonical_hash_hex": "50d858e0985ecc7f60418aaf0cc5ab587f42c2570a884095a9e8ccacd0f6545c",
		"expires_at_unix_ms": 1783000000000,
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
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
	if prepared.RequestID() != "req-1" {
		t.Fatalf("request id = %q", prepared.RequestID())
	}
	if policy := prepared.SigningMaterial().SignerPolicy(); policy == nil || policy.SignerID() != "browser-key" {
		t.Fatalf("signer policy not preserved: %#v", policy)
	}
}

func TestPreparedInvocationRejectsCanonicalHashMismatch(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"canonical_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
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
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "not valid base64",
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

func TestPreparedInvocationRejectsSubmitReadyPreparedPayload(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/device/dev-a",
			"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
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
