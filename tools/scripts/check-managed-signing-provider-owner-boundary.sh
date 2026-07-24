#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_MANAGED_SIGNING_PROVIDER_OWNER_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-managed-signing-provider-owner-boundary: %s\n' "$1" >&2
  exit 1
}

PROVIDER="src/daemon/keyring/managed_signing_provider.rs"
ABILITIES="src/daemon/keyring/abilities.rs"
CATALOG_BUILD="src/daemon/ability/catalog/build.rs"
CROSS_REALM_TEST="tests/cross_realm_user_binding_e2e.rs"

for path in "$PROVIDER" "$ABILITIES" "$CATALOG_BUILD" "$CROSS_REALM_TEST"; do
  [[ -f "$path" ]] || fail "missing $path"
done

if ! rg -n 'pub trait ManagedSigningProvider' "$PROVIDER" >/dev/null; then
  fail "managed_signing_provider.rs must own ManagedSigningProvider"
fi

if rg -n 'pub use crate::daemon::keyring::managed_signing_provider::ManagedSigningProvider' "$ABILITIES"; then
  fail "keyring ability handlers must not re-export the provider trait"
fi

if ! rg -n 'use crate::daemon::keyring::managed_signing_provider::ManagedSigningProvider;' "$ABILITIES" >/dev/null; then
  fail "keyring ability handlers must import the provider trait from its owner"
fi

if rg -n 'keyring::abilities::ManagedSigningProvider' src tests --glob '*.rs'; then
  fail "production and integration callers must use managed_signing_provider as the trait owner"
fi

if ! rg -n 'keyring::managed_signing_provider::ManagedSigningProvider' "$CATALOG_BUILD" "$CROSS_REALM_TEST" >/dev/null; then
  fail "daemon registry and cross-realm tests must depend on the provider owner path"
fi

if rg -n 're-export the full trait|provider reexport' "$PROVIDER" "$ABILITIES"; then
  fail "provider boundary comments must not preserve re-export ownership language"
fi

echo "check-managed-signing-provider-owner-boundary: ok"
