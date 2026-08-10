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
  "descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
  "expires_at_unix_ms": 1783000000000,
  "tuple": {
    "caller_ura": "easynet:///r/example/agent/alice.sdk",
    "callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
    "descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
    "descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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

func TestPreparedInvocationRejectsMissingPreparedDescriptorRef(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"expires_at_unix_ms": 1783000000000,
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"expires_at_unix_ms": 1783000000000
		},
		"submit_ready": false
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON synthesized missing descriptor_ref")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if !strings.Contains(err.Error(), "descriptor_ref is required") {
		t.Fatalf("error = %v, want explicit descriptor_ref requirement", err)
	}
}

func TestPreparedInvocationRejectsMissingCanonicalBytes(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
		"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
		"descriptor_hash_hex": "aa",
		"schema_hash_hex": "bb",
		"canonical_hash_hex": "50d858e0985ecc7f60418aaf0cc5ab587f42c2570a884095a9e8ccacd0f6545c",
		"expires_at_unix_ms": 1783000000000,
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
	if prepared.SigningMaterial().CanonicalHashHex() != prepared.CanonicalHashHex() {
		t.Fatalf(
			"signing material canonical hash = %q, prepared = %q",
			prepared.SigningMaterial().CanonicalHashHex(),
			prepared.CanonicalHashHex(),
		)
	}
}

func TestProviderManagedSignerPolicyRequiresCustodyFacts(t *testing.T) {
	for _, tc := range []struct {
		name     string
		policy   string
		expected string
	}{
		{
			name:     "missing signer_id",
			policy:   `{"mode":"provider_managed_signing","policy_ref":"policy/local"}`,
			expected: "signer_id",
		},
		{
			name:     "blank signer_id",
			policy:   `{"mode":"provider_managed_signing","signer_id":" ","policy_ref":"policy/local"}`,
			expected: "signer_id",
		},
		{
			name:     "missing policy_ref",
			policy:   `{"mode":"provider_managed_signing","signer_id":"signer-key-1"}`,
			expected: "policy_ref",
		},
		{
			name:     "blank policy_ref",
			policy:   `{"mode":"provider_managed_signing","signer_id":"signer-key-1","policy_ref":" "}`,
			expected: "policy_ref",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			raw := strings.Replace(
				preparedFixture,
				`"expires_at_unix_ms": 1783000000000
  }`,
				`"expires_at_unix_ms": 1783000000000,
    "signer_policy": `+tc.policy+`
  }`,
				1,
			)
			_, err := NewPreparedInvocationFromJSON([]byte(raw))
			if err == nil {
				t.Fatalf("NewPreparedInvocationFromJSON accepted incomplete provider-managed signer policy")
			}
			if !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
			}
			if !strings.Contains(err.Error(), "provider-managed signer_policy") || !strings.Contains(err.Error(), tc.expected) {
				t.Fatalf("error = %v, want provider-managed %s rejection", err, tc.expected)
			}
		})
	}
}

func TestPreparedInvocationRequiresExplicitExpiryFact(t *testing.T) {
	for _, tc := range []struct {
		name     string
		edit     func(string) string
		expected string
	}{
		{
			name: "missing top-level expiry",
			edit: func(raw string) string {
				return strings.Replace(raw, `  "expires_at_unix_ms": 1783000000000,
`, "", 1)
			},
			expected: "expires_at_unix_ms is required",
		},
		{
			name: "mismatched top-level expiry",
			edit: func(raw string) string {
				return strings.Replace(raw, `  "expires_at_unix_ms": 1783000000000,`, `  "expires_at_unix_ms": 1783000000001,`, 1)
			},
			expected: "expires_at_unix_ms must match signing_material.expires_at_unix_ms",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := NewPreparedInvocationFromJSON([]byte(tc.edit(preparedFixture)))
			if err == nil {
				t.Fatalf("NewPreparedInvocationFromJSON accepted invalid prepared expiry")
			}
			if !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
			}
			if !strings.Contains(err.Error(), tc.expected) {
				t.Fatalf("error = %v, want %q", err, tc.expected)
			}
		})
	}
}

func TestPreparedInvocationRejectsRequestIDOnlyPayload(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"request_id": "req-1",
		"expires_at_unix_ms": 1783000000000,
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.status@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
		"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
		"expires_at_unix_ms": 1783000000000,
		"canonical_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"expires_at_unix_ms": 1783000000000
		}
	}`))
	if err == nil {
		t.Fatalf("NewPreparedInvocationFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if !strings.Contains(err.Error(), "canonical_hash_hex does not match canonical_bytes_base64") {
		t.Fatalf("error = %v, want canonical hash mismatch", err)
	}
}

func TestPreparedInvocationRejectsInvalidCanonicalBase64(t *testing.T) {
	_, err := NewPreparedInvocationFromJSON([]byte(`{
		"prepared_id": "prepared-example-1",
		"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
		"tuple": {
			"caller_ura": "easynet:///r/example/agent/alice.sdk",
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "not valid base64",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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
			"callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
			"subject_ura": "easynet:///r/example/device/dev-a",
			"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
			"causal_context": {"form": "none"},
			"args": {},
			"content_type": "application/json"
		},
		"signing_material": {
			"canonical_bytes_base64": "ZXhhbXBsZQ==",
			"args_digest_hex": "00",
			"descriptor_ref": "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
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

func TestPreparedInvocationRejectsSignerPubkeyWithoutKeyHint(t *testing.T) {
	const pubkey = "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
	prepared, err := NewPreparedInvocationFromJSON([]byte(preparedFixture))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	_, err = prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:             "ed25519",
		SignatureBase64:       "c2lnbmF0dXJl",
		SignerPublicKeyBase64: pubkey,
	})
	if err == nil || !strings.Contains(err.Error(), "signer id is required") {
		t.Fatalf("SignWithCallerSignature error = %v, want signer id rejection", err)
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
	handle.Policy = map[string]any{"mode": "provider_managed_signing"}
	if _, err := NewSigner(handle, provider); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewSigner missing usage error = %v, want InvalidArgument", err)
	}

	handle = signerHandle("")
	handle.Policy["signer_id"] = "other-signer"
	if _, err := NewSigner(handle, provider); err == nil || !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewSigner signer_id mismatch error = %v, want InvalidArgument", err)
	}

	handle = signerHandle("")
	handle.Metadata["policy_ref"] = "provider-key-inventory:sha256:other-policy"
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
		mode:     "provider_managed_signing",
		signerID: "other-signer",
	}
	signer, err := NewSigner(signerHandle(""), &memorySignatureProvider{})
	if err != nil {
		t.Fatalf("NewSigner: %v", err)
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
	policyRef := "provider-key-inventory:sha256:test-policy"
	metadata := map[string]any{"source": "provider_key_inventory", "policy_ref": policyRef}
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
			"mode":                "provider_managed_signing",
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
