#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/check-sdk-ura-naming.sh"

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  good="$tmp/good.ts"
  bad="$tmp/bad.ts"
  cat >"$good" <<'EOF'
export const SECURITY = "identity";
export const security = "identity";
export const SecurityClass = "identity";
export const SECURITY_CLASS = "identity";
export const securityClass = "identity";
EOF
  cat >"$bad" <<EOF
export const Agent${SDK_URA_NAMING_TOKEN_UPPER} = "easynet:///r/example/agent/alice.sdk";
export const Ability${SDK_URA_NAMING_TOKEN_TITLE} = "easynet:///r/example/ability/alice.echo";
export const agent${SDK_URA_NAMING_TOKEN_TITLE} = "easynet:///r/example/agent/alice.sdk";
export const DEVICE_${SDK_URA_NAMING_TOKEN_UPPER} = "easynet:///r/example/device/dev-a";
EOF
  sdk_ura_naming_scan_files "$good"
  if sdk_ura_naming_scan_files "$bad" >"$tmp/out" 2>&1; then
    echo "check-node-sdk-seam: self-test expected retired address-token naming to fail" >&2
    exit 1
  fi
  for expected in \
    "Agent${SDK_URA_NAMING_TOKEN_UPPER}" \
    "Ability${SDK_URA_NAMING_TOKEN_TITLE}" \
    "agent${SDK_URA_NAMING_TOKEN_TITLE}" \
    "DEVICE_${SDK_URA_NAMING_TOKEN_UPPER}"
  do
    grep -Fq "$expected" "$tmp/out"
  done
  echo "check-node-sdk-seam self-test ok"
  exit 0
fi

ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"

if ! command -v node >/dev/null 2>&1; then
  echo "check-node-sdk-seam: node is required" >&2
  exit 1
fi

npm test --prefix "$ROOT/sdk/node" >/dev/null
node --check "$ROOT/sdk/node/index.js" >/dev/null

receipt_fixture="$ROOT/sdk/node/test-support/runtime-fixtures.mjs"
if [[ ! -f "$receipt_fixture" ]]; then
  echo "check-node-sdk-seam: missing shared runtime receipt fixture" >&2
  exit 1
fi
node --check "$receipt_fixture" >/dev/null

for required in \
  "export const canonicalRuntimeReceipt" \
  "payload_base64" \
  "payload_content_type" \
  "host_attestation_base64" \
  "usage: {}" \
  "proof_payload_base64" \
  "proof_hash_hex"
do
  if ! grep -Fq "$required" "$receipt_fixture"; then
    echo "check-node-sdk-seam: shared runtime receipt fixture is missing $required" >&2
    exit 1
  fi
done

if grep -RInE '\b(const|function) canonicalRuntimeReceipt\b' "$ROOT/sdk/node/test" \
  --include='*.js' --include='*.mjs' --include='*.ts' >/tmp/easynet-node-receipt-fixture-scan.out 2>/dev/null; then
  echo "check-node-sdk-seam: Node SDK tests must use the shared runtime receipt fixture" >&2
  cat /tmp/easynet-node-receipt-fixture-scan.out >&2
  rm -f /tmp/easynet-node-receipt-fixture-scan.out
  exit 1
fi
rm -f /tmp/easynet-node-receipt-fixture-scan.out

retired_name_scan="$(mktemp)"
trap 'rm -f "$retired_name_scan"' EXIT
if grep -RInE "$SDK_URA_NAMING_PATTERN" "$ROOT/sdk/node" --include='*.js' --include='*.ts' --include='*.md' --exclude-dir='node_modules' >"$retired_name_scan" 2>/dev/null; then
  echo "check-node-sdk-seam: retired address-token naming found in Node SDK" >&2
  cat "$retired_name_scan" >&2
  exit 1
fi
rm -f "$retired_name_scan"
trap - EXIT

if grep -RInE '\bbackend_ura\b|\buser_ura\b|backendURA|userURA' "$ROOT/sdk/node" --include='*.js' --include='*.ts' --include='*.md' >/tmp/easynet-node-authority-scan.out 2>/dev/null; then
  echo "check-node-sdk-seam: product-specific authority fields found in Node SDK" >&2
  cat /tmp/easynet-node-authority-scan.out >&2
  rm -f /tmp/easynet-node-authority-scan.out
  exit 1
fi
rm -f /tmp/easynet-node-authority-scan.out

echo "check-node-sdk-seam ok"
