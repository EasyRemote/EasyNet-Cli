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
  mkdir -p \
    "$dir/sdk/java" \
    "$dir/sdk/python" \
    "$dir/sdk/swift"
  cp "$ROOT"/sdk/*.md "$dir/sdk/"
  cp -R \
    "$ROOT/sdk/conformance" \
    "$ROOT/sdk/go" \
    "$ROOT/sdk/node" \
    "$ROOT/sdk/schemas" \
    "$dir/sdk/"
  cp \
    "$ROOT/sdk/java/.gitignore" \
    "$ROOT/sdk/java/pom.xml" \
    "$ROOT/sdk/java/README.md" \
    "$dir/sdk/java/"
  cp -R "$ROOT/sdk/java/src" "$dir/sdk/java/src"
  cp \
    "$ROOT/sdk/python/pyproject.toml" \
    "$ROOT/sdk/python/README.md" \
    "$dir/sdk/python/"
  cp -R \
    "$ROOT/sdk/python/easynet_sdk" \
    "$ROOT/sdk/python/tests" \
    "$dir/sdk/python/"
  cp \
    "$ROOT/sdk/swift/.gitignore" \
    "$ROOT/sdk/swift/Package.swift" \
    "$ROOT/sdk/swift/README.md" \
    "$dir/sdk/swift/"
  cp -R \
    "$ROOT/sdk/swift/Sources" \
    "$ROOT/sdk/swift/Tests" \
    "$dir/sdk/swift/"
  find "$dir/sdk" -type d -name __pycache__ -prune -exec rm -rf {} +
  mkdir -p \
    "$dir/include" \
    "$dir/src/bin" \
    "$dir/src/ffi" \
    "$dir/target" \
    "$dir/tools/sdk-conformance-runner/src" \
    "$dir/tools/scripts"
  cp "$ROOT/include/easynet_cli.h" "$dir/include/easynet_cli.h"
  cp "$ROOT/include/easynet_cli.exports.v5" "$dir/include/easynet_cli.exports.v5"
  cp "$ROOT/tools/sdk-conformance-runner/Cargo.toml" "$dir/tools/sdk-conformance-runner/Cargo.toml"
  cp "$ROOT/tools/sdk-conformance-runner/src/main.rs" "$dir/tools/sdk-conformance-runner/src/main.rs"
  mkdir -p "$dir/src/ffi/features"
  cp "$ROOT/src/ffi/features/mod.rs" "$dir/src/ffi/features/mod.rs"
  local checker
  for checker in \
    check-backend-route-family-coverage.sh \
    check-backend-sdk-only-boundary.sh \
    check-daemon-latest-input-boundary.sh \
    check-easyremote-sdk-boundary.sh \
    check-java-sdk-seam.sh \
    check-node-sdk-seam.sh \
    check-sdk-completion-audit.sh \
    check-sdk-conformance-reports.sh \
    check-sdk-canonical-public-api.sh \
    check-sdk-cutover-readiness.sh \
    check-sdk-package-metadata.sh \
    check-sdk-parity-matrix.sh \
    check-sdk-product-smokes.sh \
    check-python-sdk-static-contract.sh \
    check-sdk-receipt-ura-boundary.sh \
    check-sdk-scaffold.sh \
    check-sdk-ura-naming.sh \
    check-swift-sdk-seam.sh \
    go-sdk-live-smoke.sh \
    python-sdk-live-smoke.sh
  do
    cp "$ROOT/tools/scripts/$checker" "$dir/tools/scripts/$checker"
  done
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

FIXTURE="$SB/repo"
make_sandbox "$FIXTURE"
bash "$CHECK" "$FIXTURE" >/dev/null

printf '{}\n' >"$FIXTURE/sdk/schemas/resource-ref.schema.json"
expect_fail "$FIXTURE"
rm "$FIXTURE/sdk/schemas/resource-ref.schema.json"

python3 - "$FIXTURE/sdk/conformance/fixture-schema-bindings.json" <<'PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["bindings"].append({
    "fixture": "resource-ref.local-fs.v4.json",
    "schema": "resource-ref.schema.json",
})
path.write_text(json.dumps(data, indent=2) + "\n")
PY
expect_fail "$FIXTURE"
cp \
  "$ROOT/sdk/conformance/fixture-schema-bindings.json" \
  "$FIXTURE/sdk/conformance/fixture-schema-bindings.json"

rm "$FIXTURE/sdk/schemas/invocation.schema.json"
expect_fail "$FIXTURE"
cp \
  "$ROOT/sdk/schemas/invocation.schema.json" \
  "$FIXTURE/sdk/schemas/invocation.schema.json"

printf '{not-json}\n' >"$FIXTURE/sdk/conformance/fixtures/runtime.error.v4.json"
expect_fail "$FIXTURE"
cp \
  "$ROOT/sdk/conformance/fixtures/runtime.error.v4.json" \
  "$FIXTURE/sdk/conformance/fixtures/runtime.error.v4.json"

perl -0pi -e 's/\nexpect:\n/\n/' "$FIXTURE/sdk/conformance/cases/invocation-complete-tuple.yaml"
expect_fail "$FIXTURE"
cp \
  "$ROOT/sdk/conformance/cases/invocation-complete-tuple.yaml" \
  "$FIXTURE/sdk/conformance/cases/invocation-complete-tuple.yaml"

rm "$FIXTURE/sdk/SDK_INTERFACE_SPEC.md"
expect_fail "$FIXTURE"
cp "$ROOT/sdk/SDK_INTERFACE_SPEC.md" "$FIXTURE/sdk/SDK_INTERFACE_SPEC.md"

rm "$FIXTURE/include/easynet_cli.h"
expect_fail "$FIXTURE"

printf 'test_check_sdk_scaffold ok\n'
