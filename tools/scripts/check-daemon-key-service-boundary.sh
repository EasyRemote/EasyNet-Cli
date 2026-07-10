#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

forbidden='KeyringHandle|keyring\.json|open_default_keyring_handle|EASYNET_KEYRING_PATH|EASYNET_KEYRING_PASS([^P]|$)'

if rg -n --glob '*.rs' "$forbidden" src tests; then
  echo "daemon key-service boundary violation: legacy inventory symbol found" >&2
  exit 1
fi

if rg -n --glob '*.rs' 'KeyringRequest::Put|Put[[:space:]]*\{[^}]*seed_hex' src tests; then
  echo "daemon key-service boundary violation: seed-bearing protocol request found" >&2
  exit 1
fi

echo "daemon key-service boundary: legacy inventory gate passed"
