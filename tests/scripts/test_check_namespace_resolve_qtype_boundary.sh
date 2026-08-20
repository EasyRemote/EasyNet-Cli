#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-namespace-resolve-qtype-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
OUT="$SB/check-namespace-resolve-qtype-boundary.out"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/invocation/routing"
cp "$SCRIPT" "$SB/tools/scripts/check-namespace-resolve-qtype-boundary.sh"

write_happy_fixture() {
  cat >"$SB/src/daemon/invocation/routing/route_resolver.rs" <<'RS'
fn resolve_query_json(query: &Value) -> Value {
    match json_resolve_type(query) {
        Ok(qtype) => qtype.into(),
        Err(detail) => negative_answer_json(detail),
    }
}

fn json_resolve_type(value: &Value) -> Result<ResolveType, &'static str> {
    let raw = value
        .get("qtype")
        .ok_or("resolve query missing canonical qtype")?;
    let text = raw
        .as_str()
        .ok_or("resolve qtype must be a canonical ResolveType enum string")?
        .trim();
    if text.is_empty() {
        return Err("resolve qtype must be a non-empty canonical ResolveType enum string");
    }
    ResolveType::from_str_name(text)
        .ok_or("resolve qtype must be a canonical ResolveType enum string")
}

#[test]
fn resolve_query_json_rejects_missing_qtype_instead_of_shape_guessing() {}

#[test]
fn resolve_query_json_rejects_short_qtype_aliases() {}
RS
}

assert_fails_with() {
  local label="$1"
  local expected="$2"
  set +e
  (
    cd "$SB"
    bash tools/scripts/check-namespace-resolve-qtype-boundary.sh
  ) >"$OUT" 2>&1
  local rc=$?
  set -e
  [[ "$rc" == "1" ]] || fail "$label: expected gate failure exit 1, got $rc"
  grep -Fq "$expected" "$OUT" || fail "$label: expected failure to mention: $expected"
}

write_happy_fixture
(
  cd "$SB"
  bash tools/scripts/check-namespace-resolve-qtype-boundary.sh
) >/dev/null || fail "happy path should pass"

write_happy_fixture
perl -0pi -e 's/resolve query missing canonical qtype/resolve query can guess qtype/' "$SB/src/daemon/invocation/routing/route_resolver.rs"
assert_fails_with "missing-qtype-regression" "must reject missing qtype before shape guessing"

write_happy_fixture
perl -0pi -e 's/ResolveType::from_str_name\(text\)/ResolveType::from_str_name(text).or_else(|| { let canonical = format!("RESOLVE_TYPE_{}", text.to_ascii_uppercase()); ResolveType::from_str_name(&canonical) })/' "$SB/src/daemon/invocation/routing/route_resolver.rs"
assert_fails_with "short-alias-regression" "must not accept numeric/short qtype aliases or guess qtype from query shape"

write_happy_fixture
perl -0pi -e 's/\n#\[test\]\nfn resolve_query_json_rejects_short_qtype_aliases\(\) \{\}//' "$SB/src/daemon/invocation/routing/route_resolver.rs"
assert_fails_with "missing-test-regression" "missing regression test resolve_query_json_rejects_short_qtype_aliases"

echo "test_check_namespace_resolve_qtype_boundary.sh: all cases passed"
