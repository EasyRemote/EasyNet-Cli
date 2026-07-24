package easynet

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strings"
	"time"
)

func identitySignerPolicyRef(ownerURA string, keyID string, publicKeyBase64 string) string {
	hasher := sha256.New()
	hasher.Write([]byte(ownerURA))
	hasher.Write([]byte{0})
	hasher.Write([]byte(keyID))
	hasher.Write([]byte{0})
	hasher.Write([]byte(publicKeyBase64))
	return "provider-key-inventory:sha256:" + hex.EncodeToString(hasher.Sum(nil)[:16])
}

// RuntimeSigningIdentity is an opaque provider-owned signing capability. It
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

// Sign attaches an Ed25519 signature to canonical bytes through the provider
// signer. Canonicalization remains the caller's domain; key custody does not.
func (i RuntimeSigningIdentity) Sign(canonicalBytes []byte) ([]byte, error) {
	if i.signer == nil {
		return nil, invalidRuntimeClient("runtime signing identity is not initialized")
	}
	signature, err := i.signer.sign(i.OwnerURA, i.PublicKey, canonicalBytes)
	if err != nil {
		return nil, runtimeIdentityError(err)
	}
	return signature, nil
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

// RuntimeSigningIdentityRequest resolves an existing provider-owned identity.
// VaultPath and Passphrase are intentionally not part of this API: the SDK
// never opens the vault and the provider owns its lifecycle.
type RuntimeSigningIdentityRequest struct {
	OwnerURA   string
	SocketPath string
	Timeout    time.Duration
}

// EnsureRuntimeSigningIdentityRequest provisions an identity in the provider
// key service. The provider generates the key and returns only its public projection.
type EnsureRuntimeSigningIdentityRequest struct {
	OwnerURA   string
	SocketPath string
	Timeout    time.Duration
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
		return RuntimeSigningIdentity{}, runtimeIdentityError(err)
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
	publicKey, err := signer.ensure(owner)
	if err != nil {
		return RuntimeSigningIdentity{}, runtimeIdentityError(err)
	}
	return RuntimeSigningIdentity{OwnerURA: owner, PublicKey: publicKey, signer: signer}, nil
}

type runtimeKeyringSigner interface {
	sign(ownerURA string, publicKey ed25519.PublicKey, canonicalBytes []byte) ([]byte, error)
	publicKey(ownerURA string) (ed25519.PublicKey, error)
	ensure(ownerURA string) (ed25519.PublicKey, error)
}

type runtimeKeyringClient struct {
	service daemonKeyServiceClient
}

func newRuntimeKeyringSigner(ownerURA, socketPath string, timeout time.Duration) (string, runtimeKeyringSigner, error) {
	ownerURA = strings.TrimSpace(ownerURA)
	if ownerURA == "" {
		return "", nil, invalidRuntimeClient("runtime signing identity owner URA is required")
	}
	service, err := newDaemonKeyServiceClient(socketPath, timeout)
	if err != nil {
		return "", nil, err
	}
	return ownerURA, runtimeKeyringClient{service: service}, nil
}

func runtimeIdentityError(err error) error {
	if err == nil {
		return nil
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) {
		return err
	}
	code := sdkErr.Code
	if code == ErrNotFound {
		code = ErrCallerSignerUnavailable
	}
	return &SDKError{
		Code:         code,
		Stage:        "runtime_identity",
		Retry:        sdkErr.Retry,
		Retryable:    sdkErr.Retryable,
		Message:      sdkErr.Message,
		Source:       sdkErr.Source,
		InvocationID: sdkErr.InvocationID,
		ReceiptURA:   sdkErr.ReceiptURA,
		Details:      sdkErr.Details,
		Cause:        err,
	}
}

func (c runtimeKeyringClient) sign(ownerURA string, publicKey ed25519.PublicKey, canonicalBytes []byte) ([]byte, error) {
	if len(canonicalBytes) == 0 {
		return nil, invalidRuntimeClient("canonical bytes are required for runtime signing")
	}
	if len(canonicalBytes) > daemonKeyServiceMaxCanonicalSigningBytes {
		return nil, invalidRuntimeClient("canonical bytes exceed the 64 MiB runtime signing limit")
	}
	if len(publicKey) != ed25519.PublicKeySize {
		return nil, invalidRuntimeClient("runtime signing identity has no valid public projection")
	}
	publicKeyBase64 := base64.StdEncoding.EncodeToString(publicKey)
	response, err := c.service.call(map[string]any{
		"method":              "sign",
		"self_ura":            ownerURA,
		"public_key_b64":      publicKeyBase64,
		"signer_policy_ref":   identitySignerPolicyRef(ownerURA, ownerURA, publicKeyBase64),
		"canonical_bytes_b64": base64.StdEncoding.EncodeToString(canonicalBytes),
	})
	if err != nil {
		return nil, err
	}
	if err := requireDaemonKeyServiceResult(response, "signature", "signature_b64"); err != nil {
		return nil, err
	}
	encoded, err := daemonKeyServiceResponseString(response, "signature_b64")
	if err != nil {
		return nil, err
	}
	signature, err := decodeCanonicalDaemonKeyServiceBase64(encoded, ed25519.SignatureSize, "Ed25519 signature")
	if err != nil {
		return nil, invalidDaemonKeyServicePayload("provider key service returned an invalid Ed25519 signature", err)
	}
	if !ed25519.Verify(publicKey, canonicalBytes, signature) {
		return nil, invalidDaemonKeyServicePayload("provider key service returned a signature that does not verify against the bound runtime identity", nil)
	}
	return signature, nil
}

func (c runtimeKeyringClient) publicKey(ownerURA string) (ed25519.PublicKey, error) {
	response, err := c.service.call(map[string]any{"method": "derive_pubkey", "self_ura": ownerURA})
	if err != nil {
		return nil, err
	}
	return runtimeKeyringPublicKey(response)
}

func (c runtimeKeyringClient) ensure(ownerURA string) (ed25519.PublicKey, error) {
	response, err := c.service.call(map[string]any{
		"method":       "ensure",
		"primary_self": ownerURA,
	})
	if err != nil {
		return nil, err
	}
	return runtimeKeyringPublicKey(response)
}

func runtimeKeyringPublicKey(response map[string]json.RawMessage) (ed25519.PublicKey, error) {
	if err := requireDaemonKeyServiceResult(response, "public_key", "public_key_b64"); err != nil {
		return nil, err
	}
	encoded, err := daemonKeyServiceResponseString(response, "public_key_b64")
	if err != nil {
		return nil, err
	}
	publicKey, err := decodeCanonicalDaemonKeyServiceBase64(encoded, ed25519.PublicKeySize, "Ed25519 public key")
	if err != nil {
		return nil, invalidDaemonKeyServicePayload("provider key service returned an invalid Ed25519 public key", err)
	}
	return ed25519.PublicKey(publicKey), nil
}
