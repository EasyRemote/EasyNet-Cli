#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-companion-status-package-version-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
OUT="$SB/check-companion-status-package-version-boundary.out"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/plugins/companion"
cp "$SCRIPT" "$SB/tools/scripts/check-companion-status-package-version-boundary.sh"

write_happy_fixture() {
  cat >"$SB/src/daemon/plugins/companion/projection.rs" <<'RS'
fn project_status(obj: Obj) -> Result<Value, Error> {
    let package_version = required_string(obj, "package_version")?;
    Ok(json!({ "package_version": package_version }))
}

#[test]
fn project_status_rejects_version_alias_for_package_version() {
    let error = project_status(input_without_package_version_but_with_version()).unwrap_err();
    assert_eq!(error, Error::MissingField("package_version"));
}
RS
}

assert_fails_with() {
  local expected="$1"
  set +e
  (
    cd "$SB"
    bash tools/scripts/check-companion-status-package-version-boundary.sh
  ) >"$OUT" 2>&1
  local rc=$?
  set -e
  [[ "$rc" == "1" ]] || fail "expected gate failure exit 1, got $rc"
  grep -Fq "$expected" "$OUT" || fail "expected failure to mention: $expected"
}

write_happy_fixture
(
  cd "$SB"
  bash tools/scripts/check-companion-status-package-version-boundary.sh
) >/dev/null || fail "happy path should pass"

write_happy_fixture
perl -0pi -e 's/let package_version = required_string\(obj, "package_version"\)\?;/let package_version = required_string(obj, "package_version").or_else(|_| required_string(obj, "version"))?;/' "$SB/src/daemon/plugins/companion/projection.rs"
assert_fails_with "must require canonical package_version directly"

write_happy_fixture
perl -0pi -e 's/project_status_rejects_version_alias_for_package_version/project_status_accepts_version_alias_for_package_version/' "$SB/src/daemon/plugins/companion/projection.rs"
assert_fails_with "must not accept version as a package_version alias"

echo "test_check_companion_status_package_version_boundary.sh: all cases passed"
