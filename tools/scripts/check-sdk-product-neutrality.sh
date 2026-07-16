#!/usr/bin/env bash
set -euo pipefail

ROOT="${SDK_PRODUCT_NEUTRALITY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"
CONCEPTS="${SDK_CONCEPT_MANIFEST:-$ROOT/sdk/conformance/canonical-public-api.json}"
CONCEPT_VALIDATOR="$ROOT/sdk/conformance/sdk_concepts.py"
PYTHON_BIN="${PYTHON:-python}"

fail() {
  echo "sdk-product-neutrality: $*" >&2
  exit 1
}

canonical_core_violations() {
  rg -n -i \
    '(easynet|daemon|device_ura|owner_ura|session_id|federation\.subscribe|events\.device|events\.invocation|session\.attach)' \
    "$@"
}

route_lowering_violations() {
  rg -n \
    '\b(RouteCatalog|NewRouteCatalog|RuntimeEventRoute|RuntimeEventCursorMode|CursorProjection|SubscriptionProjection)\b' \
    "$@"
}

receipt_storage_violations() {
  rg -n '\bLedgerPath\b|ledger_path' "$@"
}

development_loader_violations() {
  rg -n 'target[/\\](debug|release)([/\\]deps)?' "$@"
}

canonical_root_output="$("$PYTHON_BIN" "$CONCEPT_VALIDATOR" --print-neutrality-roots --manifest "$CONCEPTS")" \
  || fail "canonical package manifest validation failed"
canonical_roots=()
while IFS= read -r path; do
  [[ -n "$path" ]] && canonical_roots+=("$path")
done <<<"$canonical_root_output"
((${#canonical_roots[@]} > 0)) || fail "canonical package manifest is empty"
for root in "${canonical_roots[@]}"; do
  [[ -d "$root" ]] || fail "canonical package root is missing: $root"
done

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  for root in "${canonical_roots[@]}"; do
    mkdir -p "$tmp/$root"
    cp -R "$root/." "$tmp/$root/"
  done
  baseline_sources=()
  while IFS= read -r path; do baseline_sources+=("$path"); done < <(
    {
      find "$tmp/sdk/go" -type f -name '*.go' ! -name '*_test.go'
      find "$tmp/sdk/python/easynet_sdk/core" -type f -name '*.py'
    } | LC_ALL=C sort
  )
  if ((${#baseline_sources[@]} > 0)) && canonical_core_violations "${baseline_sources[@]}"; then
    fail "self-test canonical fixtures unexpectedly contain a product concept"
  fi
  for root in "${canonical_roots[@]}"; do
    if [[ "$root" == sdk/go/* ]]; then
      injected="$tmp/$root/__neutrality_negative.go"
      printf 'package neutralitynegative\nconst leakedProviderRoute = "events.device.subscribe"\n' >"$injected"
    else
      injected="$tmp/$root/__neutrality_negative.py"
      printf 'LEAKED_PROVIDER_ROUTE = "events.device.subscribe"\n' >"$injected"
    fi
    if ! canonical_core_violations "$injected" >/dev/null; then
      fail "self-test failed to detect product route in canonical root: $root"
    fi
    rm -f "$injected"
    if [[ "$root" == sdk/go/runtimeevents || "$root" == sdk/python/easynet_sdk/core ]]; then
      if [[ "$root" == sdk/go/* ]]; then
        injected="$tmp/$root/__route_lowering_negative.go"
        printf 'package neutralitynegative\ntype RouteCatalog struct{}\n' >"$injected"
      else
        injected="$tmp/$root/__route_lowering_negative.py"
        printf 'class RouteCatalog:\n    pass\n' >"$injected"
      fi
      if ! route_lowering_violations "$injected" >/dev/null; then
        fail "self-test failed to detect route lowering in canonical root: $root"
      fi
      rm -f "$injected"
    fi
  done
  mkdir -p "$tmp/sdk/go"
  injected="$tmp/sdk/go/__receipt_storage_negative.go"
  printf 'package neutralitynegative\ntype ReceiptLedgerSource struct{ LedgerPath string `json:"ledger_path,omitempty"` }\n' >"$injected"
  if ! receipt_storage_violations "$injected" >/dev/null; then
    fail "self-test failed to detect receipt storage path in canonical SDK receipt surface"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/go/__development_loader_negative.go"
  printf 'package neutralitynegative\nconst implicitDevelopmentProvider = "target/debug/libeasynet_cli.dylib"\n' >"$injected"
  if ! development_loader_violations "$injected" >/dev/null; then
    fail "self-test failed to detect development build directory in SDK provider loader"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/python/easynet_sdk/__development_loader_negative.py"
  printf 'IMPLICIT_DEVELOPMENT_PROVIDER = "target/release/deps/libeasynet_cli.so"\n' >"$injected"
  if ! development_loader_violations "$injected" >/dev/null; then
    fail "self-test failed to detect development deps directory in SDK provider loader"
  fi
  rm -f "$injected"
  "$PYTHON_BIN" - "$CONCEPTS" "$tmp/missing-root.json" <<'PY'
import json, sys
from pathlib import Path
source = json.loads(Path(sys.argv[1]).read_text())
source["canonical_packages"]["go"][0]["path"] = "sdk/go/__missing_canonical_root__"
Path(sys.argv[2]).write_text(json.dumps(source))
PY
  if "$PYTHON_BIN" "$CONCEPT_VALIDATOR" --print-neutrality-roots \
    --manifest "$tmp/missing-root.json" >"$tmp/missing-root.out" 2>&1; then
    fail "self-test missing canonical root was accepted"
  fi
  grep -Fq 'missing_canonical_package' "$tmp/missing-root.out" \
    || fail "self-test missing canonical root failure was not specific"
  "$PYTHON_BIN" - "$CONCEPTS" "$tmp/unclassified-root.json" <<'PY'
import json, sys
from pathlib import Path
source = json.loads(Path(sys.argv[1]).read_text())
source["canonical_packages"]["go"] = [
    entry for entry in source["canonical_packages"]["go"]
    if entry["path"] != "sdk/go/runtimeevents"
]
Path(sys.argv[2]).write_text(json.dumps(source))
PY
  if "$PYTHON_BIN" "$CONCEPT_VALIDATOR" --print-neutrality-roots \
    --manifest "$tmp/unclassified-root.json" >"$tmp/unclassified-root.out" 2>&1; then
    fail "self-test unclassified discovered Go package was accepted"
  fi
  grep -Fq 'unclassified_package_roots:go' "$tmp/unclassified-root.out" \
    || fail "self-test unclassified Go package failure was not specific"
  echo "sdk-product-neutrality self-test: OK"
  exit 0
fi

for path in \
  sdk/go/profiles.go \
  sdk/go/admin.go \
  sdk/go/companion.go \
  sdk/go/compatibility.go \
  sdk/go/events.go \
  sdk/go/host_binding.go \
  sdk/go/identity.go \
  sdk/go/mission.go \
  sdk/go/publication.go \
  sdk/go/surface.go \
  sdk/go/wrappers.go \
  sdk/python/easynet_sdk/admin.py \
  sdk/python/easynet_sdk/companion.py \
  sdk/python/easynet_sdk/compatibility.py \
  sdk/python/easynet_sdk/daemon_profiles.py \
  sdk/python/easynet_sdk/events.py \
  sdk/python/easynet_sdk/host_binding.py \
  sdk/python/easynet_sdk/identity.py \
  sdk/python/easynet_sdk/mission.py \
  sdk/python/easynet_sdk/profile_bridge.py \
  sdk/python/easynet_sdk/publication.py \
  sdk/python/easynet_sdk/surface.py \
  sdk/python/easynet_sdk/system_abilities.py \
  sdk/python/easynet_sdk/wrappers.py
do
  [[ ! -e "$path" ]] || fail "retired product SDK module still exists: $path"
done

production_sources=()
while IFS= read -r path; do production_sources+=("$path"); done < <(
  {
    find sdk/go -maxdepth 1 -type f -name '*.go' ! -name '*_test.go'
    find sdk/python/easynet_sdk -type f -name '*.py'
    find sdk/node -maxdepth 1 -type f \( -name '*.js' -o -name '*.d.ts' \)
    find sdk/java/src/main -type f -name '*.java'
    find sdk/swift/Sources -type f -name '*.swift'
  } | LC_ALL=C sort
)

if ((${#production_sources[@]} == 0)); then
  fail "no SDK production sources found"
fi

if development_loader_violations "${production_sources[@]}"; then
  fail "development build-directory lookup leaked into SDK production provider loading"
fi

if receipt_storage_violations \
  sdk/go/receipt.go \
  sdk/python/easynet_sdk/receipt.py \
  "$CONCEPTS" \
  sdk/conformance/sdk-parity-matrix.json; then
  fail "receipt ledger storage path leaked into the canonical SDK receipt model"
fi

forbidden_type_pattern='\b(Mission(Client|Transport|Status|Run|Event|Plan)?|Admin(Client|Transport|Carrier|Gateway|Agent|Session)?|Gateway(Status|Client|Transport|Lifecycle)?|IdentityClient|Publication(Client|Transport|Catalog|Resource)?|HostBinding(Client|Transport|Lifecycle)?|Surface(Client|Transport|Page|Manifest|Health)?|Compatibility(Client|Transport|Carrier|Model|Chat|File)?|Wrapper(Client|Transport|Carrier|File|Terminal|Browser|Media|RemoteDesktop)?|Companion(Client|Transport|Desired|Observed|Projected|Supervisor|Boot|Stop)?|AccessControlCarrier|EventClient|EventsCarrierBase|RuntimeProfileBundle|DaemonProfileBridge|DaemonHandleProfiles)\b'

if rg -n "$forbidden_type_pattern" "${production_sources[@]}"; then
  fail "product type or profile bundle leaked into runtime SDK production source"
fi

forbidden_ability_pattern="[\"'](mission\\.|agent\\.(start|stop|refresh|list)|openai\\.|pages\\.|project_list[\"'])"
if rg -n "$forbidden_ability_pattern" "${production_sources[@]}"; then
  fail "product ability literal leaked into runtime SDK production source"
fi

for path in sdk/go/runtime_events.go sdk/python/easynet_sdk/runtime_events.py; do
  if rg -n '(federation\.subscribe_directory_v2|events\.device\.subscribe|session\.attach|events\.invocation\.subscribe|daemon_ability|device_ura|owner_ura|session_id)' "$path"; then
    fail "EasyNet runtime-event lowering leaked into canonical model: $path"
  fi
done

if rg -n 'easynet.run/cli/sdk/go/provider/easynet' sdk/go --glob '*.go' --glob '!**/*_test.go' --glob '!**/runtime_events_compat.go'; then
  fail "canonical Go implementation imports the EasyNet provider facade"
fi

canonical_core_sources=()
for root in "${canonical_roots[@]}"; do
  if [[ "$root" == sdk/go/* ]]; then
    discovered="$(find "$root" -type f -name '*.go' ! -name '*_test.go' | LC_ALL=C sort)" \
      || fail "failed to scan canonical package root: $root"
  else
    discovered="$(find "$root" -type f -name '*.py' | LC_ALL=C sort)" \
      || fail "failed to scan canonical package root: $root"
  fi
  while IFS= read -r path; do
    [[ -n "$path" ]] && canonical_core_sources+=("$path")
  done <<<"$discovered"
done
if ((${#canonical_core_sources[@]} == 0)); then
  fail "no provider-neutral canonical core source found"
fi
if canonical_core_violations "${canonical_core_sources[@]}"; then
  fail "product concept leaked into a provider-neutral canonical core"
fi
if route_lowering_violations "${canonical_core_sources[@]}"; then
  fail "provider route-lowering surface leaked into a provider-neutral canonical core"
fi

for path in \
  sdk/go/provider/easynet \
  sdk/python/easynet_sdk/providers/easynet \
  sdk/python/easynet_sdk/providers \
  sdk/conformance/easynet-provider-routes.json \
  tools/sdk-api-inventory/provider-routes
do
  [[ ! -e "$path" ]] || fail "retired EasyNet SDK provider route source still exists: $path"
done

if rg -n '(federation\.subscribe_directory_v2|events\.device\.subscribe|events\.invocation\.subscribe|session\.attach|daemon_ability|since_seq)' \
  sdk/go sdk/python/easynet_sdk tools/sdk-api-inventory \
  --glob '*.go' --glob '*.py' --glob '!**/*_test.go'; then
  fail "EasyNet provider route literal leaked into canonical SDK source"
fi

for path in \
  sdk/schemas/admin.schema.json \
  sdk/schemas/compatibility.schema.json \
  sdk/schemas/gateway.schema.json \
  sdk/schemas/publication.schema.json \
  sdk/schemas/agent-record.schema.json \
  sdk/schemas/ability-deploy-request.schema.json \
  sdk/schemas/ability-deploy-result.schema.json \
  sdk/schemas/ability-package-manifest.schema.json \
  sdk/schemas/package-validation.schema.json \
  sdk/schemas/published-ability.schema.json \
  sdk/schemas/resource-ref.schema.json \
  sdk/schemas/local-resource-ref-request.schema.json \
  sdk/schemas/lifecycle-status.schema.json
do
  [[ ! -e "$path" ]] || fail "retired product schema still exists: $path"
done

for path in \
  sdk/conformance/fixtures/ability-deploy-request.v4.json \
  sdk/conformance/fixtures/ability-package-manifest.v4.json \
  sdk/conformance/fixtures/local-resource-ref-request.v4.json \
  sdk/conformance/fixtures/package-validation.v4.json \
  sdk/conformance/fixtures/resource-ref.local-fs.v4.json
do
  [[ ! -e "$path" ]] || fail "retired product fixture still exists: $path"
done

if rg -n '(ability-deploy-request|ability-package-manifest|local-resource-ref-request|package-validation|resource-ref\.local-fs)' \
  sdk/conformance/fixture-schema-bindings.json; then
  fail "retired product fixture binding remains in the runtime SDK"
fi

if find sdk/schemas -maxdepth 1 -type f \( \
  -name 'admin-*' -o \
  -name 'ability-deploy-*' -o \
  -name 'ability-package-*' -o \
  -name 'compatibility-*' -o \
  -name 'desktop-companion-*' -o \
  -name 'host-stream-*' -o \
  -name 'local-resource-*' -o \
  -name 'mission-*' -o \
  -name 'package-validation.schema.json' -o \
  -name 'published-ability.schema.json' -o \
  -name 'resource-ref.schema.json' -o \
  -name 'agent-record.schema.json' -o \
  -name 'lifecycle-status.schema.json' -o \
  -name 'surface-*' -o \
  -name 'browser-session.schema.json' -o \
  -name 'file.schema.json' -o \
  -name 'media-session.schema.json' -o \
  -name 'remote-desktop.schema.json' -o \
  -name 'terminal.schema.json' \
\) -print -quit | grep -q .; then
  find sdk/schemas -maxdepth 1 -type f \( \
    -name 'admin-*' -o -name 'ability-deploy-*' -o -name 'ability-package-*' -o \
    -name 'compatibility-*' -o -name 'desktop-companion-*' -o \
    -name 'host-stream-*' -o -name 'local-resource-*' -o \
    -name 'package-validation.schema.json' -o -name 'published-ability.schema.json' -o \
    -name 'resource-ref.schema.json' -o -name 'agent-record.schema.json' -o \
    -name 'lifecycle-status.schema.json' -o \
    -name 'mission-*' -o -name 'surface-*' -o -name 'browser-session.schema.json' -o \
    -name 'file.schema.json' -o -name 'media-session.schema.json' -o \
    -name 'remote-desktop.schema.json' -o -name 'terminal.schema.json' \
  \) -print >&2
  fail "product schemas remain in the runtime SDK"
fi

if find sdk/conformance/cases -maxdepth 1 -type f \( \
  -name 'admin-*' -o \
  -name 'compatibility-*' -o \
  -name 'host-binding-*' -o \
  -name 'mission-*' -o \
  -name 'publication-*' -o \
  -name 'surface-*' -o \
  -name 'wrapper-*' -o \
  -name 'runtime-companion-*' \
\) -print -quit | grep -q .; then
  find sdk/conformance/cases -maxdepth 1 -type f \( \
    -name 'admin-*' -o -name 'compatibility-*' -o \
    -name 'host-binding-*' -o \
    -name 'mission-*' -o -name 'publication-*' -o -name 'surface-*' -o \
    -name 'wrapper-*' -o -name 'runtime-companion-*' \
  \) -print >&2
  fail "product conformance cases remain in the runtime SDK"
fi

if jq -e '.capability_ids[] | select(. == "mission" or . == "admin_gateway" or . == "directory_identity" or . == "publication" or . == "host_binding" or . == "events" or . == "surface" or . == "compatibility" or . == "wrappers" or . == "runtime_companion_control")' sdk/conformance/sdk-parity-matrix.json >/dev/null; then
  fail "product capability row remains in the seven-language runtime matrix"
fi

echo "sdk-product-neutrality: OK"
