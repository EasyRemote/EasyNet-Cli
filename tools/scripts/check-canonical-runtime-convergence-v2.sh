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
    "verify_signature",
    "axiom.canonical_invocation_bytes",
    "admission.run_admission",
    "admission.verify_signature",
}
for section in ("languages", "members"):
    graph = manifest.get(section, {})
    for language, values in graph.items():
        leaked = sorted(plain_helpers & set(values))
        if leaked:
            raise SystemExit(f"canonical_plain_helper_leak:{language}:{section}:{','.join(leaked)}")

quarantine = manifest.get("non_canonical", {})
metadata = manifest.get("legacy_quarantine", {})
for helper in plain_helpers:
    section = "members" if "." in helper else "languages"
    if helper not in quarantine.get(section, {}).get("rust", []):
        raise SystemExit(f"plain_helper_not_quarantined:{section}:{helper}")
    reason = metadata.get(section, {}).get("rust", {}).get(helper, {}).get("reason", "")
    if "descriptor-bound proof" not in reason:
        raise SystemExit(f"plain_helper_reason_not_bound:{section}:{helper}")
PY
}

check_active_source_contract() {
  if rg -n 'default_auth_for_subject' "$ROOT/src" "$ROOT/sdk" "$ROOT/include" \
    --glob '!sdk/node/node_modules/**' \
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

check_ura_vocabulary_contract() {
  # This gate intentionally delegates SDK surface scanning to the canonical
  # SDK naming script, then adds active SPEC coverage for the V2 document.
  bash "$ROOT/tools/scripts/check-sdk-ura-naming.sh" >/dev/null
  if rg -n '\bU(RI|ri|ri)\b|[[:lower:][:digit:]]U(RI|ri)\b|_uri\b' \
    "$ROOT/docs/spec/canonical-runtime-convergence-v2.md"; then
    fail "canonical-runtime-convergence-v2 SPEC uses retired address terminology"
  fi
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
  mkdir -p "$tmp/axon/sdk/node/src/invocation"
  mkdir -p "$tmp/axon/sdk/java/src/main/java/run/easynet/axon/invocation"
  mkdir -p "$tmp/axon/sdk/python/easynet_axon/invocation"
  mkdir -p "$tmp/axon/sdk/swift/Sources/EasyNetAxon/Invocation"
  mkdir -p "$tmp/axon/sdk/go/easynet/invocation"
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
  AXON_ROOT="$tmp/axon"
  check_manifest_contract
  check_active_source_contract
  check_ura_vocabulary_contract
  check_schema_source_derivation_contract
  check_receipt_proof_fact_contract
  echo "canonical-runtime-convergence-v2 self-test ok"
  exit 0
fi

check_manifest_contract
check_active_source_contract
check_ura_vocabulary_contract
check_schema_source_derivation_contract
check_receipt_proof_fact_contract
echo "canonical-runtime-convergence-v2: OK"
