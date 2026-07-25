#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

check_root() {
  local root="${1:-$ROOT}"
  local dispatch="$root/src/daemon/ability/dispatch.rs"
  local meta="$root/src/daemon/ability/builtins/governance/meta.rs"
  [[ -f "$dispatch" ]] || fail "missing static registration source: $dispatch"
  [[ -f "$meta" ]] || fail "missing meta.list_abilities source: $meta"

  if ! rg -n 'matches!\(owner, OwnerKind::Agent\(_\)\)' "$dispatch" >/dev/null; then
    fail "agent-owned static registration must reject missing explicit manifests"
  fi

  if ! rg -n 'requires an explicit manifest' "$dispatch" >/dev/null; then
    fail "agent-owned static registration rejection must name the missing explicit manifest"
  fi

  if ! rg -n 'fallback metadata' "$dispatch" "$meta" >/dev/null; then
    fail "agent-owned static registration gate must reject fallback metadata publication"
  fi

  if rg -n 'agent-owned fallback ability must surface|legacy_desc|assert_eq!\(legacy\["schema_summary"\]\["input"\]\["type"\], "object"\)' "$meta"; then
    fail "meta.list_abilities must not preserve agent-owned fallback descriptor expectations"
  fi

  if ! rg -n 'agent_owned_static_registration_rejects_fallback_manifest_publication' "$meta" >/dev/null; then
    fail "meta.list_abilities lacks an agent-owned fallback-manifest negative test"
  fi

  python3 - "$root/src/daemon" <<'PY'
import sys
from pathlib import Path

root = Path(sys.argv[1])
violations = []
for path in root.rglob("*.rs"):
    text = path.read_text(encoding="utf-8")
    search_from = 0
    while True:
        start = text.find("register_rpc_with_owner_and_action(", search_from)
        if start < 0:
            break
        end = text.find("\n            );", start)
        if end < 0:
            end = text.find("\n        );", start)
        if end < 0:
            end = text.find(");", start)
        if end < 0:
            end = len(text)
        block = text[start:end]
        search_from = end + 2
        if "OwnerKind::Agent" not in block:
            continue
        prefix = text[:start]
        fn_start = prefix.rfind("\n    fn ")
        enclosing = prefix[fn_start : start] if fn_start >= 0 else ""
        if "agent_owned_static_registration_rejects_fallback_manifest_publication" in enclosing:
            continue
        line = text.count("\n", 0, start) + 1
        violations.append(f"{path}:{line}: agent-owned static registration must use explicit manifest")

if violations:
    print("\n".join(violations), file=sys.stderr)
    raise SystemExit(1)
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/src/daemon/ability/builtins/governance" "$tmp/src/daemon/ability"

  cat >"$tmp/src/daemon/ability/dispatch.rs" <<'RS'
fn commit(owner: OwnerKind, ability: &str) -> anyhow::Result<()> {
    if matches!(owner, OwnerKind::Agent(_)) {
        anyhow::bail!("agent-owned ability {ability:?} requires an explicit manifest; descriptor publication must not synthesize fallback metadata");
    }
    Ok(())
}
RS
  cat >"$tmp/src/daemon/ability/builtins/governance/meta.rs" <<'RS'
fn agent_owned_static_registration_rejects_fallback_manifest_publication() {
    let _ = "fallback metadata";
}
RS
  check_root "$tmp"

  perl -0pi -e 's/if matches!\(owner, OwnerKind::Agent\(_\)\) \{\n        anyhow::bail!\("agent-owned ability \{ability:\?\} requires an explicit manifest; descriptor publication must not synthesize fallback metadata"\);\n    \}\n    //' \
    "$tmp/src/daemon/ability/dispatch.rs"
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected missing Agent manifest rejection to fail"
  fi

  cat >>"$tmp/src/daemon/ability/builtins/governance/meta.rs" <<'RS'
fn fallback_positive() {
    let legacy_desc = "(system ability)";
    assert_eq!(legacy["schema_summary"]["input"]["type"], "object");
}
RS
  cat >"$tmp/src/daemon/ability/dispatch.rs" <<'RS'
fn commit(owner: OwnerKind, ability: &str) -> anyhow::Result<()> {
    if matches!(owner, OwnerKind::Agent(_)) {
        anyhow::bail!("agent-owned ability {ability:?} requires an explicit manifest; descriptor publication must not synthesize fallback metadata");
    }
    Ok(())
}
RS
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected fallback-positive meta assertion to fail"
  fi

  cat >"$tmp/src/daemon/ability/builtins/governance/meta.rs" <<'RS'
fn agent_owned_static_registration_rejects_fallback_manifest_publication() {
    let _ = "fallback metadata";
}

fn production_fixture() {
    reg.register_rpc_with_owner_and_action(
        "agent.legacy",
        OwnerKind::Agent("agent".into()),
        AdmissionAction::Invoke,
        handler,
    );
}
RS
  if ( check_root "$tmp" ) >/dev/null 2>&1; then
    fail "self-test expected Agent owner action registration without manifest to fail"
  fi

  echo "check-static-registration-manifest-boundary self-test: ok"
  exit 0
fi

check_root "$ROOT"
echo "check-static-registration-manifest-boundary: ok"
