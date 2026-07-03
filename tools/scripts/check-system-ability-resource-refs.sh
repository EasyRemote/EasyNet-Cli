#!/usr/bin/env bash
#
# Guard host-filesystem ability manifests against raw path arguments.

set -euo pipefail

ROOT="${CHECK_SYSTEM_ABILITY_RESOURCE_REFS_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

fail() {
    echo "check-system-ability-resource-refs: $*" >&2
    exit 1
}

descriptor_path() {
    local ability="$1"
    local paths count
    paths="$(find "$ROOT/ability-descriptors/system" -type f -name "${ability}.ability.toml" -print | sort)"
    count="$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d ' ')"
    [[ "$count" == "1" ]] \
        || fail "expected exactly one descriptor for $ability, found $count"
    printf '%s\n' "$paths"
}

manifests=(
    "fs.read"
    "fs.write"
    "fs.list"
    "fs.stat"
    "fs.edit"
    "fs.transfer"
    "ability.deploy"
)

for ability in "${manifests[@]}"; do
    file="$(descriptor_path "$ability")"
    rel="${file#$ROOT/}"
    [[ -f "$file" ]] || fail "missing manifest: $rel"

    if grep -Eq '^required = \[[^]]*"path"' "$file"; then
        fail "$rel declares raw path as a required input"
    fi
    if grep -Fq '[input_schema.properties.path]' "$file"; then
        fail "$rel exposes input_schema.properties.path"
    fi
    grep -Eq '^required = \[[^]]*"resource_ref"' "$file" \
        || fail "$rel must require resource_ref"
    grep -Fq '[input_schema.properties.resource_ref]' "$file" \
        || fail "$rel must define input_schema.properties.resource_ref"
    grep -Fq 'required = ["resource_ura", "owner_ura", "namespace", "capability", "expires_unix_ms", "revision"]' "$file" \
        || fail "$rel resource_ref schema is missing the canonical required fields"
    grep -Fq 'enum = ["fs"]' "$file" \
        || fail "$rel resource_ref namespace must be fs"
done

echo "check-system-ability-resource-refs: ok"
