package easynet

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const runtimeKeyringMaxFrameBytes = 64 * 1024

var (
	ErrRuntimeIdentityNotFound    = errors.New("runtime identity: owner URA not in daemon keyring")
	ErrRuntimeIdentityUnavailable = errors.New("runtime identity: daemon keyring unavailable")
)

// RuntimeSigningIdentity is an opaque daemon-owned signing capability. It
// intentionally exposes the public key and signing operation, never seed or
// private-key bytes.
type RuntimeSigningIdentity struct {
	OwnerURA  string
	PublicKey ed25519.PublicKey
	signer    runtimeKeyringSigner
}

// CanonicalSigner is a narrow signing capability for canonical payloads. It
// is shared by invocation and authority projections so facades never need an
// Ed25519 private key merely to attach a signature.
type CanonicalSigner interface {
	SignCanonical(canonicalBytes []byte) ([]byte, error)
	SigningPublicKey() (ed25519.PublicKey, error)
}

// Sign attaches an Ed25519 signature to canonical bytes through the daemon
// keyring. Canonicalization remains the caller's domain; key custody does not.
func (i RuntimeSigningIdentity) Sign(canonicalBytes []byte) ([]byte, error) {
	if i.signer == nil {
		return nil, invalidRuntimeClient("runtime signing identity is not initialized")
	}
	return i.signer.sign(i.OwnerURA, canonicalBytes)
}

func (i RuntimeSigningIdentity) SignCanonical(canonicalBytes []byte) ([]byte, error) {
	return i.Sign(canonicalBytes)
}

func (i RuntimeSigningIdentity) SigningPublicKey() (ed25519.PublicKey, error) {
	if len(i.PublicKey) != ed25519.PublicKeySize {
		return nil, invalidRuntimeClient("runtime signing identity has no valid public key")
	}
	return append(ed25519.PublicKey(nil), i.PublicKey...), nil
}

// RuntimeSigningIdentityRequest resolves an existing daemon-owned identity.
// VaultPath and Passphrase are intentionally not part of this API: the SDK
// never opens the vault and the keyring daemon owns its lifecycle.
type RuntimeSigningIdentityRequest struct {
	OwnerURA   string
	SocketPath string
	Timeout    time.Duration
}

// EnsureRuntimeSigningIdentityRequest provisions an identity in the daemon
// keyring. The daemon generates the key and returns only its public projection.
type EnsureRuntimeSigningIdentityRequest struct {
	OwnerURA     string
	RoleOverlays []string
	SocketPath   string
	Timeout      time.Duration
}

// DefaultRuntimeIdentitySocketPath resolves the canonical daemon keyring UDS
// endpoint. It is an endpoint locator, not a vault-file fallback.
func DefaultRuntimeIdentitySocketPath() (string, error) {
	if path := strings.TrimSpace(os.Getenv("EASYNET_KEYRING_SOCKET_PATH")); path != "" {
		return path, nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve user home for keyring socket: %w", err)
	}
	return filepath.Join(home, ".easynet", "keyring.sock"), nil
}

// LoadRuntimeSigningIdentity resolves an existing identity without reading a
// keyring file or materializing private-key bytes in the facade process.
func LoadRuntimeSigningIdentity(req RuntimeSigningIdentityRequest) (RuntimeSigningIdentity, error) {
	owner, signer, err := newRuntimeKeyringSigner(req.OwnerURA, req.SocketPath, req.Timeout)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	publicKey, err := signer.publicKey(owner)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	return RuntimeSigningIdentity{OwnerURA: owner, PublicKey: publicKey, signer: signer}, nil
}

// EnsureRuntimeSigningIdentity asks the daemon keyring to provision the owner
// when absent. The facade never participates in seed generation or storage.
func EnsureRuntimeSigningIdentity(req EnsureRuntimeSigningIdentityRequest) (RuntimeSigningIdentity, error) {
	owner, signer, err := newRuntimeKeyringSigner(req.OwnerURA, req.SocketPath, req.Timeout)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	publicKey, err := signer.ensure(owner, req.RoleOverlays)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	return RuntimeSigningIdentity{OwnerURA: owner, PublicKey: publicKey, signer: signer}, nil
}

type runtimeKeyringSigner interface {
	sign(ownerURA string, canonicalBytes []byte) ([]byte, error)
	publicKey(ownerURA string) (ed25519.PublicKey, error)
	ensure(ownerURA string, roleOverlays []string) (ed25519.PublicKey, error)
}

type runtimeKeyringClient struct {
	socketPath string
	timeout    time.Duration
}

func newRuntimeKeyringSigner(ownerURA, socketPath string, timeout time.Duration) (string, runtimeKeyringSigner, error) {
	ownerURA = strings.TrimSpace(ownerURA)
	if ownerURA == "" {
		return "", nil, invalidRuntimeClient("runtime signing identity owner URA is required")
	}
	if socketPath == "" {
		var err error
		socketPath, err = DefaultRuntimeIdentitySocketPath()
		if err != nil {
			return "", nil, err
		}
	}
	if timeout <= 0 {
		timeout = 10 * time.Second
	}
	return ownerURA, runtimeKeyringClient{socketPath: socketPath, timeout: timeout}, nil
}

func (c runtimeKeyringClient) sign(ownerURA string, canonicalBytes []byte) ([]byte, error) {
	if len(canonicalBytes) == 0 {
		return nil, invalidRuntimeClient("canonical bytes are required for runtime signing")
	}
	response, err := c.call(map[string]any{
		"method":              "sign",
		"self_ura":            ownerURA,
		"canonical_bytes_b64": base64.StdEncoding.EncodeToString(canonicalBytes),
	})
	if err != nil {
		return nil, err
	}
	encoded, err := runtimeKeyringResponseString(response, "signature_b64")
	if err != nil {
		return nil, err
	}
	signature, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(signature) != ed25519.SignatureSize {
		return nil, invalidRuntimePayload("daemon keyring returned an invalid Ed25519 signature", err)
	}
	return signature, nil
}

func (c runtimeKeyringClient) publicKey(ownerURA string) (ed25519.PublicKey, error) {
	response, err := c.call(map[string]any{"method": "derive_pubkey", "self_ura": ownerURA})
	if err != nil {
		return nil, err
	}
	return runtimeKeyringPublicKey(response)
}

func (c runtimeKeyringClient) ensure(ownerURA string, roleOverlays []string) (ed25519.PublicKey, error) {
	response, err := c.call(map[string]any{
		"method":        "ensure",
		"primary_self":  ownerURA,
		"role_overlays": normalizeRuntimeIdentityOverlays(roleOverlays, ownerURA),
	})
	if err != nil {
		return nil, err
	}
	return runtimeKeyringPublicKey(response)
}

func (c runtimeKeyringClient) call(request map[string]any) (map[string]json.RawMessage, error) {
	encoded, err := json.Marshal(request)
	if err != nil {
		return nil, invalidRuntimeClient(fmt.Sprintf("encode daemon keyring request: %v", err))
	}
	if len(encoded) > runtimeKeyringMaxFrameBytes {
		return nil, invalidRuntimeClient("daemon keyring request exceeds frame limit")
	}
	connection, err := net.DialTimeout("unix", c.socketPath, c.timeout)
	if err != nil {
		return nil, fmt.Errorf("%w at %s: %v", ErrRuntimeIdentityUnavailable, c.socketPath, err)
	}
	defer connection.Close()
	if err := connection.SetDeadline(time.Now().Add(c.timeout)); err != nil {
		return nil, fmt.Errorf("set daemon keyring deadline: %w", err)
	}
	var length [4]byte
	binary.BigEndian.PutUint32(length[:], uint32(len(encoded)))
	if _, err := connection.Write(length[:]); err != nil {
		return nil, fmt.Errorf("write daemon keyring frame length: %w", err)
	}
	if _, err := connection.Write(encoded); err != nil {
		return nil, fmt.Errorf("write daemon keyring frame: %w", err)
	}
	if _, err := io.ReadFull(connection, length[:]); err != nil {
		return nil, fmt.Errorf("read daemon keyring frame length: %w", err)
	}
	responseLen := binary.BigEndian.Uint32(length[:])
	if responseLen > runtimeKeyringMaxFrameBytes {
		return nil, invalidRuntimePayload("daemon keyring response exceeds frame limit", nil)
	}
	response := make([]byte, responseLen)
	if _, err := io.ReadFull(connection, response); err != nil {
		return nil, fmt.Errorf("read daemon keyring frame: %w", err)
	}
	var decoded map[string]json.RawMessage
	if err := json.Unmarshal(response, &decoded); err != nil {
		return nil, invalidRuntimePayload("decode daemon keyring response", err)
	}
	if result, _ := runtimeKeyringResponseString(decoded, "result"); result == "error" {
		kind, _ := runtimeKeyringResponseString(decoded, "kind")
		message, _ := runtimeKeyringResponseString(decoded, "message")
		if kind == "not_found" {
			return nil, fmt.Errorf("%w: %s", ErrRuntimeIdentityNotFound, message)
		}
		return nil, fmt.Errorf("daemon keyring rejected request (%s): %s", kind, message)
	}
	return decoded, nil
}

func runtimeKeyringPublicKey(response map[string]json.RawMessage) (ed25519.PublicKey, error) {
	encoded, err := runtimeKeyringResponseString(response, "public_key_b64")
	if err != nil {
		return nil, err
	}
	publicKey, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(publicKey) != ed25519.PublicKeySize {
		return nil, invalidRuntimePayload("daemon keyring returned an invalid Ed25519 public key", err)
	}
	return ed25519.PublicKey(publicKey), nil
}

func runtimeKeyringResponseString(response map[string]json.RawMessage, field string) (string, error) {
	raw, ok := response[field]
	if !ok {
		return "", invalidRuntimePayload(fmt.Sprintf("daemon keyring response missing %s", field), nil)
	}
	var value string
	if err := json.Unmarshal(raw, &value); err != nil {
		return "", invalidRuntimePayload(fmt.Sprintf("daemon keyring response field %s is not a string", field), err)
	}
	return value, nil
}

func normalizeRuntimeIdentityOverlays(overlays []string, ownerURA string) []string {
	seen := map[string]struct{}{ownerURA: {}}
	out := make([]string, 0, len(overlays))
	for _, overlay := range overlays {
		overlay = strings.TrimSpace(overlay)
		if overlay == "" {
			continue
		}
		if _, exists := seen[overlay]; exists {
			continue
		}
		seen[overlay] = struct{}{}
		out = append(out, overlay)
	}
	return out
}
