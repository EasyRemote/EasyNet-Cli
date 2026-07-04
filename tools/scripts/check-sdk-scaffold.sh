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
  src/bin/sdk-conformance-runner.rs \
  sdk/README.md \
  sdk/SDK_INTERFACE_SPEC.md \
  sdk/SDK_PARITY.md \
  sdk/CONFORMANCE_SUITE.md \
  sdk/go/go.mod \
  sdk/go/client.go \
  sdk/go/daemon.go \
  sdk/go/daemon_test.go \
  sdk/go/directory.go \
  sdk/go/directory_test.go \
  sdk/go/identity.go \
  sdk/go/identity_test.go \
  sdk/go/receipt.go \
  sdk/go/receipt_test.go \
  sdk/go/publication.go \
  sdk/go/publication_test.go \
  sdk/go/host_binding.go \
  sdk/go/host_binding_test.go \
  sdk/go/mission.go \
  sdk/go/mission_test.go \
  sdk/go/admin.go \
  sdk/go/admin_test.go \
  sdk/go/events.go \
  sdk/go/events_test.go \
  sdk/go/surface.go \
  sdk/go/surface_test.go \
  sdk/go/compatibility.go \
  sdk/go/compatibility_test.go \
  sdk/go/wrappers.go \
  sdk/go/wrappers_test.go \
  sdk/go/connection.go \
  sdk/go/connection_test.go \
  sdk/go/errors.go \
  sdk/go/health.go \
  sdk/go/health_test.go \
  sdk/go/invocation.go \
  sdk/go/invocation_test.go \
  sdk/go/conformance_test.go \
  sdk/go/runtime.go \
  sdk/go/runtime_test.go \
  sdk/go/bidi.go \
  sdk/go/bidi_test.go \
  sdk/go/stream.go \
  sdk/go/stream_test.go \
  sdk/go/signing.go \
  sdk/go/signing_test.go \
  sdk/go/import_boundary_test.go \
  sdk/python/pyproject.toml \
  sdk/python/easynet_sdk/client.py \
  sdk/python/easynet_sdk/daemon.py \
  sdk/python/easynet_sdk/directory.py \
  sdk/python/easynet_sdk/identity.py \
  sdk/python/easynet_sdk/receipt.py \
  sdk/python/easynet_sdk/publication.py \
  sdk/python/easynet_sdk/host_binding.py \
  sdk/python/easynet_sdk/mission.py \
  sdk/python/easynet_sdk/admin.py \
  sdk/python/easynet_sdk/events.py \
  sdk/python/easynet_sdk/surface.py \
  sdk/python/easynet_sdk/compatibility.py \
  sdk/python/easynet_sdk/wrappers.py \
  sdk/python/easynet_sdk/connection.py \
  sdk/python/easynet_sdk/errors.py \
  sdk/python/easynet_sdk/health.py \
  sdk/python/easynet_sdk/invocation.py \
  sdk/python/easynet_sdk/runtime.py \
  sdk/python/easynet_sdk/bidi.py \
  sdk/python/easynet_sdk/signing.py \
  sdk/python/easynet_sdk/stream.py \
  sdk/python/tests/test_health.py \
  sdk/python/tests/test_daemon.py \
  sdk/python/tests/test_directory.py \
  sdk/python/tests/test_identity.py \
  sdk/python/tests/test_receipt.py \
  sdk/python/tests/test_publication.py \
  sdk/python/tests/test_host_binding.py \
  sdk/python/tests/test_mission.py \
  sdk/python/tests/test_admin.py \
  sdk/python/tests/test_events.py \
  sdk/python/tests/test_surface.py \
  sdk/python/tests/test_compatibility.py \
  sdk/python/tests/test_wrappers.py \
  sdk/python/tests/test_connection.py \
  sdk/python/tests/test_invocation.py \
  sdk/python/tests/test_conformance.py \
  sdk/python/tests/test_runtime.py \
  sdk/python/tests/test_bidi.py \
  sdk/python/tests/test_signing.py \
  sdk/python/tests/test_stream.py \
  sdk/python/tests/test_import_boundary.py
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
  receipt-fetch-request.schema.json
  error.schema.json
  health.schema.json
  events.schema.json
  events-directory-subscription-request.schema.json
  directory-list-devices-request.schema.json
  directory-list-agents-request.schema.json
  directory-list-abilities-request.schema.json
  directory-resolve-request.schema.json
  directory-page.schema.json
  directory-resolved-ref.schema.json
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
  surface-list-pages-request.schema.json
  surface-create-page-request.schema.json
  surface-page-project-request.schema.json
  surface-page-page.schema.json
  surface-public-page-ref.schema.json
  surface-manifest.schema.json
  surface-health.schema.json
  surface-mutation-result.schema.json
  compatibility.schema.json
  compatibility-list-models-request.schema.json
  compatibility-chat-completion-request.schema.json
  compatibility-stream-chat-completion-request.schema.json
  compatibility-file-upload-request.schema.json
  compatibility-file-request.schema.json
  compatibility-file-delete-request.schema.json
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
  receipt-fetch-request.v4.json
  receipt-fetch-invocation.v4.json
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
  directory-resolve-request.v4.json
  directory-resolve-invocation.v4.json
  directory-device-page.v4.json
  directory-agent-page.v4.json
  directory-ability-page.v4.json
  directory-resolved-ref.v4.json
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
  surface-list-pages-request.v4.json
  surface-list-pages-invocation.v4.json
  surface-create-page-request.v4.json
  surface-create-page-invocation.v4.json
  surface-delete-page-request.v4.json
  surface-delete-page-invocation.v4.json
  surface-manifest-request.v4.json
  surface-manifest-invocation.v4.json
  surface-page-record.v4.json
  surface-page-page.v4.json
  surface-public-page-ref.v4.json
  surface-manifest.v4.json
  surface-health.v4.json
  surface-mutation-result.v4.json
  compatibility-list-models-request.v4.json
  compatibility-chat-completion-request.v4.json
  compatibility-stream-chat-completion-request.v4.json
  compatibility-list-models-invocation.v4.json
  compatibility-chat-completion-invocation.v4.json
  compatibility-stream-chat-completion-invocation.v4.json
  compatibility-model-page.v4.json
  compatibility-chat-completion.v4.json
  compatibility-chat-stream.v4.json
  compatibility-file-upload-request.v4.json
  compatibility-file-request.v4.json
  compatibility-file.v4.json
  compatibility-file-delete-request.v4.json
  compatibility-file-delete-result.v4.json
  wrapper-file-record.v4.json
  wrapper-terminal-session.v4.json
  wrapper-remote-desktop-session.v4.json
  wrapper-browser-session.v4.json
  wrapper-media-session.v4.json
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
  receipt-fetch-carrier.yaml
  receipt-projection-causal-ref.yaml
  stream-bidi-lifecycle-state.yaml
  host-binding-codec-hash.yaml
  publication-resource-carriers.yaml
  mission-carrier-status.yaml
  events-directory-stream.yaml
  admin-gateway-carrier-status.yaml
  surface-page-carriers.yaml
  compatibility-openai-carrier-projection.yaml
  wrapper-profile-records.yaml
  health-api-vs-runtime.yaml
  directory-list-pagination.yaml
  directory-no-default-fanout.yaml
  directory-resolve.yaml
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
require_literal src/bin/sdk-conformance-runner.rs "ConformanceResultRecord"
require_literal src/bin/sdk-conformance-runner.rs "CONFORMANCE_MANIFEST_INVALID"
require_literal sdk/go/conformance_test.go "sdk/conformance/cases"
require_literal sdk/go/conformance_test.go "sdk/conformance/fixtures"
require_literal sdk/go/conformance_test.go "TestGoFacadeExecutesSharedRuntimeCoreConformanceCases"
require_literal sdk/go/conformance_test.go "TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases"
require_literal sdk/go/conformance_test.go "TestGoMissionFacadeExecutesSharedCarrierStatusConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoAdminGatewayFacadeExecutesSharedCarrierStatusConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoEventsFacadeExecutesSharedDirectoryStreamConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoSurfaceFacadeExecutesSharedPageCarrierConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoPublicationFacadeExecutesSharedCarrierConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoReceiptFacadeExecutesSharedProjectionConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoWrapperFacadeExecutesSharedProjectionConformanceCase"
require_literal sdk/python/tests/test_conformance.py "sdk/conformance/cases"
require_literal sdk/python/tests/test_conformance.py "sdk/conformance/fixtures"
require_literal sdk/python/tests/test_conformance.py "SharedConformanceFixtureTests"
require_literal sdk/python/tests/test_conformance.py "test_python_directory_identity_execute_shared_projection_cases"
require_literal sdk/python/tests/test_conformance.py "test_python_mission_executes_shared_carrier_status_conformance_case"
require_literal sdk/python/tests/test_conformance.py "test_python_admin_gateway_executes_shared_carrier_status_conformance_case"
require_literal sdk/python/tests/test_conformance.py "test_python_events_executes_shared_directory_stream_conformance_case"
require_literal sdk/python/tests/test_conformance.py "test_python_surface_executes_shared_page_carrier_conformance_case"
require_literal sdk/python/tests/test_conformance.py "test_python_publication_executes_shared_carrier_conformance_case"
require_literal sdk/python/tests/test_conformance.py "test_python_receipt_executes_shared_projection_conformance_case"
require_literal sdk/python/tests/test_conformance.py "test_python_wrappers_execute_shared_projection_conformance_case"
require_literal sdk/go/client.go "DiscoveryTransport"
require_literal sdk/go/daemon.go "DaemonHandle"
require_literal sdk/go/daemon.go "DaemonLifecycleState"
require_literal sdk/go/daemon.go "DaemonTransport"
require_literal sdk/go/daemon.go "StartConfig"
require_literal sdk/go/daemon.go "OpenRuntime"
require_literal sdk/go/daemon.go "ConnectLocal"
require_literal sdk/go/directory.go "DirectoryClient"
require_literal sdk/go/directory.go "DirectoryTransport"
require_literal sdk/go/directory.go "DefaultDirectoryPageSize"
require_literal sdk/go/directory.go "MaxDirectoryPageSize"
require_literal sdk/go/directory.go "ResolvedRef"
require_literal sdk/go/directory.go "DirectorySubscriptionRequest"
require_literal sdk/go/directory.go "MaxDirectorySubscriptionBufferedEvents"
require_literal sdk/go/identity.go "IdentityClient"
require_literal sdk/go/identity.go "IdentityTransport"
require_literal sdk/go/identity.go "IdentityProjection"
require_literal sdk/go/identity.go "ResourceRef"
require_literal sdk/go/identity.go "SigningKeyRecord"
require_literal sdk/go/identity.go "SigningKeyPage"
require_literal sdk/go/identity.go "SignerHandle"
require_literal sdk/go/receipt.go "ReceiptClient"
require_literal sdk/go/receipt.go "ReceiptTransport"
require_literal sdk/go/receipt.go "ReceiptSummary"
require_literal sdk/go/receipt.go "ReceiptVerification"
require_literal sdk/go/receipt.go "CausalRef"
require_literal sdk/go/publication.go "PublicationClient"
require_literal sdk/go/publication.go "PublicationTransport"
require_literal sdk/go/publication.go "AbilityDeployRequest"
require_literal sdk/go/publication.go "PackageValidation"
require_literal sdk/go/publication.go "PublishedAbility"
require_literal sdk/go/host_binding.go "HostBindingClient"
require_literal sdk/go/host_binding.go "HostBindingTransport"
require_literal sdk/go/host_binding.go "HostStreamBinding"
require_literal sdk/go/host_binding.go "HostStreamFrame"
require_literal sdk/go/host_binding.go "HostStreamHashState"
require_literal sdk/go/mission.go "MissionClient"
require_literal sdk/go/mission.go "MissionTransport"
require_literal sdk/go/mission.go "MissionRunRequest"
require_literal sdk/go/mission.go "MissionStatus"
require_literal sdk/go/mission.go "MissionChildReceipt"
require_literal sdk/go/admin.go "AdminClient"
require_literal sdk/go/admin.go "AdminTransport"
require_literal sdk/go/admin.go "AdminAgentStartRequest"
require_literal sdk/go/admin.go "GatewayStatus"
require_literal sdk/go/admin.go "AdminGatewayResult"
require_literal sdk/go/admin.go "PairingToken"
require_literal sdk/go/admin.go "DeviceCredential"
require_literal sdk/go/admin.go "DeviceSession"
require_literal sdk/go/events.go "EventClient"
require_literal sdk/go/events.go "EventTransport"
require_literal sdk/go/events.go "EventsSubscriptionRequest"
require_literal sdk/go/events.go "EventsDirectorySubscriptionRequest"
require_literal sdk/go/events.go "EventsDeviceSubscriptionRequest"
require_literal sdk/go/events.go "EventsSessionSubscriptionRequest"
require_literal sdk/go/events.go "EventsInvocationSubscriptionRequest"
require_literal sdk/go/events.go "EventsDeviceEventListRequest"
require_literal sdk/go/events.go "DeviceEventPage"
require_literal sdk/go/events.go "EventFrame"
require_literal sdk/go/events.go "EventCursor"
require_literal sdk/go/surface.go "SurfaceClient"
require_literal sdk/go/surface.go "SurfaceTransport"
require_literal sdk/go/surface.go "SurfaceCreatePageRequest"
require_literal sdk/go/surface.go "SurfaceManifest"
require_literal sdk/go/surface.go "SurfacePublicPageRef"
require_literal sdk/go/surface.go "SurfaceHealth"
require_literal sdk/go/compatibility.go "CompatibilityClient"
require_literal sdk/go/compatibility.go "CompatibilityTransport"
require_literal sdk/go/compatibility.go "CompatibilityChatCompletionRequest"
require_literal sdk/go/compatibility.go "CompatibilityModelPage"
require_literal sdk/go/compatibility.go "BuildFileUploadInvocation"
require_literal sdk/go/compatibility.go "GetFile"
require_literal sdk/go/compatibility.go "CompatibilityFileDeleteResult"
require_literal sdk/go/wrappers.go "WrapperClient"
require_literal sdk/go/wrappers.go "WrapperTransport"
require_literal sdk/go/wrappers.go "WrapperFileTransferRequest"
require_literal sdk/go/wrappers.go "WrapperTerminalStartRequest"
require_literal sdk/go/wrappers.go "WrapperFileRecord"
require_literal sdk/go/wrappers.go "WrapperTerminalSessionRecord"
require_literal sdk/go/wrappers.go "WrapperRemoteDesktopSessionRecord"
require_literal sdk/go/wrappers.go "WrapperMediaSessionRecord"
require_literal sdk/go/connection.go "RuntimeConnection"
require_literal sdk/go/connection.go "ConnectionState"
require_literal sdk/go/connection.go "RuntimeConnector"
require_literal sdk/go/connection.go "ConnectOptions"
require_literal sdk/go/errors.go "DecodeDaemonErrorJSON"
require_literal sdk/go/errors.go "RuntimeError"
require_literal sdk/go/health.go "HealthClient"
require_literal sdk/go/health.go "RuntimeHealth"
require_literal sdk/go/health.go "NewRuntimeHealthFromJSON"
require_literal sdk/go/invocation.go "InvocationBuilder"
require_literal sdk/go/invocation.go "InvocationDraft"
require_literal sdk/go/runtime.go "RuntimeClient"
require_literal sdk/go/runtime.go "RuntimeTransport"
require_literal sdk/go/runtime.go "InvokeStream"
require_literal sdk/go/runtime.go "OpenBidi"
require_literal sdk/go/runtime.go "InvocationResult"
require_literal sdk/go/runtime.go "InvocationCancel"
require_literal sdk/go/runtime.go "Invoke"
require_literal sdk/go/runtime.go "Await"
require_literal sdk/go/runtime.go "Cancel"
require_literal sdk/go/runtime.go "SubmitSigned"
require_literal sdk/go/runtime.go "PrepareBuilder"
require_literal sdk/go/signing.go "PreparedInvocation"
require_literal sdk/go/signing.go "SignedInvocation"
require_literal sdk/go/signing.go "SigningMaterial"
require_literal sdk/go/stream.go "StreamHandle"
require_literal sdk/go/stream.go "StreamState"
require_literal sdk/go/stream.go "StreamTransport"
require_literal sdk/go/stream.go "MaxStreamBufferedEvents"
require_literal sdk/go/bidi.go "BidiSession"
require_literal sdk/go/bidi.go "BidiState"
require_literal sdk/go/bidi.go "BidiTransport"
require_literal sdk/go/bidi.go "MaxBidiBufferedFrames"
require_literal sdk/go/import_boundary_test.go "TestPublicGoSDKDoesNotImportForbiddenRuntimeBoundaries"
require_literal sdk/python/easynet_sdk/client.py "DiscoveryTransport"
require_literal sdk/python/easynet_sdk/daemon.py "DaemonHandle"
require_literal sdk/python/easynet_sdk/daemon.py "DaemonLifecycleState"
require_literal sdk/python/easynet_sdk/daemon.py "DaemonTransport"
require_literal sdk/python/easynet_sdk/daemon.py "StartConfig"
require_literal sdk/python/easynet_sdk/daemon.py "open_runtime"
require_literal sdk/python/easynet_sdk/daemon.py "connect_local"
require_literal sdk/python/easynet_sdk/directory.py "DirectoryClient"
require_literal sdk/python/easynet_sdk/directory.py "DirectoryTransport"
require_literal sdk/python/easynet_sdk/directory.py "DEFAULT_DIRECTORY_PAGE_SIZE"
require_literal sdk/python/easynet_sdk/directory.py "MAX_DIRECTORY_PAGE_SIZE"
require_literal sdk/python/easynet_sdk/directory.py "ResolvedRef"
require_literal sdk/python/easynet_sdk/directory.py "DirectorySubscriptionRequest"
require_literal sdk/python/easynet_sdk/directory.py "MAX_DIRECTORY_SUBSCRIPTION_BUFFERED_EVENTS"
require_literal sdk/python/easynet_sdk/identity.py "IdentityClient"
require_literal sdk/python/easynet_sdk/identity.py "IdentityTransport"
require_literal sdk/python/easynet_sdk/identity.py "IdentityProjection"
require_literal sdk/python/easynet_sdk/identity.py "ResourceRef"
require_literal sdk/python/easynet_sdk/identity.py "SigningKeyRecord"
require_literal sdk/python/easynet_sdk/identity.py "SigningKeyPage"
require_literal sdk/python/easynet_sdk/identity.py "SignerHandle"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptClient"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptTransport"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptSummary"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptVerification"
require_literal sdk/python/easynet_sdk/receipt.py "CausalRef"
require_literal sdk/python/easynet_sdk/publication.py "PublicationClient"
require_literal sdk/python/easynet_sdk/publication.py "PublicationTransport"
require_literal sdk/python/easynet_sdk/publication.py "AbilityDeployRequest"
require_literal sdk/python/easynet_sdk/publication.py "PackageValidation"
require_literal sdk/python/easynet_sdk/publication.py "PublishedAbility"
require_literal sdk/python/easynet_sdk/host_binding.py "HostBindingClient"
require_literal sdk/python/easynet_sdk/host_binding.py "HostBindingTransport"
require_literal sdk/python/easynet_sdk/host_binding.py "HostStreamBinding"
require_literal sdk/python/easynet_sdk/host_binding.py "HostStreamFrame"
require_literal sdk/python/easynet_sdk/host_binding.py "HostStreamHashState"
require_literal sdk/python/easynet_sdk/mission.py "MissionClient"
require_literal sdk/python/easynet_sdk/mission.py "MissionTransport"
require_literal sdk/python/easynet_sdk/mission.py "MissionRunRequest"
require_literal sdk/python/easynet_sdk/mission.py "MissionStatus"
require_literal sdk/python/easynet_sdk/mission.py "MissionChildReceipt"
require_literal sdk/python/easynet_sdk/admin.py "AdminClient"
require_literal sdk/python/easynet_sdk/admin.py "AdminTransport"
require_literal sdk/python/easynet_sdk/admin.py "AdminAgentStartRequest"
require_literal sdk/python/easynet_sdk/admin.py "GatewayStatus"
require_literal sdk/python/easynet_sdk/admin.py "AdminGatewayResult"
require_literal sdk/python/easynet_sdk/admin.py "PairingToken"
require_literal sdk/python/easynet_sdk/admin.py "DeviceCredential"
require_literal sdk/python/easynet_sdk/admin.py "DeviceSession"
require_literal sdk/python/easynet_sdk/events.py "EventClient"
require_literal sdk/python/easynet_sdk/events.py "EventTransport"
require_literal sdk/python/easynet_sdk/events.py "EventsSubscriptionRequest"
require_literal sdk/python/easynet_sdk/events.py "EventsDirectorySubscriptionRequest"
require_literal sdk/python/easynet_sdk/events.py "EventsDeviceSubscriptionRequest"
require_literal sdk/python/easynet_sdk/events.py "EventsSessionSubscriptionRequest"
require_literal sdk/python/easynet_sdk/events.py "EventsInvocationSubscriptionRequest"
require_literal sdk/python/easynet_sdk/events.py "EventsDeviceEventListRequest"
require_literal sdk/python/easynet_sdk/events.py "DeviceEventPage"
require_literal sdk/python/easynet_sdk/events.py "EventFrame"
require_literal sdk/python/easynet_sdk/events.py "EventCursor"
require_literal sdk/python/easynet_sdk/surface.py "SurfaceClient"
require_literal sdk/python/easynet_sdk/surface.py "SurfaceTransport"
require_literal sdk/python/easynet_sdk/surface.py "SurfaceCreatePageRequest"
require_literal sdk/python/easynet_sdk/surface.py "SurfaceManifest"
require_literal sdk/python/easynet_sdk/surface.py "SurfacePublicPageRef"
require_literal sdk/python/easynet_sdk/surface.py "SurfaceHealth"
require_literal sdk/python/easynet_sdk/compatibility.py "CompatibilityClient"
require_literal sdk/python/easynet_sdk/compatibility.py "CompatibilityTransport"
require_literal sdk/python/easynet_sdk/compatibility.py "CompatibilityChatCompletionRequest"
require_literal sdk/python/easynet_sdk/compatibility.py "CompatibilityModelPage"
require_literal sdk/python/easynet_sdk/compatibility.py "build_file_upload_invocation"
require_literal sdk/python/easynet_sdk/compatibility.py "get_file"
require_literal sdk/python/easynet_sdk/compatibility.py "CompatibilityFileDeleteResult"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperClient"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperTransport"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperFileTransferRequest"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperTerminalStartRequest"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperFileRecord"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperTerminalSessionRecord"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperRemoteDesktopSessionRecord"
require_literal sdk/python/easynet_sdk/wrappers.py "WrapperMediaSessionRecord"
require_literal sdk/python/easynet_sdk/connection.py "RuntimeConnection"
require_literal sdk/python/easynet_sdk/connection.py "ConnectionState"
require_literal sdk/python/easynet_sdk/connection.py "RuntimeConnector"
require_literal sdk/python/easynet_sdk/connection.py "ConnectOptions"
require_literal sdk/python/easynet_sdk/errors.py "from_json"
require_literal sdk/python/easynet_sdk/errors.py "RuntimeError"
require_literal sdk/python/easynet_sdk/health.py "HealthClient"
require_literal sdk/python/easynet_sdk/health.py "RuntimeHealth"
require_literal sdk/python/easynet_sdk/invocation.py "InvocationBuilder"
require_literal sdk/python/easynet_sdk/invocation.py "InvocationDraft"
require_literal sdk/python/easynet_sdk/invocation.py "inspect"
require_literal sdk/python/easynet_sdk/runtime.py "RuntimeClient"
require_literal sdk/python/easynet_sdk/runtime.py "RuntimeTransport"
require_literal sdk/python/easynet_sdk/runtime.py "invoke_stream"
require_literal sdk/python/easynet_sdk/runtime.py "open_bidi"
require_literal sdk/python/easynet_sdk/runtime.py "InvocationResult"
require_literal sdk/python/easynet_sdk/runtime.py "InvocationCancel"
require_literal sdk/python/easynet_sdk/runtime.py "invoke"
require_literal sdk/python/easynet_sdk/runtime.py "await_result"
require_literal sdk/python/easynet_sdk/runtime.py "cancel"
require_literal sdk/python/easynet_sdk/runtime.py "submit_signed"
require_literal sdk/python/easynet_sdk/runtime.py "prepare_builder"
require_literal sdk/python/easynet_sdk/signing.py "PreparedInvocation"
require_literal sdk/python/easynet_sdk/signing.py "SignedInvocation"
require_literal sdk/python/easynet_sdk/signing.py "SigningMaterial"
require_literal sdk/python/easynet_sdk/stream.py "StreamHandle"
require_literal sdk/python/easynet_sdk/stream.py "StreamState"
require_literal sdk/python/easynet_sdk/stream.py "StreamTransport"
require_literal sdk/python/easynet_sdk/stream.py "MAX_STREAM_BUFFERED_EVENTS"
require_literal sdk/python/easynet_sdk/bidi.py "BidiSession"
require_literal sdk/python/easynet_sdk/bidi.py "BidiState"
require_literal sdk/python/easynet_sdk/bidi.py "BidiTransport"
require_literal sdk/python/easynet_sdk/bidi.py "MAX_BIDI_BUFFERED_FRAMES"
require_literal sdk/python/tests/test_import_boundary.py "test_public_python_sdk_does_not_import_forbidden_runtime_boundaries"
require_literal sdk/go/conformance_test.go "TestGoRuntimeCoreExecutesSharedLifecycleVersionErrorConformanceCases"
require_literal sdk/go/conformance_test.go "TestGoRuntimeCoreExecutesSharedInvocationSigningConformanceCases"
require_literal sdk/go/conformance_test.go "TestGoRuntimeCoreExecutesSharedStreamBidiLifecycleConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoCompatibilityFacadeExecutesSharedOpenAICarrierConformanceCase"
require_literal sdk/python/tests/test_conformance.py "test_python_runtime_core_executes_shared_lifecycle_version_error_conformance_cases"
require_literal sdk/python/tests/test_conformance.py "test_python_runtime_core_executes_shared_invocation_signing_conformance_cases"
require_literal sdk/python/tests/test_conformance.py "test_python_runtime_core_executes_shared_stream_bidi_lifecycle_conformance_case"
require_literal sdk/python/tests/test_conformance.py "test_python_compatibility_executes_shared_openai_carrier_conformance_case"
require_literal sdk/SDK_INTERFACE_SPEC.md "PreparedInvocation"
require_literal sdk/SDK_INTERFACE_SPEC.md "SignedInvocation"
require_literal sdk/SDK_INTERFACE_SPEC.md "No public object in this graph may expose raw Axon"
require_literal sdk/SDK_PARITY.md "No current language is"
require_literal sdk/CONFORMANCE_SUITE.md "sdk-conformance-runner"

if [[ "${#failures[@]}" -eq 0 ]]; then
  printf 'check-sdk-scaffold ok\n'
  exit 0
fi

printf 'check-sdk-scaffold failed:\n' >&2
printf ' - %s\n' "${failures[@]}" >&2
exit 1
