#!/usr/bin/env bash
#
# Guard SDK seam tests against product taxonomy leakage.
#
# The SDK defines the canonical runtime model. Seam tests may prove that
# downstream workflow/profile symbols are absent, but they must not encode
# concrete product client names as SDK evidence.

set -euo pipefail

ROOT="${CHECK_SDK_SEAM_TEST_VOCABULARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-sdk-seam-test-vocabulary: $*" >&2
    exit 1
}

SEAM_TESTS=(
    "sdk/node/test/runtime-core.test.mjs"
    "sdk/node/test/types.test.ts"
    "sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
    "sdk/swift/Tests/RuntimeSDKTests/RuntimeCoreSeamTests.swift"
)

for path in "${SEAM_TESTS[@]}"; do
    [[ -f "$path" ]] || fail "missing seam test: $path"
done

product_client_leaks="$(
    rg -n '\b(AdminClient|CompanionClient|CompatibilityClient|DirectoryClient|IdentityClient|EventClient|HostBindingClient|MissionClient|PublicationClient|ReceiptClient|SurfaceClient|WrapperClient)\b' \
        "${SEAM_TESTS[@]}" 2>/dev/null || true
)"
if [[ -n "$product_client_leaks" ]]; then
    fail "SDK seam tests still encode concrete product client names:
$product_client_leaks"
fi

legacy_symbol_tables="$(
    rg -n '\b(productSymbols|removedProducts)\b' "${SEAM_TESTS[@]}" 2>/dev/null || true
)"
if [[ -n "$legacy_symbol_tables" ]]; then
    fail "SDK seam tests still use product-oriented symbol table names:
$legacy_symbol_tables"
fi

for path in "${SEAM_TESTS[@]}"; do
    grep -Fq "downstreamProfileSymbols" "$path" \
        || fail "$path must use downstreamProfileSymbols as the neutral seam vocabulary"
done

for symbol in \
    "WorkflowClient" \
    "WorkflowTransport" \
    "ApplicationLifecycleClient" \
    "ApplicationDirectoryView" \
    "ApplicationReceiptPage" \
    "CompatibilityAdapter" \
    "ConvenienceWrapperClient" \
    "ProfileBundle"
do
    for path in "${SEAM_TESTS[@]}"; do
        grep -Fq "$symbol" "$path" \
            || fail "$path is missing neutral downstream profile symbol $symbol"
    done
done

for path in "${SEAM_TESTS[@]}"; do
    grep -Fq "declarations.includes(symbol)" "$path" \
        || [[ "$path" != sdk/node/test/*.test.* ]] \
        || fail "$path must assert neutral profile symbols are absent from public declarations"
done

echo "check-sdk-seam-test-vocabulary: ok"
