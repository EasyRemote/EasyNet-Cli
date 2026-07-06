//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

func TestCABIAuthorityTransportMintsViaCoreAndSigner(t *testing.T) {
	delegationValue := authorityMetadataFixture(t, map[string]any{
		"issuer_ura":    "easynet:///r/example/user/alice",
		"subject_ura":   "easynet:///r/example/user/alice",
		"caller_ura":    "easynet:///r/example/agent/backend",
		"audience":      "easynet:///r/example/device/dev-a",
		"scopes":        []string{"device.observe.*"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("cabi-signature"))
	sessionValue := authorityMetadataFixture(t, map[string]any{
		"backend_ura":   "easynet:///r/example/agent/backend",
		"user_ura":      "easynet:///r/example/user/alice",
		"session_id":    "sa-example",
		"scopes":        []string{"device.observe.*"},
		"audiences":     []string{"easynet:///r/example/device/dev-a"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("cabi-signature"))
	libraryPath := buildFakeCABIAuthorityLibrary(t, delegationValue, sessionValue)
	signer := &recordingAuthoritySigner{
		signature: AuthoritySignature{SignatureBase64: base64.StdEncoding.EncodeToString([]byte("cabi-signature"))},
	}
	client, transport, err := NewCABIAuthorityClient(libraryPath, signer)
	if err != nil {
		t.Fatalf("NewCABIAuthorityClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close: %v", err)
		}
	}()

	proof, err := client.MintDelegationProof(context.Background(), DelegationRequest{
		IssuerURA:   "easynet:///r/example/user/alice",
		SubjectURA:  "easynet:///r/example/user/alice",
		CallerURA:   "easynet:///r/example/agent/backend",
		Audience:    "easynet:///r/example/device/dev-a",
		Scopes:      []string{"device.observe.*"},
		IssuedAtMS:  1000,
		ExpiresAtMS: 2000,
	})
	if err != nil {
		t.Fatalf("MintDelegationProof: %v", err)
	}
	if proof.metadataValue != delegationValue {
		t.Fatalf("delegation metadata = %q", proof.metadataValue)
	}
	if signer.seen[0].Kind != AuthorityKindDelegation || signer.seen[0].MetadataKey != DelegationMetadataKey {
		t.Fatalf("unexpected delegation signing material: %#v", signer.seen[0])
	}

	session, err := client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		BackendURA:  "easynet:///r/example/agent/backend",
		UserURA:     "easynet:///r/example/user/alice",
		SessionID:   "sa-example",
		Scopes:      []string{"device.observe.*"},
		Audiences:   []string{"easynet:///r/example/device/dev-a"},
		IssuedAtMS:  1000,
		ExpiresAtMS: 2000,
	})
	if err != nil {
		t.Fatalf("MintSessionAuthority: %v", err)
	}
	if session.metadataValue != sessionValue {
		t.Fatalf("session metadata = %q", session.metadataValue)
	}
	if signer.seen[1].Kind != AuthorityKindSessionAuthority || signer.seen[1].MetadataKey != SessionAuthorityMetadataKey {
		t.Fatalf("unexpected session signing material: %#v", signer.seen[1])
	}
}

func TestCABIAuthorityTransportRequiresLatestSignatureEnvelope(t *testing.T) {
	libraryPath := buildFakeCABIAuthorityLibrary(t, "delegation", "session")
	transport, err := OpenCABIAuthorityTransport(libraryPath, AuthoritySignatureProviderFunc(
		func(context.Context, AuthoritySigningMaterial) (AuthoritySignature, error) {
			return AuthoritySignature{}, nil
		},
	))
	if err != nil {
		t.Fatalf("OpenCABIAuthorityTransport: %v", err)
	}
	defer func() { _ = transport.Close(context.Background()) }()

	_, err = transport.MintDelegationProof(context.Background(), []byte(`{
		"issuer_ura":"easynet:///r/example/user/alice",
		"subject_ura":"easynet:///r/example/user/alice",
		"caller_ura":"easynet:///r/example/agent/backend",
		"audience":"easynet:///r/example/device/dev-a",
		"scopes":["device.observe.*"],
		"issued_at_ms":1000,
		"expires_at_ms":2000
	}`))
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("MintDelegationProof error = %v, want invalid argument", err)
	}
}

type recordingAuthoritySigner struct {
	signature AuthoritySignature
	seen      []AuthoritySigningMaterial
}

func (s *recordingAuthoritySigner) SignAuthority(_ context.Context, material AuthoritySigningMaterial) (AuthoritySignature, error) {
	s.seen = append(s.seen, material)
	return s.signature, nil
}

func buildFakeCABIAuthorityLibrary(t *testing.T, delegationValue string, sessionValue string) string {
	t.Helper()
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_authority.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
	}
	if err := os.WriteFile(source, []byte(fmt.Sprintf(fakeCABIAuthoritySource, cStringLiteral(delegationValue), cStringLiteral(sessionValue))), 0o600); err != nil {
		t.Fatalf("write fake C ABI source: %v", err)
	}
	cmd := exec.Command("cc", "-shared", "-fPIC", source, "-o", output)
	if runtime.GOOS == "darwin" {
		cmd = exec.Command("cc", "-dynamiclib", source, "-o", output)
	}
	if raw, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build fake C ABI library: %v\n%s", err, raw)
	}
	return output
}

func cStringLiteral(value string) string {
	raw, _ := json.Marshal(value)
	return string(raw)
}

const fakeCABIAuthoritySource = `
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char *dup_json(const char *s) {
	size_t n = strlen(s) + 1;
	char *out = (char *)malloc(n);
	memcpy(out, s, n);
	return out;
}

uint32_t easynet_abi_version(void) { return 4; }
int32_t easynet_last_error_json(char **out_error_json) { *out_error_json = NULL; return 0; }
void easynet_string_free(char *s) { free(s); }

int32_t easynet_authority_prepare_delegation(const char *request_json, char **out_material_json) {
	(void)request_json;
	*out_material_json = dup_json("{\"profile\":\"authority\",\"kind\":\"delegation\",\"algorithm\":\"ed25519\",\"metadata_key\":\"x-easynet-delegation\",\"canonical_bytes_base64\":\"Y2Fub24=\",\"canonical_hash_hex\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"signed_fields\":[\"issuer_ura\"],\"payload\":{\"issuer_ura\":\"easynet:///r/example/user/alice\"}}");
	return 0;
}

int32_t easynet_authority_materialize_delegation(const char *request_json, const char *signature_json, char **out_metadata_json) {
	(void)request_json;
	if (strstr(signature_json, "signature_base64") == NULL) { return 2; }
	const char *value = %s;
	size_t n = strlen(value) * 2 + 128;
	char *out = (char *)malloc(n);
	snprintf(out, n, "{\"metadata_value\":\"%%s\",\"metadata\":{\"x-easynet-delegation\":\"%%s\"}}", value, value);
	*out_metadata_json = out;
	return 0;
}

int32_t easynet_authority_prepare_session(const char *request_json, char **out_material_json) {
	(void)request_json;
	*out_material_json = dup_json("{\"profile\":\"authority\",\"kind\":\"session_authority\",\"algorithm\":\"ed25519\",\"metadata_key\":\"x-easynet-session-authority\",\"canonical_bytes_base64\":\"Y2Fub24=\",\"canonical_hash_hex\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"signed_fields\":[\"backend_ura\"],\"payload\":{\"backend_ura\":\"easynet:///r/example/agent/backend\"}}");
	return 0;
}

int32_t easynet_authority_materialize_session(const char *request_json, const char *signature_json, char **out_metadata_json) {
	(void)request_json;
	if (strstr(signature_json, "signature_base64") == NULL) { return 2; }
	const char *value = %s;
	size_t n = strlen(value) * 2 + 144;
	char *out = (char *)malloc(n);
	snprintf(out, n, "{\"metadata_value\":\"%%s\",\"metadata\":{\"x-easynet-session-authority\":\"%%s\"}}", value, value);
	*out_metadata_json = out;
	return 0;
}
`
