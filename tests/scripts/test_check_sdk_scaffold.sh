#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-sdk-scaffold.sh"

fail() {
  printf 'test_check_sdk_scaffold: %s\n' "$1" >&2
  exit 1
}

make_sandbox() {
  local dir="$1"
  mkdir -p "$dir"
  cp -R "$ROOT/sdk" "$dir/sdk"
  mkdir -p "$dir/src/bin"
  cp "$ROOT/src/bin/sdk-conformance-runner.rs" "$dir/src/bin/sdk-conformance-runner.rs"
  cp "$ROOT/PROJECT_STRUCTURE.md" "$dir/PROJECT_STRUCTURE.md"
}

expect_fail() {
  local dir="$1"
  if bash "$CHECK" "$dir" >/tmp/sdk-scaffold-test.out 2>&1; then
    cat /tmp/sdk-scaffold-test.out >&2
    fail "expected check to fail for $dir"
  fi
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB" /tmp/sdk-scaffold-test.out' EXIT

make_sandbox "$SB/pass"
bash "$CHECK" "$SB/pass" >/dev/null

make_sandbox "$SB/missing-schema"
rm "$SB/missing-schema/sdk/schemas/invocation.schema.json"
expect_fail "$SB/missing-schema"

make_sandbox "$SB/invalid-json"
printf '{not-json}\n' >"$SB/invalid-json/sdk/conformance/fixtures/runtime.error.v4.json"
expect_fail "$SB/invalid-json"

make_sandbox "$SB/broken-case"
perl -0pi -e 's/\nexpect:\n/\n/' "$SB/broken-case/sdk/conformance/cases/invocation-complete-tuple.yaml"
expect_fail "$SB/broken-case"

make_sandbox "$SB/missing-doc"
rm "$SB/missing-doc/sdk/SDK_INTERFACE_SPEC.md"
expect_fail "$SB/missing-doc"

printf 'test_check_sdk_scaffold ok\n'
