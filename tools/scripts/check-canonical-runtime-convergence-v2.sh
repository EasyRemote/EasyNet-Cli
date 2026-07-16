#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
AXON_ROOT="${EASYNET_AXON_ROOT:-$ROOT/../EasyNet-Axon}"
PYTHON_BIN="${PYTHON:-python3}"
MANIFEST="$ROOT/sdk/conformance/canonical-public-api.json"
MATRIX="$ROOT/sdk/conformance/sdk-parity-matrix.json"

fail() {
  echo "canonical-runtime-convergence-v2: $*" >&2
  exit 1
}

check_manifest_contract() {
  "$PYTHON_BIN" - "$MANIFEST" "$MATRIX" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text())
matrix = json.loads(Path(sys.argv[2]).read_text())
expected_status_names = {
    "unsupported": "Unsupported",
    "seam": "Seam",
    "provider-backed": "ProviderBacked",
    "cutover-ready": "CutoverReady",
}
expected_actions = [
    "bidi_open",
    "cancel",
    "child_dispatch",
    "deadline",
    "dispatch",
    "restart_recover",
    "start",
    "stream_open",
    "terminal_receipt",
]
for name, document in (("manifest", manifest), ("matrix", matrix)):
    if document.get("status_canonical_names") != expected_status_names:
        raise SystemExit(f"{name}:status_canonical_names")
    if document.get("lifecycle_actions") != expected_actions:
        raise SystemExit(f"{name}:lifecycle_actions")

plain_helpers = {
    "canonical_invocation_bytes",
    "run_admission",
    "sign_invocation",
    "verify_invocation_signature",
    "verify_phase",
    "verify_signature",
    "axiom.canonical_invocation_bytes",
    "axiom.sign_invocation",
    "axiom.verify_invocation_signature",
    "admission.run_admission",
    "admission.verify_phase",
    "admission.verify_signature",
}
fallback_signer_helpers = {
    "default_auth_for_subject",
    "GeneratedSubjectAuth",
    "generate_private_agent_auth",
    "generate_private_hub_auth",
    "generate_subject_auth",
    "DefaultAuthForSubject",
    "GenerateSubjectAuth",
    "ProcessLocalSigner",
    "PrivateKeyAuthenticator",
    "runtime_admin.GeneratedSubjectAuth",
    "runtime_admin.generate_private_agent_auth",
    "runtime_admin.generate_private_hub_auth",
    "runtime_admin.generate_subject_auth",
}
for section in ("languages", "members"):
    graph = manifest.get(section, {})
    for language, values in graph.items():
        leaked = sorted(plain_helpers & set(values))
        if leaked:
            raise SystemExit(f"canonical_plain_helper_leak:{language}:{section}:{','.join(leaked)}")
        fallback_leaked = sorted(fallback_signer_helpers & set(values))
        if fallback_leaked:
            raise SystemExit(
                f"fallback_signer_helper_leak:{language}:{section}:{','.join(fallback_leaked)}"
            )

quarantine = manifest.get("non_canonical", {})
metadata = manifest.get("legacy_quarantine", {})
for section in ("languages", "members"):
    graph = quarantine.get(section, {})
    for language, values in graph.items():
        legacy_plain = sorted(plain_helpers & set(values))
        if legacy_plain:
            raise SystemExit(
                f"plain_helper_legacy_export:{language}:{section}:{','.join(legacy_plain)}"
            )
for section in ("languages", "members"):
    graph = quarantine.get(section, {})
    for language, values in graph.items():
        for helper in sorted(fallback_signer_helpers & set(values)):
            reason = metadata.get(section, {}).get(language, {}).get(helper, {}).get("reason", "")
            if "Process-local signer fallback" not in reason:
                raise SystemExit(f"fallback_signer_reason_not_bound:{section}:{language}:{helper}")
PY
}

check_active_source_contract() {
  if rg -n 'default_auth_for_subject' "$ROOT/src" "$ROOT/sdk" "$ROOT/include" \
    --glob '!sdk/node/node_modules/**' \
    --glob '!sdk/conformance/**' \
    --glob '!target/**' \
    --glob '!sdk/go/internal/axonpb/**' \
    --glob '!sdk/python/easynet_sdk/_axon_pb/**'; then
    fail "process-local fallback signer path is present"
  fi

  if rg -n '\b(MissionState|MissionControl)\b' "$ROOT/src" "$ROOT/sdk" "$ROOT/include" \
    --glob '!sdk/node/node_modules/**' \
    --glob '!target/**' \
    --glob '!sdk/go/internal/axonpb/**' \
    --glob '!sdk/python/easynet_sdk/_axon_pb/**' \
    --glob '!src/eal/**'; then
    fail "Mission/EAL state leaked outside daemon-owned execution boundary"
  fi
}

check_sdk_product_neutrality_contract() {
  bash "$ROOT/tools/scripts/check-sdk-product-neutrality.sh" >/dev/null
}

check_daemon_tuple_route_contract() {
  bash "$ROOT/tools/scripts/check-daemon-invocation-migration.sh" >/dev/null
}

check_key_custody_boundary_contract() {
  bash "$ROOT/tools/scripts/check-daemon-key-service-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-product-key-custody-boundary.sh" >/dev/null
}

check_daemon_mission_eal_boundary_contract() {
  bash "$ROOT/tools/scripts/check-dispatch-mission-context-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-runtime-abilities-manifest-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-orchestration-service-boundary.sh" >/dev/null
}

check_axon_product_protocol_boundary_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for product protocol boundary contract: $AXON_ROOT"
  fi

  local path
  for path in \
    core/proto/axon/v1/voice.proto \
    core/proto/axon/v1/remote_desktop.proto \
    core/runtime-rs/client-sdk/proto/axon/v1/voice.proto \
    core/runtime-rs/client-sdk/proto/axon/v1/remote_desktop.proto \
    sdk/rust/proto/axon/v1/voice.proto \
    sdk/rust/proto/axon/v1/remote_desktop.proto \
    sdk/rust/src/audio.rs \
    sdk/rust/src/mcp.rs \
    sdk/rust/src/voice.rs \
    sdk/rust/src/remote_desktop.rs \
    sdk/go/easynet/audio.go \
    sdk/go/easynet/audio_stub.go \
    sdk/go/easynet/tool_adapter.go \
    sdk/go/easynet/mcp/server.go \
    sdk/python/easynet_axon/audio.py \
    sdk/python/easynet_axon/tool_adapter.py \
    sdk/python/easynet_axon/mcp/server.py \
    sdk/python/easynet_axon/presets/remote_control/descriptor.py \
    sdk/node/src/audio.ts \
    sdk/node/src/tool_adapter.ts \
    sdk/node/src/mcp/server.ts \
    sdk/node/src/presets/remote_control/descriptor.ts \
    sdk/node/src/presets/ability_dispatch.ts \
    sdk/node/src/presets/remote_control_case.ts \
    sdk/java/src/main/java/run/easynet/axon/Audio.java \
    sdk/java/src/main/java/run/easynet/axon/AbilityToolAdapter.java \
    sdk/java/src/main/java/run/easynet/axon/AxonMcpException.java \
    sdk/java/src/main/java/run/easynet/axon/DeployMcpListDirRequest.java \
    sdk/java/src/main/java/run/easynet/axon/UpdateMcpListDirRequest.java \
    sdk/java/src/main/java/run/easynet/axon/VoiceBridge.java \
    sdk/java/src/main/java/run/easynet/axon/VoiceService.java \
    sdk/java/src/main/java/run/easynet/axon/mcp/StdioMcpServer.java \
    sdk/java/src/main/java/run/easynet/axon/presets/remote_control/RemoteControlDescriptor.java \
    sdk/java/src/main/java/run/easynet/axon/cases/ability_dispatch/AbilityDispatchCase.java \
    sdk/swift/Sources/EasyNetAxon/Audio.swift \
    sdk/swift/Sources/EasyNetAxon/StdioMcpServer.swift \
    sdk/swift/Sources/EasyNetAxon/ToolAdapter.swift
  do
    [[ ! -e "$AXON_ROOT/$path" ]] \
      || fail "product-owned file remains in canonical Axon surface: $path"
  done

  if [[ -d "$AXON_ROOT/sdk/go" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/go 2>/dev/null | grep -Eq '/(audio|voice|tool_adapter|mcp)([^/]*|/.*)$'); then
    fail "Go SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/python" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/python 2>/dev/null | grep -Eq '/(audio|tool_adapter|mcp|presets/(remote_control|ability_dispatch|federation))([^/]*|/.*)$'); then
    fail "Python SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/node" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/node 2>/dev/null | grep -Eq '/(audio|tool_adapter|mcp|presets/(remote_control|ability_dispatch)|remote_control_case)([^/]*|/.*)$'); then
    fail "Node SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/java" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/java 2>/dev/null | grep -Eq '/(Audio|AbilityToolAdapter|AxonMcpException|DeployMcpListDirRequest|UpdateMcpListDirRequest|VoiceBridge|VoiceService)\.java$|/(mcp|presets/remote_control|cases/ability_dispatch)/'); then
    fail "Java SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/swift" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/swift 2>/dev/null | grep -Eq '/(Audio|StdioMcpServer|ToolAdapter)\.swift$'); then
    fail "Swift SDK tracks a product-owned canonical package"
  fi

  local rust_lib="$AXON_ROOT/sdk/rust/src/lib.rs"
  if [[ -f "$rust_lib" ]] \
    && grep -Eq 'pub (mod|use) (audio|mcp|voice|remote_desktop|presets|tool_adapter)\b' "$rust_lib"; then
    fail "Rust SDK exports a product-owned module"
  fi

  local proto_root="$AXON_ROOT/core/proto/axon/v1"
  if [[ -d "$proto_root" ]] \
    && grep -R -nE '^[[:space:]]*(message|service|enum)[[:space:]]+(Mcp|MCP|Voice|RemoteDesktop|EasyNet)' "$proto_root"; then
    fail "canonical Axon proto declares a product protocol type"
  fi

  local proto_mirrors=(
    "$AXON_ROOT/core/proto/axon/v1"
    "$AXON_ROOT/core/runtime-rs/client-sdk/proto/axon/v1"
    "$AXON_ROOT/sdk/rust/proto/axon/v1"
  )
  existing_proto_mirrors=()
  for path in "${proto_mirrors[@]}"; do
    [[ -d "$path" ]] && existing_proto_mirrors+=("$path")
  done
  if ((${#existing_proto_mirrors[@]} > 0)) \
    && grep -R -nE '\b(McpToolSpec|McpToolTarget|EasyNetContext|EasyNetHook|ObjectiveWeights)\b' "${existing_proto_mirrors[@]}"; then
    fail "canonical Axon proto mirrors contain a product protocol type"
  fi

  local dendrite_paths=(
    "$AXON_ROOT/core/runtime-rs/dendrite-bridge/src"
    "$AXON_ROOT/core/runtime-rs/dendrite-bridge/include"
    "$AXON_ROOT/packaging/sdk-pack/build_sdk_packs.sh"
  )
  existing_dendrite_paths=()
  for path in "${dendrite_paths[@]}"; do
    [[ -e "$path" ]] && existing_dendrite_paths+=("$path")
  done
  if ((${#existing_dendrite_paths[@]} > 0)) \
    && grep -R -n 'axon_dendrite_voice_' "${existing_dendrite_paths[@]}"; then
    fail "Dendrite exports a voice product client"
  fi

  for path in \
    core/runtime-rs/build.rs \
    core/runtime-rs/client-sdk/build.rs \
    sdk/rust/build.rs
  do
    if [[ -f "$AXON_ROOT/$path" ]]; then
      grep -q 'CANONICAL_AXON_PROTO_FILES' "$AXON_ROOT/$path" \
        || fail "$path does not use the canonical proto allowlist"
    fi
  done

  local rfc004="$AXON_ROOT/document/rfcs/004-mcp-binding.md"
  if [[ -f "$rfc004" ]]; then
    grep -q 'Withdrawn from Axon canonical protocol' "$rfc004" \
      || fail "RFC 004 still claims Axon MCP ownership"
  fi
  local sdk_parity="$AXON_ROOT/sdk/SDK_PARITY.md"
  if [[ -f "$sdk_parity" ]]; then
    grep -q 'Product-Owned Surfaces' "$sdk_parity" \
      || fail "SDK parity does not declare the product ownership boundary"
  fi
}

check_axon_plain_proof_public_boundary_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for plain proof boundary contract: $AXON_ROOT"
  fi

  local rust_invocation="$AXON_ROOT/sdk/rust/src/invocation"
  if [[ -d "$rust_invocation" ]] \
    && rg -n 'pub fn (canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|verify_phase|run_admission)\b|pub use (admission|axiom)::\{[^}]*\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|verify_phase|run_admission)\b' "$rust_invocation"; then
    fail "Axon Rust exposes plain proof/admission helpers"
  fi

  local python_invocation="$AXON_ROOT/sdk/python/easynet_axon/invocation"
  if [[ -d "$python_invocation" ]] \
    && rg -n '^def (canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|run_admission)\b|from \.axiom import \([^)]*\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature)\b|from \.admission import \([^)]*\b(verify_signature|run_admission)\b|"(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|run_admission)"' "$python_invocation"; then
    fail "Axon Python exposes plain proof/admission helpers"
  fi
}

check_ura_vocabulary_contract() {
  # This gate intentionally delegates SDK surface scanning to the canonical
  # SDK naming script, then adds active SPEC coverage for the V2 document.
  bash "$ROOT/tools/scripts/check-sdk-ura-naming.sh" >/dev/null
  if rg -n '\bU(RI|ri|ri)\b|[[:lower:][:digit:]]U(RI|ri)\b|_uri\b' \
    "$ROOT/docs/spec/canonical-runtime-convergence-v2.md"; then
    fail "canonical-runtime-convergence-v2 SPEC uses retired address terminology"
  fi
}

check_active_ura_transport_classification_contract() {
  "$PYTHON_BIN" - "$@" <<'PY'
import re
import sys
from pathlib import Path

roots = [Path(arg) for arg in sys.argv[1:]]
if not roots:
    raise SystemExit("active_ura_transport_classification:missing_roots")

retired = re.compile(
    r"(^|[^A-Za-z0-9])(URI|Uri|uri)([A-Z0-9]|[^A-Za-z0-9]|$)"
    r"|[a-z0-9](URI|Uri)([A-Z0-9]|[^A-Za-z0-9]|$)"
)
transport = re.compile(
    r"\b(?:hyper::Uri|http::Uri|tonic::transport::Uri|url::Url)\b"
    r"|use\s+(?:hyper|tonic::transport)::\{[^}]*\bUri\b[^}]*\}"
    r"|\bconnect_with_connector\b"
    r"|\btower::service_fn\(move \|_:\s*Uri\|"
    r"|\breq\.uri\(\)"
    r"|\.uri\("
)
transport_target = re.compile(r"\btarget_uri\b|\brequest_uri\b")
semantic = re.compile(
    r"\b(?:ability|agent|callee|caller|device|invocation|owner|principal|receipt|resource|subject)"
    r"[A-Za-z0-9_]*(?:uri|url|address)\b"
    r"|\b(?:uri|url|address)[A-Za-z0-9_]*(?:ability|agent|callee|caller|device|invocation|owner|principal|receipt|resource|subject)\b",
    re.IGNORECASE,
)
ura = re.compile(r"ura", re.IGNORECASE)
skip_parts = {
    ".git",
    "target",
    "node_modules",
    "__pycache__",
}

def iter_files(root: Path):
    if root.is_file():
        yield root
        return
    if not root.exists():
        return
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        parts = set(path.parts)
        if parts & skip_parts:
            continue
        if "/tests/scripts/" in path.as_posix():
            continue
        if path.suffix in {".png", ".jpg", ".jpeg", ".gif", ".webp", ".wasm", ".lock"}:
            continue
        yield path

violations = []
for root in roots:
    for path in iter_files(root):
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for line_number, line in enumerate(text.splitlines(), start=1):
            if not retired.search(line):
                continue
            if transport.search(line):
                continue
            if transport_target.search(line) and "easynet:///" not in line:
                continue
            match = semantic.search(line)
            if match and ura.search(match.group(0)):
                continue
            violations.append(f"{path}:{line_number}:{line.strip()}")

if violations:
    print(
        "active source still uses URI terminology outside transport-library APIs:",
        file=sys.stderr,
    )
    print("\n".join(violations), file=sys.stderr)
    raise SystemExit(1)
PY
}

check_schema_source_derivation_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for schema-source derivation contract: $AXON_ROOT"
  fi

  local syncer="$AXON_ROOT/scripts/proto/sync_axon_v1.sh"
  if [[ ! -f "$syncer" ]]; then
    fail "Axon proto source derivation gate is missing: ${syncer#$AXON_ROOT/}"
  fi

  if ! bash "$syncer" --check >/dev/null; then
    fail "Axon proto mirrors diverged from canonical core/proto source"
  fi
}

check_receipt_proof_fact_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for receipt proof-fact contract: $AXON_ROOT"
  fi

  local java_axiom="$AXON_ROOT/sdk/java/src/main/java/run/easynet/axon/invocation/Axiom.java"
  local java_bundle="$AXON_ROOT/sdk/java/src/main/java/run/easynet/axon/invocation/Bundle.java"
  local python_axiom="$AXON_ROOT/sdk/python/easynet_axon/invocation/axiom.py"
  local node_invocation="$AXON_ROOT/sdk/node/src/invocation"
  local swift_invocation="$AXON_ROOT/sdk/swift/Sources/EasyNetAxon/Invocation"
  local go_invocation="$AXON_ROOT/sdk/go/easynet/invocation"

  if rg -n 'AuthorityBinding\.self\(callerBinding\.ura\)|ReceiptProofFacts\.empty\(\)\);' "$java_axiom" "$java_bundle"; then
    fail "Java receipt construction/parsing still synthesizes authority or proof facts"
  fi

  if rg -n 'field\(default_factory=ReceiptProofFacts\)|AuthorityBinding\.self_\(r\.caller_binding\.ura\)|proof_facts if .*else .*ReceiptProofFacts\(\)' "$python_axiom" "$AXON_ROOT/sdk/python/easynet_axon/invocation/audit.py"; then
    fail "Python receipt construction still defaults authority or proof facts"
  fi

  if rg -n 'proofFacts \?\? EMPTY_RECEIPT_PROOF_FACTS|authorityBinding \?\? AuthorityBinding\.self_|readonly proofFacts\?:|proofFacts\?: ReceiptProofFacts|authorityBinding\?: AuthorityBinding' "$node_invocation" \
    --glob '!axiom-authority.test.ts'; then
    fail "Node receipt construction still allows omitted authority or proof facts"
  fi

  if rg -n 'authorityBinding: AuthorityBinding\? = nil|proofFacts: ReceiptProofFacts = \.empty|\?\? \.selfAuthority' "$swift_invocation"; then
    fail "Swift receipt construction still defaults authority or proof facts"
  fi

  if rg -n 'normaliseAuthority\(r\.AuthorityBinding|ProofFacts:\s*ReceiptProofFacts\{|return ReceiptProofFacts\{' "$go_invocation" \
    --glob '!axiom.go'; then
    fail "Go receipt construction still omits constructor-backed proof facts"
  fi
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$ROOT/target/canonical-runtime-convergence-v2.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  cp "$MANIFEST" "$tmp/manifest.json"
  cp "$MATRIX" "$tmp/matrix.json"
  "$PYTHON_BIN" - "$tmp/manifest.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["languages"]["rust"].append("canonical_invocation_bytes")
data["languages"]["rust"].sort()
path.write_text(json.dumps(data))
PY
  if "$PYTHON_BIN" - "$tmp/manifest.json" "$tmp/matrix.json" <<'PY' >/dev/null 2>&1
import json
import sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[1]).read_text())
plain = {"canonical_invocation_bytes"}
if plain & set(manifest["languages"]["rust"]):
    raise SystemExit("canonical_plain_helper_leak")
PY
  then
    fail "self-test expected canonical helper leak to fail"
  fi
  cp "$MANIFEST" "$tmp/plain-legacy-manifest.json"
  "$PYTHON_BIN" - "$tmp/plain-legacy-manifest.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["non_canonical"]["members"]["rust"].append("axiom.sign_invocation")
data["non_canonical"]["members"]["rust"].sort()
path.write_text(json.dumps(data))
PY
  if "$PYTHON_BIN" - "$tmp/plain-legacy-manifest.json" "$tmp/matrix.json" <<'PY' >/dev/null 2>&1
import json
import sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[1]).read_text())
plain = {"axiom.sign_invocation"}
if plain & set(manifest["non_canonical"]["members"]["rust"]):
    raise SystemExit("plain_helper_legacy_export")
PY
  then
    fail "self-test expected legacy plain helper export to fail"
  fi
  cp "$MANIFEST" "$tmp/fallback-manifest.json"
  "$PYTHON_BIN" - "$tmp/fallback-manifest.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["languages"]["go"].append("GeneratedSubjectAuth")
data["languages"]["go"].sort()
path.write_text(json.dumps(data))
PY
  if "$PYTHON_BIN" - "$tmp/fallback-manifest.json" "$tmp/matrix.json" <<'PY' >/dev/null 2>&1
import json
import sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[1]).read_text())
fallback = {"GeneratedSubjectAuth"}
if fallback & set(manifest["languages"]["go"]):
    raise SystemExit("fallback_signer_helper_leak")
PY
  then
    fail "self-test expected fallback signer leak to fail"
  fi
  mkdir -p "$tmp/axon/sdk/node/src/invocation"
  mkdir -p "$tmp/axon/sdk/java/src/main/java/run/easynet/axon/invocation"
  mkdir -p "$tmp/axon/sdk/python/easynet_axon/invocation"
  mkdir -p "$tmp/axon/sdk/swift/Sources/EasyNetAxon/Invocation"
  mkdir -p "$tmp/axon/sdk/go/easynet/invocation"
  mkdir -p "$tmp/axon/core/proto/axon/v1"
  mkdir -p "$tmp/axon/core/runtime-rs/client-sdk/proto/axon/v1"
  mkdir -p "$tmp/axon/sdk/rust/proto/axon/v1"
  mkdir -p "$tmp/axon/sdk/rust/src"
  mkdir -p "$tmp/axon/sdk/go/easynet"
  mkdir -p "$tmp/axon/sdk/python/easynet_axon"
  mkdir -p "$tmp/axon/core/runtime-rs" "$tmp/axon/core/runtime-rs/client-sdk"
  printf 'pub mod invocation;\n' > "$tmp/axon/sdk/rust/src/lib.rs"
  printf 'const CANONICAL_AXON_PROTO_FILES: &[&str] = &[];\n' > "$tmp/axon/core/runtime-rs/build.rs"
  printf 'const CANONICAL_AXON_PROTO_FILES: &[&str] = &[];\n' > "$tmp/axon/core/runtime-rs/client-sdk/build.rs"
  printf 'const CANONICAL_AXON_PROTO_FILES: &[&str] = &[];\n' > "$tmp/axon/sdk/rust/build.rs"
  mkdir -p "$tmp/axon/document/rfcs" "$tmp/axon/sdk"
  printf 'Withdrawn from Axon canonical protocol\n' > "$tmp/axon/document/rfcs/004-mcp-binding.md"
  printf '## Product-Owned Surfaces\n' > "$tmp/axon/sdk/SDK_PARITY.md"
  touch "$tmp/axon/sdk/java/src/main/java/run/easynet/axon/invocation/Axiom.java"
  touch "$tmp/axon/sdk/java/src/main/java/run/easynet/axon/invocation/Bundle.java"
  touch "$tmp/axon/sdk/python/easynet_axon/invocation/axiom.py"
  touch "$tmp/axon/sdk/python/easynet_axon/invocation/audit.py"
  touch "$tmp/axon/sdk/swift/Sources/EasyNetAxon/Invocation/Axiom.swift"
  touch "$tmp/axon/sdk/go/easynet/invocation/axiom.go"
  printf 'export interface ReceiptBody { readonly proofFacts?: ReceiptProofFacts; }\n' \
    > "$tmp/axon/sdk/node/src/invocation/axiom.d.ts"
  if ! rg -n 'proofFacts\?: ReceiptProofFacts' "$tmp/axon/sdk/node/src/invocation" >/dev/null; then
    fail "self-test expected receipt proof-fact default gate to fail"
  fi
  printf '' > "$tmp/axon/sdk/node/src/invocation/axiom.d.ts"
  mkdir -p "$tmp/axon/scripts/proto"
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'if [[ "${1:-}" != "--check" ]]; then' \
    '  exit 2' \
    'fi' \
    'if [[ -f "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/SCHEMA_DERIVATION_BROKEN" ]]; then' \
    '  exit 1' \
    'fi' \
    > "$tmp/axon/scripts/proto/sync_axon_v1.sh"
  mkdir -p "$tmp/axon-bad/scripts/proto"
  cp "$tmp/axon/scripts/proto/sync_axon_v1.sh" "$tmp/axon-bad/scripts/proto/sync_axon_v1.sh"
  touch "$tmp/axon-bad/SCHEMA_DERIVATION_BROKEN"
  if ( AXON_ROOT="$tmp/axon-bad"; check_schema_source_derivation_contract ) >/dev/null 2>&1; then
    fail "self-test expected schema-source derivation gate to fail"
  fi
  mkdir -p "$tmp/axon-product/sdk/rust/src"
  cp -R "$tmp/axon/core" "$tmp/axon-product/core"
  cp -R "$tmp/axon/document" "$tmp/axon-product/document"
  cp -R "$tmp/axon/sdk/SDK_PARITY.md" "$tmp/axon-product/sdk/SDK_PARITY.md"
  cp "$tmp/axon/sdk/rust/build.rs" "$tmp/axon-product/sdk/rust/build.rs"
  printf 'pub mod audio;\n' > "$tmp/axon-product/sdk/rust/src/lib.rs"
  touch "$tmp/axon-product/sdk/rust/src/audio.rs"
  mkdir -p "$tmp/axon-product/sdk/go/easynet/mcp"
  touch "$tmp/axon-product/sdk/go/easynet/tool_adapter.go"
  mkdir -p "$tmp/axon-product/sdk/python/easynet_axon/presets/remote_control"
  touch "$tmp/axon-product/sdk/python/easynet_axon/audio.py"
  mkdir -p "$tmp/axon-product/sdk/node/src/mcp"
  touch "$tmp/axon-product/sdk/node/src/tool_adapter.ts"
  mkdir -p "$tmp/axon-product/sdk/java/src/main/java/run/easynet/axon/mcp"
  touch "$tmp/axon-product/sdk/java/src/main/java/run/easynet/axon/AbilityToolAdapter.java"
  mkdir -p "$tmp/axon-product/sdk/swift/Sources/EasyNetAxon"
  touch "$tmp/axon-product/sdk/swift/Sources/EasyNetAxon/ToolAdapter.swift"
  if ( AXON_ROOT="$tmp/axon-product"; check_axon_product_protocol_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon product protocol boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-plain-proof"
  printf 'def canonical_invocation_bytes(env):\n  return b""\n' \
    > "$tmp/axon-plain-proof/sdk/python/easynet_axon/invocation/axiom.py"
  if ( AXON_ROOT="$tmp/axon-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon plain proof boundary gate to fail"
  fi
  printf '%s\n' \
    'use tonic::transport::{Channel, Endpoint, Uri};' \
    'let _ = endpoint.connect_with_connector(tower::service_fn(move |_: Uri| async {}));' \
    'let path = req.uri().path().to_string();' \
    'let request = hyper::Request::builder().uri("/v1/models");' \
    'let target_uri: hyper::Uri = "http://127.0.0.1/mcp".parse().unwrap();' \
    > "$tmp/transport-uri.rs"
  printf '%s\n' \
    'const caller_uri: &str = "easynet:///r/example/agent/alice";' \
    'fn rejects_empty_callee_URI() {}' \
    > "$tmp/semantic-uri.rs"
  check_active_ura_transport_classification_contract "$tmp/transport-uri.rs"
  if check_active_ura_transport_classification_contract "$tmp/semantic-uri.rs" >/dev/null 2>&1; then
    fail "self-test expected semantic URI terminology to fail"
  fi
  AXON_ROOT="$tmp/axon"
  check_manifest_contract
  check_active_source_contract
  check_sdk_product_neutrality_contract
  check_daemon_tuple_route_contract
  check_key_custody_boundary_contract
  check_daemon_mission_eal_boundary_contract
  check_ura_vocabulary_contract
  check_active_ura_transport_classification_contract "$ROOT/src" "$ROOT/tests" "$ROOT/include"
  check_schema_source_derivation_contract
  check_axon_product_protocol_boundary_contract
  check_axon_plain_proof_public_boundary_contract
  check_receipt_proof_fact_contract
  echo "canonical-runtime-convergence-v2 self-test ok"
  exit 0
fi

check_manifest_contract
check_active_source_contract
check_sdk_product_neutrality_contract
check_daemon_tuple_route_contract
check_key_custody_boundary_contract
check_daemon_mission_eal_boundary_contract
check_ura_vocabulary_contract
check_active_ura_transport_classification_contract "$ROOT/src" "$ROOT/tests" "$ROOT/include"
check_schema_source_derivation_contract
check_axon_product_protocol_boundary_contract
check_axon_plain_proof_public_boundary_contract
check_receipt_proof_fact_contract
echo "canonical-runtime-convergence-v2: OK"
