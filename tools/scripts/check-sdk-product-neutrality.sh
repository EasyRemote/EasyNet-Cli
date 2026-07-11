#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  echo "sdk-product-neutrality: $*" >&2
  exit 1
}

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

forbidden_type_pattern='\b(Mission(Client|Transport|Status|Run|Event|Plan)?|Admin(Client|Transport|Carrier|Gateway|Agent|Session)?|Gateway(Status|Client|Transport|Lifecycle)?|IdentityClient|Publication(Client|Transport|Catalog|Resource)?|HostBinding(Client|Transport|Lifecycle)?|Surface(Client|Transport|Page|Manifest|Health)?|Compatibility(Client|Transport|Carrier|Model|Chat|File)?|Wrapper(Client|Transport|Carrier|File|Terminal|Browser|Media|RemoteDesktop)?|Companion(Client|Transport|Desired|Observed|Projected|Supervisor|Boot|Stop)?|AccessControl(Client|Transport|Carrier)?|EventClient|EventsCarrierBase|RuntimeProfileBundle|DaemonProfileBridge|DaemonHandleProfiles)\b'

if rg -n "$forbidden_type_pattern" "${production_sources[@]}"; then
  fail "product type or profile bundle leaked into runtime SDK production source"
fi

forbidden_ability_pattern="[\"'](mission\\.|agent\\.(start|stop|refresh|list)|openai\\.|pages\\.|project_list[\"']|invocation\\.history\\.)"
if rg -n "$forbidden_ability_pattern" "${production_sources[@]}"; then
  fail "product ability literal leaked into runtime SDK production source"
fi

for path in \
  sdk/schemas/admin.schema.json \
  sdk/schemas/compatibility.schema.json \
  sdk/schemas/gateway.schema.json \
  sdk/schemas/publication.schema.json
do
  [[ ! -e "$path" ]] || fail "retired product schema still exists: $path"
done

if find sdk/schemas -maxdepth 1 -type f \( \
  -name 'admin-*' -o \
  -name 'compatibility-*' -o \
  -name 'desktop-companion-*' -o \
  -name 'host-stream-*' -o \
  -name 'mission-*' -o \
  -name 'surface-*' -o \
  -name 'browser-session.schema.json' -o \
  -name 'file.schema.json' -o \
  -name 'media-session.schema.json' -o \
  -name 'remote-desktop.schema.json' -o \
  -name 'terminal.schema.json' \
\) -print -quit | grep -q .; then
  find sdk/schemas -maxdepth 1 -type f \( \
    -name 'admin-*' -o -name 'compatibility-*' -o -name 'desktop-companion-*' -o \
    -name 'host-stream-*' -o \
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

if jq -e '.capabilities[] | select(.capability_id == "mission" or .capability_id == "admin_gateway" or .capability_id == "directory_identity" or .capability_id == "publication" or .capability_id == "host_binding" or .capability_id == "events" or .capability_id == "surface" or .capability_id == "compatibility" or .capability_id == "wrappers" or .capability_id == "access_control" or .capability_id == "runtime_companion_control")' sdk/conformance/sdk-parity-matrix.json >/dev/null; then
  fail "product capability row remains in Go/Python runtime matrix"
fi

echo "sdk-product-neutrality: OK"
