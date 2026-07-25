#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

check_root() {
  local root="${1:-$ROOT}"
  local surface="$root/src/daemon/ability/descriptors/surface.rs"
  local control="$root/src/daemon/ability/control_plane.rs"
  local errors="$root/src/daemon/ability/control_plane_error.rs"

  [[ -f "$surface" ]] || fail "missing descriptor surface source: $surface"
  [[ -f "$control" ]] || fail "missing control-plane source: $control"
  [[ -f "$errors" ]] || fail "missing control-plane error source: $errors"

  if rg -n 'pub fn from_registry_manifest\([^)]*manifest:\s*Option<|manifest:\s*Option<&[^)]*AbilityManifest' "$surface"; then
    fail "AbilityDescriptor::from_registry_manifest must require a provider-backed manifest, not Option"
  fi

  if rg -n 'missing_manifest_normalizes_to_owner_only_scope|None\s*=>\s*\{[^}]*owner_only|unwrap_or\(Visibility::Scoped\)' "$surface"; then
    fail "descriptor materialization still preserves the retired missing-manifest owner-only fallback"
  fi

  if ! rg -n 'MissingManifest\s*\{\s*ability:\s*String\s*\}' "$errors" >/dev/null; then
    fail "control-plane errors must expose a typed MissingManifest materialization failure"
  fi

  if ! rg -n 'manifest\.ok_or_else\(\|\| AbilityControlPlaneError::MissingManifest' "$control" >/dev/null; then
    fail "control-plane materialization must fail closed before descriptor construction when manifest is absent"
  fi

  if rg -n 'if manifest\.is_none\(\).*descriptor\.version|manifest_descriptor_version\(manifest\).*unwrap_or\(DEFAULT_ABILITY_DESCRIPTOR_VERSION\)' "$control"; then
    fail "control-plane materialization still carries missing-manifest descriptor-version fallback logic"
  fi

  if ! rg -n 'register_rejects_missing_manifest_before_descriptor_materialization' "$control" >/dev/null; then
    fail "control-plane manifest materialization boundary lacks a missing-manifest negative test"
  fi
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/src/daemon/ability/descriptors" "$tmp/src/daemon/ability"
  cat >"$tmp/src/daemon/ability/descriptors/surface.rs" <<'RS'
pub fn from_registry_manifest(
    registry_ability: impl Into<String>,
    owner_ura: impl Into<String>,
    manifest: &crate::daemon::ability::manifest::AbilityManifest,
) -> Result<(), ()> {
    let _ = (registry_ability, owner_ura, manifest);
    Ok(())
}
RS
  cat >"$tmp/src/daemon/ability/control_plane.rs" <<'RS'
fn materialize(manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>) -> Result<(), AbilityControlPlaneError> {
    let _manifest = manifest.ok_or_else(|| AbilityControlPlaneError::MissingManifest {
        ability: "demo.echo".to_string(),
    })?;
    Ok(())
}

#[test]
fn register_rejects_missing_manifest_before_descriptor_materialization() {}
RS
  cat >"$tmp/src/daemon/ability/control_plane_error.rs" <<'RS'
enum AbilityControlPlaneError {
    MissingManifest { ability: String },
}
RS
  check_root "$tmp"

  perl -0pi -e 's/manifest: &crate::daemon::ability::manifest::AbilityManifest/manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>/' \
    "$tmp/src/daemon/ability/descriptors/surface.rs"
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected optional descriptor manifest to fail"
  fi
  perl -0pi -e 's/manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>/manifest: &crate::daemon::ability::manifest::AbilityManifest/' \
    "$tmp/src/daemon/ability/descriptors/surface.rs"

  cat >>"$tmp/src/daemon/ability/descriptors/surface.rs" <<'RS'
fn retired_missing_manifest_path() {
    let _visibility = maybe_manifest.map(|_| Visibility::Public).unwrap_or(Visibility::Scoped);
}
RS
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected missing-manifest fallback vocabulary to fail"
  fi
  echo "check-control-plane-manifest-materialization-boundary self-test: ok"
  exit 0
fi

check_root "$ROOT"
echo "check-control-plane-manifest-materialization-boundary: ok"
