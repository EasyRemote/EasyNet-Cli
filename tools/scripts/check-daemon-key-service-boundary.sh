#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

forbidden='KeyringHandle|keyring\.json|open_default_keyring_handle|EASYNET_KEYRING_PATH|EASYNET_KEYRING_PASS([^P]|$)'

if rg -n --glob '*.rs' "$forbidden" src tests; then
  echo "daemon key-service boundary violation: legacy inventory symbol found" >&2
  exit 1
fi

# The passphrase is a key-service-process concern. Lifecycle managers,
# product runtimes, and test harnesses must never inject or forward it through
# process environment. The store module is the only allowed location should
# the historical variable name ever need an explicit rejection test.
if rg -n --glob '*.rs' --glob '!src/daemon/keyring/passphrase.rs' \
  'EASYNET_KEYRING_PASSPHRASE' src tests; then
  echo "daemon key-service boundary violation: passphrase escaped key-service custody" >&2
  exit 1
fi

if rg -n --glob '*.rs' 'KeyringRequest::Put|Put[[:space:]]*\{[^}]*seed_hex' src tests; then
  echo "daemon key-service boundary violation: seed-bearing protocol request found" >&2
  exit 1
fi

if rg -n --glob '*.rs' 'export_seed|export.*private[_ -]?key' src tests; then
  echo "daemon key-service boundary violation: private key export surface found" >&2
  exit 1
fi

if rg -n --glob '*.rs' \
  'derive_subject_keypair|derive_owner_public_key_b64|derive_hub_public_key_b64|SessionSigningSeed|hub_signing_seed|signing_seed' \
  src tests; then
  echo "daemon key-service boundary violation: legacy seed-shaped signing path found" >&2
  exit 1
fi

if rg -n --glob '*.go' --glob '!**/*_test.go' \
  'NewKeyFromSeed|ed25519\.PrivateKey|privateKeySeed|NewEd25519SignatureProvider|func New[A-Za-z0-9_]*(PrivateKey|Seed)|PrivateKey[[:space:]]+\[\]byte' \
  sdk/go; then
  echo "daemon key-service boundary violation: Go production SDK accepts private signing material" >&2
  exit 1
fi

# SDK facades must bind an explicit daemon endpoint. Product directory layout
# and environment discovery are daemon policy, never generic SDK behavior.
if rg -n --glob '*.go' --glob '!**/*_test.go' \
  'EASYNET_KEYRING_SOCKET_PATH|\.easynet.*keyring\.sock' sdk/go; then
  echo "daemon key-service boundary violation: Go SDK derives a product daemon endpoint" >&2
  exit 1
fi

# The daemon-local loopback caller is a runtime owner, not an independent
# process key. It may only hold a public projection and a key-service port.
if rg -n \
  'SigningKey|signing_key|fill_bytes\(&mut seed\)|OsRng' \
  src/daemon/identity/local_invocation.rs; then
  echo "daemon key-service boundary violation: local system caller owns private signing material" >&2
  exit 1
fi

# Private-key field names remain valid only inside fail-closed metadata
# rejection lists. Match declarations, assignments, constructors, and crypto
# materialization so those negative guards are not mistaken for custody.
if rg -n --glob '*.py' \
  'Ed25519SignatureProvider|def[[:space:]]+from_seed_|private_key_seed[[:space:]]*(:|=)|Ed25519PrivateKey\.from_private_bytes|def[[:space:]]+__init__\([^)]*(private_key|seed)' \
  sdk/python/easynet_sdk; then
  echo "daemon key-service boundary violation: Python production SDK accepts private signing material" >&2
  exit 1
fi

if rg -n --glob '*.py' --glob '!test_*.py' \
  'EASYNET_KEYRING_SOCKET_PATH|\.easynet.*keyring\.sock' sdk/python/easynet_sdk; then
  echo "daemon key-service boundary violation: Python SDK derives a product daemon endpoint" >&2
  exit 1
fi

if rg -n --glob '*.rs' 'role_overlays|role overlay|role-overlay' src; then
  echo "daemon key-service boundary violation: runtime owner key alias found" >&2
  exit 1
fi

if rg -n --glob '*.rs' \
  'pub (struct|enum) (Vault|VaultPlaintext|KeyringEntry|ManagedSigningKey|MasterKeySource)([[:space:]]|\{)' \
  src/daemon/keyring; then
  echo "daemon key-service boundary violation: private vault model is publicly reachable" >&2
  exit 1
fi

if rg -n 'register_rpc\(name\("sign"\)' src/daemon/keyring/abilities.rs; then
  echo "daemon key-service boundary violation: raw signing exposed as an Invocation ability" >&2
  exit 1
fi

if rg -n --glob '*.rs' \
  'pub enum Keyring(Request|Response)|pub fn inventory_sign\(|KeyringRequest::List|KeyringResponse::List' \
  src; then
  echo "daemon key-service boundary violation: raw or unbounded signing protocol is public" >&2
  exit 1
fi

echo "daemon key-service boundary: custody and legacy inventory gates passed"
