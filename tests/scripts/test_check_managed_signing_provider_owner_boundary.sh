#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-managed-signing-provider-owner-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" \
  "$SB/src/daemon/keyring" \
  "$SB/src/daemon/ability/catalog" \
  "$SB/tests"
cp "$SCRIPT" "$SB/tools/scripts/check-managed-signing-provider-owner-boundary.sh"

cat >"$SB/src/daemon/keyring/managed_signing_provider.rs" <<'RS'
pub trait ManagedSigningProvider {}
RS

cat >"$SB/src/daemon/keyring/abilities.rs" <<'RS'
use crate::daemon::keyring::managed_signing_provider::ManagedSigningProvider;
pub fn handle_create(provider: &dyn ManagedSigningProvider) {}
RS

cat >"$SB/src/daemon/ability/catalog/build.rs" <<'RS'
fn key_service_for_daemon() -> std::sync::Arc<dyn crate::daemon::keyring::managed_signing_provider::ManagedSigningProvider> {
    todo!()
}
RS

cat >"$SB/tests/cross_realm_user_binding_e2e.rs" <<'RS'
use easynet_cli::daemon::keyring::managed_signing_provider::ManagedSigningProvider;
RS

(
  cd "$SB"
  bash tools/scripts/check-managed-signing-provider-owner-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >"$SB/src/daemon/keyring/abilities.rs" <<'RS'
pub use crate::daemon::keyring::managed_signing_provider::ManagedSigningProvider;
pub fn handle_create(provider: &dyn ManagedSigningProvider) {}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-managed-signing-provider-owner-boundary.sh
) >/tmp/check-managed-signing-provider-owner-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "provider re-export should exit 1 (got $rc)"
grep -Fq "must not re-export the provider trait" \
  /tmp/check-managed-signing-provider-owner-boundary.out \
  || fail "re-export failure should name provider ownership"

cat >"$SB/src/daemon/keyring/abilities.rs" <<'RS'
use crate::daemon::keyring::managed_signing_provider::ManagedSigningProvider;
pub fn handle_create(provider: &dyn ManagedSigningProvider) {}
RS
cat >"$SB/src/daemon/ability/catalog/build.rs" <<'RS'
fn key_service_for_daemon() -> std::sync::Arc<dyn crate::daemon::keyring::abilities::ManagedSigningProvider> {
    todo!()
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-managed-signing-provider-owner-boundary.sh
) >/tmp/check-managed-signing-provider-owner-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "abilities-owned caller path should exit 1 (got $rc)"
grep -Fq "must use managed_signing_provider as the trait owner" \
  /tmp/check-managed-signing-provider-owner-boundary.out \
  || fail "caller failure should name managed_signing_provider owner"

echo "test_check_managed_signing_provider_owner_boundary.sh: all cases passed"
