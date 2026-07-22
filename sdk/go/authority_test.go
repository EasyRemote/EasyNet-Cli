package easynet

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestDelegationProofFromMetadataProjectsTypedAuthority(t *testing.T) {
	value := authorityMetadataFixture(t, map[string]any{
		"issuer_ura":    "easynet:///r/example/user/alice",
		"subject_ura":   "easynet:///r/example/user/alice",
		"caller_ura":    "easynet:///r/example/agent/backend",
		"audience":      "easynet:///r/example/device/dev-a",
		"scopes":        []string{"device.observe.*"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("delegation-signature"))

	proof, err := NewDelegationProofFromMetadata(value)
	if err != nil {
		t.Fatalf("NewDelegationProofFromMetadata: %v", err)
	}
	if proof.IssuerURA != "easynet:///r/example/user/alice" || proof.CallerURA != "easynet:///r/example/agent/backend" {
		t.Fatalf("unexpected delegation projection: %#v", proof)
	}
	if len(proof.Signature) == 0 || proof.ExpiresAtMS != 2000 {
		t.Fatalf("signature/expiry not projected: %#v", proof)
	}
	metadata, err := proof.Metadata()
	if err != nil {
		t.Fatalf("Metadata: %v", err)
	}
	if metadata.Key() != DelegationMetadataKey || metadata.Value() != value {
		t.Fatalf("metadata key/value = %q/%q", metadata.Key(), metadata.Value())
	}
}

func TestSessionAuthorityFromMetadataProjectsTypedAuthority(t *testing.T) {
	value := authorityMetadataFixture(t, sessionAuthorityPayloadFixture(), []byte("session-signature"))

	authority, err := NewSessionAuthorityFromMetadata(value)
	if err != nil {
		t.Fatalf("NewSessionAuthorityFromMetadata: %v", err)
	}
	if authority.IssuerURA != "easynet:///r/example/agent/backend" ||
		authority.SessionID != "session-1" ||
		authority.SubjectURA != "easynet:///r/example/resource/user.alice/session/session-1" {
		t.Fatalf("unexpected session authority projection: %#v", authority)
	}
	metadata, err := authority.Metadata()
	if err != nil {
		t.Fatalf("Metadata: %v", err)
	}
	if metadata.Key() != SessionAuthorityMetadataKey || metadata.Value() != value {
		t.Fatalf("metadata key/value = %q/%q", metadata.Key(), metadata.Value())
	}
}

func TestSessionAuthorityRejectsAllZeroOwner(t *testing.T) {
	payload := sessionAuthorityPayloadFixture()
	payload["session_owner_user_id"] = "00000000-0000-0000-0000-000000000000"
	value := authorityMetadataFixture(t, payload, []byte("session-signature"))

	_, err := NewSessionAuthorityFromMetadata(value)

	if err == nil || !strings.Contains(err.Error(), "session_owner_user_id must not be all-zero") {
		t.Fatalf("all-zero owner error = %v", err)
	}
}

func TestSessionAuthorityRawSigningRoundTrip(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatalf("GenerateKey: %v", err)
	}
	authority := &SessionAuthority{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "session-1",
		SessionOwnerUserID:       "alice",
		CreatorPrincipalID:       "easynet:///r/example/agent/backend",
		CalleeURA:                "easynet:///r/example/device/dev-a",
		SubjectURA:               "easynet:///r/example/resource/user.alice/session/session-1",
		Audience:                 "easynet:///r/example/device/dev-a",
		Scopes:                   []string{"device.observe.*"},
		AllowedActions:           []string{"read"},
		AllowedFollowupAbilities: []string{"device.observe.health"},
		IssuedAtMS:               1000,
		ExpiresAtMS:              2000,
	}

	payload, err := authority.CanonicalPayload()
	if err != nil {
		t.Fatalf("CanonicalPayload: %v", err)
	}
	var projected map[string]any
	if err := json.Unmarshal(payload, &projected); err != nil {
		t.Fatalf("decode canonical payload: %v", err)
	}
	if projected["issuer_ura"] != authority.IssuerURA ||
		projected["session_id"] != authority.SessionID ||
		projected["subject_ura"] != authority.SubjectURA {
		t.Fatalf("canonical payload used non-generic authority fields: %#v", projected)
	}

	if err := authority.SignWith(testCanonicalSigner{
		publicKey: publicKey,
		sign: func(payload []byte) ([]byte, error) {
			return ed25519.Sign(privateKey, payload), nil
		},
	}); err != nil {
		t.Fatalf("SignWith: %v", err)
	}
	if len(authority.Signature) == 0 {
		t.Fatalf("Sign did not populate signature")
	}
	if err := authority.Verify(publicKey); err != nil {
		t.Fatalf("Verify: %v", err)
	}
	raw, err := authority.MarshalRaw()
	if err != nil {
		t.Fatalf("MarshalRaw: %v", err)
	}
	if len(raw) == 0 || raw[0] != '{' {
		t.Fatalf("MarshalRaw must remain raw JSON wire payload, got %q", string(raw))
	}
	metadataValue, err := authority.MarshalMetadataValue()
	if err != nil {
		t.Fatalf("MarshalMetadataValue: %v", err)
	}
	if metadataValue == "" || metadataValue[0] == '{' {
		t.Fatalf("MarshalMetadataValue must be base64 metadata value, got %q", metadataValue)
	}
	metadataDecoded, err := NewSessionAuthorityFromMetadata(metadataValue)
	if err != nil {
		t.Fatalf("NewSessionAuthorityFromMetadata(metadata value): %v", err)
	}
	if metadataDecoded.IssuerURA != authority.IssuerURA ||
		metadataDecoded.SubjectURA != authority.SubjectURA ||
		metadataDecoded.Audience != authority.Audience {
		t.Fatalf("unexpected metadata decoded authority: %#v", metadataDecoded)
	}
	decoded, err := UnmarshalRawSessionAuthority(raw)
	if err != nil {
		t.Fatalf("UnmarshalRawSessionAuthority: %v", err)
	}
	if decoded.IssuerURA != authority.IssuerURA || decoded.SubjectURA != authority.SubjectURA || decoded.Audience != authority.Audience {
		t.Fatalf("unexpected decoded authority: %#v", decoded)
	}
	if err := decoded.Verify(publicKey); err != nil {
		t.Fatalf("decoded Verify: %v", err)
	}
	if !decoded.MatchesScope("device.observe.health") || decoded.MatchesScope("device.write.health") {
		t.Fatalf("scope matching drifted for decoded authority")
	}
	if !decoded.MatchesAudience("easynet:///r/example/device/dev-a") ||
		decoded.MatchesAudience("easynet:///r/example/device/dev-b") {
		t.Fatalf("audience matching drifted for decoded authority")
	}
	if decoded.IsExpired(time.UnixMilli(1999)) || !decoded.IsExpired(time.UnixMilli(2000)) {
		t.Fatalf("expiry boundary drifted for decoded authority")
	}
}

type testCanonicalSigner struct {
	publicKey ed25519.PublicKey
	sign      func([]byte) ([]byte, error)
}

func (sign testCanonicalSigner) SignCanonical(payload []byte) ([]byte, error) {
	return sign.sign(payload)
}

func (sign testCanonicalSigner) SigningPublicKey() (ed25519.PublicKey, error) {
	return append(ed25519.PublicKey(nil), sign.publicKey...), nil
}

func TestInvocationBuilderAttachesOneAuthorityMetadata(t *testing.T) {
	proof, err := NewDelegationProofFromMetadata(authorityMetadataFixture(t, map[string]any{
		"issuer_ura":    "easynet:///r/example/user/alice",
		"subject_ura":   "easynet:///r/example/user/alice",
		"caller_ura":    "easynet:///r/example/agent/backend",
		"audience":      "*",
		"scopes":        []string{"*"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("signature")))
	if err != nil {
		t.Fatalf("NewDelegationProofFromMetadata: %v", err)
	}
	metadata, err := proof.Metadata()
	if err != nil {
		t.Fatalf("Metadata: %v", err)
	}

	draft, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/backend").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/user/alice").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		WithMetadata(map[string]any{"trace": "t-1"}).
		WithAuthorityMetadata(metadata).
		Build()
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	if draft.Metadata()["trace"] != "t-1" || draft.Metadata()[DelegationMetadataKey] != metadata.Value() {
		t.Fatalf("authority metadata not merged: %#v", draft.Metadata())
	}
}

func TestInvocationBuilderRejectsAmbiguousAuthorityMetadata(t *testing.T) {
	_, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/backend").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/user/alice").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		WithMetadata(map[string]any{
			DelegationMetadataKey:       "delegation",
			SessionAuthorityMetadataKey: "session",
		}).
		Build()
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Build error = %v, want invalid argument", err)
	}
}

func TestAuthorityClientMintsDelegationThroughTransport(t *testing.T) {
	value := authorityMetadataFixture(t, map[string]any{
		"issuer_ura":    "easynet:///r/example/user/alice",
		"subject_ura":   "easynet:///r/example/user/alice",
		"caller_ura":    "easynet:///r/example/agent/backend",
		"audience":      "easynet:///r/example/device/dev-a",
		"scopes":        []string{"device.observe.*"},
		"issued_at_ms":  float64(1000),
		"expires_at_ms": float64(2000),
	}, []byte("delegation-signature"))
	transport := &memoryAuthorityTransport{
		delegationJSON: []byte(`{"metadata_value":"` + value + `"}`),
	}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

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
	if proof.CallerURA != "easynet:///r/example/agent/backend" || proof.metadataValue != value {
		t.Fatalf("unexpected proof: %#v", proof)
	}
	if transport.seenDelegation["caller_ura"] != "easynet:///r/example/agent/backend" {
		t.Fatalf("transport did not receive delegation request: %#v", transport.seenDelegation)
	}
}

func TestAuthorityClientMintsSessionAuthorityThroughTransport(t *testing.T) {
	value := authorityMetadataFixture(t, sessionAuthorityPayloadFixture(), []byte("session-signature"))
	transport := &memoryAuthorityTransport{
		sessionJSON: []byte(`{"metadata":{"` + SessionAuthorityMetadataKey + `":"` + value + `"}}`),
	}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

	authority, err := client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "session-1",
		SessionOwnerUserID:       "alice",
		CreatorPrincipalID:       "easynet:///r/example/agent/backend",
		CalleeURA:                "easynet:///r/example/device/dev-a",
		SubjectURA:               "easynet:///r/example/resource/user.alice/session/session-1",
		Audience:                 "easynet:///r/example/device/dev-a",
		Scopes:                   []string{"device.observe.*"},
		AllowedActions:           []string{"read"},
		AllowedFollowupAbilities: []string{"device.observe.health"},
		IssuedAtMS:               1000,
		ExpiresAtMS:              2000,
	})
	if err != nil {
		t.Fatalf("MintSessionAuthority: %v", err)
	}
	if authority.Audience != "easynet:///r/example/device/dev-a" || authority.metadataValue != value {
		t.Fatalf("unexpected authority: %#v", authority)
	}
	if transport.seenSession["audience"] != "easynet:///r/example/device/dev-a" {
		t.Fatalf("transport did not receive session request: %#v", transport.seenSession)
	}
}

func TestAuthorityClientProjectsCanonicalPrincipalURAsToCurrentSessionWire(t *testing.T) {
	payload := sessionAuthorityPayloadFixture()
	payload["creator_principal_id"] = "easynet:///r/example/authority"
	value := authorityMetadataFixture(t, payload, []byte("session-signature"))
	transport := &memoryAuthorityTransport{
		sessionJSON: []byte(`{"metadata":{"` + SessionAuthorityMetadataKey + `":"` + value + `"}}`),
	}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

	authority, err := client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "session-1",
		SessionOwnerURA:          "easynet:///r/example/user/alice",
		CreatorPrincipalURA:      "easynet:///r/example/authority",
		CalleeURA:                "easynet:///r/example/device/dev-a",
		SubjectURA:               "easynet:///r/example/resource/user.alice/session/session-1",
		Audience:                 "easynet:///r/example/device/dev-a",
		Scopes:                   []string{"device.observe.*"},
		AllowedActions:           []string{"read"},
		AllowedFollowupAbilities: []string{"device.observe.health"},
		IssuedAtMS:               1000,
		ExpiresAtMS:              2000,
	})
	if err != nil {
		t.Fatalf("MintSessionAuthority: %v", err)
	}
	if authority.SessionOwnerURA != "easynet:///r/example/user/alice" || authority.CreatorPrincipalURA != "easynet:///r/example/authority" {
		t.Fatalf("canonical authority principals not projected: %#v", authority)
	}
	if transport.seenSession["session_owner_user_id"] != "alice" || transport.seenSession["creator_principal_id"] != "easynet:///r/example/authority" {
		t.Fatalf("canonical principals not lowered to current wire: %#v", transport.seenSession)
	}
	if _, ok := transport.seenSession["session_owner_ura"]; ok {
		t.Fatalf("staged daemon wire must not leak session_owner_ura yet: %#v", transport.seenSession)
	}
}

func TestAuthorityClientRejectsConflictingCanonicalPrincipalURAs(t *testing.T) {
	client, err := NewAuthorityClient(&memoryAuthorityTransport{sessionJSON: []byte(`{"metadata":{}}`)})
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

	_, err = client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "session-1",
		SessionOwnerUserID:       "bob",
		SessionOwnerURA:          "easynet:///r/example/user/alice",
		CreatorPrincipalID:       "easynet:///r/example/agent/backend",
		CalleeURA:                "easynet:///r/example/device/dev-a",
		SubjectURA:               "easynet:///r/example/resource/user.alice/session/session-1",
		Audience:                 "easynet:///r/example/device/dev-a",
		Scopes:                   []string{"device.observe.*"},
		AllowedActions:           []string{"read"},
		AllowedFollowupAbilities: []string{"device.observe.health"},
		IssuedAtMS:               1000,
		ExpiresAtMS:              2000,
	})
	if !IsCode(err, ErrInvalidArgument) || !strings.Contains(err.Error(), "session_owner_user_id must match session_owner_ura") {
		t.Fatalf("MintSessionAuthority mismatch error = %v", err)
	}
}

func TestCanonicalAuthorityClientMintsSessionMetadataWithOpaqueSigner(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatalf("GenerateKey: %v", err)
	}
	client, err := NewCanonicalAuthorityClient(testCanonicalSigner{
		publicKey: publicKey,
		sign: func(payload []byte) ([]byte, error) {
			return ed25519.Sign(privateKey, payload), nil
		},
	})
	if err != nil {
		t.Fatalf("NewCanonicalAuthorityClient: %v", err)
	}

	authority, err := client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "session-1",
		SessionOwnerUserID:       "alice",
		CreatorPrincipalID:       "easynet:///r/example/agent/backend",
		CalleeURA:                "easynet:///r/example/device/dev-a",
		SubjectURA:               "easynet:///r/example/resource/user.alice/session/session-1",
		Audience:                 "easynet:///r/example/device/dev-a",
		Scopes:                   []string{"device.observe.*"},
		AllowedActions:           []string{"read"},
		AllowedFollowupAbilities: []string{"device.observe.health"},
		IssuedAtMS:               1000,
		ExpiresAtMS:              2000,
	})
	if err != nil {
		t.Fatalf("MintSessionAuthority: %v", err)
	}
	if err := authority.Verify(publicKey); err != nil {
		t.Fatalf("authority signature: %v", err)
	}
	metadata, err := authority.Metadata()
	if err != nil {
		t.Fatalf("authority metadata: %v", err)
	}
	if metadata.Key() != SessionAuthorityMetadataKey || metadata.Value() == "" {
		t.Fatalf("metadata = %#v", metadata)
	}

	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	_, err = client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "session-2",
		SessionOwnerUserID:       "alice",
		CreatorPrincipalID:       "easynet:///r/example/agent/backend",
		CalleeURA:                "easynet:///r/example/device/dev-a",
		SubjectURA:               "easynet:///r/example/resource/user.alice/session/session-2",
		Audience:                 "easynet:///r/example/device/dev-a",
		Scopes:                   []string{"device.observe.*"},
		AllowedActions:           []string{"read"},
		AllowedFollowupAbilities: []string{"device.observe.health"},
		IssuedAtMS:               1000,
		ExpiresAtMS:              2000,
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("MintSessionAuthority after close = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestAuthorityClientRejectsInvalidMintBeforeTransport(t *testing.T) {
	transport := &memoryAuthorityTransport{}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		t.Fatalf("NewAuthorityClient: %v", err)
	}

	_, err = client.MintDelegationProof(context.Background(), DelegationRequest{
		IssuerURA:   "easynet:///r/example/user/alice",
		SubjectURA:  "easynet:///r/example/user/alice",
		CallerURA:   "easynet:///r/example/agent/backend",
		Audience:    "easynet:///r/example/device/dev-a",
		IssuedAtMS:  1000,
		ExpiresAtMS: 2000,
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("MintDelegationProof error = %v, want invalid argument", err)
	}
	if transport.delegationCalls != 0 {
		t.Fatalf("transport called for invalid delegation request")
	}

	_, err = client.MintSessionAuthority(context.Background(), SessionAuthorityRequest{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "session-1",
		SessionOwnerUserID:       "alice",
		CreatorPrincipalID:       "easynet:///r/example/agent/backend",
		CalleeURA:                "easynet:///r/example/device/dev-a",
		SubjectURA:               "easynet:///r/example/resource/user.alice/session/session-1",
		Audience:                 "easynet:///r/example/device/dev-a",
		Scopes:                   []string{"device.observe.*"},
		AllowedActions:           []string{"read"},
		AllowedFollowupAbilities: []string{"device.observe.health"},
		IssuedAtMS:               2000,
		ExpiresAtMS:              1000,
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("MintSessionAuthority error = %v, want invalid argument", err)
	}
	if transport.sessionCalls != 0 {
		t.Fatalf("transport called for invalid session request")
	}
}

func TestAuthoritySigningMaterialProjectionValidatesRuntimeCoreOutput(t *testing.T) {
	raw := []byte(`{
		"profile":"authority",
		"kind":"delegation",
		"algorithm":"ed25519",
		"metadata_key":"x-runtime-delegation",
		"canonical_bytes_base64":"eyJjYWxsZXJfdXJhIjoiZWFzeW5ldDovLy9yL2V4YW1wbGUvYWdlbnQvYmFja2VuZCJ9",
		"canonical_hash_hex":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		"signed_fields":["caller_ura"],
		"payload":{"caller_ura":"easynet:///r/example/agent/backend"}
	}`)
	material, err := newAuthoritySigningMaterial(raw, DelegationMetadataKey, AuthorityKindDelegation)
	if err != nil {
		t.Fatalf("newAuthoritySigningMaterial: %v", err)
	}
	if material.Profile != authorityProfile || material.Kind != AuthorityKindDelegation {
		t.Fatalf("unexpected material: %#v", material)
	}
}

func TestAuthoritySignatureJSONRejectsLegacySignatureField(t *testing.T) {
	if _, err := authoritySignatureJSON(AuthoritySignature{}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("empty authoritySignatureJSON error = %v, want invalid argument", err)
	}
	raw, err := authoritySignatureJSON(AuthoritySignature{
		SignatureBase64: base64.StdEncoding.EncodeToString([]byte("signature")),
	})
	if err != nil {
		t.Fatalf("authoritySignatureJSON: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatalf("decode signature JSON: %v", err)
	}
	if _, ok := decoded["signature"]; ok {
		t.Fatalf("legacy signature field must not be emitted: %#v", decoded)
	}
	if decoded["signature_base64"] == "" {
		t.Fatalf("signature_base64 missing: %#v", decoded)
	}
}

func authorityMetadataFixture(t *testing.T, payload map[string]any, signature []byte) string {
	t.Helper()
	wire, err := json.Marshal(map[string]any{
		"payload":   payload,
		"signature": base64.StdEncoding.EncodeToString(signature),
	})
	if err != nil {
		t.Fatalf("marshal authority fixture: %v", err)
	}
	return base64.StdEncoding.EncodeToString(wire)
}

func sessionAuthorityPayloadFixture() map[string]any {
	return map[string]any{
		"issuer_ura":                 "easynet:///r/example/agent/backend",
		"session_id":                 "session-1",
		"session_owner_user_id":      "alice",
		"creator_principal_id":       "easynet:///r/example/agent/backend",
		"callee_ura":                 "easynet:///r/example/device/dev-a",
		"subject_ura":                "easynet:///r/example/resource/user.alice/session/session-1",
		"audience":                   "easynet:///r/example/device/dev-a",
		"scopes":                     []string{"device.observe.*"},
		"allowed_actions":            []string{"read"},
		"allowed_followup_abilities": []string{"device.observe.health"},
		"issued_at_ms":               float64(1000),
		"expires_at_ms":              float64(2000),
	}
}

type memoryAuthorityTransport struct {
	delegationJSON []byte
	sessionJSON    []byte

	delegationCalls int
	sessionCalls    int
	seenDelegation  map[string]any
	seenSession     map[string]any
}

func (m *memoryAuthorityTransport) MintDelegationProof(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.delegationCalls++
	if err := json.Unmarshal(requestJSON, &m.seenDelegation); err != nil {
		return nil, err
	}
	return m.delegationJSON, nil
}

func (m *memoryAuthorityTransport) MintSessionAuthority(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.sessionCalls++
	if err := json.Unmarshal(requestJSON, &m.seenSession); err != nil {
		return nil, err
	}
	return m.sessionJSON, nil
}
