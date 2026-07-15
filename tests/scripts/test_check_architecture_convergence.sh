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
    "$CLI/src/daemon/execution/mcp" \
    "$CLI/src/daemon/ability/builtins/governance" \
    "$CLI/src/daemon/ability/builtins/resources" \
    "$CLI/src/daemon/ability/catalog/profiles" \
    "$CLI/src/daemon/boot/invocation" \
    "$CLI/src/daemon/invocation/admission" \
    "$CLI/src/daemon/invocation/dispatch" \
    "$CLI/src/daemon/persistence" \
    "$CLI/docs/spec" \
    "$CLI/sdk/go" \
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
  cat >"$CLI/src/daemon/ability/builtins/agents/lifecycle.rs" <<'EOF'
struct AgentLifecycleProjectionStore;

impl AgentLifecycleProjectionStore {
    fn persist_registry(&self, registry: &AgentRegistry) {
        agents::save_agents(registry);
    }

    fn persist_identities(&self, identities: &local_agents::LocalAgentsFile) {
        local_agents::save(identities);
    }

    fn restore_uncommitted_purge_snapshots(&self, journal: &AgentPurgeJournal) {
        self.persist_registry(&journal.original_registry);
        self.persist_identities(&journal.original_local_agents);
    }
}

impl AgentLifecycleTransaction {
    fn persist_registry_projection(&mut self, registry: &AgentRegistry) {
        self.projections.persist_registry(registry);
    }

    fn persist_identity_projection(&mut self, identities: &local_agents::LocalAgentsFile) {
        self.projections.persist_identities(identities);
    }
}

fn stop_agent_locked(registry: &AgentRegistry, identities: &local_agents::LocalAgentsFile) {
    transaction.persist_registry_projection(&registry);
    transaction.persist_identity_projection(&identities);
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
  cat >"$CLI/src/daemon/execution/mission/invocation_gateway.rs" <<'EOF'
struct DaemonMissionInvocationGateway {
    parent: AbilityContext,
}

impl DaemonMissionInvocationGateway {
    async fn invoke_child(&self, child: ChildInvocationRequest) {
        let prepared = self.parent.prepare_child_dispatch(child).await.unwrap();
        runtime.invoke_descriptor_bound_request_async(prepared.into_descriptor_request()).await.unwrap();
    }
}

#[cfg(test)]
struct CatalogMissionInvocationGateway {
    catalog: AxonAbilityCatalog,
}
EOF
  cat >"$CLI/src/daemon/execution/mcp/mod.rs" <<'EOF'
const MAX_CHILD_STDIO_LINE_BYTES: usize = 4 * 1024 * 1024;
const MAX_CHILD_STDIO_FRAME_BYTES: usize = 4 * 1024 * 1024;

async fn read_bounded_child_stdio_line(stdout: &mut ChildStdout, line: &mut Vec<u8>, max: usize) {}

async fn read_mcp_frame(stdout: &mut ChildStdout, len: usize) {
    if len > MAX_CHILD_STDIO_FRAME_BYTES {
        return;
    }
    let mut body = vec![0_u8; len];
}
EOF
  cat >"$CLI/src/daemon/execution/mcp/stdio.rs" <<'EOF'
const MAX_LINE_LENGTH: usize = 4 * 1024 * 1024;

fn read_bounded_line(reader: &mut Reader, line: &mut Vec<u8>, max: usize) {}

fn run(input: Input) {
    let mut input = Reader::new(input);
    let mut line = Vec::new();
    read_bounded_line(&mut input, &mut line, MAX_LINE_LENGTH);
}
EOF
  cat >"$CLI/src/daemon/ability/catalog/profiles/mcp.rs" <<'EOF'
fn descriptor_is_mcp_callable(
    descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
) -> bool {
    descriptor.call_mode() == crate::daemon::ability::descriptors::CallMode::Rpc
}

impl McpToolRouteTable {
    pub fn from_descriptors(
        descriptors: &[crate::daemon::ability::descriptors::AbilityDescriptor],
    ) -> Self {
        for (index, descriptor) in descriptors.iter().enumerate() {
            if !descriptor_is_mcp_callable(descriptor) {
                continue;
            }
            routes.push(ToolRoute { index });
        }
        Self { routes }
    }
}

#[test]
fn provider_excludes_geometries_it_cannot_invoke() {}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/cancellation.rs" <<'EOF'
struct RegistryState {
    terminal_order: VecDeque<String>,
}

impl RegistryState {
    fn retain_terminal_key(&mut self, key: &str) {
        if !self.terminal_order.iter().any(|retained| retained == key) {
            self.terminal_order.push_back(key.to_string());
        }
    }
}

fn mark_terminal(state: &mut RegistryState, key: &str) {
    state.retain_terminal_key(key);
}

#[test]
fn terminal_retention_order_is_idempotent() {}
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
  cat >"$CLI/src/daemon/ability/builtins/governance/access_control.rs" <<'EOF'
fn revoke_handler(args: Value) -> anyhow::Result<Value> {
    let request: RevokeRequest = serde_json::from_value(args)?;
    let owner_user_id = owner_user_id_from_mutation_boundary(request.owner_ura.as_deref())?;
    let actor_ura = require_actor_ura(request.actor_ura.as_deref())?;
    let mut store = AccessControlStore::open_or_create(owner_user_id.clone())?;
    let grant = store.revoke_grant(&request.grant_id, &owner_user_id, actor_ura, request.reason)?;
    Ok(json!({ "grant": grant }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeRequest {
    grant_id: String,
    #[serde(default)]
    owner_ura: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    actor_ura: Option<String>,
}

fn require_actor_ura(actor_ura: Option<&str>) -> anyhow::Result<&str> {
    let actor_ura = actor_ura
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("actor_ura is required for an audited mutation"))?;
    parse_ura(actor_ura)
        .map_err(|err| anyhow::anyhow!("actor_ura must be a canonical URA: {err}"))?;
    Ok(actor_ura)
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/resources/voice_contract.rs" <<'EOF'
use std::sync::Arc;

pub struct VoiceCallRepositoryQualification;

impl VoiceCallRepositoryQualification {
    fn validate_production(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn production(provider_id: String) -> Self {
        Self
    }

    #[cfg(test)]
    fn unqualified(provider_id: String) -> Self {
        Self
    }
}

pub trait VoiceCallRepository: std::fmt::Debug + Send + Sync {
    fn qualification(&self) -> VoiceCallRepositoryQualification;
}

pub struct VoiceCallProviderAssembly {
    repository: Arc<dyn VoiceCallRepository>,
    qualification: VoiceCallRepositoryQualification,
}

impl VoiceCallProviderAssembly {
    pub fn try_new(repository: Arc<dyn VoiceCallRepository>) -> anyhow::Result<Self> {
        let qualification = repository.qualification();
        qualification.validate_production()?;
        Ok(Self { repository, qualification })
    }
}

#[cfg(test)]
struct TestVoiceCallRepository;

#[cfg(test)]
impl VoiceCallRepository for TestVoiceCallRepository {
    fn qualification(&self) -> VoiceCallRepositoryQualification {
        VoiceCallRepositoryQualification::unqualified("test-in-memory".to_string())
    }
}
EOF
  cat >"$CLI/src/daemon/persistence/voice_calls.rs" <<'EOF'
use crate::daemon::ability::builtins::resources::voice_contract::{
    VoiceCallRepository, VoiceCallRepositoryQualification,
};
use crate::daemon::persistence::file_lock::{ExclusiveFileLock, SharedFileLock};

pub const VOICE_SHARED_ROOT_ENV: &str = "EASYNET_HUB_VOICE_SHARED_ROOT";

pub struct HubRealmVoiceCallRepository;

impl HubRealmVoiceCallRepository {
    pub fn from_env(realm: &str) -> anyhow::Result<Option<std::sync::Arc<Self>>> {
        let Some(root) = std::env::var_os(VOICE_SHARED_ROOT_ENV) else {
            return Ok(None);
        };
        Self::open_qualified(root, realm).map(|repository| Some(std::sync::Arc::new(repository)))
    }

    fn open_qualified(root: impl Into<std::path::PathBuf>, realm: &str) -> anyhow::Result<Self> {
        let root = root.into();
        if !root.is_absolute() {
            anyhow::bail!("{VOICE_SHARED_ROOT_ENV} must be absolute");
        }
        Ok(Self)
    }
}

impl VoiceCallRepository for HubRealmVoiceCallRepository {
    fn qualification(&self) -> VoiceCallRepositoryQualification {
        VoiceCallRepositoryQualification::production("shared-posix".to_string())
    }
}

fn guarded(path: &std::path::Path) -> anyhow::Result<()> {
    let _write = ExclusiveFileLock::acquire_for_data_path(path)?;
    let _read = SharedFileLock::acquire_for_data_path(path)?;
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/ability/catalog/build.rs" <<'EOF'
pub struct RegistrySharedStores {
    pub voice_calls: Option<VoiceCallProviderAssembly>,
}

impl RegistrySharedStores {
    pub fn with_voice_call_provider_assembly(mut self, provider: VoiceCallProviderAssembly) -> Self {
        self.voice_calls = Some(provider);
        self
    }
}

fn build_registry(shared_stores: RegistrySharedStores, hosts_hub_authority: bool) {
    let voice_provider_assembly = shared_stores.voice_calls.clone();
    if hosts_hub_authority {
        if let Some(provider) = voice_provider_assembly.as_ref() {
            voice_call_ability::register(&mut reg, provider.clone());
        }
    }
    let evidence = VoiceAssemblyEvidence {
        repository_assembled: voice_provider_assembly.is_some(),
        executable_delivery_evidence: false,
    };
}
EOF
  cat >>"$CLI/src/daemon/ability/builtins/resources/voice.rs" <<'EOF'
pub fn register(reg: &mut AxonAbilityCatalog, provider: VoiceCallProviderAssembly) {
    register_with_repository(reg, provider.repository());
}
EOF
  cat >"$CLI/src/daemon/boot/invocation/mod.rs" <<'EOF'
enum PublicationRecoveryOwner {
    None,
    UpstreamSession,
    Unsupported,
}

struct InvocationModeCapabilities {
    device_identity: bool,
    hub_runtime: bool,
    publication_recovery: PublicationRecoveryOwner,
}

impl InvocationModeCapabilities {
    fn for_mode(mode: DaemonMode) -> Self {
        match mode {
            DaemonMode::Device => Self {
                device_identity: true,
                hub_runtime: false,
                publication_recovery: PublicationRecoveryOwner::UpstreamSession,
            },
            DaemonMode::Hub => Self {
                device_identity: false,
                hub_runtime: true,
                publication_recovery: PublicationRecoveryOwner::None,
            },
            DaemonMode::Both => Self {
                device_identity: true,
                hub_runtime: true,
                publication_recovery: PublicationRecoveryOwner::Unsupported,
            },
        }
    }

    fn validate(self, mode: DaemonMode) -> anyhow::Result<()> {
        if self.device_identity && self.publication_recovery == PublicationRecoveryOwner::Unsupported {
            anyhow::bail!("{} mode has no owner; refusing before lifecycle mutation", mode.as_str());
        }
        Ok(())
    }

    fn owns_upstream_session(self) -> bool {
        self.publication_recovery == PublicationRecoveryOwner::UpstreamSession
    }
}

fn start_daemon_invocation_transport(config: DaemonConfig) -> anyhow::Result<()> {
    let capabilities = InvocationModeCapabilities::for_mode(config.mode());
    capabilities.validate(config.mode())?;
    if capabilities.owns_upstream_session() {
        register_purge_recovery_on_outbox_ready(outbox, registrar);
    }
    spawn_tcp_tls_listener(
        &config,
        listen_tcp,
        service.with_transport_boundary(AdmissionTransportBoundary::OffBoxStrict),
    )?;
    recover_pending_purge_on_boot(registrar)?;
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/receipt_projection.rs" <<'EOF'
fn project_terminal_receipt(receipt: &InvocationReceipt) -> ReceiptView {
    ReceiptView::from(receipt)
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/daemon_route_runtime.rs" <<'EOF'
use std::sync::Arc;

pub(crate) struct DaemonRouteRuntimeAdapter {
    runtime: Arc<LocalRuntime>,
}

impl DaemonRouteRuntimeAdapter {
    async fn register(&self, registrations: Vec<AbilityRegistration>) {
        self.runtime.register_many(registrations).await;
    }

    async fn dispatch(&self, route: DaemonUnaryRoute, request: &InvokeRequest, ingress: DaemonRouteIngress) {
        dispatch_rpc_admitted(&self.runtime, route, request, ingress).await;
    }
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/unary_dispatcher.rs" <<'EOF'
impl UnaryDispatcher {
    pub(crate) async fn dispatch_daemon_route_runtime(
        &self,
        route: DaemonUnaryRoute,
        request: &InvokeRequest,
        ingress: DaemonRouteIngress,
    ) {
        DaemonRouteRuntimeAdapter::new(runtime, cancellations)
            .dispatch(route, request, ingress)
            .await
    }
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'EOF'
impl DaemonInvocationService {
    pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.admission = self.admission.with_transport_boundary(boundary);
        self
    }

    pub(crate) async fn register_daemon_unary_routes(&self, owner_ura: &str) {
        DaemonRouteRuntimeAdapter::new(runtime, cancellations)
            .register(owner_ura, catalog.as_ref(), provider)
            .await;
    }

    async fn dispatch_daemon_unary_route(&self, route: DaemonUnaryRoute, request: &InvokeRequest, ingress: DaemonRouteIngress) {
        self.unary_dispatcher()
            .dispatch_daemon_route_runtime(route, request, ingress)
            .await;
    }
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/cancellation.rs" <<'EOF'
pub const ABILITY_INVOCATION_CANCEL: &str = "invocation.cancel";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationCancelCommand {
    pub target_lifecycle_hash: String,
    pub reason: String,
}

impl InvocationCancelCommand {
    pub fn new(target_lifecycle_hash: String, target_invocation_id: Option<String>, reason: String) -> Result<Self> {
        Ok(Self { target_lifecycle_hash, reason })
    }
}

pub fn invocation_lifecycle_hash(envelope: &DescriptorBoundEnvelope) -> String {
    hex::encode(Sha256::digest(envelope.canonical_bytes()))
}

struct RegistryState {
    terminal_order: VecDeque<String>,
}

impl RegistryState {
    fn retain_terminal_key(&mut self, key: &str) {
        if !self.terminal_order.iter().any(|retained| retained == key) {
            self.terminal_order.push_back(key.to_string());
        }
    }
}

fn mark_terminal(state: &mut RegistryState, key: &str) {
    state.retain_terminal_key(key);
}
EOF
  cat >"$CLI/src/daemon/ability/dispatch.rs" <<'EOF'
enum HotAgentAuthorityInventoryError {
    CounterOverflow,
}

struct HotAgentAuthorityEnrollment {
    agent: String,
}

struct HotAgentAuthorityInventoryState {
    generation: u64,
    next_incarnation: u64,
}

impl HotAgentAuthorityInventoryState {
    fn allocate_incarnation(&mut self, agent: &str) -> Result<u64, HotAgentAuthorityInventoryError> {
        let incarnation = self.next_incarnation;
        self.next_incarnation = self
            .next_incarnation
            .checked_add(1)
            .ok_or(HotAgentAuthorityInventoryError::CounterOverflow)?;
        Ok(incarnation)
    }

    fn advance_generation(&mut self, agent: &str) -> Result<(), HotAgentAuthorityInventoryError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(HotAgentAuthorityInventoryError::CounterOverflow)?;
        Ok(())
    }
}

fn enroll_persisted(
    state: &mut HotAgentAuthorityInventoryState,
    agent: &str,
) -> Result<(), HotAgentAuthorityInventoryError> {
    let _incarnation = state.allocate_incarnation(agent)?;
    state.advance_generation(agent)?;
    Ok(())
}

fn rollback_enrollment(
    state: &mut HotAgentAuthorityInventoryState,
    enrollment: HotAgentAuthorityEnrollment,
) -> Result<(), HotAgentAuthorityInventoryError> {
    state.advance_generation(&enrollment.agent)?;
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/request.rs" <<'EOF'
impl SignedInvocation {
    pub(crate) fn prepare_cancel_command(&self, reason: String) -> Result<PreparedInvocation> {
        let target = self.prepared.tuple();
        let command = InvocationCancelCommand::new(
            self.prepared.canonical_hash_hex(),
            None,
            reason,
        )?;
        DaemonInvocation::builder(
            &target.caller_ura,
            &target.callee_ura,
            ABILITY_INVOCATION_CANCEL,
            &target.subject_ura,
        )?
        .args_json(&serde_json::to_value(command)?)?
        .build_draft()?
        .prepare(PrepareOptions {
            signer_id: Some(target.caller_ura),
            policy_ref: Some("invocation.cancel.caller".to_string()),
        })
    }
}

#[test]
fn signed_invocation_prepares_independent_cancel_command() {
    let target_hash = prepared.canonical_hash_hex().to_string();
    let target_nonce = prepared.draft().invocation.nonce();
    let cancel = signed
        .prepare_cancel_command("operator stop".to_string())
        .expect("prepare independent cancel command");
    assert_ne!(cancel.draft().invocation.nonce(), target_nonce);
    let command: InvocationCancelCommand = serde_json::from_slice(cancel.draft().invocation.args()).unwrap();
    assert_eq!(command.target_lifecycle_hash, target_hash);
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/client.rs" <<'EOF'
impl RuntimeClient {
    pub async fn request_cancel_signed(
        &self,
        signed: SignedInvocation,
        reason: String,
    ) -> Result<InvocationHandle> {
        let caller_ura = signed.prepared().tuple().caller_ura;
        let prepared = signed.prepare_cancel_command(reason)?;
        let signer = RuntimeSigningIdentity::load_default(caller_ura)?;
        let signed_cancel = prepared.sign_with_canonical_signer(&signer).await?;
        let response = self
            .inner
            .invoke(signed_cancel.into_daemon_invocation())
            .await?;
        Ok(InvocationHandle::from_response(response))
    }
}
EOF
  cat >"$CLI/src/daemon/invocation/admission/admission_facade.rs" <<'EOF'
pub struct AdmissionFacade {
    transport_boundary: AdmissionTransportBoundary,
}

pub enum AdmissionTransportBoundary {
    LocalOnlyIpc,
    OffBoxStrict,
}

impl AdmissionTransportBoundary {
    fn admits_local_self(self) -> bool {
        matches!(self, Self::LocalOnlyIpc)
    }
}

impl AdmissionFacade {
    pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.transport_boundary = boundary;
        self
    }

    fn accepts_local_self_caller(&self, caller_ura: &str) -> bool {
        self.transport_boundary.admits_local_self() && caller_ura == self.daemon_ura()
    }
}

#[test]
fn off_box_facade_does_not_accept_daemon_ura_spoof_as_local_self() {}

#[test]
fn off_box_facade_does_not_accept_local_system_self_admission() {}

#[test]
fn signed_invocation_cancel_command_replay_is_rejected() {
    assert!(replay_store.rejects_duplicate("invocation.cancel"));
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
  cat >"$CLI/docs/spec/ffi-abi-v5.md" <<'EOF'
# EasyNet Generic C ABI v5

## Ownership state machines

- stream cancel and bidi cancel are cancel-request operations at this provider
  boundary; they release local callback/reader resources and must not claim
  lifecycle terminality without a canonical terminal receipt.
- stream close and bidi close are local resource release operations. Bidi
  close-send is a non-terminal local half-close.
EOF
  cat >"$CLI/sdk/go/cabi_runtime.go" <<'EOF'
func (s *cabiStreamTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
    return []byte(fmt.Sprintf(`{"stream_id":%q,"cancel_requested":true,"cancelled":false,"state":"CancelRequested","terminal":false}`, streamID)), nil
}

func (b *cabiBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
    return []byte(fmt.Sprintf(`{"session_id":%q,"state":"CancelRequested","terminal":false,"reason":"cancelled"}`, bidiID)), nil
}
EOF
  cat >"$CLI/sdk/python/easynet_sdk/_cabi.py" <<'EOF'
class _CABIStreamTransport:
    def cancel(self, reason: str) -> bytes:
        return _json_bytes(
            {
                "stream_id": str(self.stream_id),
                "cancel_requested": True,
                "cancelled": False,
                "state": "CancelRequested",
                "terminal": False,
            }
        )


class _CABIBidiTransport:
    def cancel(self, reason: str) -> bytes:
        return _json_bytes(
            {
                "session_id": str(self.bidi_id),
                "state": "CancelRequested",
                "terminal": False,
                "reason": reason,
            }
        )
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
cat >"$CLI/src/daemon/execution/mission/invocation_gateway.rs" <<'EOF'
struct DaemonMissionInvocationGateway {
    parent: AbilityContext,
}

struct CatalogMissionInvocationGateway {
    catalog: AxonAbilityCatalog,
}
EOF
expect_fail \
  "Mission catalog gateway escaped test cfg" \
  "R1_MISSION_CATALOG_GATEWAY_PRODUCTION"

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
cat >"$CLI/src/daemon/invocation/dispatch/unary_dispatcher.rs" <<'EOF'
impl UnaryDispatcher {
    pub(crate) async fn dispatch_daemon_route_runtime(
        &self,
        route: DaemonUnaryRoute,
        request: &InvokeRequest,
        ingress: DaemonRouteIngress,
    ) {
        direct_exact_route_handler(route, request, ingress).await;
    }
}
EOF
expect_fail \
  "daemon exact route runtime owner fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/agents/lifecycle.rs" <<'EOF'
fn stop_agent_locked(registry: &AgentRegistry, identities: &local_agents::LocalAgentsFile) {
    agents::save_agents(registry);
    local_agents::save(identities);
}
EOF
expect_fail \
  "agent lifecycle projection owner fork" \
  "R22_AGENT_LIFECYCLE_PROJECTION_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/execution/mcp/stdio.rs" <<'EOF'
const MAX_LINE_LENGTH: usize = 4 * 1024 * 1024;

fn run(mut input: Reader) {
    let mut line = String::new();
    input.read_line(&mut line).unwrap();
}
EOF
expect_fail \
  "mcp stdio unbounded frame reader" \
  "R23_MCP_STDIO_UNBOUNDED_FRAME_READER"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/cancellation.rs" <<'EOF'
struct RegistryState {
    terminal_order: VecDeque<String>,
}

fn mark_terminal(state: &mut RegistryState, key: &str) {
    state.terminal_order.push_back(key.to_string());
}

#[test]
fn terminal_retention_order_is_idempotent() {}
EOF
expect_fail \
  "cancel retention duplicate terminal token" \
  "R24_CANCEL_RETENTION_IDEMPOTENCY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/dispatch.rs" <<'EOF'
struct HotAgentAuthorityInventoryState {
    generation: u64,
    next_incarnation: u64,
}

fn enroll_persisted(state: &mut HotAgentAuthorityInventoryState) {
    state.next_incarnation = state.next_incarnation.wrapping_add(1);
    state.generation = state.generation.wrapping_add(1);
}
EOF
expect_fail \
  "hot authority generation wrapping" \
  "R25_HOT_AUTHORITY_GENERATION_WRAP"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/profiles/mcp.rs" <<'EOF'
impl McpToolRouteTable {
    pub fn from_descriptors(
        descriptors: &[crate::daemon::ability::descriptors::AbilityDescriptor],
    ) -> Self {
        for (index, descriptor) in descriptors.iter().enumerate() {
            routes.push(ToolRoute { index });
        }
        Self { routes }
    }
}
EOF
expect_fail \
  "mcp callable geometry publishes stream descriptors" \
  "R26_MCP_CALLABLE_GEOMETRY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/admission/admission_facade.rs" <<'EOF'
pub struct AdmissionFacade {
    loopback_trusted: bool,
}

impl AdmissionFacade {
    pub fn with_loopback_trusted(mut self, loopback_trusted: bool) -> Self {
        self.loopback_trusted = loopback_trusted;
        self
    }

    fn is_loopback(&self, caller_ura: &str) -> bool {
        self.loopback_trusted && caller_ura == self.daemon_ura()
    }
}

#[test]
fn signed_invocation_cancel_command_replay_is_rejected() {}
EOF
expect_fail \
  "admission transport boundary regressed to loopback bool" \
  "R27_ADMISSION_TRANSPORT_BOUNDARY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/boot/invocation/mod.rs" <<'EOF'
enum PublicationRecoveryOwner {
    None,
    UpstreamSession,
    Unsupported,
}

struct InvocationModeCapabilities {
    device_identity: bool,
    hub_runtime: bool,
    publication_recovery: PublicationRecoveryOwner,
}

impl InvocationModeCapabilities {
    fn for_mode(mode: DaemonMode) -> Self {
        match mode {
            DaemonMode::Device => Self {
                device_identity: true,
                hub_runtime: false,
                publication_recovery: PublicationRecoveryOwner::UpstreamSession,
            },
            DaemonMode::Hub => Self {
                device_identity: false,
                hub_runtime: true,
                publication_recovery: PublicationRecoveryOwner::None,
            },
            DaemonMode::Both => Self {
                device_identity: true,
                hub_runtime: true,
                publication_recovery: PublicationRecoveryOwner::UpstreamSession,
            },
        }
    }

    fn validate(self, mode: DaemonMode) -> anyhow::Result<()> {
        Ok(())
    }

    fn owns_upstream_session(self) -> bool {
        self.publication_recovery == PublicationRecoveryOwner::UpstreamSession
    }
}

fn start_daemon_invocation_transport(config: DaemonConfig) -> anyhow::Result<()> {
    let capabilities = InvocationModeCapabilities::for_mode(config.mode());
    if capabilities.owns_upstream_session() {
        register_purge_recovery_on_outbox_ready(outbox, registrar);
    }
    recover_pending_purge_on_boot(registrar)?;
    capabilities.validate(config.mode())?;
    Ok(())
}
EOF
expect_fail \
  "purge publication mode owner fork" \
  "R17_PURGE_PUBLICATION_MODE_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/governance/access_control.rs" <<'EOF'
fn revoke_handler(args: Value) -> anyhow::Result<Value> {
    let request: RevokeRequest = serde_json::from_value(args)?;
    let owner_user_id = request.owner_user_id.clone();
    let actor_ura = request
        .actor_ura
        .as_deref()
        .unwrap_or(owner_user_id.as_str());
    let mut store = AccessControlStore::open_or_create(owner_user_id.clone())?;
    let grant = store.revoke_grant(&request.grant_id, &owner_user_id, actor_ura, request.reason)?;
    Ok(json!({ "grant": grant }))
}

#[derive(Debug, Deserialize)]
struct RevokeRequest {
    grant_id: String,
    owner_user_id: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    actor_ura: Option<String>,
}
EOF
expect_fail \
  "access-control actor URA fork" \
  "R18_ACCESS_CONTROL_ACTOR_URA_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/resources/voice.rs" <<'EOF'
pub fn register(reg: &mut AxonAbilityCatalog, repository: Arc<dyn VoiceCallRepository>) {
    register_with_repository(reg, repository);
}
EOF
cat >"$CLI/src/daemon/persistence/voice_calls.rs" <<'EOF'
pub const VOICE_SHARED_ROOT_ENV: &str = "EASYNET_HUB_VOICE_SHARED_ROOT";

pub struct HubRealmVoiceCallRepository;

impl HubRealmVoiceCallRepository {
    pub fn from_env(realm: &str) -> anyhow::Result<Option<std::sync::Arc<Self>>> {
        let root = config::state_dir().join("voice-calls");
        Self::open_qualified(root, realm).map(|repository| Some(std::sync::Arc::new(repository)))
    }

    fn open_qualified(root: impl Into<std::path::PathBuf>, realm: &str) -> anyhow::Result<Self> {
        let root = root.into();
        Ok(Self)
    }
}
EOF
cat >"$CLI/src/daemon/ability/catalog/build.rs" <<'EOF'
fn build_registry(shared_stores: RegistrySharedStores, hosts_hub_authority: bool) {
    let repository = TestVoiceCallRepository::default();
    voice_call_ability::register(&mut reg, Arc::new(repository));
}
EOF
expect_fail \
  "voice provider boundary fork" \
  "R19_VOICE_PROVIDER_BOUNDARY_FORK"

make_good_fixture
cat >"$CLI/docs/spec/ffi-abi-v5.md" <<'EOF'
# EasyNet Generic C ABI v5

## Ownership state machines

- stream cancel/close and bidi cancel/close are terminal. Bidi close-send is a
  non-terminal local half-close.
EOF
cat >"$CLI/sdk/go/cabi_runtime.go" <<'EOF'
func (s *cabiStreamTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
    return []byte(fmt.Sprintf(`{"stream_id":%q,"cancelled":true,"state":"Cancelled","terminal":true}`, streamID)), nil
}

func (b *cabiBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
    return []byte(fmt.Sprintf(`{"session_id":%q,"state":"Cancelled","terminal":true}`, bidiID)), nil
}
EOF
cat >"$CLI/sdk/python/easynet_sdk/_cabi.py" <<'EOF'
class _CABIStreamTransport:
    def cancel(self, reason: str) -> bytes:
        return _json_bytes({"state": "Cancelled", "terminal": True})


class _CABIBidiTransport:
    def cancel(self, reason: str) -> bytes:
        return _json_bytes({"state": "Cancelled", "terminal": True})
EOF
expect_fail \
  "stream bidi cancel terminal authority fork" \
  "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/request.rs" <<'EOF'
impl SignedInvocation {
    pub(crate) fn prepare_cancel_command(&self, reason: String) -> Result<PreparedInvocation> {
        let target = self.prepared.tuple();
        let command = serde_json::json!({
            "reason": reason,
        });
        DaemonInvocation::builder(
            &target.caller_ura,
            &target.callee_ura,
            ABILITY_INVOCATION_CANCEL,
            &target.subject_ura,
        )?
        .args_json(&command)?
        .prepare(PrepareOptions::default())
    }
}
EOF
cat >"$CLI/src/daemon/invocation/dispatch/client.rs" <<'EOF'
impl RuntimeClient {
    pub async fn request_cancel_signed(
        &self,
        signed: SignedInvocation,
        reason: String,
    ) -> Result<InvocationHandle> {
        let response = self.inner.invoke(signed.into_daemon_invocation()).await?;
        Ok(InvocationHandle::from_response(response))
    }
}
EOF
expect_fail \
  "unary cancel signed command fork" \
  "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK"

make_good_fixture
expect_pass "fixture restored after all negative cases"

printf 'test_check_architecture_convergence.sh: all cases passed\n'
