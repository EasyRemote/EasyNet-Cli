package easynet

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestManagedSigningClientRequiresExplicitRuntimeKeyServiceEndpoint(t *testing.T) {
	for _, socketPath := range []string{"", " \t\n "} {
		_, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
		if !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("NewManagedSigningClient(%q) error = %v, want INVALID_ARGUMENT", socketPath, err)
		}
	}
}

func TestManagedSigningClientConformsToRuntimeKeyServiceProtocol(t *testing.T) {
	privateKey1 := ed25519.NewKeyFromSeed(bytesOf(1, ed25519.SeedSize))
	publicKey1 := privateKey1.Public().(ed25519.PublicKey)
	publicKey2 := ed25519.NewKeyFromSeed(bytesOf(2, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	peerPublicKey := ed25519.NewKeyFromSeed(bytesOf(3, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	signature := ed25519.Sign(privateKey1, []byte("canonical"))
	fingerprint := sha256.Sum256(peerPublicKey)
	const (
		keyID1          = "managed-key-1"
		keyID2          = "managed-key-2"
		subjectURA      = "easynet:///r/acme/agent/signer.main"
		peerURA         = "easynet:///r/peer/agent/verifier.main"
		viaAuthorityURA = "easynet:///r/acme/authority"
	)

	requestCount := 0
	done := make(chan struct{})
	socketPath := startRuntimeKeyServiceTestServer(t, func(request map[string]any) map[string]any {
		requestCount++
		assertNoPrivateKeyRequestFields(t, request)
		var response map[string]any
		switch requestCount {
		case 1:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.create", "purpose": "invocation", "bound_subject": subjectURA,
			})
			response = map[string]any{"result": "inventory_key", "entry": managedSigningKeyFixture(keyID1, publicKey1, "active", 0, "", subjectURA)}
		case 2:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.list", "purpose": "invocation", "status": "active",
				"limit": float64(ManagedSigningDefaultPageLimit),
			})
			response = map[string]any{
				"result": "inventory_keys", "entries": []any{managedSigningKeyFixture(keyID1, publicKey1, "active", 0, "", subjectURA)},
				"next_cursor": nil,
			}
		case 3:
			assertManagedSigningRequest(t, request, map[string]any{"method": "inventory.public_key", "key_id": keyID1})
			response = map[string]any{"result": "inventory_key", "entry": managedSigningKeyFixture(keyID1, publicKey1, "active", 0, "", subjectURA)}
		case 4:
			assertManagedSigningRequest(t, request, map[string]any{"method": "inventory.public_key", "key_id": keyID1})
			response = map[string]any{"result": "inventory_key", "entry": managedSigningKeyFixture(keyID1, publicKey1, "active", 0, "", subjectURA)}
		case 5:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.sign", "key_id": keyID1,
				"subject_ura":         subjectURA,
				"expected_purpose":    "invocation",
				"signer_policy_ref":   canonicalManagedSignerPolicyRef("invocation", subjectURA, keyID1, publicKey1),
				"canonical_bytes_b64": base64.StdEncoding.EncodeToString([]byte("canonical")),
			})
			response = map[string]any{"result": "signature", "signature_b64": base64.StdEncoding.EncodeToString(signature)}
		case 6:
			assertManagedSigningRequest(t, request, map[string]any{"method": "inventory.rotate", "key_id": keyID1})
			response = map[string]any{"result": "inventory_key", "entry": managedSigningKeyFixture(keyID2, publicKey2, "active", 1, keyID1, subjectURA)}
		case 7:
			assertManagedSigningRequest(t, request, map[string]any{"method": "inventory.revoke", "key_id": keyID1})
			response = map[string]any{"result": "inventory_revoked", "revoked_unix_ms": int64(1700000000100)}
		case 8:
			assertManagedSigningRequest(t, request, map[string]any{"method": "inventory.set_expiry", "key_id": keyID2, "expires_unix_ms": float64(1700000010000)})
			response = map[string]any{"result": "ok"}
		case 9:
			assertManagedSigningRequest(t, request, map[string]any{"method": "inventory.bind_subject", "key_id": keyID2, "subject_ura": subjectURA})
			response = map[string]any{"result": "ok"}
		case 10:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.peer_add", "peer_ura": peerURA,
				"public_key_b64": base64.StdEncoding.EncodeToString(peerPublicKey), "via_authority": viaAuthorityURA,
			})
			response = map[string]any{"result": "inventory_peer_added", "added": true}
		case 11:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.peer_list", "limit": float64(ManagedSigningDefaultPageLimit),
			})
			response = map[string]any{"result": "inventory_peers", "peers": []any{map[string]any{
				"peer_ura": peerURA, "fingerprint_b64": base64.StdEncoding.EncodeToString(fingerprint[:]),
				"public_key_b64": base64.StdEncoding.EncodeToString(peerPublicKey), "via_authority": viaAuthorityURA,
				"added_unix_ms": int64(1700000000200), "last_seen_unix_ms": int64(1700000000300),
			}}, "next_cursor": nil}
			close(done)
		default:
			t.Errorf("unexpected extra managed-signing request: %#v", request)
			response = map[string]any{"result": "error", "kind": "policy", "message": "unexpected request"}
		}
		return response
	})

	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	created, err := client.Create(ManagedSigningCreateRequest{Purpose: "invocation", BoundSubjectURA: subjectURA})
	if err != nil {
		t.Fatalf("Create: %v", err)
	}
	if created.KeyID != keyID1 || !created.PublicKey.Equal(publicKey1) || created.Status != ManagedSigningStatusActive {
		t.Fatalf("unexpected created projection: %#v", created)
	}
	listed, err := client.List(ManagedSigningKeyFilter{Purpose: "invocation", Status: ManagedSigningStatusActive})
	if err != nil || len(listed) != 1 || listed[0].KeyID != keyID1 {
		t.Fatalf("List = %#v, %v", listed, err)
	}
	projection, err := client.PublicProjection(keyID1)
	if err != nil || projection.SignerPolicyRef == "" {
		t.Fatalf("PublicProjection = %#v, %v", projection, err)
	}
	actualSignature, err := client.Sign(keyID1, []byte("canonical"))
	if err != nil || string(actualSignature) != string(signature) {
		t.Fatalf("Sign = %x, %v", actualSignature, err)
	}
	rotated, err := client.Rotate(keyID1)
	if err != nil || rotated.KeyID != keyID2 || rotated.RotatedFrom != keyID1 || rotated.RotationEpoch != 1 {
		t.Fatalf("Rotate = %#v, %v", rotated, err)
	}
	revokedAt, err := client.Revoke(keyID1)
	if err != nil || revokedAt != 1700000000100 {
		t.Fatalf("Revoke = %d, %v", revokedAt, err)
	}
	if err := client.SetExpiry(keyID2, 1700000010000); err != nil {
		t.Fatalf("SetExpiry: %v", err)
	}
	if err := client.BindSubject(keyID2, subjectURA); err != nil {
		t.Fatalf("BindSubject: %v", err)
	}
	added, err := client.AddPeer(ManagedSigningPeerRegistration{PeerURA: peerURA, PublicKey: peerPublicKey, ViaAuthorityURA: viaAuthorityURA})
	if err != nil || !added {
		t.Fatalf("AddPeer = %t, %v", added, err)
	}
	peers, err := client.ListPeers()
	if err != nil || len(peers) != 1 || peers[0].PeerURA != peerURA || !peers[0].PublicKey.Equal(peerPublicKey) || peers[0].ViaAuthorityURA != viaAuthorityURA {
		t.Fatalf("ListPeers = %#v, %v", peers, err)
	}

	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("fake provider key service did not receive the complete managed-signing operation sequence")
	}
}

func TestManagedSigningClientProjectsTypedLifecycleRejection(t *testing.T) {
	socketPath := startRuntimeKeyServiceTestServer(t, func(request map[string]any) map[string]any {
		assertNoPrivateKeyRequestFields(t, request)
		return map[string]any{
			"result": "error", "kind": "lifecycle", "message": "only active keys can sign",
		}
	})
	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	_, err = client.Sign("retired-key", []byte("canonical"))
	if !IsCode(err, ErrPolicyDenied) {
		t.Fatalf("Sign error = %v, want policy-denied SDK error", err)
	}
	var sdkError *SDKError
	if !errors.As(err, &sdkError) || sdkError.Details["kind"] != "lifecycle" {
		t.Fatalf("Sign rejection details = %#v, want lifecycle kind", sdkError)
	}
}

func TestManagedSigningClientRejectsWrongResponseVariant(t *testing.T) {
	socketPath := startRuntimeKeyServiceTestServer(t, func(request map[string]any) map[string]any {
		return map[string]any{"result": "signature", "signature_b64": base64.StdEncoding.EncodeToString(bytesOf(1, 64))}
	})
	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	_, err = client.PublicProjection("managed-key-1")
	if !IsCode(err, ErrProtocol) {
		t.Fatalf("PublicProjection error = %v, want protocol error", err)
	}
}

func TestManagedSigningClientAutoPaginatesBoundedInventories(t *testing.T) {
	keyPublic1 := ed25519.NewKeyFromSeed(bytesOf(11, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	keyPublic2 := ed25519.NewKeyFromSeed(bytesOf(12, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	peerPublic1 := ed25519.NewKeyFromSeed(bytesOf(13, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	peerPublic2 := ed25519.NewKeyFromSeed(bytesOf(14, ed25519.SeedSize)).Public().(ed25519.PublicKey)

	requestCount := 0
	socketPath := startRuntimeKeyServiceTestServer(t, func(request map[string]any) map[string]any {
		requestCount++
		switch requestCount {
		case 1:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.list", "limit": float64(ManagedSigningDefaultPageLimit),
			})
			return map[string]any{
				"result": "inventory_keys", "entries": []any{managedSigningKeyFixture("key-1", keyPublic1, "active", 0, "", "")},
				"next_cursor": "keys:1",
			}
		case 2:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.list", "limit": float64(ManagedSigningDefaultPageLimit), "cursor": "keys:1",
			})
			return map[string]any{
				"result": "inventory_keys", "entries": []any{managedSigningKeyFixture("key-2", keyPublic2, "active", 0, "", "")},
				"next_cursor": nil,
			}
		case 3:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.peer_list", "limit": float64(ManagedSigningDefaultPageLimit),
			})
			return map[string]any{
				"result": "inventory_peers", "peers": []any{managedSigningPeerFixture("easynet:///r/peer/a", peerPublic1)},
				"next_cursor": "peers:1",
			}
		case 4:
			assertManagedSigningRequest(t, request, map[string]any{
				"method": "inventory.peer_list", "limit": float64(ManagedSigningDefaultPageLimit), "cursor": "peers:1",
			})
			return map[string]any{
				"result": "inventory_peers", "peers": []any{managedSigningPeerFixture("easynet:///r/peer/b", peerPublic2)},
				"next_cursor": nil,
			}
		default:
			t.Fatalf("unexpected pagination request: %#v", request)
			return nil
		}
	})
	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	keys, err := client.List(ManagedSigningKeyFilter{})
	if err != nil || len(keys) != 2 || keys[0].KeyID != "key-1" || keys[1].KeyID != "key-2" {
		t.Fatalf("List = %#v, %v", keys, err)
	}
	peers, err := client.ListPeers()
	if err != nil || len(peers) != 2 || peers[0].PeerURA != "easynet:///r/peer/a" || peers[1].PeerURA != "easynet:///r/peer/b" {
		t.Fatalf("ListPeers = %#v, %v", peers, err)
	}
}

func TestManagedSigningPageAPIsRejectUnboundedOrNonAdvancingPages(t *testing.T) {
	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: filepath.Join(t.TempDir(), "unused.sock")})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	if _, err := client.ListPage(ManagedSigningKeyFilter{}, ManagedSigningPageOptions{Limit: ManagedSigningMaxPageLimit + 1}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("oversized ListPage error = %v, want INVALID_ARGUMENT", err)
	}
	if _, err := client.ListPeersPage(ManagedSigningPageOptions{Cursor: " cursor "}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("non-canonical cursor error = %v, want INVALID_ARGUMENT", err)
	}
	if _, err := client.ListPeersPage(ManagedSigningPageOptions{Cursor: strings.Repeat("x", managedSigningMaxCursorBytes+1)}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("oversized cursor error = %v, want INVALID_ARGUMENT", err)
	}

	publicKey := ed25519.NewKeyFromSeed(bytesOf(15, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	socketPath := startRuntimeKeyServiceTestServer(t, func(request map[string]any) map[string]any {
		return map[string]any{
			"result": "inventory_keys", "entries": []any{managedSigningKeyFixture("key-1", publicKey, "active", 0, "", "")},
			"next_cursor": "cursor:1",
		}
	})
	client, err = NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	if _, err := client.ListPage(ManagedSigningKeyFilter{}, ManagedSigningPageOptions{Limit: 1, Cursor: "cursor:1"}); !IsCode(err, ErrProtocol) {
		t.Fatalf("non-advancing page error = %v, want PROTOCOL", err)
	}
}

func TestManagedSigningClientRejectsUnknownAndCustodyResponseFields(t *testing.T) {
	publicKey := ed25519.NewKeyFromSeed(bytesOf(16, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	tests := []struct {
		name   string
		mutate func(map[string]any, map[string]any)
	}{
		{
			name: "unknown top-level field",
			mutate: func(response, _ map[string]any) {
				response["debug"] = true
			},
		},
		{
			name: "unknown projection field",
			mutate: func(_, entry map[string]any) {
				entry["future_field"] = true
			},
		},
		{
			name: "nested seed field",
			mutate: func(_, entry map[string]any) {
				entry["seed_hex"] = strings.Repeat("00", ed25519.SeedSize)
			},
		},
		{
			name: "nested master key field",
			mutate: func(_, entry map[string]any) {
				entry["master_key"] = "forbidden"
			},
		},
		{
			name: "nested ciphertext field",
			mutate: func(_, entry map[string]any) {
				entry["ciphertext"] = "forbidden"
			},
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			entry := managedSigningKeyFixture("key-1", publicKey, "active", 0, "", "")
			response := map[string]any{"result": "inventory_key", "entry": entry}
			test.mutate(response, entry)
			socketPath := startRuntimeKeyServiceTestServer(t, func(map[string]any) map[string]any { return response })
			client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
			if err != nil {
				t.Fatalf("NewManagedSigningClient: %v", err)
			}
			if _, err := client.PublicProjection("key-1"); !IsCode(err, ErrProtocol) {
				t.Fatalf("PublicProjection error = %v, want PROTOCOL", err)
			}
		})
	}
}

func TestManagedSignerVerifiesRuntimeKeyServiceSignatureAgainstBoundProjection(t *testing.T) {
	privateKey := ed25519.NewKeyFromSeed(bytesOf(17, ed25519.SeedSize))
	publicKey := privateKey.Public().(ed25519.PublicKey)
	canonical := []byte("canonical managed invocation")
	requestCount := 0
	socketPath := startRuntimeKeyServiceTestServer(t, func(request map[string]any) map[string]any {
		requestCount++
		switch requestCount {
		case 1:
			entry := managedSigningKeyFixture("key-1", publicKey, "active", 0, "", "easynet:///r/acme/agent/signer")
			entry["expires_unix_ms"] = int64(1800000000000)
			return map[string]any{
				"result": "inventory_key",
				"entry":  entry,
			}
		case 2:
			if request["method"] != "inventory.sign" {
				t.Fatalf("unexpected signer request: %#v", request)
			}
			return map[string]any{
				"result":        "signature",
				"signature_b64": base64.StdEncoding.EncodeToString(ed25519.Sign(privateKey, canonical)),
			}
		default:
			t.Fatalf("unexpected signer request: %#v", request)
			return nil
		}
	})
	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	signer, err := client.Signer("key-1")
	if err != nil {
		t.Fatalf("Signer: %v", err)
	}
	authority := signer.Projection()
	if authority.KeyID != "key-1" || authority.BoundSubjectURA != "easynet:///r/acme/agent/signer" ||
		authority.SignerPolicyRef == "" || authority.ExpiresUnixMS == nil || *authority.ExpiresUnixMS != 1800000000000 {
		t.Fatalf("Projection = %#v, want complete bound authority projection", authority)
	}
	authority.PublicKey[0] ^= 0xff
	*authority.ExpiresUnixMS = 1
	authorityAgain := signer.Projection()
	if !authorityAgain.PublicKey.Equal(publicKey) || authorityAgain.ExpiresUnixMS == nil || *authorityAgain.ExpiresUnixMS != 1800000000000 {
		t.Fatal("Projection did not return a defensive authority copy")
	}
	projected, err := signer.SigningPublicKey()
	if err != nil || !projected.Equal(publicKey) {
		t.Fatalf("SigningPublicKey = %x, %v", projected, err)
	}
	projected[0] ^= 0xff
	projectedAgain, _ := signer.SigningPublicKey()
	if !projectedAgain.Equal(publicKey) {
		t.Fatal("SigningPublicKey did not return a defensive copy")
	}
	signature, err := signer.SignCanonical(canonical)
	if err != nil || !ed25519.Verify(publicKey, canonical, signature) {
		t.Fatalf("SignCanonical = %x, %v", signature, err)
	}
}

func TestManagedSigningClientSignCannotBypassSignatureVerification(t *testing.T) {
	privateKey := ed25519.NewKeyFromSeed(bytesOf(18, ed25519.SeedSize))
	publicKey := privateKey.Public().(ed25519.PublicKey)
	requestCount := 0
	socketPath := startRuntimeKeyServiceTestServer(t, func(map[string]any) map[string]any {
		requestCount++
		if requestCount == 1 {
			return map[string]any{"result": "inventory_key", "entry": managedSigningKeyFixture("key-1", publicKey, "active", 0, "", "easynet:///r/acme/agent/signer.main")}
		}
		wrongKey := ed25519.NewKeyFromSeed(bytesOf(19, ed25519.SeedSize))
		return map[string]any{
			"result": "signature", "signature_b64": base64.StdEncoding.EncodeToString(ed25519.Sign(wrongKey, []byte("canonical"))),
		}
	})
	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	if _, err := client.Sign("key-1", []byte("canonical")); !IsCode(err, ErrProtocol) {
		t.Fatalf("Sign error = %v, want PROTOCOL", err)
	}
}

func TestManagedSigningProjectionValidatesCanonicalPolicyAndPeerFingerprint(t *testing.T) {
	publicKey := ed25519.NewKeyFromSeed(bytesOf(20, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	t.Run("signer policy reference", func(t *testing.T) {
		entry := managedSigningKeyFixture("key-1", publicKey, "active", 0, "", "easynet:///r/acme/agent/signer")
		entry["signer_policy_ref"] = "provider-key-inventory:sha256:00000000000000000000000000000000"
		socketPath := startRuntimeKeyServiceTestServer(t, func(map[string]any) map[string]any {
			return map[string]any{"result": "inventory_key", "entry": entry}
		})
		client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
		if err != nil {
			t.Fatalf("NewManagedSigningClient: %v", err)
		}
		if _, err := client.PublicProjection("key-1"); !IsCode(err, ErrProtocol) {
			t.Fatalf("PublicProjection error = %v, want PROTOCOL", err)
		}
	})

	t.Run("purpose is part of signer policy", func(t *testing.T) {
		entry := managedSigningKeyFixture("key-1", publicKey, "active", 0, "", "easynet:///r/acme/agent/signer")
		entry["purpose"] = "different-purpose"
		socketPath := startRuntimeKeyServiceTestServer(t, func(map[string]any) map[string]any {
			return map[string]any{"result": "inventory_key", "entry": entry}
		})
		client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
		if err != nil {
			t.Fatalf("NewManagedSigningClient: %v", err)
		}
		if _, err := client.PublicProjection("key-1"); !IsCode(err, ErrProtocol) {
			t.Fatalf("PublicProjection error = %v, want PROTOCOL", err)
		}
	})

	t.Run("peer SHA-256 fingerprint", func(t *testing.T) {
		peer := managedSigningPeerFixture("easynet:///r/peer/a", publicKey)
		peer["fingerprint_b64"] = base64.StdEncoding.EncodeToString(bytesOf(1, sha256.Size))
		socketPath := startRuntimeKeyServiceTestServer(t, func(map[string]any) map[string]any {
			return map[string]any{"result": "inventory_peers", "peers": []any{peer}, "next_cursor": nil}
		})
		client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
		if err != nil {
			t.Fatalf("NewManagedSigningClient: %v", err)
		}
		if _, err := client.ListPeersPage(ManagedSigningPageOptions{Limit: 1}); !IsCode(err, ErrProtocol) {
			t.Fatalf("ListPeersPage error = %v, want PROTOCOL", err)
		}
	})
}

func TestManagedSigningCanonicalProjectionFixtures(t *testing.T) {
	publicKey := ed25519.NewKeyFromSeed(bytesOf(1, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	policyRef := canonicalManagedSignerPolicyRef(
		"invocation",
		"easynet:///r/acme/agent/signer",
		"managed-key-1",
		publicKey,
	)
	const expectedPolicyRef = "managed-signing:v2:sha256:e7e82ca6208b6a4ebf2369739a2c260a"
	if policyRef != expectedPolicyRef {
		t.Fatalf("canonical signer policy ref = %q, want %q", policyRef, expectedPolicyRef)
	}
	fingerprint := sha256.Sum256(publicKey)
	const expectedFingerprintBase64 = "NHUPmL1Z/PyUbaRaqr6TO+FUpLUJThxKv0KGZQXzyX4="
	if actual := base64.StdEncoding.EncodeToString(fingerprint[:]); actual != expectedFingerprintBase64 {
		t.Fatalf("canonical peer fingerprint = %q, want %q", actual, expectedFingerprintBase64)
	}
}

func TestRuntimeKeyServiceSigningFrameCoversCanonicalRuntimeMaximum(t *testing.T) {
	if runtimeKeyServiceProtocolVersion != 2 {
		t.Fatalf("runtime key-service protocol version = %d, want 2", runtimeKeyServiceProtocolVersion)
	}
	if runtimeKeyServiceMaxFrameBytes != 90*1024*1024 {
		t.Fatalf("runtime key-service frame limit = %d, want canonical 90 MiB", runtimeKeyServiceMaxFrameBytes)
	}
	emptyRequest, err := encodeRuntimeKeyServiceRequest(map[string]any{
		"method": "inventory.sign", "key_id": "key-1", "expected_purpose": "invocation",
		"subject_ura": "easynet:///r/acme/agent/signer", "signer_policy_ref": "managed-signing:v2:sha256:fixture",
		"canonical_bytes_b64": "",
	})
	if err != nil {
		t.Fatalf("encode empty signing request: %v", err)
	}
	maximumWireBytes := len(emptyRequest) + base64.StdEncoding.EncodedLen(runtimeKeyServiceMaxCanonicalSigningBytes)
	if maximumWireBytes > runtimeKeyServiceMaxFrameBytes {
		t.Fatalf("maximum canonical signing request requires %d bytes, frame limit is %d", maximumWireBytes, runtimeKeyServiceMaxFrameBytes)
	}

	client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: filepath.Join(t.TempDir(), "unused.sock")})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	oversized := make([]byte, runtimeKeyServiceMaxCanonicalSigningBytes+1)
	if _, err := client.Sign("key-1", oversized); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("oversized signing error = %v, want INVALID_ARGUMENT", err)
	}
}

func TestRuntimeKeyServiceErrorTaxonomySeparatesAvailabilityTransportAndExecution(t *testing.T) {
	t.Run("connect failure", func(t *testing.T) {
		client, err := NewManagedSigningClient(ManagedSigningClientOptions{
			SocketPath: filepath.Join(t.TempDir(), "missing.sock"),
			Timeout:    50 * time.Millisecond,
		})
		if err != nil {
			t.Fatalf("NewManagedSigningClient: %v", err)
		}
		_, err = client.PublicProjection("key-1")
		assertManagedSigningSDKError(t, err, ErrRuntimeOffline, RetrySafe, true)
	})

	t.Run("post-connect IO failure", func(t *testing.T) {
		socketPath := startClosingKeyServiceTestServer(t)
		client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath, Timeout: time.Second})
		if err != nil {
			t.Fatalf("NewManagedSigningClient: %v", err)
		}
		_, err = client.PublicProjection("key-1")
		assertManagedSigningSDKError(t, err, ErrTransport, RetrySafe, true)
	})

	t.Run("runtime key-service IO rejection", func(t *testing.T) {
		socketPath := startRuntimeKeyServiceTestServer(t, func(map[string]any) map[string]any {
			return map[string]any{"result": "error", "kind": "io", "message": "vault persistence failed"}
		})
		client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
		if err != nil {
			t.Fatalf("NewManagedSigningClient: %v", err)
		}
		_, err = client.PublicProjection("key-1")
		assertManagedSigningSDKError(t, err, ErrExecutionFailed, RetryNever, false)
	})

	t.Run("peer replacement policy rejection", func(t *testing.T) {
		socketPath := startRuntimeKeyServiceTestServer(t, func(map[string]any) map[string]any {
			return map[string]any{"result": "error", "kind": "policy", "message": "explicit retrust is required"}
		})
		client, err := NewManagedSigningClient(ManagedSigningClientOptions{SocketPath: socketPath})
		if err != nil {
			t.Fatalf("NewManagedSigningClient: %v", err)
		}
		_, err = client.AddPeer(ManagedSigningPeerRegistration{
			PeerURA:   "easynet:///r/peer/agent/verifier.main",
			PublicKey: ed25519.PublicKey(bytesOf(1, ed25519.PublicKeySize)),
		})
		assertManagedSigningSDKError(t, err, ErrPolicyDenied, RetryNever, false)
	})
}

func managedSigningKeyFixture(keyID string, publicKey ed25519.PublicKey, status string, epoch int, rotatedFrom, subjectURA string) map[string]any {
	fixture := map[string]any{
		"key_id": keyID, "purpose": "invocation",
		"public_key_b64": base64.StdEncoding.EncodeToString(publicKey), "status": status,
		"rotation_epoch": epoch, "created_unix_ms": int64(1700000000000),
		"expires_unix_ms": nil, "revoked_unix_ms": nil,
	}
	if subjectURA == "" {
		fixture["bound_subject"] = nil
		fixture["signer_policy_ref"] = nil
	} else {
		fixture["bound_subject"] = subjectURA
		fixture["signer_policy_ref"] = canonicalManagedSignerPolicyRef("invocation", subjectURA, keyID, publicKey)
	}
	if rotatedFrom == "" {
		fixture["rotated_from"] = nil
	} else {
		fixture["rotated_from"] = rotatedFrom
	}
	return fixture
}

func managedSigningPeerFixture(peerURA string, publicKey ed25519.PublicKey) map[string]any {
	fingerprint := sha256.Sum256(publicKey)
	return map[string]any{
		"peer_ura": peerURA, "fingerprint_b64": base64.StdEncoding.EncodeToString(fingerprint[:]),
		"public_key_b64": base64.StdEncoding.EncodeToString(publicKey), "via_authority": nil,
		"added_unix_ms": int64(1700000000200), "last_seen_unix_ms": int64(1700000000300),
	}
}

func assertManagedSigningSDKError(
	t *testing.T,
	err error,
	code ErrorCode,
	retry RetryHint,
	retryable bool,
) {
	t.Helper()
	var sdkError *SDKError
	if !errors.As(err, &sdkError) {
		t.Fatalf("error = %v, want SDKError", err)
	}
	if sdkError.Code != code || sdkError.Retry != retry || sdkError.Retryable != retryable {
		t.Fatalf("SDKError = %#v, want code=%s retry=%s retryable=%t", sdkError, code, retry, retryable)
	}
}

func startClosingKeyServiceTestServer(t *testing.T) string {
	t.Helper()
	directory, err := os.MkdirTemp("/tmp", "runtime-key-service-close-")
	if err != nil {
		t.Fatalf("create closing key-service directory: %v", err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(directory) })
	socketPath := filepath.Join(directory, "runtime-key-service.sock")
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: socketPath, Net: "unix"})
	if err != nil {
		t.Fatalf("listen closing key-service socket: %v", err)
	}
	t.Cleanup(func() { _ = listener.Close() })
	go func() {
		connection, err := listener.AcceptUnix()
		if err == nil {
			_ = connection.Close()
		}
	}()
	return socketPath
}

func assertManagedSigningRequest(t *testing.T, actual, expected map[string]any) {
	t.Helper()
	if len(actual) != len(expected) {
		t.Errorf("managed-signing request fields = %#v, want %#v", actual, expected)
		return
	}
	for key, want := range expected {
		if fmt.Sprint(actual[key]) != fmt.Sprint(want) {
			t.Errorf("managed-signing request[%s] = %#v, want %#v", key, actual[key], want)
		}
	}
}

func assertNoPrivateKeyRequestFields(t *testing.T, request map[string]any) {
	t.Helper()
	for field := range request {
		lower := strings.ToLower(field)
		for _, forbidden := range []string{"seed", "private", "vault", "passphrase"} {
			if strings.Contains(lower, forbidden) {
				t.Errorf("managed-signing request leaked custody field %q", field)
			}
		}
	}
}

func bytesOf(value byte, count int) []byte {
	result := make([]byte, count)
	for index := range result {
		result[index] = value
	}
	return result
}
