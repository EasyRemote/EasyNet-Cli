#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-sdk-seam-test-vocabulary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-sdk-seam-test-vocabulary.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p \
        "$sandbox/sdk/node/test" \
        "$sandbox/sdk/java/src/test/java/run/runtime/sdk" \
        "$sandbox/sdk/swift/Tests/RuntimeSDKTests"
    cp "$REPO_ROOT/sdk/node/test/runtime-core.test.mjs" "$sandbox/sdk/node/test/runtime-core.test.mjs"
    cp "$REPO_ROOT/sdk/node/test/types.test.ts" "$sandbox/sdk/node/test/types.test.ts"
    cp "$REPO_ROOT/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java" \
        "$sandbox/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
    cp "$REPO_ROOT/sdk/swift/Tests/RuntimeSDKTests/RuntimeCoreSeamTests.swift" \
        "$sandbox/sdk/swift/Tests/RuntimeSDKTests/RuntimeCoreSeamTests.swift"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    (cd "$sandbox" && CHECK_SDK_SEAM_TEST_VOCABULARY_ROOT="$sandbox" bash "$SCRIPT")
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: neutral SDK seam vocabulary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/WorkflowClient/AdminClient/' "$SB/sdk/node/test/runtime-core.test.mjs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "product client vocabulary should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/downstreamProfileSymbols/productSymbols/' "$SB/sdk/node/test/types.test.ts"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "product-oriented symbol table name should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/downstreamProfileSymbols/removedProducts/' "$SB/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "removedProducts seam table should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/\n\s+"ProfileBundle",//' "$SB/sdk/swift/Tests/RuntimeSDKTests/RuntimeCoreSeamTests.swift"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing neutral downstream profile symbol should exit 1 (got $rc)"

echo "test_check_sdk_seam_test_vocabulary.sh: all cases passed"
