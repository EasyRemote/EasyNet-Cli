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
  events-directory-subscription-request.schema.json
  directory-list-devices-request.schema.json
  directory-list-agents-request.schema.json
  directory-list-abilities-request.schema.json
  directory-page.schema.json
  local-resource-ref-request.schema.json
  publication.schema.json
  ability-package-manifest.schema.json
  package-validation.schema.json
  ability-deploy-request.schema.json
  ability-deploy-result.schema.json
  published-ability.schema.json
  ability-impl-id.schema.json
  resource-ref.schema.json
  host-stream-binding-request.schema.json
  host-stream-binding.schema.json
  host-stream-envelope.schema.json
  host-stream-request.schema.json
  host-stream-frame.schema.json
  host-stream-terminal-summary.schema.json
  host-stream-hash-state.schema.json
  mission-run-request.schema.json
  mission-run-file-request.schema.json
  mission-track-request.schema.json
  mission-cancel-request.schema.json
  mission-status.schema.json
  admin.schema.json
  gateway.schema.json
  agent-record.schema.json
  admin-agent-list-request.schema.json
  admin-agent-start-request.schema.json
  admin-agent-stop-request.schema.json
  admin-agent-refresh-request.schema.json
  admin-session-list-request.schema.json
  surface-page.schema.json
  compatibility.schema.json
  compatibility-list-models-request.schema.json
  compatibility-chat-completion-request.schema.json
  compatibility-stream-chat-completion-request.schema.json
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
  event.directory-drop-report.v4.json
  event.directory-terminal.v4.json
  events-directory-subscription-request.v4.json
  events-directory-subscription-invocation.v4.json
  directory-list-devices-request.v4.json
  directory-list-agents-request.v4.json
  directory-list-abilities-request.v4.json
  directory-list-devices-invocation.v4.json
  directory-list-agents-invocation.v4.json
  directory-list-abilities-invocation.v4.json
  directory-device-page.v4.json
  directory-agent-page.v4.json
  directory-ability-page.v4.json
  health.ready.v4.json
  host-stream-binding-request.v4.json
  host-stream-binding.v4.json
  host-stream-request.v4.json
  host-stream-frame.v4.json
  host-stream-terminal.v4.json
  host-stream-hash-state.v4.json
  local-resource-ref-request.v4.json
  resource-ref.local-fs.v4.json
  ability-package-manifest.v4.json
  package-validation.v4.json
  ability-deploy-request.v4.json
  publication-deploy-invocation.v4.json
  publication-unpublish-invocation.v4.json
  mission-run-request.v4.json
  mission-run-file-request.v4.json
  mission-track-request.v4.json
  mission-cancel-request.v4.json
  mission-run-invocation.v4.json
  mission-track-invocation.v4.json
  mission-cancel-invocation.v4.json
  mission-status.v4.json
  admin-agent-list-request.v4.json
  admin-agent-start-request.v4.json
  admin-agent-stop-request.v4.json
  admin-agent-refresh-request.v4.json
  admin-session-list-request.v4.json
  admin-agent-list-invocation.v4.json
  admin-agent-start-invocation.v4.json
  admin-agent-stop-invocation.v4.json
  admin-agent-refresh-invocation.v4.json
  admin-session-list-invocation.v4.json
  gateway-status.v4.json
  admin-agent-records.v4.json
  admin-agent-lifecycle-result.v4.json
  compatibility-list-models-request.v4.json
  compatibility-chat-completion-request.v4.json
  compatibility-stream-chat-completion-request.v4.json
  compatibility-list-models-invocation.v4.json
  compatibility-chat-completion-invocation.v4.json
  compatibility-stream-chat-completion-invocation.v4.json
  compatibility-model-page.v4.json
  compatibility-chat-completion.v4.json
  compatibility-chat-stream.v4.json
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
  host-binding-codec-hash.yaml
  publication-resource-carriers.yaml
  mission-carrier-status.yaml
  events-directory-stream.yaml
  admin-gateway-carrier-status.yaml
  compatibility-openai-carrier-projection.yaml
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
