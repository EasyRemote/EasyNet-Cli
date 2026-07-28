#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-sdk-doc-product-vocabulary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-sdk-doc-product-vocabulary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/sdk/go" "$sandbox/sdk/python" "$sandbox/sdk/node" "$sandbox/sdk/java" "$sandbox/sdk/swift" "$sandbox/tools/scripts"
    cp "$SCRIPT" "$sandbox/tools/scripts/check-sdk-doc-product-vocabulary.sh"
    cp "$REPO_ROOT/sdk/README.md" "$sandbox/sdk/README.md"
    cp "$REPO_ROOT/sdk/SDK_PARITY.md" "$sandbox/sdk/SDK_PARITY.md"
    cp "$REPO_ROOT/sdk/SDK_INTERFACE_SPEC.md" "$sandbox/sdk/SDK_INTERFACE_SPEC.md"
    cp "$REPO_ROOT/sdk/go/doc.go" "$sandbox/sdk/go/doc.go"
    cp "$REPO_ROOT/sdk/go/README.md" "$sandbox/sdk/go/README.md"
    cp "$REPO_ROOT/sdk/python/README.md" "$sandbox/sdk/python/README.md"
    cp "$REPO_ROOT/sdk/node/README.md" "$sandbox/sdk/node/README.md"
    cp "$REPO_ROOT/sdk/java/README.md" "$sandbox/sdk/java/README.md"
    cp "$REPO_ROOT/sdk/swift/README.md" "$sandbox/sdk/swift/README.md"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    (
        cd "$sandbox"
        CHECK_SDK_DOC_PRODUCT_VOCABULARY_ROOT="$sandbox" \
            bash tools/scripts/check-sdk-doc-product-vocabulary.sh
    )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: product-neutral SDK docs should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
printf '\nThe SDK is not an EasyRemote SDK.\n' >>"$SB/sdk/python/README.md"
rc=0
run_check "$SB" >/tmp/check-sdk-doc-product-vocabulary.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "named downstream product should exit 1 (got $rc)"
grep -Fq "product vocabulary" /tmp/check-sdk-doc-product-vocabulary.out \
    || fail "named downstream product failure should name product vocabulary"

SB="$(make_sandbox)"
printf '\nMission and OpenAI compatibility clients are not exposed.\n' >>"$SB/sdk/SDK_PARITY.md"
rc=0
run_check "$SB" >/tmp/check-sdk-doc-product-vocabulary.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "workflow-specific product examples should exit 1 (got $rc)"
grep -Fq "product vocabulary" /tmp/check-sdk-doc-product-vocabulary.out \
    || fail "workflow-specific product failure should name product vocabulary"

SB="$(make_sandbox)"
printf '\n// EasyNet provider docs must not live in SDK Godoc.\n' >>"$SB/sdk/go/doc.go"
rc=0
run_check "$SB" >/tmp/check-sdk-doc-product-vocabulary.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "Go SDK Godoc product vocabulary should exit 1 (got $rc)"
grep -Fq "sdk/go/doc.go" /tmp/check-sdk-doc-product-vocabulary.out \
    || fail "Go SDK Godoc failure should point at sdk/go/doc.go"

SB="$(make_sandbox)"
perl -0pi -e 's/Downstream applications build typed local facades/Applications build local facades/' "$SB/sdk/README.md"
rc=0
run_check "$SB" >/tmp/check-sdk-doc-product-vocabulary.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing generic boundary wording should exit 1 (got $rc)"
grep -Fq "missing product-neutral boundary wording" /tmp/check-sdk-doc-product-vocabulary.out \
    || fail "missing boundary wording failure should name required phrase"

echo "test_check_sdk_doc_product_vocabulary.sh: all cases passed"
