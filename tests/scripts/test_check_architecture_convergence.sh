#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-architecture-convergence.sh"
SB="$(mktemp -d)"
CLI="$SB/EasyNet-Cli"
AXON="$SB/EasyNet-Axon"
OUT="$SB/check.out"
trap 'rm -rf "$SB"' EXIT

fail() {
  printf 'test_check_architecture_convergence: %s\n' "$1" >&2
  exit 1
}

run_check() {
  bash "$CHECK" --root "$CLI" --axon-root "$AXON" >"$OUT" 2>&1
}

expect_pass() {
  local label="$1"
  if ! run_check; then
    cat "$OUT" >&2
    fail "$label should pass"
  fi
}

expect_fail() {
  local label="$1"
  shift
  local rc
  set +e
  run_check
  rc=$?
  set -e
  [[ "$rc" == 1 ]] || {
    cat "$OUT" >&2
    fail "$label should exit 1 (got $rc)"
  }
  local marker
  for marker in "$@"; do
    grep -Fq "$marker" "$OUT" || {
      cat "$OUT" >&2
      fail "$label did not report $marker"
    }
  done
}

make_good_fixture() {
  rm -rf "$CLI" "$AXON"
  mkdir -p \
    "$CLI/src/eal/interpreter" \
    "$CLI/src/daemon/ability/builtins/automation" \
    "$CLI/src/daemon/ability/builtins/agents" \
    "$CLI/src/daemon/execution/mission" \
    "$CLI/src/daemon/ability/builtins/resources" \
    "$CLI/src/daemon/invocation/dispatch" \
    "$CLI/sdk/python/easynet_sdk" \
    "$AXON/core/runtime-rs/src/services/invocation" \
    "$AXON/sdk/rust/src/invocation"

  cat >"$CLI/src/eal/interpreter/dispatch.rs" <<'EOF'
fn dispatch_step(client: &InvocationClient, child: ChildInvocation) {
    client.invoke_remote(child);
}

// A direct executor name in documentation is not an execution edge:
// run_shell_exec and invoke_direct_with_progress.
EOF
  cat >"$CLI/src/daemon/ability/builtins/automation/mission.rs" <<'EOF'
fn register(reg: &mut Catalog) {
    reg.register_rpc_with_owner("mission.run", handler);
}

fn handler(client: &InvocationClient, child: ChildInvocation) {
    client.invoke_child(child);
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/agents/chat.rs" <<'EOF'
fn register(reg: &mut Catalog) {
    reg.register_rpc_with_owner("agents.chat", handler);
}

fn handler(binding: &ChatImplementationBinding) {
    binding.execute_admitted();
}
EOF
  cat >"$CLI/src/daemon/execution/mission/orchestration.rs" <<'EOF'
struct MissionRunAggregate {
    meta: MissionRunMeta,
}

impl MissionRunAggregate {
    fn apply_terminal(&mut self, terminal: MissionRunMeta) {
        self.meta = terminal;
    }

    fn cancel(&mut self) {
        self.meta.status = MissionRunStatus::Cancelled;
    }
}

fn write_mission_meta(path: &Path, meta: &MissionRunMeta) {
    let tmp_path = path.join(".meta.json.tmp");
    fs::write(tmp_path, serde_json::to_string(meta).unwrap()).unwrap();
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/resources/voice.rs" <<'EOF'
use easynet_axon::{
    VoiceCallState,
    VoiceEndReason,
    VoiceEventType,
    VoiceNetworkMetrics,
};

fn register(reg: &mut Catalog) {
    reg.register_rpc_with_owner("voice.list_calls", OwnerKind::Hub, handler);
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/receipt_projection.rs" <<'EOF'
fn project_terminal_receipt(receipt: &InvocationReceipt) -> ReceiptView {
    ReceiptView::from(receipt)
}
EOF
  cat >"$CLI/sdk/python/easynet_sdk/runtime.py" <<'EOF'
class InvocationHandle:
    def __init__(self, subject_ura: str):
        self.subject_ura = subject_ura

# Product words in comments and private symbols do not create public models.
class _MissionFixture:
    pass
EOF

  cat >"$AXON/core/runtime-rs/src/services/invocation/terminal_finalization.rs" <<'EOF'
struct TerminalFinalizationService<'a> {
    runtime: &'a Runtime,
}

impl TerminalFinalizationService<'_> {
    async fn finalize(&self, outcome: TerminalOutcome) {
        self.runtime.emit_terminal_receipt_from_admission(outcome);
        self.commit_side_effects().await;
    }

    async fn commit_side_effects(&self) {}
}
EOF
  cat >"$AXON/core/runtime-rs/src/services/invocation/receipt_emitter.rs" <<'EOF'
impl Runtime {
    fn emit_terminal_receipt_from_admission(&self, outcome: TerminalOutcome) -> InvocationReceipt {
        self.receipt_factory.emit(outcome)
    }
}
EOF
  cat >"$AXON/sdk/rust/src/lib.rs" <<'EOF'
pub mod invocation;
pub mod transport;
EOF
  cat >"$AXON/sdk/rust/src/invocation/mod.rs" <<'EOF'
pub struct AbilityUra {
    pub value: String,
}

pub struct InvocationEnvelope {
    pub subject_ura: String,
}
EOF
  cat >"$AXON/sdk/rust/src/transport.rs" <<'EOF'
pub struct TransportEndpoint {
    pub endpoint: tonic::transport::Uri,
}
EOF
}

bash -n "$CHECK"
make_good_fixture
expect_pass "canonical fixture"

cat >>"$CLI/src/eal/interpreter/dispatch.rs" <<'EOF'
fn bypass_chat() {
    invoke_direct_with_progress("agent", args);
}
EOF
expect_fail "EAL direct chat bypass" "R1_INVOCATION_BYPASS"

make_good_fixture
cat >>"$CLI/src/daemon/ability/builtins/automation/mission.rs" <<'EOF'
fn bypass_child(registry: &Catalog) {
    registry.invoke_rpc_json("agents.chat", args);
}
EOF
expect_fail "Mission direct catalog bypass" "R1_INVOCATION_BYPASS"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/bidi_dispatcher.rs" <<'EOF'
fn build_bidi_terminal_receipt() -> InvocationReceipt {
    InvocationReceipt { state: InvocationState::Failed }
}
EOF
expect_fail "second terminal writer" "R2_TERMINAL_WRITER_OUTSIDE_OWNER"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/manual_ledger.rs" <<'EOF'
struct RuntimePlane {
    invocation_ledger: Arc<InvocationLedger>,
}

fn record_unary_invocation() {}
EOF
expect_fail \
  "second ledger writer" \
  "R2_LEDGER_WRITER_OUTSIDE_AXON_SINK"

make_good_fixture
cat >>"$CLI/src/daemon/execution/mission/orchestration.rs" <<'EOF'
impl MissionRunDir {
    pub fn write_meta(&self, meta: &MissionRunMeta) {
        fs::write(self.path.join("meta.json"), serde_json::to_string(meta).unwrap()).unwrap();
    }
}
EOF
expect_fail \
  "mission meta public writer" \
  "R12_MISSION_RUN_META_WRITER_FORK"

make_good_fixture
cat >>"$CLI/src/daemon/execution/mission/orchestration.rs" <<'EOF'
fn write_meta_elsewhere(path: &Path, meta: &MissionRunMeta) {
    fs::write(path.join("meta.json"), serde_json::to_string(meta).unwrap()).unwrap();
}
EOF
expect_fail \
  "mission meta direct writer" \
  "R12_MISSION_RUN_META_WRITER_FORK"

make_good_fixture
cat >>"$CLI/src/daemon/ability/builtins/automation/mission.rs" <<'EOF'
fn run_through_cli() {
    crate::cli::commands::mission_runs::run_mission_inproc();
}
EOF
expect_fail \
  "daemon to CLI dependency" \
  "R5_DAEMON_DEPENDS_ON_CLI"

make_good_fixture
cat >"$AXON/sdk/rust/src/voice.rs" <<'EOF'
enum VoiceCallState {
    Active,
    Ended,
}

struct VoiceNetworkMetrics {
    packet_loss_ratio: f64,
}
EOF
expect_fail \
  "product voice protocol in Axon" \
  "R6_PRODUCT_PROTOCOL_IN_AXON"

make_good_fixture
cat >"$AXON/core/runtime-rs/src/services/invocation/terminal_finalization.rs" <<'EOF'
struct TerminalProjection;
EOF
expect_fail "missing terminal owner" "R2_TERMINAL_OWNER_MISSING"

make_good_fixture
cat >>"$CLI/sdk/python/easynet_sdk/runtime.py" <<'EOF'
class MissionRun:
    pass
EOF
expect_fail "Runtime facade product model" "R3_RUNTIME_PRODUCT_MODEL_PUBLIC"

make_good_fixture
cat >>"$CLI/sdk/python/easynet_sdk/runtime.py" <<'EOF'
def run_mission():
    pass
EOF
expect_fail "Runtime facade product operation" "R3_RUNTIME_PRODUCT_OPERATION_PUBLIC"

for adapter in audio mcp presets; do
  make_good_fixture
  printf 'pub mod %s;\n' "$adapter" >>"$AXON/sdk/rust/src/lib.rs"
  if [[ "$adapter" == "presets" ]]; then
    mkdir -p "$AXON/sdk/rust/src/presets"
    printf 'pub struct RuntimeConfig;\n' >"$AXON/sdk/rust/src/presets/mod.rs"
  else
    printf 'pub struct Adapter;\n' >"$AXON/sdk/rust/src/$adapter.rs"
  fi
  expect_fail "Axon $adapter adapter module" "R3_AXON_ADAPTER_MODULE"
done

make_good_fixture
cat >"$AXON/sdk/rust/src/ability.rs" <<'EOF'
pub fn deploy_to_node() {}
EOF
expect_fail "Axon deploy adapter operation" "R3_AXON_ADAPTER_OPERATION"

make_good_fixture
cat >>"$AXON/sdk/rust/src/invocation/mod.rs" <<'EOF'
pub struct AbilityAddress {
    pub ability_uri: hyper::Uri,
    pub subject_ura: tonic::transport::Uri,
}
EOF
expect_fail \
  "non-URA semantic addresses" \
  "R4_NON_URA_SEMANTIC_ADDRESS" \
  "R4_TRANSPORT_LOCATOR_AS_SEMANTIC_URA"

make_good_fixture
cat >>"$CLI/sdk/python/easynet_sdk/runtime.py" <<'EOF'
def decode_unary_result(decoded):
    return decoded.get("receipt")
EOF
expect_fail \
  "unary result receipt alias" \
  "R7_UNARY_RESULT_RECEIPT_ALIAS"

make_good_fixture
cat >"$CLI/sdk/python/easynet_sdk/direct_runtime.py" <<'EOF'
def emit_unary_result(terminal_receipt):
    return {"receipt": terminal_receipt}
EOF
expect_fail \
  "direct runtime unary receipt alias" \
  "R7_UNARY_RESULT_RECEIPT_ALIAS"

make_good_fixture
cat >"$CLI/sdk/python/easynet_sdk/stream.py" <<'EOF'
def decode_stream_event(decoded):
    return decoded.get("receipt")
EOF
expect_fail \
  "stream frame receipt alias" \
  "R11_STREAM_BIDI_RECEIPT_ALIAS"

make_good_fixture
mkdir -p "$CLI/sdk/go"
cat >"$CLI/sdk/go/bidi.go" <<'EOF'
type bidiFrameDTO struct {
    Receipt []byte `json:"receipt"`
}
EOF
expect_fail \
  "bidi frame receipt alias" \
  "R11_STREAM_BIDI_RECEIPT_ALIAS"

make_good_fixture
cat >"$CLI/sdk/python/easynet_sdk/direct_runtime.py" <<'EOF'
def emit_stream_event(event, terminal_receipt):
    event["receipt"] = terminal_receipt

def emit_bidi_event(event, receipt):
    event["payload_json"] = {"receipt": receipt}
EOF
expect_fail \
  "direct runtime stream bidi receipt alias" \
  "R11_STREAM_BIDI_RECEIPT_ALIAS"

make_good_fixture
mkdir -p "$CLI/src/ffi"
cat >"$CLI/src/ffi/errors.rs" <<'EOF'
fn typed_error_json() -> serde_json::Value {
    serde_json::json!({"details": {"legacy_untyped": true}})
}
EOF
expect_fail \
  "FFI legacy error detail" \
  "R8_FFI_LEGACY_ERROR_DETAIL"

make_good_fixture
cat >>"$CLI/src/daemon/ability/builtins/resources/voice.rs" <<'EOF'
fn register_device_mirror(reg: &mut Catalog) {
    reg.register_rpc_with_owner("voice.list_calls", OwnerKind::Device, handler);
}
EOF
expect_fail \
  "voice owner fork" \
  "R9_VOICE_OWNER_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/persistence"
cat >"$CLI/src/daemon/persistence/voice_calls.rs" <<'EOF'
struct LocalFileVoiceCallRepository;

fn open_local_voice_state() {
    let path = crate::daemon::persistence::config::state_dir().join("voice_calls.json");
}
EOF
expect_fail \
  "voice local state fork" \
  "R9_VOICE_LOCAL_STATE_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/bidi/state"
cat >"$CLI/src/daemon/invocation/bidi/state/presence.rs" <<'EOF'
fn insert_legacy() {
    let contract = SessionContract::legacy();
}
EOF
expect_fail \
  "retired carrier fallback" \
  "R10_RETIRED_CARRIER_FALLBACK"

make_good_fixture
cat >"$CLI/src/daemon/execution/mission/workspace_legacy.rs" <<'EOF'
fn legacy_agent_root() -> PathBuf {
    state_dir().join("workspaces")
}
EOF
expect_fail \
  "agent root fallback" \
  "R12_AGENT_ROOT_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/agents/list.rs" <<'EOF'
fn project_agent_row(entry: AgentEntry, name: &str) -> PathBuf {
    entry.root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(name))
}
EOF
expect_fail \
  "agent root_path fallback" \
  "R15_AGENT_ROOTPATH_FALLBACK"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/receipts"
cat >"$CLI/src/daemon/invocation/receipts/finalization_projection.rs" <<'EOF'
use easynet_axon::invocation::FinalizationCheckpointVerifier;

fn verify_wire(receipt: InvocationReceipt) {
    easynet_axon::invocation::wire::try_receipt_from_wire(receipt);
}
EOF
expect_pass "receipt proof primitives owned by adapter"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/manual_receipt_verify.rs" <<'EOF'
fn verify_wire(receipt: InvocationReceipt) {
    easynet_axon::invocation::wire::try_receipt_from_wire(receipt);
}
EOF
expect_fail \
  "receipt proof primitive outside adapter" \
  "R14_RECEIPT_PROOF_OWNER_FORK"

make_good_fixture
expect_pass "fixture restored after all negative cases"

printf 'test_check_architecture_convergence.sh: all cases passed\n'
