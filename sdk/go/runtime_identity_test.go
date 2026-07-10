package easynet

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"golang.org/x/crypto/argon2"
)

func TestLoadRuntimeSigningIdentityPrimary(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "keyring.enc")
	seed := runtimeIdentitySeedForTest(t)
	ownerURA := "easynet:///r/acme/device/dev-a"
	sealRuntimeIdentityVaultForTest(t, path, "passphrase", []runtimeIdentityEntry{{
		PrimarySelf: ownerURA,
		SeedHex:     hex.EncodeToString(seed),
	}})

	identity, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{
		OwnerURA:   ownerURA,
		VaultPath:  path,
		Passphrase: "passphrase",
	})
	if err != nil {
		t.Fatalf("LoadRuntimeSigningIdentity: %v", err)
	}
	if identity.OwnerURA != ownerURA || identity.PrimaryURA != ownerURA || identity.MatchedURA != ownerURA {
		t.Fatalf("identity URAs = %#v", identity)
	}
	wantPublic := ed25519.NewKeyFromSeed(seed).Public().(ed25519.PublicKey)
	gotPublic := identity.PrivateKey.Public().(ed25519.PublicKey)
	if !gotPublic.Equal(wantPublic) {
		t.Fatal("runtime signing identity public key mismatch")
	}
}

func TestLoadRuntimeSigningIdentityRoleOverlay(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "keyring.enc")
	seed := runtimeIdentitySeedForTest(t)
	primaryURA := "easynet:///r/acme/device/dev-a"
	hubURA := "easynet:///r/acme/hub"
	sealRuntimeIdentityVaultForTest(t, path, "passphrase", []runtimeIdentityEntry{{
		PrimarySelf:  primaryURA,
		RoleOverlays: []string{hubURA},
		SeedHex:      hex.EncodeToString(seed),
	}})

	primary, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{
		OwnerURA:   primaryURA,
		VaultPath:  path,
		Passphrase: "passphrase",
	})
	if err != nil {
		t.Fatalf("LoadRuntimeSigningIdentity primary: %v", err)
	}
	overlay, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{
		OwnerURA:   hubURA,
		VaultPath:  path,
		Passphrase: "passphrase",
	})
	if err != nil {
		t.Fatalf("LoadRuntimeSigningIdentity overlay: %v", err)
	}
	if overlay.PrimaryURA != primaryURA || overlay.MatchedURA != hubURA {
		t.Fatalf("overlay identity = %#v", overlay)
	}
	message := []byte("canonical authority bytes")
	if string(ed25519.Sign(primary.PrivateKey, message)) != string(ed25519.Sign(overlay.PrivateKey, message)) {
		t.Fatal("role overlay must sign with the same keypair as primary")
	}
}

func TestLoadRuntimeSigningIdentityMissingOwnerFailsClosed(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "keyring.enc")
	seed := runtimeIdentitySeedForTest(t)
	sealRuntimeIdentityVaultForTest(t, path, "passphrase", []runtimeIdentityEntry{{
		PrimarySelf: "easynet:///r/acme/device/dev-a",
		SeedHex:     hex.EncodeToString(seed),
	}})

	_, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{
		OwnerURA:   "easynet:///r/acme/hub",
		VaultPath:  path,
		Passphrase: "passphrase",
	})
	if !errors.Is(err, ErrRuntimeIdentityNotFound) {
		t.Fatalf("error = %v, want ErrRuntimeIdentityNotFound", err)
	}
}

func TestLoadRuntimeSigningIdentityMissingVaultFailsClosed(t *testing.T) {
	_, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{
		OwnerURA:   "easynet:///r/acme/hub",
		VaultPath:  filepath.Join(t.TempDir(), "missing.enc"),
		Passphrase: "passphrase",
	})
	if !errors.Is(err, ErrRuntimeIdentityVaultMissing) {
		t.Fatalf("error = %v, want ErrRuntimeIdentityVaultMissing", err)
	}
}

func TestEnsureRuntimeSigningIdentityCreatesLoadableVault(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "keyring.enc")
	ownerURA := "easynet:///r/acme/hub"

	created, err := EnsureRuntimeSigningIdentity(EnsureRuntimeSigningIdentityRequest{
		OwnerURA:   ownerURA,
		VaultPath:  path,
		Passphrase: "passphrase",
	})
	if err != nil {
		t.Fatalf("EnsureRuntimeSigningIdentity: %v", err)
	}
	if created.OwnerURA != ownerURA || created.PrimaryURA != ownerURA {
		t.Fatalf("created identity = %#v", created)
	}
	loaded, err := LoadRuntimeSigningIdentity(RuntimeSigningIdentityRequest{
		OwnerURA:   ownerURA,
		VaultPath:  path,
		Passphrase: "passphrase",
	})
	if err != nil {
		t.Fatalf("LoadRuntimeSigningIdentity: %v", err)
	}
	message := []byte("canonical runtime bytes")
	if !bytes.Equal(ed25519.Sign(created.PrivateKey, message), ed25519.Sign(loaded.PrivateKey, message)) {
		t.Fatal("ensured runtime identity did not persist")
	}
}

func TestEnsureRuntimeSigningIdentityReturnsExistingOverlay(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "keyring.enc")
	seed := runtimeIdentitySeedForTest(t)
	primaryURA := "easynet:///r/acme/device/dev-a"
	hubURA := "easynet:///r/acme/hub"
	sealRuntimeIdentityVaultForTest(t, path, "passphrase", []runtimeIdentityEntry{{
		PrimarySelf:  primaryURA,
		RoleOverlays: []string{hubURA},
		SeedHex:      hex.EncodeToString(seed),
	}})

	identity, err := EnsureRuntimeSigningIdentity(EnsureRuntimeSigningIdentityRequest{
		OwnerURA:   hubURA,
		VaultPath:  path,
		Passphrase: "passphrase",
	})
	if err != nil {
		t.Fatalf("EnsureRuntimeSigningIdentity: %v", err)
	}
	if identity.PrimaryURA != primaryURA || identity.MatchedURA != hubURA {
		t.Fatalf("identity = %#v", identity)
	}
	wantPublic := ed25519.NewKeyFromSeed(seed).Public().(ed25519.PublicKey)
	gotPublic := identity.PrivateKey.Public().(ed25519.PublicKey)
	if !gotPublic.Equal(wantPublic) {
		t.Fatal("ensure must return existing overlay keypair")
	}
}

func runtimeIdentitySeedForTest(t *testing.T) []byte {
	t.Helper()
	seed := make([]byte, runtimeIdentityEd25519SeedLen)
	if _, err := rand.Read(seed); err != nil {
		t.Fatalf("seed: %v", err)
	}
	return seed
}

func sealRuntimeIdentityVaultForTest(
	t *testing.T,
	path string,
	passphrase string,
	entries []runtimeIdentityEntry,
) {
	t.Helper()
	salt := make([]byte, runtimeIdentityKDFSaltLen)
	if _, err := rand.Read(salt); err != nil {
		t.Fatalf("salt: %v", err)
	}
	nonce := make([]byte, runtimeIdentityAESNonceLen)
	if _, err := rand.Read(nonce); err != nil {
		t.Fatalf("nonce: %v", err)
	}
	masterKey := argon2.IDKey(
		[]byte(passphrase),
		salt,
		runtimeIdentityKDFTimeCost,
		runtimeIdentityKDFMemoryKiB,
		runtimeIdentityKDFParallelism,
		runtimeIdentityKDFKeyLen,
	)
	plaintext, err := json.Marshal(runtimeIdentityVaultPlaintext{Entries: entries})
	if err != nil {
		t.Fatalf("marshal plaintext: %v", err)
	}
	block, err := aes.NewCipher(masterKey)
	if err != nil {
		t.Fatalf("aes: %v", err)
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		t.Fatalf("gcm: %v", err)
	}
	file := runtimeIdentityVaultFile{
		Version:            runtimeIdentityCurrentVersion,
		KDFSaltB64:         base64.StdEncoding.EncodeToString(salt),
		VaultNonceB64:      base64.StdEncoding.EncodeToString(nonce),
		VaultCiphertextB64: base64.StdEncoding.EncodeToString(gcm.Seal(nil, nonce, plaintext, nil)),
	}
	raw, err := json.MarshalIndent(file, "", "  ")
	if err != nil {
		t.Fatalf("marshal file: %v", err)
	}
	raw = append(raw, '\n')
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatalf("write vault: %v", err)
	}
}
