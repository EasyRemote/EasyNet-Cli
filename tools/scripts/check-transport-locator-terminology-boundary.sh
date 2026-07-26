#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

scan_paths=()
for path in src tests; do
  [[ -d "$path" ]] && scan_paths+=("$path")
done

if ((${#scan_paths[@]} == 0)); then
  fail "transport locator terminology scan has no source roots"
fi

transport_matches="$(rg -n 'tonic::transport::\{[^}]*\bUri\b|tonic::transport::Uri\b|\|_:\s*Uri\b' \
  "${scan_paths[@]}" \
  --glob '*.rs' \
  --glob '!tests/scripts/**' || true)"
transport_violations=()
while IFS= read -r line; do
  [[ -n "$line" ]] || continue
  [[ "$line" == *"Uri as GrpcEndpointLocator"* ]] && continue
  transport_violations+=("$line")
done <<<"$transport_matches"
if ((${#transport_violations[@]} > 0)); then
  printf '%s\n' "${transport_violations[@]}" >&2
  fail "gRPC transport locators must be aliased as GrpcEndpointLocator instead of exposing URI terminology"
fi

if rg -in '\b(caller|callee|ability|subject|owner|resource|receipt|principal|device|agent)[A-Za-z0-9_]*(?:_uri|Uri|URI)\b|\b(?:uri_|Uri|URI)[A-Za-z0-9_]*(caller|callee|ability|subject|owner|resource|receipt|principal|device|agent)\b' \
  "${scan_paths[@]}" \
  --glob '*.rs' \
  --glob '!tests/scripts/**'; then
  fail "semantic runtime identities must use URA terminology, not URI"
fi

echo "check-transport-locator-terminology-boundary: ok"
