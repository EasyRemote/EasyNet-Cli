#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

failures=()

fail() {
  failures+=("$1")
}

require_file() {
  local path="$1"
  [[ -f "$ROOT/$path" ]] || fail "missing file: $path"
}

require_dir() {
  local path="$1"
  [[ -d "$ROOT/$path" ]] || fail "missing directory: $path"
}

require_literal() {
  local path="$1"
  local literal="$2"
  if [[ -f "$ROOT/$path" ]] && ! grep -Fq "$literal" "$ROOT/$path"; then
    fail "missing literal in $path: $literal"
  fi
}

validate_json_file() {
  local path="$1"
  if ! python3 -m json.tool "$ROOT/$path" >/dev/null 2>&1; then
    fail "invalid json: $path"
  fi
}

if ! command -v python3 >/dev/null 2>&1; then
  fail "python3 is required to validate SDK JSON scaffold"
fi

for path in \
  PROJECT_STRUCTURE.md \
  sdk/README.md \
  sdk/SDK_INTERFACE_SPEC.md \
  sdk/SDK_PARITY.md \
  sdk/CONFORMANCE_SUITE.md
do
  require_file "$path"
done

require_dir sdk/schemas
require_dir sdk/conformance/cases
require_dir sdk/conformance/fixtures
require_dir sdk/conformance/runner

schema_files=(
  invocation.schema.json
  identity.schema.json
  receipt.schema.json
  error.schema.json
  health.schema.json
  events.schema.json
  directory-page.schema.json
  publication.schema.json
  resource-ref.schema.json
  host-stream-binding.schema.json
  host-stream-frame.schema.json
  mission-status.schema.json
  admin.schema.json
  gateway.schema.json
  agent-record.schema.json
  surface-page.schema.json
  compatibility.schema.json
  file.schema.json
  terminal.schema.json
  remote-desktop.schema.json
  browser-session.schema.json
  media-session.schema.json
  stream-event.schema.json
  bidi-frame.schema.json
  authority.schema.json
  lifecycle-status.schema.json
)

for schema in "${schema_files[@]}"; do
  path="sdk/schemas/$schema"
  require_file "$path"
  validate_json_file "$path"
  require_literal "$path" '"$schema"'
  require_literal "$path" '"title"'
done

fixture_files=(
  invocation.complete.v4.json
  prepared.signing-material.v4.json
  identity.descriptor-ref.v4.json
  receipt.summary.v4.json
  runtime.error.v4.json
  event.directory.v4.json
  health.ready.v4.json
)

for fixture in "${fixture_files[@]}"; do
  path="sdk/conformance/fixtures/$fixture"
  require_file "$path"
  validate_json_file "$path"
done

case_files=(
  version-abi-compatible.yaml
  version-abi-incompatible.yaml
  daemon-control-only.yaml
  invocation-complete-tuple.yaml
  invocation-builder-handle-state.yaml
  invocation-handle-terminal-monotonicity.yaml
  invocation-canonical-material.yaml
  invocation-prepared-not-submittable.yaml
  invocation-presigned-submit.yaml
  invocation-local-daemon-signing-boundary.yaml
  error-typed-json.yaml
  identity-ura-descriptor-projection.yaml
  receipt-projection-causal-ref.yaml
  stream-bidi-lifecycle-state.yaml
  health-api-vs-runtime.yaml
  directory-list-pagination.yaml
  directory-no-default-fanout.yaml
  memc-profile-exclusivity.yaml
)

for case_file in "${case_files[@]}"; do
  path="sdk/conformance/cases/$case_file"
  require_file "$path"
  require_literal "$path" "id:"
  require_literal "$path" "profile:"
  require_literal "$path" "required_for:"
  require_literal "$path" "steps:"
  require_literal "$path" "expect:"
done

require_file sdk/conformance/runner/README.md
require_literal sdk/SDK_INTERFACE_SPEC.md "PreparedInvocation"
require_literal sdk/SDK_INTERFACE_SPEC.md "SignedInvocation"
require_literal sdk/SDK_INTERFACE_SPEC.md "No public object in this graph may expose raw Axon"
require_literal sdk/SDK_PARITY.md "No current language is"
require_literal sdk/CONFORMANCE_SUITE.md "Runner Contract"

if [[ "${#failures[@]}" -eq 0 ]]; then
  printf 'check-sdk-scaffold ok\n'
  exit 0
fi

printf 'check-sdk-scaffold failed:\n' >&2
printf ' - %s\n' "${failures[@]}" >&2
exit 1
