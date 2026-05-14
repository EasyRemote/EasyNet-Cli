#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

violations="$tmp/violations.txt"
: >"$violations"

# URA construction and scheme-prefix parsing must stay centralized in
# src/ura.rs. Other modules should call crate::ura builders/parsers so
# ontology changes cannot leave stale hand-built route fragments behind.
rg -n \
  'format!\([^"\n]*"easynet:///r|strip_prefix\("easynet:///r/"\)|starts_with\("easynet:///r/' \
  src \
  --glob '!src/ura.rs' \
  >"$violations" || true

if [[ -s "$violations" ]]; then
  echo "raw URA construction/parsing found outside src/ura.rs:" >&2
  cat "$violations" >&2
  exit 1
fi
