package easynet

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"golang.org/x/crypto/argon2"
)

const (
	runtimeIdentityKDFMemoryKiB    uint32 = 64 * 1024
	runtimeIdentityKDFTimeCost     uint32 = 3
	runtimeIdentityKDFParallelism  uint8  = 4
	runtimeIdentityKDFKeyLen       uint32 = 32
	runtimeIdentityKDFSaltLen      int    = 16
	runtimeIdentityAESNonceLen     int    = 12
	runtimeIdentityEd25519SeedLen  int    = 32
	runtimeIdentityCurrentVersion  uint32 = 1
	runtimeIdentityDefaultVaultRel string = ".easynet/keyring.enc"
)

var (
	ErrRuntimeIdentityNotFound     = errors.New("runtime identity: owner URA not in keyring")
	ErrRuntimeIdentityVaultMissing = errors.New("runtime identity: keyring vault missing")
)

// RuntimeSigningIdentity is SDK-owned signing material for one runtime owner.
// It is generic runtime identity, not a product-specific backend or hub type.
type RuntimeSigningIdentity struct {
	OwnerURA   string
	MatchedURA string
	PrimaryURA string
	PrivateKey ed25519.PrivateKey
}

// RuntimeSigningIdentityRequest describes a keyring-backed signing identity
// lookup. OwnerURA may match either the primary runtime identity or an
// authorized role overlay.
type RuntimeSigningIdentityRequest struct {
	OwnerURA   string
	VaultPath  string
	Passphrase string
}

// EnsureRuntimeSigningIdentityRequest describes a runtime identity provisioning
// request. OwnerURA becomes the primary runtime identity when a new key is
// created; RoleOverlays are additional URAs authorized to sign with the same
// keypair.
type EnsureRuntimeSigningIdentityRequest struct {
	OwnerURA     string
	RoleOverlays []string
	VaultPath    string
	Passphrase   string
}

// DefaultRuntimeIdentityVaultPath returns the EasyNet-Cli SDK keyring vault
// path used by the local daemon runtime.
func DefaultRuntimeIdentityVaultPath() (string, error) {
	if p := os.Getenv("EASYNET_KEYRING_VAULT_PATH"); p != "" {
		return p, nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("user home dir: %w", err)
	}
	return filepath.Join(home, runtimeIdentityDefaultVaultRel), nil
}

// LoadRuntimeSigningIdentity resolves one owner signing identity from the
// daemon SDK keyring provider. It never creates keys and never falls back to a
// product-owned identity file.
func LoadRuntimeSigningIdentity(req RuntimeSigningIdentityRequest) (RuntimeSigningIdentity, error) {
	ownerURA, path, passphrase, err := normalizeRuntimeIdentityRequest(req.OwnerURA, req.VaultPath, req.Passphrase)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	vault, err := openRuntimeIdentityVault(path, passphrase)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	return vault.lookup(ownerURA)
}

// EnsureRuntimeSigningIdentity returns the existing runtime identity for
// OwnerURA, or creates one in the daemon SDK keyring when the owner is absent.
// It is a generic runtime provisioning primitive; product-specific concepts
// such as "backend", "hub", or "remote" do not belong here.
func EnsureRuntimeSigningIdentity(req EnsureRuntimeSigningIdentityRequest) (RuntimeSigningIdentity, error) {
	ownerURA, path, passphrase, err := normalizeRuntimeIdentityRequest(req.OwnerURA, req.VaultPath, req.Passphrase)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	vault, err := openOrInitRuntimeIdentityVault(path, passphrase)
	if err != nil {
		return RuntimeSigningIdentity{}, err
	}
	if identity, err := vault.lookup(ownerURA); err == nil {
		return identity, nil
	} else if !errors.Is(err, ErrRuntimeIdentityNotFound) {
		return RuntimeSigningIdentity{}, err
	}
	seed := make([]byte, runtimeIdentityEd25519SeedLen)
	if _, err := rand.Read(seed); err != nil {
		return RuntimeSigningIdentity{}, fmt.Errorf("generate runtime signing seed: %w", err)
	}
	entry := runtimeIdentityEntry{
		PrimarySelf:  ownerURA,
		RoleOverlays: normalizeRuntimeIdentityOverlays(req.RoleOverlays, ownerURA),
		SeedHex:      hex.EncodeToString(seed),
	}
	vault.entries = append(vault.entries, entry)
	if err := vault.seal(); err != nil {
		return RuntimeSigningIdentity{}, err
	}
	return runtimeIdentityFromEntry(&entry, entry.PrimarySelf, entry.PrimarySelf)
}

func normalizeRuntimeIdentityRequest(ownerURA string, vaultPath string, passphrase string) (string, string, string, error) {
	ownerURA = strings.TrimSpace(ownerURA)
	if ownerURA == "" {
		return "", "", "", invalidRuntimeClient("runtime signing identity owner URA is required")
	}
	if passphrase == "" {
		passphrase = os.Getenv("EASYNET_KEYRING_PASSPHRASE")
	}
	if passphrase == "" {
		return "", "", "", invalidRuntimeClient("EASYNET_KEYRING_PASSPHRASE is required to open the runtime keyring")
	}
	if vaultPath == "" {
		var err error
		vaultPath, err = DefaultRuntimeIdentityVaultPath()
		if err != nil {
			return "", "", "", err
		}
	}
	return ownerURA, vaultPath, passphrase, nil
}

func normalizeRuntimeIdentityOverlays(overlays []string, ownerURA string) []string {
	out := make([]string, 0, len(overlays))
	seen := map[string]struct{}{ownerURA: {}}
	for _, overlay := range overlays {
		overlay = strings.TrimSpace(overlay)
		if overlay == "" {
			continue
		}
		if _, ok := seen[overlay]; ok {
			continue
		}
		seen[overlay] = struct{}{}
		out = append(out, overlay)
	}
	return out
}

type runtimeIdentityVaultFile struct {
	Version            uint32 `json:"version"`
	KDFSaltB64         string `json:"kdf_salt_b64"`
	VaultNonceB64      string `json:"vault_nonce_b64"`
	VaultCiphertextB64 string `json:"vault_ciphertext_b64"`
}

type runtimeIdentityVaultPlaintext struct {
	Entries []runtimeIdentityEntry `json:"entries"`
}

type runtimeIdentityEntry struct {
	PrimarySelf  string   `json:"primary_self"`
	RoleOverlays []string `json:"role_overlays"`
	SeedHex      string   `json:"seed_hex"`
}

type runtimeIdentityVault struct {
	path      string
	salt      []byte
	masterKey []byte
	entries   []runtimeIdentityEntry
}

func openRuntimeIdentityVault(path string, passphrase string) (*runtimeIdentityVault, error) {
	return openRuntimeIdentityVaultMode(path, passphrase, false)
}

func openOrInitRuntimeIdentityVault(path string, passphrase string) (*runtimeIdentityVault, error) {
	return openRuntimeIdentityVaultMode(path, passphrase, true)
}

func openRuntimeIdentityVaultMode(path string, passphrase string, initIfMissing bool) (*runtimeIdentityVault, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			if initIfMissing {
				return initRuntimeIdentityVault(path, passphrase)
			}
			return nil, ErrRuntimeIdentityVaultMissing
		}
		return nil, fmt.Errorf("read runtime keyring: %w", err)
	}
	var file runtimeIdentityVaultFile
	if err := json.Unmarshal(raw, &file); err != nil {
		return nil, fmt.Errorf("parse runtime keyring %q: %w", path, err)
	}
	if file.Version != runtimeIdentityCurrentVersion {
		return nil, fmt.Errorf(
			"runtime keyring version %d unsupported (expected %d)",
			file.Version,
			runtimeIdentityCurrentVersion,
		)
	}
	salt, err := decodeRuntimeIdentityFixed(file.KDFSaltB64, runtimeIdentityKDFSaltLen, "kdf_salt")
	if err != nil {
		return nil, err
	}
	nonce, err := decodeRuntimeIdentityFixed(file.VaultNonceB64, runtimeIdentityAESNonceLen, "vault_nonce")
	if err != nil {
		return nil, err
	}
	ciphertext, err := base64.StdEncoding.DecodeString(file.VaultCiphertextB64)
	if err != nil {
		return nil, fmt.Errorf("base64 vault_ciphertext: %w", err)
	}
	masterKey := argon2.IDKey(
		[]byte(passphrase),
		salt,
		runtimeIdentityKDFTimeCost,
		runtimeIdentityKDFMemoryKiB,
		runtimeIdentityKDFParallelism,
		runtimeIdentityKDFKeyLen,
	)
	plaintext, err := decryptRuntimeIdentityAESGCM(masterKey, nonce, ciphertext)
	if err != nil {
		zeroRuntimeIdentityBytes(masterKey)
		return nil, fmt.Errorf("runtime keyring decrypt: %w", err)
	}
	var decoded runtimeIdentityVaultPlaintext
	if err := json.Unmarshal(plaintext, &decoded); err != nil {
		zeroRuntimeIdentityBytes(masterKey)
		return nil, fmt.Errorf("parse decrypted runtime keyring: %w", err)
	}
	return &runtimeIdentityVault{
		path:      path,
		salt:      salt,
		masterKey: masterKey,
		entries:   decoded.Entries,
	}, nil
}

func initRuntimeIdentityVault(path string, passphrase string) (*runtimeIdentityVault, error) {
	salt := make([]byte, runtimeIdentityKDFSaltLen)
	if _, err := rand.Read(salt); err != nil {
		return nil, fmt.Errorf("generate runtime keyring salt: %w", err)
	}
	masterKey := deriveRuntimeIdentityMasterKey(passphrase, salt)
	return &runtimeIdentityVault{
		path:      path,
		salt:      salt,
		masterKey: masterKey,
		entries:   nil,
	}, nil
}

func (v *runtimeIdentityVault) lookup(ownerURA string) (RuntimeSigningIdentity, error) {
	for i := range v.entries {
		entry := &v.entries[i]
		if strings.TrimSpace(entry.PrimarySelf) == ownerURA {
			return runtimeIdentityFromEntry(entry, entry.PrimarySelf, entry.PrimarySelf)
		}
		for _, overlay := range entry.RoleOverlays {
			if strings.TrimSpace(overlay) == ownerURA {
				return runtimeIdentityFromEntry(entry, overlay, entry.PrimarySelf)
			}
		}
	}
	return RuntimeSigningIdentity{}, ErrRuntimeIdentityNotFound
}

func runtimeIdentityFromEntry(
	entry *runtimeIdentityEntry,
	matchedURA string,
	primaryURA string,
) (RuntimeSigningIdentity, error) {
	seed, err := hex.DecodeString(entry.SeedHex)
	if err != nil {
		return RuntimeSigningIdentity{}, fmt.Errorf("runtime keyring seed_hex decode: %w", err)
	}
	if len(seed) != runtimeIdentityEd25519SeedLen {
		return RuntimeSigningIdentity{}, fmt.Errorf(
			"runtime keyring seed length: expected %d, got %d",
			runtimeIdentityEd25519SeedLen,
			len(seed),
		)
	}
	return RuntimeSigningIdentity{
		OwnerURA:   strings.TrimSpace(matchedURA),
		MatchedURA: strings.TrimSpace(matchedURA),
		PrimaryURA: strings.TrimSpace(primaryURA),
		PrivateKey: ed25519.NewKeyFromSeed(seed),
	}, nil
}

func (v *runtimeIdentityVault) seal() error {
	if v.path == "" {
		return invalidRuntimeClient("runtime keyring path is required")
	}
	nonce := make([]byte, runtimeIdentityAESNonceLen)
	if _, err := rand.Read(nonce); err != nil {
		return fmt.Errorf("generate runtime keyring nonce: %w", err)
	}
	plaintext, err := json.Marshal(runtimeIdentityVaultPlaintext{Entries: v.entries})
	if err != nil {
		return fmt.Errorf("encode runtime keyring plaintext: %w", err)
	}
	ciphertext, err := encryptRuntimeIdentityAESGCM(v.masterKey, nonce, plaintext)
	if err != nil {
		return fmt.Errorf("runtime keyring encrypt: %w", err)
	}
	file := runtimeIdentityVaultFile{
		Version:            runtimeIdentityCurrentVersion,
		KDFSaltB64:         base64.StdEncoding.EncodeToString(v.salt),
		VaultNonceB64:      base64.StdEncoding.EncodeToString(nonce),
		VaultCiphertextB64: base64.StdEncoding.EncodeToString(ciphertext),
	}
	raw, err := json.MarshalIndent(file, "", "  ")
	if err != nil {
		return fmt.Errorf("encode runtime keyring file: %w", err)
	}
	raw = append(raw, '\n')
	if err := os.MkdirAll(filepath.Dir(v.path), 0o700); err != nil {
		return fmt.Errorf("create runtime keyring directory: %w", err)
	}
	tmp := v.path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		return fmt.Errorf("write runtime keyring: %w", err)
	}
	if err := os.Rename(tmp, v.path); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("commit runtime keyring: %w", err)
	}
	return nil
}

func decodeRuntimeIdentityFixed(value string, size int, field string) ([]byte, error) {
	raw, err := base64.StdEncoding.DecodeString(value)
	if err != nil {
		return nil, fmt.Errorf("base64 %s: %w", field, err)
	}
	if len(raw) != size {
		return nil, fmt.Errorf("%s length: expected %d, got %d", field, size, len(raw))
	}
	return raw, nil
}

func deriveRuntimeIdentityMasterKey(passphrase string, salt []byte) []byte {
	return argon2.IDKey(
		[]byte(passphrase),
		salt,
		runtimeIdentityKDFTimeCost,
		runtimeIdentityKDFMemoryKiB,
		runtimeIdentityKDFParallelism,
		runtimeIdentityKDFKeyLen,
	)
}

func decryptRuntimeIdentityAESGCM(key []byte, nonce []byte, ciphertext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, fmt.Errorf("aes: %w", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, fmt.Errorf("gcm: %w", err)
	}
	return gcm.Open(nil, nonce, ciphertext, nil)
}

func encryptRuntimeIdentityAESGCM(key []byte, nonce []byte, plaintext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, fmt.Errorf("aes: %w", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, fmt.Errorf("gcm: %w", err)
	}
	return gcm.Seal(nil, nonce, plaintext, nil), nil
}

func zeroRuntimeIdentityBytes(value []byte) {
	for i := range value {
		value[i] = 0
	}
}
