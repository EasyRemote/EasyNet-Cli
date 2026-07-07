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

validate_declared_file_list() {
  local array_name="$1"
  local directory="$2"
  local suffix="$3"
  if ! python3 - "$ROOT" "$array_name" "$directory" "$suffix" "$0" <<'PY'
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
array_name = sys.argv[2]
directory = sys.argv[3]
suffix = sys.argv[4]
script_path = Path(sys.argv[5])

text = script_path.read_text()
match = re.search(rf"{re.escape(array_name)}=\(\n(.*?)\n\)", text, re.S)
if not match:
    print(f"missing declared list: {array_name}")
    raise SystemExit(1)

declared = {
    line.strip()
    for line in match.group(1).splitlines()
    if line.strip() and not line.strip().startswith("#")
}
actual = {path.name for path in (root / directory).glob(f"*{suffix}")}
missing = sorted(actual - declared)
extra = sorted(declared - actual)

if missing or extra:
    if missing:
        print(f"{array_name} is missing declared files: " + ", ".join(missing))
    if extra:
        print(f"{array_name} declares missing files: " + ", ".join(extra))
    raise SystemExit(1)
PY
  then
    fail "declared file list is not closed: $array_name"
  fi
}

validate_fixture_schema_bindings() {
  if ! python3 - "$ROOT" <<'PY'
import json
import sys
from collections import Counter
from pathlib import Path

root = Path(sys.argv[1])
manifest_path = root / "sdk/conformance/fixture-schema-bindings.json"
try:
    manifest = json.loads(manifest_path.read_text())
except Exception as exc:
    print(f"invalid fixture schema bindings: {exc}")
    raise SystemExit(1)

if manifest.get("schema_version") != 1:
    print("fixture schema bindings schema_version must be 1")
    raise SystemExit(1)

bindings = manifest.get("bindings")
if not isinstance(bindings, list) or not bindings:
    print("fixture schema bindings must contain a non-empty bindings array")
    raise SystemExit(1)

fixtures = sorted(path.name for path in (root / "sdk/conformance/fixtures").glob("*.v4.json"))
bound = [entry.get("fixture") for entry in bindings]
duplicates = sorted(name for name, count in Counter(bound).items() if count > 1)
missing = sorted(set(fixtures) - set(bound))
extra = sorted(set(bound) - set(fixtures))
bad_schemas = []

for entry in bindings:
    fixture = entry.get("fixture")
    schema = entry.get("schema")
    if not isinstance(fixture, str) or not fixture.endswith(".v4.json"):
        print(f"invalid fixture binding name: {fixture!r}")
        raise SystemExit(1)
    if not isinstance(schema, str) or not schema.endswith(".schema.json"):
        print(f"invalid fixture schema binding target for {fixture!r}: {schema!r}")
        raise SystemExit(1)
    if not (root / "sdk/schemas" / schema).is_file():
        bad_schemas.append(f"{fixture} -> {schema}")

if duplicates or missing or extra or bad_schemas:
    if duplicates:
        print("duplicate fixture schema bindings: " + ", ".join(duplicates))
    if missing:
        print("fixtures missing schema bindings: " + ", ".join(missing))
    if extra:
        print("fixture schema bindings without fixture files: " + ", ".join(extra))
    if bad_schemas:
        print("fixture schema bindings with missing schemas: " + ", ".join(bad_schemas))
    raise SystemExit(1)
PY
  then
    fail "invalid fixture schema bindings"
  fi
}

validate_c_abi_header() {
  if ! command -v cc >/dev/null 2>&1; then
    fail "cc is required to validate include/easynet_cli.h"
    return
  fi

  local tmp
  tmp="$(mktemp "${TMPDIR:-/tmp}/easynet-c-abi-header.XXXXXX.c")"
  printf '#include "include/easynet_cli.h"\n' >"$tmp"
  if ! cc -fsyntax-only -I"$ROOT" "$tmp" >/dev/null 2>&1; then
    fail "include/easynet_cli.h does not compile as C"
  fi
  rm -f "$tmp"

  if ! python3 - "$ROOT/include/easynet_cli.h" <<'PY'
import re
import sys
from collections import Counter
from pathlib import Path

header = Path(sys.argv[1]).read_text(encoding="utf-8")
names = re.findall(r"\bint32_t\s+(easynet_[A-Za-z0-9_]+)\s*\(", header)
duplicates = sorted(name for name, count in Counter(names).items() if count > 1)
if duplicates:
    print("duplicate C ABI declarations: " + ", ".join(duplicates))
    raise SystemExit(1)
PY
  then
    fail "include/easynet_cli.h contains duplicate declarations"
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
  sdk/go/environment.go \
  sdk/go/environment_test.go \
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
  sdk/go/live_smoke_cabi_test.go \
  tools/scripts/check-backend-sdk-only-boundary.sh \
  tools/scripts/check-backend-route-family-coverage.sh \
  tools/scripts/check-daemon-latest-input-boundary.sh \
  tools/scripts/check-sdk-completion-audit.sh \
  tools/scripts/check-sdk-conformance-reports.sh \
  tools/scripts/check-easyremote-sdk-boundary.sh \
  tools/scripts/check-sdk-cutover-readiness.sh \
  tools/scripts/check-sdk-package-metadata.sh \
  tools/scripts/check-sdk-parity-matrix.sh \
  tools/scripts/check-sdk-product-smokes.sh \
  tools/scripts/check-sdk-ura-naming.sh \
  tools/scripts/check-java-sdk-seam.sh \
  tools/scripts/check-swift-sdk-seam.sh \
  tools/scripts/go-sdk-live-smoke.sh \
  tools/scripts/python-sdk-live-smoke.sh \
  sdk/java/README.md \
  sdk/java/.gitignore \
  sdk/java/pom.xml \
  sdk/java/src/main/java/run/easynet/daemon/AsyncRuntimeClient.java \
  sdk/java/src/main/java/run/easynet/daemon/BidiFrame.java \
  sdk/java/src/main/java/run/easynet/daemon/BidiSession.java \
  sdk/java/src/main/java/run/easynet/daemon/BidiSource.java \
  sdk/java/src/main/java/run/easynet/daemon/Client.java \
  sdk/java/src/main/java/run/easynet/daemon/DiscoveryTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/AbilityQuery.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminAgentListRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminAgentPage.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminAgentRecord.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminAgentRefreshRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminAgentStartRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminAgentStopRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminCarrierBase.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminClient.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminGatewayResult.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminGatewayStatusRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminJoinHubRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminLeaveHubRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminSessionListRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/AdminTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/AuthorityClient.java \
  sdk/java/src/main/java/run/easynet/daemon/AuthorityMetadata.java \
  sdk/java/src/main/java/run/easynet/daemon/AuthoritySupport.java \
  sdk/java/src/main/java/run/easynet/daemon/AuthorityTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/CreateDeviceSessionRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/CreatePairingRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DeleteDeviceSessionRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DelegationProof.java \
  sdk/java/src/main/java/run/easynet/daemon/DelegationRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DescriptorRefBuildRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DescriptorRefRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DeviceCredential.java \
  sdk/java/src/main/java/run/easynet/daemon/DeviceCredentialVerification.java \
  sdk/java/src/main/java/run/easynet/daemon/DeviceSession.java \
  sdk/java/src/main/java/run/easynet/daemon/DeviceSessionPage.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryClient.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryIdentitySupport.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryListRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryPage.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryQueryBase.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryResolveRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryResolvedRef.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectorySubscription.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectorySubscriptionCursor.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectorySubscriptionRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/DirectoryTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/ErrorClass.java \
  sdk/java/src/main/java/run/easynet/daemon/ErrorCode.java \
  sdk/java/src/main/java/run/easynet/daemon/AbilityDeployRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/AbilityPackageManifest.java \
  sdk/java/src/main/java/run/easynet/daemon/DeviceEventPage.java \
  sdk/java/src/main/java/run/easynet/daemon/EventClient.java \
  sdk/java/src/main/java/run/easynet/daemon/EventCursor.java \
  sdk/java/src/main/java/run/easynet/daemon/EventDropReportInput.java \
  sdk/java/src/main/java/run/easynet/daemon/EventFilter.java \
  sdk/java/src/main/java/run/easynet/daemon/EventFrame.java \
  sdk/java/src/main/java/run/easynet/daemon/EventProjectionInput.java \
  sdk/java/src/main/java/run/easynet/daemon/EventStream.java \
  sdk/java/src/main/java/run/easynet/daemon/EventTerminalInput.java \
  sdk/java/src/main/java/run/easynet/daemon/EventTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/EventsCarrierBase.java \
  sdk/java/src/main/java/run/easynet/daemon/EventsDeviceEventListRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/EventsSubscriptionRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/EventsSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/FeatureSet.java \
  sdk/java/src/main/java/run/easynet/daemon/GatewayListener.java \
  sdk/java/src/main/java/run/easynet/daemon/GatewayStatus.java \
  sdk/java/src/main/java/run/easynet/daemon/DiagnosticCheck.java \
  sdk/java/src/main/java/run/easynet/daemon/DiagnosticsReport.java \
  sdk/java/src/main/java/run/easynet/daemon/DiagnosticsTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/HealthClient.java \
  sdk/java/src/main/java/run/easynet/daemon/HealthTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/HostBindingClient.java \
  sdk/java/src/main/java/run/easynet/daemon/HostBindingSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/HostBindingTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamBinding.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamBindingRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamCleanup.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamEnvelope.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamFrame.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamHashState.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamLifecycleController.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamLifecycleProvider.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamLifecycleState.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamReadiness.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/HostStreamTerminalSummary.java \
  sdk/java/src/main/java/run/easynet/daemon/IdentityClient.java \
  sdk/java/src/main/java/run/easynet/daemon/IdentityProjection.java \
  sdk/java/src/main/java/run/easynet/daemon/IdentityTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/InvocationBuilder.java \
  sdk/java/src/main/java/run/easynet/daemon/InvocationDraft.java \
  sdk/java/src/main/java/run/easynet/daemon/InvocationHandle.java \
  sdk/java/src/main/java/run/easynet/daemon/InvocationResult.java \
  sdk/java/src/main/java/run/easynet/daemon/InvocationSignature.java \
  sdk/java/src/main/java/run/easynet/daemon/InvocationTerminalState.java \
  sdk/java/src/main/java/run/easynet/daemon/InvocationTuple.java \
  sdk/java/src/main/java/run/easynet/daemon/JsonValueReader.java \
  sdk/java/src/main/java/run/easynet/daemon/JsonValueWriter.java \
  sdk/java/src/main/java/run/easynet/daemon/LocalResourceRefRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionCancelRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionCarrierBase.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionChildInvocation.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionChildReceipt.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionClient.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionEvent.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionEventPage.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionEventStream.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionEventsRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionOutputRef.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionRun.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionRunFileRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionRunRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionStatus.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionTrackRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/MissionTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/OwnerAbilityRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/PackageValidation.java \
  sdk/java/src/main/java/run/easynet/daemon/PairingPreflight.java \
  sdk/java/src/main/java/run/easynet/daemon/PairingPreflightRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/PairingToken.java \
  sdk/java/src/main/java/run/easynet/daemon/PreparedInvocation.java \
  sdk/java/src/main/java/run/easynet/daemon/PublicationClient.java \
  sdk/java/src/main/java/run/easynet/daemon/PublicationSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/PublicationTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptChain.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptChainVerification.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptClient.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptFetchRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptRef.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptSummary.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/ReceiptVerification.java \
  sdk/java/src/main/java/run/easynet/daemon/RetryHint.java \
  sdk/java/src/main/java/run/easynet/daemon/ResolveQuery.java \
  sdk/java/src/main/java/run/easynet/daemon/ResourceRef.java \
  sdk/java/src/main/java/run/easynet/daemon/RevokeDeviceRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/RuntimeClient.java \
  sdk/java/src/main/java/run/easynet/daemon/RuntimeHealth.java \
  sdk/java/src/main/java/run/easynet/daemon/RuntimeFuture.java \
  sdk/java/src/main/java/run/easynet/daemon/RuntimeTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/SDKError.java \
  sdk/java/src/main/java/run/easynet/daemon/SessionAuthority.java \
  sdk/java/src/main/java/run/easynet/daemon/SessionAuthorityRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/SignedInvocation.java \
  sdk/java/src/main/java/run/easynet/daemon/SigningMaterial.java \
  sdk/java/src/main/java/run/easynet/daemon/StreamEvent.java \
  sdk/java/src/main/java/run/easynet/daemon/StreamHandle.java \
  sdk/java/src/main/java/run/easynet/daemon/StreamSource.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceCarrierBase.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceClient.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceCreatePageRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceDeletePageRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceHealth.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceHealthCheck.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceHealthRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceListPagesRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceManifest.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceManifestRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceMutationResult.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfacePagePage.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfacePageRecord.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfacePublicPageRef.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/SurfaceTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityCarrierBase.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityChatCompletion.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityChatCompletionChunk.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityChatCompletionRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityChatCompletionStream.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityClient.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityFile.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityFileDeleteRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityFileDeleteResult.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityFileRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityFileUploadRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityListModelsRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityModel.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityModelPage.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityStreamChatCompletionRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilitySupport.java \
  sdk/java/src/main/java/run/easynet/daemon/CompatibilityTransport.java \
  sdk/java/src/main/java/run/easynet/daemon/UnpublishAbilityRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/ValidatePackageOptions.java \
  sdk/java/src/main/java/run/easynet/daemon/ValidatePairingRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/VerifyDeviceCredentialRequest.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperBrowserSession.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperClient.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperFileRecord.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperMediaSession.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperRemoteDesktopSession.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperSessionRecord.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperSupport.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperTerminalSession.java \
  sdk/java/src/main/java/run/easynet/daemon/WrapperTransport.java \
  sdk/java/src/test/java/run/easynet/daemon/RuntimeCoreSeamTest.java \
  sdk/swift/.gitignore \
  sdk/swift/Package.swift \
  sdk/swift/README.md \
  sdk/swift/Sources/EasyNetDaemonSDK/Admin.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Authority.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Bidi.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Client.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Compatibility.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/DirectoryIdentity.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Events.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Health.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/HostBinding.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Invocation.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Mission.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Publication.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Receipt.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Runtime.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/SDKError.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Stream.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Surface.swift \
  sdk/swift/Sources/EasyNetDaemonSDK/Wrappers.swift \
  sdk/swift/Tests/EasyNetDaemonSDKTests/RuntimeCoreSeamTests.swift \
  sdk/conformance/backend-route-family-coverage.json \
  sdk/conformance/fixture-schema-bindings.json \
  sdk/conformance/sdk-parity-matrix.json \
  sdk/conformance/runner/rust-action-adapter-report.json \
  sdk/conformance/runner/c-abi-action-adapter-report.json \
  sdk/conformance/runner/go-action-adapter-report.json \
  sdk/conformance/runner/python-action-adapter-report.json \
  sdk/conformance/runner/node-action-adapter-report.json \
  sdk/conformance/runner/java-action-adapter-report.json \
  sdk/conformance/runner/swift-action-adapter-report.json \
  sdk/python/pyproject.toml \
  sdk/python/easynet_sdk/client.py \
  sdk/python/easynet_sdk/_cabi.py \
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
  sdk/python/easynet_sdk/environment.py \
  sdk/python/easynet_sdk/errors.py \
  sdk/python/easynet_sdk/health.py \
  sdk/python/easynet_sdk/invocation.py \
  sdk/python/easynet_sdk/runtime.py \
  sdk/python/easynet_sdk/bidi.py \
  sdk/python/easynet_sdk/signing.py \
  sdk/python/easynet_sdk/stream.py \
  sdk/python/tests/test_health.py \
  sdk/python/tests/test_cabi.py \
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
  sdk/python/tests/test_environment.py \
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
validate_json_file sdk/conformance/backend-route-family-coverage.json
validate_json_file sdk/conformance/fixture-schema-bindings.json
validate_fixture_schema_bindings
validate_c_abi_header
validate_json_file sdk/conformance/sdk-parity-matrix.json
validate_json_file sdk/conformance/runner/rust-action-adapter-report.json
validate_json_file sdk/conformance/runner/c-abi-action-adapter-report.json
validate_json_file sdk/conformance/runner/go-action-adapter-report.json
validate_json_file sdk/conformance/runner/python-action-adapter-report.json
validate_json_file sdk/conformance/runner/node-action-adapter-report.json
validate_json_file sdk/conformance/runner/java-action-adapter-report.json
validate_json_file sdk/conformance/runner/swift-action-adapter-report.json

schema_files=(
  invocation.schema.json
  prepared-invocation.schema.json
  identity.schema.json
  receipt.schema.json
  receipt-ref.schema.json
  receipt-fetch-request.schema.json
  error.schema.json
  feature-discovery.schema.json
  health.schema.json
  events.schema.json
  events-directory-subscription-request.schema.json
  events-device-subscription-request.schema.json
  events-session-subscription-request.schema.json
  events-invocation-subscription-request.schema.json
  events-device-event-list-request.schema.json
  events-device-event-page.schema.json
  directory-list-devices-request.schema.json
  directory-list-agents-request.schema.json
  directory-list-abilities-request.schema.json
  directory-resolve-request.schema.json
  directory-subscription-request.schema.json
  directory-page.schema.json
  directory-resolved-ref.schema.json
  directory-subscription.schema.json
  diagnostics.schema.json
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
  mission-events-request.schema.json
  mission-status.schema.json
  mission-event-page.schema.json
  admin.schema.json
  gateway.schema.json
  agent-record.schema.json
  admin-agent-list-request.schema.json
  admin-agent-start-request.schema.json
  admin-agent-stop-request.schema.json
  admin-agent-refresh-request.schema.json
  admin-session-list-request.schema.json
  admin-pairing-preflight-request.schema.json
  admin-pairing-create-request.schema.json
  admin-pairing-validate-request.schema.json
  admin-device-session-create-request.schema.json
  admin-device-session-delete-request.schema.json
  surface-page.schema.json
  surface-list-pages-request.schema.json
  surface-create-page-request.schema.json
  surface-delete-page-request.schema.json
  surface-manifest-request.schema.json
  surface-page-project-request.schema.json
  surface-health-request.schema.json
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
validate_declared_file_list schema_files sdk/schemas .schema.json

fixture_files=(
  invocation.complete.v4.json
  prepared.signing-material.v4.json
  identity.descriptor-ref.v4.json
  receipt-fetch-request.v4.json
  receipt-fetch-invocation.v4.json
  receipt-ref.v4.json
  receipt.summary.v4.json
  runtime.error.v4.json
  event.directory.v4.json
  event.device-page.v4.json
  event.device-live.v4.json
  event.directory-drop-report.v4.json
  event.directory-terminal.v4.json
  event.invocation-live.v4.json
  events-directory-subscription-request.v4.json
  events-directory-subscription-invocation.v4.json
  events-device-subscription-request.v4.json
  events-device-subscription-invocation.v4.json
  events-device-event-list-request.v4.json
  events-device-history-invocation.v4.json
  events-session-subscription-request.v4.json
  events-session-subscription-invocation.v4.json
  events-invocation-subscription-request.v4.json
  events-invocation-subscription-invocation.v4.json
  feature-discovery.v4.json
  directory-list-devices-request.v4.json
  directory-list-agents-request.v4.json
  directory-list-abilities-request.v4.json
  directory-list-devices-invocation.v4.json
  directory-list-agents-invocation.v4.json
  directory-list-abilities-invocation.v4.json
  directory-resolve-request.v4.json
  directory-resolve-invocation.v4.json
  directory-subscription-request.v4.json
  directory-subscription-invocation.v4.json
  directory-subscription.v4.json
  directory-device-page.v4.json
  directory-agent-page.v4.json
  directory-ability-page.v4.json
  directory-resolved-ref.v4.json
  diagnostics.ready.v4.json
  health.ready.v4.json
  host-stream-binding-request.v4.json
  host-stream-binding.v4.json
  host-stream-request.v4.json
  host-stream-frame.v4.json
  host-stream-terminal.v4.json
  host-stream-hash-state.v4.json
  host-stream-hash-state-corrupted-gap.v4.json
  host-stream-hash-state-corrupted-zero.v4.json
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
  mission-events-request.v4.json
  mission-run-invocation.v4.json
  mission-track-invocation.v4.json
  mission-cancel-invocation.v4.json
  mission-status.v4.json
  mission-event-page.v4.json
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
  admin-pairing-preflight-request.v4.json
  admin-pairing-preflight.v4.json
  admin-pairing-create-request.v4.json
  admin-pairing-token.v4.json
  admin-pairing-validate-request.v4.json
  admin-device-credential.v4.json
  admin-device-session-create-request.v4.json
  admin-device-session.v4.json
  admin-device-session-page.v4.json
  admin-device-session-delete-request.v4.json
  admin-device-session-delete-result.v4.json
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
  surface-health-request.v4.json
  surface-health-invocation.v4.json
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
  authority-metadata.v4.json
)

for fixture in "${fixture_files[@]}"; do
  path="sdk/conformance/fixtures/$fixture"
  require_file "$path"
  validate_json_file "$path"
done
validate_declared_file_list fixture_files sdk/conformance/fixtures .v4.json

case_files=(
  version-abi-compatible.yaml
  version-abi-incompatible.yaml
  environment-process-root.yaml
  daemon-control-only.yaml
  invocation-complete-tuple.yaml
  invocation-builder-handle-state.yaml
  invocation-handle-terminal-monotonicity.yaml
  invocation-canonical-material.yaml
  invocation-prepared-not-submittable.yaml
  invocation-presigned-submit.yaml
  invocation-local-daemon-signing-boundary.yaml
  authority-mutual-exclusion.yaml
  invocation-descriptor-ref-helper-delegation.yaml
  error-typed-json.yaml
  error-profile-source-refs.yaml
  backend-sdk-only-import-ban.yaml
  backend-route-family-coverage.yaml
  sdk-go-python-parity-matrix.yaml
  identity-ura-descriptor-projection.yaml
  receipt-fetch-carrier.yaml
  receipt-projection-causal-ref.yaml
  receipt-axon-chain-verification.yaml
  stream-bidi-lifecycle-state.yaml
  stream-backpressure-bound.yaml
  host-binding-codec-hash.yaml
  publication-resource-carriers.yaml
  mission-carrier-status.yaml
  mission-plan-child-invocation.yaml
  events-directory-stream.yaml
  events-device-invocation-history.yaml
  events-session-stream.yaml
  admin-gateway-carrier-status.yaml
  surface-page-carriers.yaml
  compatibility-openai-carrier-projection.yaml
  wrapper-profile-records.yaml
  python-easyremote-no-raw-ffi.yaml
  python-easyremote-no-invocation-codec.yaml
  python-easyremote-no-raw-receipt-continuity.yaml
  python-easyremote-context-causal.yaml
  python-easyremote-profile-extraction.yaml
  health-api-vs-runtime.yaml
  directory-list-pagination.yaml
  directory-no-default-fanout.yaml
  directory-resolve.yaml
  directory-subscription-stream.yaml
  memc-profile-exclusivity.yaml
  memc-consumer-coverage.yaml
  memc-no-core-bloat.yaml
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
validate_declared_file_list case_files sdk/conformance/cases .yaml

bash "$ROOT/tools/scripts/check-backend-sdk-only-boundary.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-backend-route-family-coverage.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-sdk-completion-audit.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-sdk-conformance-reports.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-easyremote-sdk-boundary.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-sdk-package-metadata.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-sdk-parity-matrix.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-sdk-product-smokes.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-sdk-ura-naming.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/python-sdk-live-smoke.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/go-sdk-live-smoke.sh" --self-test >/dev/null

require_file sdk/conformance/runner/README.md
require_literal src/bin/sdk-conformance-runner.rs "ConformanceResultRecord"
require_literal src/bin/sdk-conformance-runner.rs "CONFORMANCE_MANIFEST_INVALID"
require_literal src/bin/sdk-conformance-runner.rs "ManifestCaseIndex"
require_literal src/bin/sdk-conformance-runner.rs "does not match any manifest case"
require_literal src/bin/sdk-conformance-runner.rs "is not declared for language"
require_literal src/bin/sdk-conformance-runner.rs "FixtureSchemaBindings"
require_literal src/bin/sdk-conformance-runner.rs "schema validation against"
require_literal sdk/conformance/runner/README.md "closed over the shared manifest"
require_literal sdk/conformance/runner/README.md "fixture-schema-bindings.json"
require_literal sdk/go/conformance_test.go "sdk/conformance/cases"
require_literal sdk/go/conformance_test.go "sdk/conformance/fixtures"
require_literal sdk/go/conformance_test.go "TestGoFacadeExecutesSharedRuntimeCoreConformanceCases"
require_literal sdk/go/conformance_test.go "TestGoEnvironmentExecutesSharedProcessRootConformanceCase"
require_literal sdk/go/conformance_test.go "environment/process_root"
require_literal sdk/go/conformance_test.go "TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases"
require_literal sdk/go/conformance_test.go "TestGoMissionFacadeExecutesSharedCarrierStatusConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoAdminGatewayFacadeExecutesSharedCarrierStatusConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoEventsFacadeExecutesSharedDirectoryStreamConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoSurfaceFacadeExecutesSharedPageCarrierConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoPublicationFacadeExecutesSharedCarrierConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoReceiptFacadeExecutesSharedProjectionConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoWrapperFacadeExecutesSharedProjectionConformanceCase"
require_literal sdk/go/conformance_test.go "TestGoMEMCExecutesSharedProfileExclusivityConformanceCase"
require_literal sdk/go/conformance_test.go "memc/profile_exclusivity"
require_literal sdk/go/conformance_test.go "TestGoMEMCExecutesSharedConsumerCoverageConformanceCase"
require_literal sdk/go/conformance_test.go "memc/consumer_coverage"
require_literal sdk/go/conformance_test.go "TestGoMEMCExecutesSharedNoCoreBloatConformanceCase"
require_literal sdk/go/conformance_test.go "memc/no_core_bloat"
require_literal sdk/go/conformance_test.go "TestGoBackendCutoverExecutesSharedRouteFamilyCoverageConformanceCase"
require_literal sdk/go/conformance_test.go "backend/hub_route_family_coverage"
require_literal sdk/go/conformance_test.go "TestGoSDKExecutesSharedParityMatrixConformanceCase"
require_literal sdk/go/conformance_test.go "sdk/go_python_parity_matrix"
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
require_literal sdk/python/tests/test_conformance.py "test_python_memc_executes_shared_profile_exclusivity_conformance_case"
require_literal sdk/python/tests/test_conformance.py "memc/profile_exclusivity"
require_literal sdk/python/tests/test_conformance.py "test_python_memc_executes_shared_consumer_coverage_conformance_case"
require_literal sdk/python/tests/test_conformance.py "memc/consumer_coverage"
require_literal sdk/python/tests/test_conformance.py "test_python_memc_executes_shared_no_core_bloat_conformance_case"
require_literal sdk/python/tests/test_conformance.py "memc/no_core_bloat"
require_literal sdk/python/tests/test_conformance.py "test_python_sdk_executes_shared_parity_matrix_conformance_case"
require_literal sdk/python/tests/test_conformance.py "sdk/go_python_parity_matrix"
require_literal sdk/go/client.go "DiscoveryTransport"
require_literal sdk/go/environment.go "SdkEnvironment"
require_literal sdk/go/environment.go "NewSdkEnvironment"
require_literal sdk/go/environment.go "ConnectLocal"
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
require_literal sdk/go/receipt.go "BuildReceiptFetchInvocation"
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
require_literal sdk/go/health.go "DiagnosticsReport"
require_literal sdk/go/health.go "NewRuntimeHealthFromJSON"
require_literal sdk/go/health.go "NewDiagnosticsReportFromJSON"
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
require_literal sdk/python/easynet_sdk/_cabi.py "CLILibrary"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIDiscoveryTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIDaemonTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIIdentityTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIRuntimeTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIReceiptTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIPublicationTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIHostBindingTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIMissionTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIAdminTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABIEventTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABISurfaceTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "CABICompatibilityTransport"
require_literal sdk/python/easynet_sdk/_cabi.py "open_cabi_daemon_transport"
require_literal sdk/python/easynet_sdk/_cabi.py "open_cabi_runtime_transport"
require_literal sdk/python/easynet_sdk/_cabi.py "open_cabi_receipt_transport"
require_literal sdk/python/easynet_sdk/_cabi.py "open_cabi_host_binding_transport"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_daemon_start"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_daemon_open_client"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_invocation_invoke"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_runtime_health"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_invocation_stream_open"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_invocation_bidi_open"
require_literal sdk/python/easynet_sdk/_cabi.py "EXPECTED_ABI_VERSION"
require_literal sdk/python/easynet_sdk/environment.py "SdkEnvironment"
require_literal sdk/python/easynet_sdk/environment.py "default_environment"
require_literal sdk/python/easynet_sdk/environment.py "connect_local"
require_literal sdk/python/easynet_sdk/environment.py "receipt_client"
require_literal sdk/python/easynet_sdk/environment.py "publication_client"
require_literal sdk/python/easynet_sdk/environment.py "host_binding_client"
require_literal sdk/python/easynet_sdk/environment.py "mission_client"
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
require_literal sdk/python/easynet_sdk/directory.py "build_list_devices_invocation"
require_literal sdk/python/easynet_sdk/directory.py "build_resolve_invocation"
require_literal sdk/python/easynet_sdk/directory.py "project_resolved_ref"
require_literal sdk/python/easynet_sdk/directory.py "MAX_DIRECTORY_SUBSCRIPTION_BUFFERED_EVENTS"
require_literal sdk/python/easynet_sdk/identity.py "IdentityClient"
require_literal sdk/python/easynet_sdk/identity.py "IdentityTransport"
require_literal sdk/python/easynet_sdk/identity.py "IdentityProjection"
require_literal sdk/python/easynet_sdk/identity.py "ResourceRef"
require_literal sdk/python/easynet_sdk/identity.py "parse_ura"
require_literal sdk/python/easynet_sdk/identity.py "owner_ability_ura"
require_literal sdk/python/easynet_sdk/identity.py "owner_ura_for_ability"
require_literal sdk/python/easynet_sdk/identity.py "canonical_ability_descriptor_ref"
require_literal sdk/python/easynet_sdk/identity.py "SigningKeyRecord"
require_literal sdk/python/easynet_sdk/identity.py "SigningKeyPage"
require_literal sdk/python/easynet_sdk/identity.py "SignerHandle"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptClient"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptTransport"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptSummary"
require_literal sdk/python/easynet_sdk/receipt.py "ReceiptVerification"
require_literal sdk/python/easynet_sdk/receipt.py "CausalRef"
require_literal sdk/python/easynet_sdk/receipt.py "build_receipt_fetch_invocation"
require_literal sdk/python/easynet_sdk/receipt.py "descriptor_ref"
require_literal sdk/schemas/receipt-fetch-request.schema.json '"descriptor_ref"'
require_literal sdk/conformance/fixtures/receipt-fetch-request.v4.json '"descriptor_ref"'
require_literal sdk/conformance/cases/invocation-descriptor-ref-helper-delegation.yaml "invocation/descriptor_ref_helper_delegation"
require_literal sdk/conformance/cases/invocation-descriptor-ref-helper-delegation.yaml "canonical_helper_owner: axon"
require_literal sdk/conformance/cases/invocation-descriptor-ref-helper-delegation.yaml "facade_descriptor_concat: false"
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
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_compatibility_build_file_upload_invocation"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_compatibility_build_file_retrieve_invocation"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_compatibility_build_file_delete_invocation"
require_literal include/easynet_cli.h "easynet_compatibility_build_file_upload_invocation"
require_literal include/easynet_cli.h "easynet_compatibility_build_file_retrieve_invocation"
require_literal include/easynet_cli.h "easynet_compatibility_build_file_delete_invocation"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_wrappers_build_file_transfer_invocation"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_wrappers_build_terminal_session_invocation"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_wrappers_build_remote_desktop_session_invocation"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_wrappers_build_browser_session_invocation"
require_literal sdk/python/easynet_sdk/_cabi.py "easynet_wrappers_build_media_session_invocation"
require_literal include/easynet_cli.h "easynet_wrappers_build_file_transfer_invocation"
require_literal include/easynet_cli.h "easynet_wrappers_build_terminal_session_invocation"
require_literal include/easynet_cli.h "easynet_wrappers_build_remote_desktop_session_invocation"
require_literal include/easynet_cli.h "easynet_wrappers_build_browser_session_invocation"
require_literal include/easynet_cli.h "easynet_wrappers_build_media_session_invocation"
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
require_literal sdk/python/easynet_sdk/health.py "DiagnosticsReport"
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
require_literal sdk/go/conformance_test.go "receipt/fetch_carrier"
require_literal sdk/go/conformance_test.go "invocation/descriptor_ref_helper_delegation"
require_literal sdk/go/conformance_test.go "TestGoCompatibilityFacadeExecutesSharedOpenAICarrierConformanceCase"
require_literal sdk/python/tests/test_conformance.py "test_python_runtime_core_executes_shared_lifecycle_version_error_conformance_cases"
require_literal sdk/python/tests/test_conformance.py "test_python_runtime_core_executes_shared_invocation_signing_conformance_cases"
require_literal sdk/python/tests/test_conformance.py "test_python_runtime_core_executes_shared_stream_bidi_lifecycle_conformance_case"
require_literal sdk/python/tests/test_conformance.py "receipt/fetch_carrier"
require_literal sdk/python/tests/test_conformance.py "invocation/descriptor_ref_helper_delegation"
require_literal sdk/python/tests/test_conformance.py "owner_ability_ura"
require_literal sdk/python/tests/test_conformance.py "canonical_ability_descriptor_ref"
require_literal sdk/python/tests/test_conformance.py "test_python_compatibility_executes_shared_openai_carrier_conformance_case"
require_literal sdk/SDK_INTERFACE_SPEC.md "PreparedInvocation"
require_literal sdk/SDK_INTERFACE_SPEC.md "SignedInvocation"
require_literal sdk/SDK_INTERFACE_SPEC.md "No public object in this graph may expose raw Axon"
require_literal sdk/README.md "sdk/conformance/sdk-parity-matrix.json"
require_literal sdk/README.md "Go facade | provider-backed"
require_literal sdk/README.md "Python facade | provider-backed"
require_literal sdk/README.md "Node / TypeScript facade | Runtime Core seam"
require_literal sdk/README.md "Java / JVM facade | Runtime Core seam"
require_literal sdk/README.md "Swift facade | Runtime Core seam"
require_literal sdk/README.md "Package metadata"
require_literal sdk/README.md "P0 consumer cutover readiness"
require_literal sdk/README.md "Node/Java/Swift seam action-adapter reports"
require_literal sdk/SDK_PARITY.md "Package metadata is machine-checked"
require_literal sdk/SDK_PARITY.md "P0 consumer cutover readiness is"
require_literal sdk/CONFORMANCE_SUITE.md "sdk-conformance-runner"
bash "$ROOT/tools/scripts/check-node-sdk-seam.sh" >/dev/null
bash "$ROOT/tools/scripts/check-java-sdk-seam.sh" >/dev/null
bash "$ROOT/tools/scripts/check-swift-sdk-seam.sh" >/dev/null

if [[ "${#failures[@]}" -eq 0 ]]; then
  printf 'check-sdk-scaffold ok\n'
  exit 0
fi

printf 'check-sdk-scaffold failed:\n' >&2
printf ' - %s\n' "${failures[@]}" >&2
exit 1
