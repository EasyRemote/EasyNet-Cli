#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

if ! command -v node >/dev/null 2>&1; then
  echo "check-node-sdk-seam: node is required" >&2
  exit 1
fi

npm test --prefix "$ROOT/sdk/node" >/dev/null
node --check "$ROOT/sdk/node/index.js" >/dev/null

if grep -RInE '\bURI\b|\bUri\b|\buri\b|_uri\b|uri_' "$ROOT/sdk/node" --include='*.js' --include='*.ts' --include='*.md' | grep -v 'node_modules' >/tmp/easynet-node-uri-scan.out 2>/dev/null; then
  echo "check-node-sdk-seam: URI-era naming found in Node SDK" >&2
  cat /tmp/easynet-node-uri-scan.out >&2
  rm -f /tmp/easynet-node-uri-scan.out
  exit 1
fi
rm -f /tmp/easynet-node-uri-scan.out

if grep -RInE '\bbackend_ura\b|\buser_ura\b|backendURA|userURA' "$ROOT/sdk/node" --include='*.js' --include='*.ts' --include='*.md' >/tmp/easynet-node-authority-scan.out 2>/dev/null; then
  echo "check-node-sdk-seam: product-specific authority fields found in Node SDK" >&2
  cat /tmp/easynet-node-authority-scan.out >&2
  rm -f /tmp/easynet-node-authority-scan.out
  exit 1
fi
rm -f /tmp/easynet-node-authority-scan.out

echo "check-node-sdk-seam ok"
