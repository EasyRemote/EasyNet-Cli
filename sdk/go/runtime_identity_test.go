package easynet

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"io"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestLoadRuntimeSigningIdentityUsesDaemonKeyringProjection(t *testing.T) {
	publicKey := ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	socketPath := startRuntimeKeyringTestServer(t, func(request map[string]any) map[string]any {
		if request["method"] != "derive_pubkey" || request["self_ura"] != "easynet:///r/acme/hub" {
			t.Fatalf("unexpected request: %#v", request)
		}
		if _, containsVaultField := request["vault_path"]; containsVaultField {
			t.Fatal("SDK must not send a vault path to the daemon keyring")
		}
		return map[string]any{
			"result":         "public_key",
			"public_key_b64": base64.StdEncoding.EncodeToString(publicKey),
		}
	})

	identity, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{
		OwnerURA:   "easynet:///r/acme/hub",
		SocketPath: socketPath,
	})
	if err != nil {
		t.Fatalf("LoadRuntimeSigningIdentity: %v", err)
	}
	if !identity.PublicKey.Equal(publicKey) {
		t.Fatal("identity public key did not match daemon projection")
	}
}

func TestRuntimeSigningIdentitySignsThroughDaemonKeyring(t *testing.T) {
	publicKey := ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	message := []byte("canonical invocation bytes")
	signature := ed25519.Sign(ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)), message)
	requests := 0
	socketPath := startRuntimeKeyringTestServer(t, func(request map[string]any) map[string]any {
		requests++
		switch requests {
		case 1:
			return map[string]any{
				"result":         "public_key",
				"public_key_b64": base64.StdEncoding.EncodeToString(publicKey),
			}
		case 2:
			if request["method"] != "sign" || request["self_ura"] != "easynet:///r/acme/hub" {
				t.Fatalf("unexpected signing request: %#v", request)
			}
			if request["canonical_bytes_b64"] != base64.StdEncoding.EncodeToString(message) {
				t.Fatalf("canonical bytes were not forwarded exactly: %#v", request)
			}
			return map[string]any{
				"result":        "signature",
				"signature_b64": base64.StdEncoding.EncodeToString(signature),
			}
		default:
			t.Fatalf("unexpected extra keyring request: %#v", request)
			return nil
		}
	})
	identity, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{OwnerURA: "easynet:///r/acme/hub", SocketPath: socketPath})
	if err != nil {
		t.Fatalf("LoadRuntimeSigningIdentity: %v", err)
	}
	actual, err := identity.Sign(message)
	if err != nil {
		t.Fatalf("identity.Sign: %v", err)
	}
	if string(actual) != string(signature) {
		t.Fatal("signature did not match daemon response")
	}
}

func TestEnsureRuntimeSigningIdentityDelegatesKeyGeneration(t *testing.T) {
	publicKey := ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	socketPath := startRuntimeKeyringTestServer(t, func(request map[string]any) map[string]any {
		if request["method"] != "ensure" || request["primary_self"] != "easynet:///r/acme/hub" {
			t.Fatalf("unexpected ensure request: %#v", request)
		}
		if _, containsSeed := request["seed_hex"]; containsSeed {
			t.Fatal("SDK must not generate or transmit key seeds")
		}
		return map[string]any{
			"result":         "public_key",
			"public_key_b64": base64.StdEncoding.EncodeToString(publicKey),
		}
	})
	identity, err := EnsureRuntimeSigningIdentity(EnsureRuntimeSigningIdentityRequest{
		OwnerURA:     "easynet:///r/acme/hub",
		RoleOverlays: []string{"easynet:///r/acme/device/node-a"},
		SocketPath:   socketPath,
	})
	if err != nil {
		t.Fatalf("EnsureRuntimeSigningIdentity: %v", err)
	}
	if !identity.PublicKey.Equal(publicKey) {
		t.Fatal("ensure returned the wrong public key")
	}
}

func startRuntimeKeyringTestServer(t *testing.T, handle func(map[string]any) map[string]any) string {
	t.Helper()
	dir, err := os.MkdirTemp("/tmp", "easynet-keyring-")
	if err != nil {
		t.Fatalf("create keyring test directory: %v", err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	socketPath := filepath.Join(dir, "keyring.sock")
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: socketPath, Net: "unix"})
	if err != nil {
		t.Fatalf("listen keyring test socket: %v", err)
	}
	t.Cleanup(func() { _ = listener.Close() })
	go func() {
		for {
			connection, err := listener.AcceptUnix()
			if err != nil {
				return
			}
			func() {
				defer connection.Close()
				_ = connection.SetDeadline(time.Now().Add(5 * time.Second))
				var length [4]byte
				if _, err := io.ReadFull(connection, length[:]); err != nil {
					return
				}
				body := make([]byte, binary.BigEndian.Uint32(length[:]))
				if _, err := io.ReadFull(connection, body); err != nil {
					return
				}
				var request map[string]any
				if err := json.Unmarshal(body, &request); err != nil {
					t.Errorf("decode keyring request: %v", err)
					return
				}
				response, err := json.Marshal(handle(request))
				if err != nil {
					t.Errorf("encode keyring response: %v", err)
					return
				}
				binary.BigEndian.PutUint32(length[:], uint32(len(response)))
				_, _ = connection.Write(length[:])
				_, _ = connection.Write(response)
			}()
		}
	}()
	return socketPath
}
