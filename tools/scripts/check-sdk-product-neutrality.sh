#!/usr/bin/env bash
set -euo pipefail

ROOT="${SDK_PRODUCT_NEUTRALITY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"
CONCEPTS="${SDK_CONCEPT_MANIFEST:-$ROOT/sdk/conformance/canonical-public-api.json}"
CONCEPT_VALIDATOR="$ROOT/sdk/conformance/sdk_concepts.py"
source "$ROOT/sdk/conformance/python_toolchain.sh"
source "$ROOT/sdk/conformance/toolchain_path.sh"
resolve_sdk_toolchain_path "$ROOT"
resolve_sdk_python_toolchain "$ROOT"
PYTHON_BIN="$SDK_CONFORMANCE_PYTHON"

fail() {
  echo "sdk-product-neutrality: $*" >&2
  exit 1
}

canonical_core_violations() {
  rg -n -i -P \
    '(easynet(?!:///r/)|daemon|device_ura|session_id|federation\.subscribe|events\.device|events\.invocation|session\.attach)' \
    "$@" \
    | grep -vF '"easynet.run/cli/sdk/go"' \
    | grep -vE '^sdk/python/easynet_sdk/providers/runtime/control\.py:[0-9]+:.*(daemon_identity|daemon_version|_ControlDaemonIdentity)'
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

provider_profile_projection_violations() {
  rg -n '\b(DaemonStartProjection|RuntimeHostStartProjection|from_profile|def\s+(hub|device|start_daemon|attach_daemon|discover_daemon|connect_local)\s*\(|func\s+(Hub|Device)StartProjection)\b' "$@"
}

runtime_identity_alias_violations() {
  rg -n '\bErrRuntimeIdentity(NotFound|Unavailable)\b' "$@"
}

daemon_error_decoder_violations() {
  rg -n '\bfunc\s+DecodeDaemonErrorJSON\s*\(' "$@"
}

runtime_device_revoke_violations() {
  rg -n '\b(RuntimeDeviceRevoke(Request|Result)|RevokeDevice|revoke_device)\b' "$@"
}

python_consumer_boundary_product_marker_violations() {
  local consumer_boundary="${1:-sdk/python/easynet_sdk/consumer_boundary.py}"
  [[ -f "$consumer_boundary" ]] || return 0
  rg -n \
    '(easynet-(daemon|control)|unix:///tmp/easynet|runtime_subprocess_targets[^)]*["'\'']easynet["'\'']|EasyRemote)' \
    "$consumer_boundary"
}

python_cabi_product_adapter_name_violations() {
  local cabi="${1:-sdk/python/easynet_sdk/_cabi.py}"
  [[ -f "$cabi" ]] || return 0
  rg -n '\bCLILibrary\b|EasyNet-Cli C ABI|EasyNet CLI C ABI' "$cabi"
}

python_runtime_control_product_state_dir_violations() {
  local control="${1:-sdk/python/easynet_sdk/providers/runtime/control.py}"
  [[ -f "$control" ]] || return 0
  rg -n '(\.easynet|\.easy["'\''][[:space:]]*\+[[:space:]]*["'\'']net|easy["'\''][[:space:]]*\+[[:space:]]*["'\'']net|EASYNET_[A-Z0-9_]*CONTROL)' "$control"
}

sdk_conformance_backend_case_violations() {
  local scan_root="${1:-$ROOT}"
  local output
  output="$(
    {
      if [[ -d "$scan_root/sdk/conformance/cases" ]]; then
        find "$scan_root/sdk/conformance/cases" -maxdepth 1 -type f -name '*.yaml' -print0 \
          | xargs -0 rg -n '(^id:[[:space:]]*backend/|^profile:[[:space:]]*backend_cutover|backend_cutover)' \
          || true
      fi
      for file in \
        "$scan_root/sdk/conformance/canonical-public-api.json" \
        "$scan_root/sdk/conformance/sdk-parity-matrix.json" \
        "$scan_root/sdk/conformance/runner/go-runtime-conformance-report.json" \
        "$scan_root/sdk/conformance/runner/execution-manifest.json"
      do
        [[ -f "$file" ]] || continue
        rg -n '("backend/|backend_cutover)' "$file" || true
      done
    } 2>/dev/null
  )"
  [[ -z "$output" ]] && return 1
  printf '%s\n' "$output"
}

python_runtime_admin_session_projection_violations() {
  local runtime_admin="${1:-sdk/python/easynet_sdk/runtime_admin.py}"
  [[ -f "$runtime_admin" ]] || return 0
  "$PYTHON_BIN" - "$runtime_admin" <<'PY'
import ast
import sys
from pathlib import Path

path = Path(sys.argv[1])
tree = ast.parse(path.read_text(), filename=str(path))
text = path.read_text()

session_class = next(
    (
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "RuntimeSession"
    ),
    None,
)
if session_class is None:
    raise SystemExit("python_runtime_admin_session_projection:missing_runtime_session")
fields = {
    target.id
    for stmt in session_class.body
    if isinstance(stmt, ast.AnnAssign) and isinstance(stmt.target, ast.Name)
    for target in [stmt.target]
}
for retired in ("device_ura", "authority_ura"):
    if retired in fields:
        raise SystemExit(f"python_runtime_admin_session_projection:retired_field:{retired}")
for required in ("runtime_host_ura", "control_authority_ura"):
    if required not in fields:
        raise SystemExit(f"python_runtime_admin_session_projection:missing_field:{required}")
for required_mapping in (
    'runtime_host_ura=_required_admin_string(row, "runtime_host_ura")',
    'control_authority_ura=_required_admin_string(',
):
    if required_mapping not in text:
        raise SystemExit(
            "python_runtime_admin_session_projection:missing_canonical_wire_mapping:"
            + required_mapping
        )
for retired_mapping in (
    'row.get("device_ura")',
    'row.get("authority_ura")',
):
    if retired_mapping in text:
        raise SystemExit(
            "python_runtime_admin_session_projection:retired_wire_mapping:"
            + retired_mapping
        )
if "retired device_ura field" not in text:
    raise SystemExit("python_runtime_admin_session_projection:retired_wire_rejection_missing")
PY
}

go_runtime_admin_session_projection_violations() {
  local runtime_admin="${1:-sdk/go/runtime_admin.go}"
  [[ -f "$runtime_admin" ]] || return 0
  "$PYTHON_BIN" - "$runtime_admin" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
match = re.search(r"type\s+RuntimeSession\s+struct\s*\{(?P<body>.*?)\n\}", text, re.S)
if match is None:
    raise SystemExit("go_runtime_admin_session_projection:missing_runtime_session")
body = match.group("body")
for retired, pattern in (
    ("DeviceURA", r"\bDeviceURA\b"),
    ("AuthorityURA", r"\bAuthorityURA\b"),
    ('`json:"device_ura,omitempty"`', r'`json:"device_ura,omitempty"`'),
    ('`json:"authority_ura,omitempty"`', r'`json:"authority_ura,omitempty"`'),
):
    if re.search(pattern, body):
        raise SystemExit(f"go_runtime_admin_session_projection:retired_field:{retired}")
for required in (
    "RuntimeHostURA",
    "ControlAuthorityURA",
    '`json:"runtime_host_ura,omitempty"`',
    '`json:"control_authority_ura,omitempty"`',
):
    if required not in body:
        raise SystemExit(f"go_runtime_admin_session_projection:missing_field:{required}")
for required_mapping in (
    r"RuntimeHostURA:\s+runtimeHostURA",
    r"ControlAuthorityURA:\s+controlAuthorityURA",
):
    if not re.search(required_mapping, text):
        raise SystemExit(
            "go_runtime_admin_session_projection:missing_canonical_wire_mapping:"
            + required_mapping
        )
for retired_mapping in (
    r"runtimeAdminString\(row,\s*\"device_ura\"\)",
    r"runtimeAdminString\(row,\s*\"authority_ura\"\)",
):
    if re.search(retired_mapping, text):
        raise SystemExit(
            "go_runtime_admin_session_projection:retired_wire_mapping:"
            + retired_mapping
        )
if "retired device_ura field" not in text:
    raise SystemExit("go_runtime_admin_session_projection:retired_wire_rejection_missing")
PY
}

retired_product_sdk_modules() {
  cat <<'EOF'
sdk/go/profiles.go
sdk/go/admin.go
sdk/go/companion.go
sdk/go/compatibility.go
sdk/go/daemon_compat.go
sdk/go/events.go
sdk/go/host_binding.go
sdk/go/identity.go
sdk/go/mission.go
sdk/go/publication.go
sdk/go/surface.go
sdk/go/wrappers.go
sdk/python/easynet_sdk/admin.py
sdk/python/easynet_sdk/companion.py
sdk/python/easynet_sdk/compatibility.py
sdk/python/easynet_sdk/_key_service.py
sdk/python/easynet_sdk/daemon_profiles.py
sdk/python/easynet_sdk/events.py
sdk/python/easynet_sdk/host_binding.py
sdk/python/easynet_sdk/identity.py
sdk/python/easynet_sdk/mission.py
sdk/python/easynet_sdk/profile_bridge.py
sdk/python/easynet_sdk/publication.py
sdk/python/easynet_sdk/surface.py
sdk/python/easynet_sdk/system_abilities.py
sdk/python/easynet_sdk/wrappers.py
sdk/python/easynet_sdk/providers/easynet
sdk/go/provider/easynet
sdk/node/provider/easynet
sdk/rust/provider/easynet
sdk/java/src/main/java/run/runtime/sdk/provider/easynet
sdk/java/src/test/java/run/runtime/sdk/provider/easynet
EOF
}

retired_product_sdk_module_violations() {
  local scan_root="${1:-.}"
  local path
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    [[ ! -e "$scan_root/$path" ]] || printf '%s\n' "$path"
  done < <(retired_product_sdk_modules)
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
  injected="$tmp/sdk/go/__runtime_identity_alias_negative.go"
  printf 'package neutralitynegative\nvar ErrRuntimeIdentityNotFound = ErrDaemonKeyServiceNotFound\n' >"$injected"
  if ! runtime_identity_alias_violations "$injected" >/dev/null; then
    fail "self-test failed to detect retired runtime identity error alias"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/go/__daemon_error_decoder_negative.go"
  printf 'package neutralitynegative\nfunc DecodeDaemonErrorJSON(raw []byte) any { return nil }\n' >"$injected"
  if ! daemon_error_decoder_violations "$injected" >/dev/null; then
    fail "self-test failed to detect exported daemon error decoder"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/go/__runtime_device_revoke_negative.go"
  printf 'package neutralitynegative\ntype RuntimeDeviceRevokeRequest struct{}\nfunc (c RuntimeAdminAbilityClient) RevokeDevice() {}\n' >"$injected"
  if ! runtime_device_revoke_violations "$injected" >/dev/null; then
    fail "self-test failed to detect runtime device revoke surface"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/python/easynet_sdk/__runtime_device_revoke_negative.py"
  printf 'class RuntimeDeviceRevokeRequest:\n    pass\ndef revoke_device(request):\n    return None\n' >"$injected"
  if ! runtime_device_revoke_violations "$injected" >/dev/null; then
    fail "self-test failed to detect Python runtime device revoke surface"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/python/easynet_sdk/consumer_boundary.py"
  mkdir -p "$(dirname "$injected")"
  printf 'PRODUCT_SOCKET = "unix:///tmp/easynet-daemon.sock"\nruntime_subprocess_targets = ("easynet",)\n' >"$injected"
  if ! python_consumer_boundary_product_marker_violations "$injected" >/dev/null; then
    fail "self-test failed to detect product runtime-host marker in Python consumer boundary"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/python/easynet_sdk/_cabi.py"
  mkdir -p "$(dirname "$injected")"
  printf 'class CLILibrary:\n    """Typed binding for the generic EasyNet-Cli C ABI v6 surface."""\n' >"$injected"
  if ! python_cabi_product_adapter_name_violations "$injected" >/dev/null; then
    fail "self-test failed to detect product C ABI adapter naming in Python SDK"
  fi
  rm -f "$injected"
  injected="$tmp/sdk/python/easynet_sdk/providers/runtime/control.py"
  mkdir -p "$(dirname "$injected")"
  printf '_CONTROL_STATE_DIR_NAME = ".easy" + "net"\n' >"$injected"
  if ! python_runtime_control_product_state_dir_violations "$injected" >/dev/null; then
    fail "self-test failed to detect product runtime state directory in Python SDK control provider"
  fi
  rm -f "$injected"
  mkdir -p "$tmp/sdk/node/provider/easynet"
  retired_output="$(retired_product_sdk_module_violations "$tmp")"
  if ! grep -Fxq "sdk/node/provider/easynet" <<<"$retired_output"; then
    fail "self-test failed to detect retired Node product provider root"
  fi
  rmdir "$tmp/sdk/node/provider/easynet"
  injected="$tmp/sdk/python/easynet_sdk/runtime_admin.py"
  mkdir -p "$(dirname "$injected")"
  cat >"$injected" <<'PY'
from dataclasses import dataclass
@dataclass(frozen=True)
class RuntimeSession:
    device_ura: str = ""
    authority_ura: str = ""
PY
  if python_runtime_admin_session_projection_violations "$injected" >/dev/null 2>&1; then
    fail "self-test failed to detect product runtime-admin session projection"
  fi
  cat >"$injected" <<'PY'
from dataclasses import dataclass
@dataclass(frozen=True)
class RuntimeSession:
    runtime_host_ura: str = ""
    control_authority_ura: str = ""
def _runtime_session_page(row):
    return RuntimeSession(
        runtime_host_ura=_required_admin_string(row, "runtime_host_ura"),
        control_authority_ura=_required_admin_string(row, "control_authority_ura"),
    )
def _reject(row):
    raise Exception("retired device_ura field")
PY
  python_runtime_admin_session_projection_violations "$injected" \
    || fail "self-test rejected neutral runtime-admin session projection"
  rm -f "$injected"
  injected="$tmp/sdk/go/runtime_admin.go"
  mkdir -p "$(dirname "$injected")"
  cat >"$injected" <<'GO'
package easynet
type RuntimeSession struct {
	DeviceURA    string `json:"device_ura,omitempty"`
	AuthorityURA string `json:"authority_ura,omitempty"`
}
GO
  if go_runtime_admin_session_projection_violations "$injected" >/dev/null 2>&1; then
    fail "self-test failed to detect product Go runtime-admin session projection"
  fi
  cat >"$injected" <<'GO'
package easynet
type RuntimeSession struct {
	RuntimeHostURA      string `json:"runtime_host_ura,omitempty"`
	ControlAuthorityURA string `json:"control_authority_ura,omitempty"`
}
func runtimeSessionPage(row map[string]any) {
	runtimeHostURA := requiredRuntimeAdminString(row, "runtime_host_ura")
	controlAuthorityURA := requiredRuntimeAdminString(row, "control_authority_ura")
	_ = "retired device_ura field"
	_ = RuntimeSession{
		RuntimeHostURA:      runtimeHostURA,
		ControlAuthorityURA: controlAuthorityURA,
	}
}
GO
  go_runtime_admin_session_projection_violations "$injected" \
    || fail "self-test rejected neutral Go runtime-admin session projection"
  rm -f "$injected"
  injected="$tmp/sdk/python/easynet_sdk/providers/easynet/lifecycle.py"
  mkdir -p "$(dirname "$injected")"
  printf 'class DaemonStartProjection:\n    @classmethod\n    def hub(cls):\n        return cls.from_profile(mode="hub")\ndef start_daemon(transport, config):\n    return None\n' >"$injected"
  if ! provider_profile_projection_violations "$injected" >/dev/null; then
    fail "self-test failed to detect provider product start projection"
  fi
  rm -f "$injected"
  mkdir -p "$tmp/sdk/python/easynet_sdk"
  injected="$tmp/sdk/python/easynet_sdk/_key_service.py"
  printf '"""Compatibility exports for the EasyNet key-service provider."""\n' >"$injected"
  retired_output="$(retired_product_sdk_module_violations "$tmp")"
  if ! grep -Fxq "sdk/python/easynet_sdk/_key_service.py" <<<"$retired_output"; then
    fail "self-test failed to detect retired Python key-service facade"
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
  mkdir -p "$tmp/sdk/conformance/cases" "$tmp/sdk/conformance/runner"
  printf 'id: backend/import_ban\nprofile: backend_cutover\n' \
    >"$tmp/sdk/conformance/cases/backend-negative.yaml"
  printf '{"case_ids":["backend/import_ban"]}\n' \
    >"$tmp/sdk/conformance/canonical-public-api.json"
  if ! sdk_conformance_backend_case_violations "$tmp" >/dev/null; then
    fail "self-test failed to detect backend case ownership inside SDK conformance"
  fi
  rm -f "$tmp/sdk/conformance/cases/backend-negative.yaml" \
    "$tmp/sdk/conformance/canonical-public-api.json"
  echo "sdk-product-neutrality self-test: OK"
  exit 0
fi

[[ ! -e "$ROOT/sdk/conformance/edge_adapter_policy.py" ]] \
  || fail "retired edge-adapter policy script still exists"
[[ ! -e "$ROOT/sdk/conformance/edge-adapter-policy.v1.json" ]] \
  || fail "retired edge-adapter policy manifest still exists"

retired_module_output="$(retired_product_sdk_module_violations "$ROOT")"
if [[ -n "$retired_module_output" ]]; then
  echo "$retired_module_output" >&2
  fail "retired product SDK module still exists"
fi

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

provider_profile_sources=()
while IFS= read -r path; do provider_profile_sources+=("$path"); done < <(
  find sdk/go/provider sdk/python/easynet_sdk/providers sdk/node/provider \
    \( -path '*/__pycache__/*' -o -path '*/.pytest_cache/*' \) -prune -o \
    -type f \( -name '*.go' -o -name '*.py' -o -name '*.js' -o -name '*.ts' \) \
    ! -name '*_test.go' \
    ! -name '*test.py' \
    ! -name '*.test.mjs' \
    -print 2>/dev/null | LC_ALL=C sort
)
if ((${#provider_profile_sources[@]} > 0)) \
  && provider_profile_projection_violations "${provider_profile_sources[@]}"; then
  fail "product profile start projection leaked into SDK provider package"
fi

if receipt_storage_violations \
  sdk/go/receipt.go \
  sdk/python/easynet_sdk/receipt.py \
  "$CONCEPTS" \
  sdk/conformance/sdk-parity-matrix.json; then
  fail "receipt ledger storage path leaked into the canonical SDK receipt model"
fi

product_directory_pattern='\b(DirectoryAgentSummary|DirectorySigningAuthority|ParseDirectoryEntry|parse_directory_entry)\b'
if rg -n "$product_directory_pattern" "${production_sources[@]}"; then
  fail "product-owned Directory wire DTO leaked into canonical runtime SDK source"
fi
if find sdk/go sdk/python/easynet_sdk sdk/node \
  \( -path '*/__pycache__/*' -o -path '*/.mypy_cache/*' \) -prune -o \
  -path '*directory_wire*' -type f -print -quit | grep -q .; then
  find sdk/go sdk/python/easynet_sdk sdk/node \
    \( -path '*/__pycache__/*' -o -path '*/.mypy_cache/*' \) -prune -o \
    -path '*directory_wire*' -type f -print >&2
  fail "product-owned Directory wire projection file remains in canonical runtime SDK"
fi

forbidden_type_pattern='\b(Mission(Client|Transport|Status|Run|Event|Plan)?|Admin(Client|Transport|Carrier|Gateway|Agent|Session)?|Gateway(Status|Client|Transport|Lifecycle)?|IdentityClient|Publication(Client|Transport|Catalog|Resource)?|HostBinding(Client|Transport|Lifecycle)?|Surface(Client|Transport|Page|Manifest|Health)?|Compatibility(Client|Transport|Carrier|Model|Chat|File)?|Wrapper(Client|Transport|Carrier|File|Terminal|Browser|Media|RemoteDesktop)?|Companion(Client|Transport|Desired|Observed|Projected|Supervisor|Boot|Stop)?|AccessControlCarrier|EventClient|EventsCarrierBase|RuntimeProfileBundle|DaemonProfileBridge|DaemonHandleProfiles)\b'

if rg -n "$forbidden_type_pattern" "${production_sources[@]}"; then
  fail "product type or profile bundle leaked into runtime SDK production source"
fi

forbidden_ability_pattern="[\"'](mission\\.|agent\\.(start|stop|refresh|list)|openai\\.|pages\\.|project_list[\"'])"
if rg -n "$forbidden_ability_pattern" "${production_sources[@]}"; then
  fail "product ability literal leaked into runtime SDK production source"
fi

if runtime_identity_alias_violations "${production_sources[@]}"; then
  fail "retired runtime identity error alias leaked into runtime SDK production source"
fi

if daemon_error_decoder_violations "${production_sources[@]}"; then
  fail "exported daemon error decoder leaked into runtime SDK production source"
fi

if runtime_device_revoke_violations "${production_sources[@]}"; then
  fail "runtime device revoke surface leaked into runtime SDK production source"
fi

if python_consumer_boundary_product_marker_violations \
  "$ROOT/sdk/python/easynet_sdk/consumer_boundary.py"; then
  fail "product runtime-host marker leaked into Python SDK consumer boundary policy"
fi

if python_cabi_product_adapter_name_violations \
  "$ROOT/sdk/python/easynet_sdk/_cabi.py"; then
  fail "product C ABI adapter naming leaked into Python SDK runtime transport"
fi

if python_runtime_control_product_state_dir_violations \
  "$ROOT/sdk/python/easynet_sdk/providers/runtime/control.py"; then
  fail "product runtime state directory leaked into Python SDK control provider"
fi

python_runtime_admin_session_projection_violations \
  "$ROOT/sdk/python/easynet_sdk/runtime_admin.py" \
  || fail "product runtime-admin session projection leaked into Python SDK"

go_runtime_admin_session_projection_violations \
  "$ROOT/sdk/go/runtime_admin.go" \
  || fail "product runtime-admin session projection leaked into Go SDK"

for path in sdk/go/runtime_events.go sdk/python/easynet_sdk/runtime_events.py; do
  if rg -n '(federation\.subscribe_directory_v2|events\.device\.subscribe|session\.attach|events\.invocation\.subscribe|daemon_ability|device_ura|owner_ura|session_id)' "$path"; then
    fail "EasyNet runtime-event lowering leaked into canonical model: $path"
  fi
done

if rg -n 'easynet.run/cli/sdk/go/provider/easynet' sdk/go \
  --glob '*.go' \
  --glob '!**/provider/**' \
  --glob '!**/*_test.go' \
  --glob '!**/runtime_events_compat.go'; then
  fail "canonical Go implementation imports the retired product provider facade"
fi

if rg -n 'providers\.easynet\.key|providers/easynet/key' sdk/python/easynet_sdk \
  --glob '*.py' \
  --glob '!providers/easynet/**' \
  --glob '!**/__pycache__/**'; then
  fail "canonical Python SDK imports the EasyNet key custody provider facade"
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
  sdk/conformance/easynet-provider-routes.json \
  tools/sdk-api-inventory/provider-routes
do
  [[ ! -e "$path" ]] || fail "retired EasyNet SDK provider route source still exists: $path"
done

if rg -n '(federation\.subscribe_directory_v2|events\.device\.subscribe|events\.invocation\.subscribe|session\.attach|daemon_ability|since_seq)' \
  sdk/go sdk/python/easynet_sdk tools/sdk-api-inventory \
  --glob '*.go' --glob '*.py' --glob '!**/*_test.go'; then
  fail "product provider route literal leaked into canonical SDK source"
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

if sdk_conformance_backend_case_violations "$ROOT"; then
  fail "backend/product migration gate leaked into SDK conformance ownership"
fi

if jq -e '.capability_ids[] | select(. == "mission" or . == "admin_gateway" or . == "directory_identity" or . == "publication" or . == "host_binding" or . == "events" or . == "surface" or . == "compatibility" or . == "wrappers" or . == "runtime_companion_control")' sdk/conformance/sdk-parity-matrix.json >/dev/null; then
  fail "product capability row remains in the seven-language runtime matrix"
fi

echo "sdk-product-neutrality: OK"
