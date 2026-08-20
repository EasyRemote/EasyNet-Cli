#!/usr/bin/env bash
#
# Guard SDK-facing documentation against product-specific vocabulary.

set -euo pipefail

ROOT="${CHECK_SDK_DOC_PRODUCT_VOCABULARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-sdk-doc-product-vocabulary: $*" >&2
    exit 1
}

DOCS=(
    "sdk/README.md"
    "sdk/SDK_PARITY.md"
    "sdk/SDK_INTERFACE_SPEC.md"
    "sdk/go/doc.go"
    "sdk/go/README.md"
    "sdk/python/README.md"
    "sdk/node/README.md"
    "sdk/java/README.md"
    "sdk/swift/README.md"
)

for doc in "${DOCS[@]}"; do
    [[ -f "$doc" ]] || fail "missing $doc"
done

bad="$(
    rg -n '\b(EasyNet|EasyRemote|Mission|EAL|OpenAI|HostBinding|Publication|Surface|Pages|Companion|Wrapper|Wrappers)\b|DaemonControl|DaemonHandle|desktop companion|Product profiles are deliberately absent|EasyNet provider|Native EasyNet' \
        "${DOCS[@]}" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "SDK documentation still defines runtime boundaries with product vocabulary:
$bad"
fi

for required in \
    "Downstream applications build typed local facades" \
    "Workflow-specific directory pages" \
    "Native provider ABIs are the lowering layers" \
    "Downstream products own their ability names" \
    "Downstream workflow profiles are deliberately absent"
do
    if ! rg -Fq "$required" "${DOCS[@]}"; then
        fail "SDK documentation is missing product-neutral boundary wording: $required"
    fi
done

echo "check-sdk-doc-product-vocabulary: ok"
