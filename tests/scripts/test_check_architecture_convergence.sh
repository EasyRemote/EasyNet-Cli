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
    "$CLI/src/cli/commands" \
    "$CLI/src/cli/commands/groups" \
    "$CLI/src/cli/presentation" \
    "$CLI/src/eal/interpreter" \
    "$CLI/src/daemon/ability/builtins/automation" \
    "$CLI/src/daemon/ability/builtins/agents" \
    "$CLI/src/daemon/ability/builtins/device_control" \
    "$CLI/src/daemon/ability/builtins/device_control/ability_management" \
    "$CLI/src/daemon/ability/builtins/integrations" \
    "$CLI/src/daemon/ability/builtins/integrations/a2a" \
    "$CLI/src/daemon/ability/builtins/integrations/mcp" \
    "$CLI/src/daemon/execution/mission" \
    "$CLI/src/daemon/execution/mcp" \
    "$CLI/src/daemon/ability/builtins/governance" \
    "$CLI/src/daemon/ability/builtins/resources" \
    "$CLI/src/daemon/ability/builtins/resources/files_store" \
    "$CLI/src/daemon/ability/builtins/resources/skills" \
    "$CLI/src/daemon/ability/catalog/profiles" \
    "$CLI/src/daemon/axon_bridge" \
    "$CLI/src/daemon/boot/invocation" \
    "$CLI/src/daemon/boot/lifecycle" \
    "$CLI/src/daemon/identity" \
    "$CLI/src/daemon/invocation/admission" \
    "$CLI/src/daemon/invocation/bidi/session_initiator" \
    "$CLI/src/daemon/invocation/dispatch" \
    "$CLI/src/daemon/invocation/routing" \
    "$CLI/src/daemon/invocation/streams" \
    "$CLI/src/daemon/persistence" \
    "$CLI/src/daemon/resources/context" \
    "$CLI/src/daemon/resources/pages" \
    "$CLI/src/daemon/resources/skills" \
    "$CLI/src/support/platform" \
    "$CLI/ability-descriptors/system/agents" \
    "$CLI/docs/spec" \
    "$CLI/sdk/go" \
    "$CLI/sdk/python/easynet_sdk" \
    "$CLI/sdk/python/tests" \
    "$AXON/core/runtime-rs/src/services/invocation" \
    "$AXON/sdk/rust/src/invocation"

  cat >"$CLI/src/eal/interpreter/dispatch.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;
use crate::daemon::execution::mission::invocation_gateway::MissionInvocationGateway;

fn load_registry_or_warn() {
    AgentAggregateRepository::load_registered_agent_registry_projection();
}

fn dispatch_step(gateway: &dyn MissionInvocationGateway, request: MissionInvocationRequest) {
    gateway.invoke_step(request);
}

// A direct executor name in documentation is not an execution edge:
// run_shell_exec and invoke_direct_with_progress.
EOF
  cat >"$CLI/src/daemon/resources/pages/pages_http_projection.rs" <<'EOF'
use serde_json::Value;

struct ServedBytes {
    status: u16,
    bytes: Vec<u8>,
    content_type: String,
    force_attachment: bool,
    sha256: String,
}

fn bytes_from_value(value: Value) -> anyhow::Result<ServedBytes> {
    use base64::Engine;
    let b64 = required_non_empty_string(&value, "bytes_b64")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
    let content_type = required_non_empty_string(&value, "content_type")?.to_string();
    let force_attachment = value
        .get("force_attachment")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("force_attachment must be a boolean"))?;
    let sha256 = required_non_empty_string(&value, "sha256")?.to_string();
    let actual_sha256 = hex_sha256(&bytes);
    if sha256 != actual_sha256 {
        anyhow::bail!("sha mismatch");
    }
    Ok(ServedBytes {
        status: 200,
        bytes,
        content_type,
        force_attachment,
        sha256,
    })
}
EOF
  cat >"$CLI/src/daemon/resources/pages/pages_listener.rs" <<'EOF'
use super::pages_http_projection::{serve_bytes, ServedBytes};

fn handle() {
    let _ = serve_bytes;
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/automation/mission.rs" <<'EOF'
fn register(reg: &mut Catalog) {
    reg.register_rpc_with_owner("mission.run", handler);
}

fn handler(client: &InvocationClient, child: ChildInvocation) {
    client.invoke_child(child);
}
EOF
  cat >"$CLI/src/cli/commands/abilities.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn local_agent_ura(agent: &str) -> anyhow::Result<String> {
    let snapshot = AgentAggregateRepository::load_hosted_identity_snapshot()?;
    snapshot.hosted_llm_agent_ura(agent).map(str::to_string).ok_or_else(|| anyhow::anyhow!("missing"))
}
EOF
  cat >"$CLI/src/cli/commands/reset.rs" <<'EOF'
use crate::daemon::lifecycle::{RuntimeLifecycleService, RuntimeLifecycleStatus};

fn run(args: ResetArgs) -> anyhow::Result<()> {
    let lifecycle_report = RuntimeLifecycleService::new().status()?;
    if !args.force && reset_runtime_is_active(lifecycle_report.status()) {
        anyhow::bail!("runtime is currently running");
    }
    if matches!(
        lifecycle_report.status(),
        RuntimeLifecycleStatus::ProjectionPresentProcessMissing
    ) {
        crate::daemon::persistence::config::remove()?;
    }
    crate::daemon::persistence::config::delete_credentials()?;
    Ok(())
}

fn reset_runtime_is_active(status: RuntimeLifecycleStatus) -> bool {
    matches!(
        status,
        RuntimeLifecycleStatus::Running
            | RuntimeLifecycleStatus::ProjectionMissingProcessRunning
            | RuntimeLifecycleStatus::ControlOnlyInvocationDown
    )
}
EOF
  cat >"$CLI/src/cli/commands/groups/mcp.rs" <<'EOF'
use crate::daemon::lifecycle::{RuntimeLifecycleService, RuntimeStatusReport};

fn run_status() -> anyhow::Result<()> {
    let report = RuntimeLifecycleService::new().status()?;
    if report.daemon().has_daemon_fact() {
        render_lifecycle_details(&report);
    }
    Ok(())
}

fn render_lifecycle_details(report: &RuntimeStatusReport) {
    if let Some(projection) = report.projection() {
        let _state = projection.as_runtime_state();
        return;
    }
    if let Some(_discovery) = report.daemon().control_discovery() {}
}
EOF
  cat >"$CLI/src/cli/presentation/banner.rs" <<'EOF'
use crate::daemon::lifecycle::{RuntimeLifecycleError, RuntimeLifecycleService, RuntimeLifecycleStatus, RuntimeStatusReport};

fn write_runtime_status() {
    let lifecycle = RuntimeLifecycleService::new().status();
    let _observation = BannerDaemonObservation::from_lifecycle_result(&lifecycle);
    let _binding = runtime_user_binding_display(&creds);
}

struct BannerDaemonObservation;

impl BannerDaemonObservation {
    fn from_lifecycle_result(
        lifecycle: &Result<RuntimeStatusReport, RuntimeLifecycleError>,
    ) -> Self {
        match lifecycle {
            Ok(report) => Self::from_lifecycle_status(report.status()),
            Err(error) => {
                let _message = format!("metadata unavailable  ·  {error}");
                Self
            }
        }
    }

    fn from_lifecycle_status(_status: RuntimeLifecycleStatus) -> Self {
        Self
    }
}
EOF
  cat >"$CLI/src/cli/presentation/identity.rs" <<'EOF'
pub enum RuntimeUserBindingDisplayState {
    Bound,
    Unbound,
    Invalid,
}

pub fn runtime_user_binding_display(creds: &config::Credentials) -> RuntimeUserBindingDisplayState {
    RuntimeUserBindingDisplayState::Invalid
}
EOF
  cat >"$CLI/src/cli/commands/status.rs" <<'EOF'
fn render_status(creds: &config::Credentials) {
    let _state = runtime_user_binding_display(creds);
}
EOF
  cat >"$CLI/src/cli/commands/auth.rs" <<'EOF'
fn render_auth(creds: &config::Credentials) {
    let _state = runtime_user_binding_display(creds);
}
EOF
  cat >"$CLI/src/cli/commands/groups/device.rs" <<'EOF'
struct DeviceLocalIdentity {
    realm: String,
    node_id: String,
}

impl DeviceLocalIdentity {
    fn from_credentials(creds: &Credentials) -> anyhow::Result<Self> {
        Ok(Self {
            realm: creds.realm_str().trim().to_string(),
            node_id: creds.node_id.trim().to_string(),
        })
    }

    fn device_ura(&self) -> String {
        crate::core::ura::device_ura(&self.realm, &self.node_id)
    }
}

fn load_local_device_identity(operation: &str) -> anyhow::Result<DeviceLocalIdentity> {
    let creds = crate::daemon::persistence::config::load_credentials()
        .with_context(|| format!("{operation} requires complete local device credentials"))?;
    DeviceLocalIdentity::from_credentials(&creds)
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let local_identity = load_local_device_identity("device remove")?;
    let target_ura = canonicalize_remove_target_ura(args.node_id.trim())?;
    let local_ura = local_identity.device_ura();
    invoke_revoke(&target_ura, "reason", local_ura.as_str())
}

fn describe_target(node_id: &str) -> anyhow::Result<Value> {
    let trimmed = node_id.trim();
    if trimmed.eq_ignore_ascii_case("local") {
        return invoke_local();
    }
    let local_identity = load_local_device_identity("device show")?;
    match classify_device_show_target(trimmed, &local_identity)? {
        DeviceShowTarget::Local => invoke_local(),
        DeviceShowTarget::RemoteDevice(target_ura) => invoke_remote_describe(&target_ura),
    }
}

fn classify_device_show_target(raw: &str, local_identity: &DeviceLocalIdentity) -> anyhow::Result<DeviceShowTarget> {
    let target = raw.trim();
    if target == local_identity.node_id || target == local_identity.device_ura() {
        return Ok(DeviceShowTarget::Local);
    }
    Ok(DeviceShowTarget::RemoteDevice(target.to_string()))
}
EOF
  cat >"$CLI/src/daemon/resources/pages/pages_listener.rs" <<'EOF'
fn parse_pages_api_body(body_bytes: &[u8]) -> Result<serde_json::Value, serde_json::Error> {
    if body_bytes.is_empty() {
        Ok(serde_json::Value::Null)
    } else {
        serde_json::from_slice(body_bytes)
    }
}
EOF
  cat >"$CLI/src/daemon/boot/lifecycle/projection.rs" <<'EOF'
use crate::daemon::persistence::config;

pub struct RuntimeProjectionStore;
pub struct RuntimeSessionProjection;

impl RuntimeProjectionStore {
    pub fn load(&self) -> anyhow::Result<Option<RuntimeSessionProjection>> {
        RuntimeSessionProjection::load_current()
    }
}

impl RuntimeSessionProjection {
    pub fn from_state(_state: config::RuntimeState) -> Self {
        Self
    }

    pub fn load_current() -> anyhow::Result<Option<Self>> {
        Ok(config::load_optional_runtime_state()?.map(Self::from_state))
    }
}
EOF
  cat >"$CLI/src/daemon/boot/lifecycle/service.rs" <<'EOF'
pub struct RuntimeLifecycleService {
    projection_store: RuntimeProjectionStore,
}

impl RuntimeLifecycleService {
    pub fn status(&self) -> Result<RuntimeStatusReport, RuntimeLifecycleError> {
        let projection = self.projection_store.load().map_err(|source| {
            RuntimeLifecycleError::ProjectionLoadFailed {
                message: source.to_string(),
            }
        })?;
        Ok(RuntimeStatusReport::from_parts(projection))
    }

    pub fn stop_plan(&self) -> Result<RuntimeStopPlan, RuntimeLifecycleError> {
        Ok(RuntimeStopPlan::from_report(&self.status()?))
    }
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/agents/discover.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::{AgentAggregateRepository, AgentHostedIdentitySnapshot};

struct LocalAgentAbilityOwners {
    snapshot: AgentHostedIdentitySnapshot,
}

impl LocalAgentAbilityOwners {
    fn load() -> anyhow::Result<Self> {
        Ok(Self {
            snapshot: AgentAggregateRepository::load_hosted_identity_snapshot()?,
        })
    }

    fn owner_ura_for(&self, agent_name: &str) -> Option<String> {
        self.snapshot.hosted_llm_agent_ura(agent_name).map(str::to_string)
    }
}
EOF
  cat >"$CLI/src/daemon/ability/catalog/profiles/mod.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn load_host_descriptors() -> Vec<AbilityDescriptor> {
    let snapshot = AgentAggregateRepository::load_hosted_identity_snapshot().unwrap();
    let projection = snapshot.host_descriptor_identity_projection();
    let device = projection.host_device_agent_ura();
    let consent = projection.consent_agent_ura();
    let mcp = projection.mcp_agent_ura();
    let llm = projection.llm_agent_uras();
    all_descriptors_for_host(device.unwrap(), consent, mcp, llm)
}
EOF
  cat >"$CLI/src/daemon/identity/local_invocation.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn persisted_local_device_ura() -> Option<String> {
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_status().ok()?;
    hosted_identity.host_device_agent_ura().map(str::to_string)
}
EOF
  cat >"$CLI/src/daemon/invocation/bidi/session_wire.rs" <<'EOF'
enum SessionDispatch {
    BidiInput {
        call_id: u64,
        payload: Vec<u8>,
        eof: bool,
    },
    Request {
        call_id: [u8; 16],
    },
    RequestResult {
        call_id: [u8; 16],
    },
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/local_session_dispatcher.rs" <<'EOF'
fn canonical_carrier_control_failure(call_id: u64) -> DispatchResult {
    DispatchResult {
        call_id,
        terminal: false,
        failure: Some(control_failure()),
        ..Default::default()
    }
}

async fn send_bidi_terminal(
    outbound: &SessionUpSender,
    call_id: u64,
    finalized: &FinalizedInvocation,
) {
    outbound.send(DispatchResult {
        call_id,
        terminal: true,
        terminal_receipt: Some(receipt_to_session_wire(
            &finalized.terminal_receipt,
        )),
        ..Default::default()
    });
}

fn callee_ura_from_envelope(envelope: &Envelope) -> anyhow::Result<String> {
    crate::daemon::invocation::dispatch::invocation_wire::callee_ura_from_envelope(envelope)
}
EOF
  cat >"$CLI/src/daemon/axon_bridge/runtime_factory.rs" <<'EOF'
fn ledger_invocation_ura() {
    panic!("LedgerSink cannot derive invocation record URA from binding subject=`x` callee=`y` caller=`z` invocation_id=`i`")
}

fn ledger_route_ura() {
    panic!("LedgerSink cannot derive ability URA from binding callee=`y` caller=`z` ability=`a`")
}
EOF
  cat >"$CLI/src/daemon/resources/context/clipboard_tracker.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

pub fn spawn() {
    let device_ura = AgentAggregateRepository::load_hosted_identity_status()
        .ok()
        .and_then(|status| status.host_device_agent_ura().map(str::to_string))
        .unwrap_or_default();
    run_loop(&device_ura);
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/agents/chat.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn register(reg: &mut Catalog) {
    reg.register_rpc_with_owner("agents.chat", handler);
}

fn handler(binding: &ChatImplementationBinding) {
    binding.execute_admitted();
}

fn build_discover_handler_for() {
    AgentAggregateRepository::load_snapshot()
        .map(|snapshot| snapshot.registered_agent_registry_projection());
}

fn build_invoke_handler_for() {
    AgentAggregateRepository::load_snapshot()
        .map(|snapshot| snapshot.registered_agent_registry_projection());
}

fn enumerate_other_agent_specs() -> anyhow::Result<Vec<AgentAbilitySpec>> {
    let snapshot = AgentAggregateRepository::load_snapshot().map_err(|error| {
        anyhow::anyhow!("load cross-agent ability registry projection: {error:#}")
    })?;
    Ok(snapshot.registered_agents().collect())
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/agents/lifecycle.rs" <<'EOF'
struct AgentLifecycleProjectionStore;

pub const ABILITY_PURGE_AGENT: &str = crate::daemon::ability::names::agents::AGENT_PURGE;

fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        ABILITY_PURGE_AGENT,
        OwnerKind::Device,
        Arc::new(move |args: Value| purge_agent_handler(args, &registrar_for_purge)),
    );
}

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

fn bootstrap_local_agent_projection(plan: &BootstrapPlan) {
    let _mutation_guard = AgentLifecycleMutationGuard::acquire()
        .map_err(|error| anyhow::anyhow!("agent.bootstrap: acquire lifecycle transaction: {error:#}"))?;
    let mut identities = local_agents::load()?;
    let outcomes = bootstrap::bootstrap_local_agents(plan, &mut identities, &UuidMinter);
    AgentLifecycleProjectionStore::default().persist_identities(&identities)?;
    Ok(outcomes)
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
    if args.get("purge").is_some() {
        anyhow::bail!("agent.stop: `purge` is not accepted; invoke `agent.purge`");
    }
    let registry_key = canonical_agent_registry_key(&agent_id)?;
    registry.agents.remove(&registry_key);
    transaction.persist_registry_projection(&registry);
    transaction.persist_identity_projection(&identities);
}

fn start_agent_locked(registry: &mut AgentRegistry, agent_id: AgentId, entry: AgentEntry) {
    let registry_key = agent_id.to_string();
    registry.agents.insert(registry_key.clone(), entry.clone());
}

fn canonical_agent_registry_key(agent_id: &AgentId) -> anyhow::Result<String> {
    Ok(agent_id.to_string())
}

struct PlatformTreeDeletion;

impl PlatformTreeDeletion {
    fn require_supported() -> anyhow::Result<()> {
        Ok(())
    }

    fn remove_quarantined_directory_identity_bound(
        quarantine: &std::path::Path,
        expected_identity: &AgentRootIdentity,
    ) -> anyhow::Result<()> {
        delete_identity_bound_quarantine(quarantine, expected_identity)
    }
}

fn purge_agent_handler(args: Value, hot_registrar: &SharedHotRegistrarCell) -> anyhow::Result<Value> {
    PlatformTreeDeletion::require_supported()?;
    purge_agent_locked(args, hot_registrar)
}

fn finalize_committed_purge(journal: &AgentPurgeJournal) -> anyhow::Result<()> {
    PlatformTreeDeletion::remove_quarantined_directory_identity_bound(
        &journal.quarantine_path,
        journal.root_identity.as_ref().unwrap(),
    )
}

pub fn purge_agent_input_schema() -> Value {
    stop_agent_input_schema()
}

pub fn purge_agent_description() -> &'static str {
    "Destructively remove an LLM sub-agent and the exact canonical root_path stored in its registry row. Requires Manage authority."
}
EOF
  cat >"$CLI/src/daemon/persistence/agent_aggregate.rs" <<'EOF'
enum AgentAggregateSnapshotLoadError {
    RegistryUnreadable { source: anyhow::Error },
    IdentityUnreadable { source: anyhow::Error },
}

enum AgentRegistryProjectionLoadError {
    RegistryUnreadable { source: anyhow::Error },
}

pub(crate) struct AgentAggregateSnapshot {
    pub(crate) registry: AgentRegistry,
    pub(crate) local_agents: local_agents::LocalAgentsFile,
}

impl AgentAggregateSnapshot {
    fn has_registered_agent(&self, agent: &str) -> bool {
        let registry_key = AgentId::parse(agent).unwrap().to_string();
        self.registry.agents.contains_key(&registry_key)
    }
}

enum HostedLlmAgentIdentity<'a> {
    Missing,
    Present(&'a HostedAgentEntry),
    Ambiguous,
}

enum HostedAgentNameLookupError {
    Ambiguous,
    InvalidUra,
    NonAgentUra,
}

struct HostedAgentIdentityProjection<'a> {
    profile: &'a str,
    name: &'a str,
    agent_ura: &'a str,
    signing_authority: &'a str,
}

impl AgentAggregateRepository {
    fn load_snapshot() -> anyhow::Result<AgentAggregateSnapshot> {
        let registry = agent_registry::load_agents()?;
        let local_agents = local_agents::load_for_fresh_host_projection()?;
        Ok(AgentAggregateSnapshot { registry, local_agents })
    }
}

struct AgentLocalTargetProjection {
    hosted_agent_targets: BTreeSet<HostedAgentTarget>,
    registered_agent_ids: BTreeSet<String>,
}

struct HostedAgentTarget {
    realm: String,
    user_id: String,
    agent_id: String,
}

struct AgentHostedPlacementProjection {
    by_agent_ura: BTreeMap<String, AgentHostedPlacement>,
}

struct AgentHostedPlacement {
    agent_ura: String,
    host_device_ura: String,
    host_node_id: Option<String>,
}

struct AgentRegisteredWorkspace;
struct AgentRegisteredRuntimeProjection;
enum AgentSkillLayout {
    ClaudeCode,
    Codex,
    External,
}

enum AgentRegisteredWorkspaceLookupError {
    Missing,
    InvalidWorkspace,
}

impl AgentRegisteredWorkspace {
    fn root_path(&self) -> &Path {
        Path::new("/tmp/agent")
    }

    fn skill_layout(&self) -> AgentSkillLayout {
        AgentSkillLayout::ClaudeCode
    }
}

impl AgentRegisteredRuntimeProjection {
    fn ability_manifest_path(&self, ability: &str) -> Option<PathBuf> {
        Some(PathBuf::from("abilities").join(format!("{ability}.ability.toml")))
    }
}

	struct AgentHostedIdentityStatus {
	    host_device_agent_ura: Option<String>,
	    hosted_agent_count: usize,
	}

	struct AgentHostedSkillOwnerProjection;

	impl AgentHostedSkillOwnerProjection {
	    fn hosted_ura_for(&self, agent_name: &str) -> Option<&str> {
	        Some("easynet:///r/acme/agent/user.claude")
	    }

	    fn owner_name_for_agent_ura(&self, agent_ura: &str) -> Option<&str> {
	        Some("claude")
	    }
	}

struct AgentHostDescriptorIdentityProjection {
    host_device_agent_ura: Option<String>,
    consent_agent_ura: Option<String>,
    mcp_agent_ura: Option<String>,
    llm_agent_uras: Vec<(String, String)>,
}

impl AgentHostDescriptorIdentityProjection {
    fn host_device_agent_ura(&self) -> Option<&str> {
        self.host_device_agent_ura.as_deref()
    }

    fn consent_agent_ura(&self) -> Option<&str> {
        self.consent_agent_ura.as_deref()
    }

    fn mcp_agent_ura(&self) -> Option<&str> {
        self.mcp_agent_ura.as_deref()
    }

    fn llm_agent_uras(&self) -> &[(String, String)] {
        &self.llm_agent_uras
    }
}

struct AgentHostedIdentitySnapshot;
struct AgentHostedAdvertiseEntry;

impl AgentHostedIdentitySnapshot {
    fn host_descriptor_identity_projection(&self) -> AgentHostDescriptorIdentityProjection {
        AgentHostDescriptorIdentityProjection {
            host_device_agent_ura: Some("easynet:///r/acme/device/dev-1".to_string()),
            consent_agent_ura: Some("easynet:///r/acme/agent/user.consent".to_string()),
            mcp_agent_ura: Some("easynet:///r/acme/agent/user.mcp".to_string()),
            llm_agent_uras: vec![
                ("claude".to_string(), "easynet:///r/acme/agent/user.claude".to_string()),
            ],
        }
    }

    fn hosted_llm_agent_ura(&self, agent: &str) -> Option<&str> {
        Some("easynet:///r/acme/agent/user.claude")
    }

    fn hosted_advertise_entries(&self, realm: &str, user_segment: &str) -> Vec<AgentHostedAdvertiseEntry> {
        vec![AgentHostedAdvertiseEntry]
    }

    fn hosted_agent_authority_roots(&self) -> Vec<String> {
        vec!["easynet:///r/acme/agent/user.claude".to_string()]
    }
}

impl AgentHostedAdvertiseEntry {
    fn agent_ura(&self) -> &str {
        "easynet:///r/acme/agent/user.claude"
    }

    fn short_label(&self) -> &str {
        "user.claude"
    }
}

impl AgentAggregateSnapshot {
    fn has_registered_agent(&self, agent: &str) -> bool {
        self.registry.agents.contains_key(agent)
    }

    fn registered_agent_surface_names(&self) -> BTreeSet<String> {
        BTreeSet::new()
    }

    fn registered_agent_registry_projection(&self) -> AgentRegistry {
        self.registry.clone()
    }

    fn registered_agent_names(&self) -> impl Iterator<Item = &str> {
        self.registry.agents.keys().map(String::as_str)
    }

    fn registered_agent_workspace(&self, owner_id: &str, operation: &str) -> anyhow::Result<AgentRegisteredWorkspace> {
        Ok(AgentRegisteredWorkspace)
    }

    fn registered_agent_runtime_projection(&self, owner_id: &str) -> Option<AgentRegisteredRuntimeProjection> {
        Some(AgentRegisteredRuntimeProjection)
    }

    fn registered_agents(&self) -> impl Iterator<Item = (&str, &AgentEntry)> {
        self.registry.agents.iter().map(|(name, entry)| (name.as_str(), entry))
    }

    fn host_device_agent_ura(&self) -> &str {
        &self.local_agents.host_device_agent_ura
    }

	    fn hosted_identity_status(&self) -> AgentHostedIdentityStatus {
	        AgentHostedIdentityStatus {
	            host_device_agent_ura: Some(self.local_agents.host_device_agent_ura.clone()),
	            hosted_agent_count: self.local_agents.hosted_agents.len(),
	        }
	    }

	    fn hosted_skill_owner_projection(&self) -> AgentHostedSkillOwnerProjection {
	        AgentHostedSkillOwnerProjection
	    }

    fn hosted_llm_agent_identity(&self, agent: &str) -> HostedLlmAgentIdentity<'_> {
        HostedLlmAgentIdentity::Present(&self.local_agents.hosted_agents[0])
    }

    fn has_hosted_llm_agent_identity(&self, agent: &str) -> bool {
        self.local_agents.hosted_agents.iter().any(|entry| entry.profile == "llm" && entry.name == agent)
    }

    fn hosted_llm_agent_ura(&self, agent: &str) -> Option<&str> {
        Some("easynet:///r/acme/agent/user.claude")
    }

    fn hosted_agent_ura_by_name(&self, agent: &str) -> Result<Option<&str>, HostedAgentNameLookupError> {
        Ok(Some("easynet:///r/acme/agent/user.claude"))
    }

    fn hosted_agent_identity_by_name(&self, agent: &str) -> Result<Option<HostedAgentIdentityProjection<'_>>, HostedAgentNameLookupError> {
        Ok(Some(HostedAgentIdentityProjection {
            profile: "llm",
            name: agent,
            agent_ura: "easynet:///r/acme/agent/user.claude",
            signing_authority: "hosted_by:easynet:///r/acme/agent/user.device",
        }))
    }

    fn hosted_agent_identity_by_ura(&self, agent_ura: &str) -> Option<HostedAgentIdentityProjection<'_>> {
        Some(HostedAgentIdentityProjection {
            profile: "llm",
            name: "claude",
            agent_ura,
            signing_authority: "hosted_by:easynet:///r/acme/agent/user.device",
        })
    }

    fn local_target_projection(&self) -> AgentLocalTargetProjection {
        AgentLocalTargetProjection {
            hosted_agent_targets: BTreeSet::new(),
            registered_agent_ids: BTreeSet::new(),
        }
    }

    fn hosted_agent_placements(&self) -> AgentHostedPlacementProjection {
        AgentHostedPlacementProjection {
            by_agent_ura: BTreeMap::new(),
        }
    }
}

pub(crate) struct AgentAggregateRepository;

impl AgentAggregateRepository {
    fn load_registered_agent_registry_projection() -> Result<AgentRegistry, AgentRegistryProjectionLoadError> {
        agent_registry::load_agents()
            .map_err(|source| AgentRegistryProjectionLoadError::RegistryUnreadable { source })
    }

    pub(crate) fn load_snapshot() -> anyhow::Result<AgentAggregateSnapshot> {
        Self::try_load_snapshot().map_err(Into::into)
    }

    pub(crate) fn load_hosted_identity_snapshot() -> anyhow::Result<AgentHostedIdentitySnapshot> {
        Ok(AgentHostedIdentitySnapshot)
    }

    pub(crate) fn load_hosted_identity_status() -> anyhow::Result<AgentHostedIdentityStatus> {
        Ok(AgentHostedIdentityStatus {
            host_device_agent_ura: Some(Self::load_hosted_identity_projection()?.host_device_agent_ura),
            hosted_agent_count: 0,
        })
    }

    pub(crate) fn load_registered_agent_workspace(owner_id: &str, operation: &str) -> anyhow::Result<AgentRegisteredWorkspace> {
        Ok(AgentRegisteredWorkspace)
    }

    pub(crate) fn try_load_snapshot() -> Result<AgentAggregateSnapshot, AgentAggregateSnapshotLoadError> {
        Ok(AgentAggregateSnapshot {
            registry: agent_registry::load_agents()?,
            local_agents: Self::load_hosted_identity_projection()?,
        })
    }

    fn load_hosted_identity_projection() -> Result<LocalAgentsFile, AgentAggregateSnapshotLoadError> {
        local_agents::load()
    }
}
EOF
  cat >"$CLI/src/daemon/persistence/mod.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

pub fn hosted_agent_authority_roots() -> anyhow::Result<Vec<String>> {
    Ok(AgentAggregateRepository::load_hosted_identity_snapshot()?
        .hosted_agent_authority_roots())
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/governance/teach.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, HostedAgentNameLookupError,
};

fn hosted_agent_lookup_error(error: HostedAgentNameLookupError) {}

fn authorize_teach(owner: &str, learner_ura: &str) -> anyhow::Result<()> {
    let snapshot = AgentAggregateRepository::load_snapshot()?;
    let owner_identity = snapshot.hosted_agent_identity_by_name(owner)
        .map_err(hosted_agent_lookup_error)?;
    let learner_identity = snapshot.hosted_agent_identity_by_ura(learner_ura);
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/governance/admin_status.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn handler() -> anyhow::Result<()> {
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_status()?;
    let _joined = hosted_identity.is_joined();
    let _count = hosted_identity.hosted_agent_count();
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/governance/network_health.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn handler() -> anyhow::Result<()> {
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_status()?;
    let _host = hosted_identity.host_device_agent_ura();
    let _count = hosted_identity.hosted_agent_count();
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/governance/meta.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn describe_handler() -> usize {
    let _scope = serde_json::json!({
        "owner_ura": "easynet:///r/example/device/dev-a",
        "ability_ura": "easynet:///r/example/ability/device.dev-a.meta.list_abilities",
    });
    AgentAggregateRepository::load_hosted_identity_status()
        .map(|status| status.hosted_agent_count())
        .unwrap_or_default()
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/governance/invocation_history.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn ledger_resource_ura() -> anyhow::Result<Option<String>> {
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_status()?;
    ledger_resource_ura_from_host_device_agent_ura(hosted_identity.host_device_agent_ura())
}

fn ledger_governance_owner() -> &'static str {
    "easynet:///r/default/device/dev-1"
}

fn ledger_resource_ura_from_host_device_agent_ura(host_device_agent_ura: Option<&str>) -> anyhow::Result<Option<String>> {
    let Some(host_device_agent_ura) = host_device_agent_ura else {
        return Ok(None);
    };
    let parsed = crate::core::ura::parse_ura(host_device_agent_ura)?;
    Ok(Some(crate::core::ura::resource_dot_ura(&parsed.realm, "device.dev-1", "billing/invocations")))
}
EOF
	  cat >"$CLI/src/daemon/ability/builtins/agents/list.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateSnapshot;

pub fn register<F>(reg: &mut AxonAbilityCatalog, snapshot_provider: F)
where
    F: Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync> =
        Arc::new(snapshot_provider);
    reg.register_rpc_with_owner(
        ABILITY_LIST_AGENTS,
        OwnerKind::Device,
        Arc::new(move |_args: Value| list_agents_handler(&provider)),
    );
}

fn list_agents_handler(
    registry_provider: &Arc<dyn Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync>,
) -> anyhow::Result<Value> {
    let snapshot = registry_provider()?;
    Ok(json!({ "agents": agent_rows(&snapshot)? }))
}

fn agent_rows(snapshot: &AgentAggregateSnapshot) -> anyhow::Result<Vec<Value>> {
    snapshot
        .registered_agents()
        .map(|(registry_key, _entry)| {
            let agent_id = AgentId::parse(registry_key)?;
            let name = agent_id.name.as_str();
            let ura = snapshot.hosted_llm_agent_ura(name);
            Ok(json!({ "name": name, "ura": ura }))
        })
        .collect()
}
EOF
  cat >"$CLI/src/daemon/invocation/bidi/session_initiator/prelude.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentHostedAdvertiseEntry,
};

fn run_hosted_agent_advertise_prelude() -> anyhow::Result<()> {
    let realm = "acme".to_string();
    let user_segment = "user".to_string();
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_snapshot()?;
    let entries = hosted_identity.hosted_advertise_entries(&realm, &user_segment);
    let labels = entries
        .iter()
        .map(AgentHostedAdvertiseEntry::short_label)
        .collect::<Vec<_>>();
    for entry in &entries {
        advertise_hosted_agent_entry(entry);
    }
    Ok(())
}

fn advertise_hosted_agent_entry(entry: &AgentHostedAdvertiseEntry) {
    let plan = HostedAgentPreludePublicationPlan::prepare(entry.agent_ura())?;
    send_advertise_agent_prelude(entry.agent_ura(), plan.generation());
    let mut advertise_ctx = HostedAgentAbilityAdvertiseContext;
    advertise_hosted_agent_abilities(&mut advertise_ctx, entry, &plan);
}

struct HostedAgentPreludePublicationPlan;

impl HostedAgentPreludePublicationPlan {
    fn prepare(agent_ura: &str) -> anyhow::Result<Self> {
        owner_projection::prepare_and_persist(agent_ura)?;
        Ok(Self)
    }

    fn generation(&self) -> u64 {
        2
    }
}

struct HostedAgentAbilityAdvertiseContext;

fn advertise_hosted_agent_abilities(
    ctx: &mut HostedAgentAbilityAdvertiseContext,
    entry: &AgentHostedAdvertiseEntry,
    plan: &HostedAgentPreludePublicationPlan,
) {
    send_prepared_advertise_abilities_prelude(ctx, entry.agent_ura(), plan);
}

fn send_advertise_agent_prelude(agent_ura: &str, generation: u64) {
    let body = json!({
        "agent_ura": agent_ura,
        "generation": generation,
    });
}

fn send_prepared_advertise_abilities_prelude(
    ctx: &mut HostedAgentAbilityAdvertiseContext,
    agent_ura: &str,
    plan: &HostedAgentPreludePublicationPlan,
) {
}

fn resolved_public_keys(result: &[u8]) -> anyhow::Result<Vec<String>> {
    let response: serde_json::Value = serde_json::from_slice(result)
        .map_err(|err| anyhow::anyhow!("resolve_key_response_json_invalid: {err}"))?;
    let keys = response
        .get("public_keys_b64")
        .ok_or_else(|| anyhow::anyhow!("resolve_key_response_missing_public_keys_b64"))?
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("resolve_key_response_public_keys_b64_not_array"))?;
    let mut out = Vec::new();
    for (index, value) in keys.iter().enumerate() {
        let key = value.as_str().ok_or_else(|| {
            anyhow::anyhow!("resolve_key_response_public_keys_b64[{index}]_not_string")
        })?;
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("resolve_key_response_public_keys_b64[{index}]_empty");
        }
        out.push(key.to_string());
    }
    Ok(out)
}

fn paired_user_resolve_key_args(user_ura: &str, presented_pubkey_b64: Option<&str>) -> anyhow::Result<Vec<u8>> {
    Ok(Vec::new())
}

enum UserTrustBootstrapError {
    CredentialsUnavailable,
}

async fn sync_paired_user_trust_prelude() -> anyhow::Result<()> {
    let Some(creds) = crate::daemon::persistence::config::load_credentials_optional()
        .map_err(|_| UserTrustBootstrapError::CredentialsUnavailable)?
    else {
        return Ok(());
    };
    let user_ura = creds
        .user_ura()
        .map_err(|_| UserTrustBootstrapError::CredentialsUnavailable)?;
    for presented_pubkey_b64 in local_public_keys {
        let args = paired_user_resolve_key_args(&user_ura, presented_pubkey_b64)?;
        let response = invoke_prelude_unary(args).await?;
        let pubkeys = resolved_public_keys(&response.result).map_err(|err| anyhow::anyhow!("{err}"))?;
    }
    Ok(())
}
EOF
	  cat >"$CLI/src/daemon/ability/builtins/resources/skills/list.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentHostedSkillOwnerProjection,
};

fn handle(args: Value) -> anyhow::Result<Value> {
    let snapshot = AgentAggregateRepository::load_snapshot()?;
    let hosted_skill_owners = snapshot.hosted_skill_owner_projection();
    let scope = SkillListScope::from_args(&args, &hosted_skill_owners)?;
    let rows = SkillInventoryBuilder::new(snapshot.skill_inventory_projection(), &scope).collect()?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            scoped_skill_resource_ura(
                &hosted_skill_owners,
                scope.agent_ura_for_row(&row.agent_id),
                &row.agent_id,
                &row.name,
            )
        })
        .collect();
    Ok(json!({ "items": items }))
}

impl SkillListScope {
    fn from_args(args: &Value, hosted_skill_owners: &AgentHostedSkillOwnerProjection) -> anyhow::Result<Self> {
        let scoped_owner = owner_name_for_agent_ura(
            hosted_skill_owners,
            "easynet:///r/acme/agent/user.claude",
        )?;
        Ok(Self::new(scoped_owner))
    }
}

fn owner_name_for_agent_ura(
    hosted_skill_owners: &AgentHostedSkillOwnerProjection,
    agent_ura: &str,
) -> anyhow::Result<String> {
    hosted_skill_owners
        .owner_name_for_agent_ura(agent_ura)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing"))
}

fn scoped_skill_resource_ura(
    hosted_skill_owners: &AgentHostedSkillOwnerProjection,
    explicit_agent_ura: Option<&str>,
    agent_name: &str,
    skill_name: &str,
) -> Option<String> {
    let agent_ura = explicit_agent_ura.or_else(|| hosted_skill_owners.hosted_ura_for(agent_name))?;
    crate::daemon::federation::read_model::owner_projection::skill_resource_ura(agent_ura, skill_name)
}
EOF
	  cat >"$CLI/src/daemon/ability/builtins/resources/skills/publish.rs" <<'EOF'
use crate::daemon::persistence::{
    agent_aggregate::{AgentAggregateRepository, AgentSkillLayout},
};

fn resolve_owner_root_and_layout(owner_id: &str) -> anyhow::Result<(PathBuf, AgentSkillLayout)> {
    let owner = AgentAggregateRepository::load_registered_agent_workspace(owner_id, "skill.publish")?;
    let root = owner.root_path().to_path_buf();
    if !root.is_dir() {
        anyhow::bail!("owner agent {owner_id:?} has no on-disk workspace at {}", root.display());
    }
    Ok((root, owner.skill_layout()))
}

enum SkillPublishProvenance {
    DirectPublish,
}

impl SkillPublishProvenance {
    fn as_kind(&self) -> &'static str {
        match self {
            Self::DirectPublish => "direct_publish",
        }
    }
}

fn publish_handler(args: Value) -> anyhow::Result<Value> {
    let provenance = SkillPublishProvenance::DirectPublish;
    Ok(json!({ "provenance": provenance.as_kind() }))
}

#[test]
fn publish_without_run_id_records_direct_publish_provenance() {}
EOF
	  cat >"$CLI/src/daemon/ability/catalog/catalog_metadata.rs" <<'EOF'
fn registration_hints(owner_ura: &str, registry_name: &str, call_mode: DescriptorCallMode) -> AbilityHints {
    let public_name = crate::core::ura::descriptor_public_ability_name(owner_ura, registry_name);
    AbilityHints {
        destructive: public_name == agent_names::AGENT_PURGE,
        ..Default::default()
    }
}

fn description_for(name: &str) -> &'static str {
    match name {
        agent_names::AGENT_PURGE => agent_lifecycle_ability::purge_agent_description(),
        agent_names::AGENT_PURGE_RECONCILE => agent_lifecycle_ability::purge_reconcile_description(),
        _ => "generic",
    }
}

fn input_schema_for(name: &str) -> Value {
    match name {
        agent_names::AGENT_PURGE => agent_lifecycle_ability::purge_agent_input_schema(),
        agent_names::AGENT_PURGE_RECONCILE => agent_lifecycle_ability::purge_reconcile_input_schema(),
        _ => json!({}),
    }
}
EOF
  cat >"$CLI/ability-descriptors/system/agents/agent.purge.ability.toml" <<'EOF'
name = "agent.purge"
description = "Destructively remove an LLM sub-agent and the exact canonical root_path stored in its registry row. Requires Manage authority."
admission_action = "manage"
hints_json = "{\"read_only\":false,\"destructive\":true,\"idempotent\":false,\"streaming_only\":false,\"bidi_only\":false}"
EOF
  cat >"$CLI/ability-descriptors/system/agents/agent.stop.ability.toml" <<'EOF'
name = "agent.stop"
description = "Remove an LLM sub-agent registry row by name or Agent URA. Idempotent: ack=false when the row didn't exist. The registered root directory is always preserved."
admission_action = "manage"
hints_json = "{\"read_only\":false,\"destructive\":false,\"idempotent\":false,\"streaming_only\":false,\"bidi_only\":false}"
EOF
  cat >"$CLI/src/cli/commands/start.rs" <<'EOF'
fn bootstrap_local_agent_projection(creds: &Credentials) {
    let plan = build_bootstrap_plan(creds)?;
    lifecycle::bootstrap_local_agent_projection(&plan)
}

fn load_and_verify_credentials_with<F>(verify: F) -> anyhow::Result<(Credentials, bool)>
where
    F: Fn(&Credentials) -> CredentialCheck,
{
    let creds = config::load_credentials()?;
    if has_daemon_native_join_lineage(&creds) {
        return Ok((creds, true));
    }
    match verify(&creds) {
        CredentialCheck::Valid => Ok((creds, true)),
        CredentialCheck::NetworkUnavailable => anyhow::bail!("hub credential verification unavailable"),
        CredentialCheck::Revoked(msg) => anyhow::bail!(msg),
    }
}

fn has_daemon_native_join_lineage(creds: &Credentials) -> bool {
    creds.credential_token.trim().is_empty()
        && creds.join_receipt_hash.as_deref().is_some_and(|value| !value.trim().is_empty())
        && creds.hub_pubkey_b64.as_deref().is_some_and(|value| !value.trim().is_empty())
}
EOF
  cat >"$CLI/src/cli/commands/groups/principal.rs" <<'EOF'
enum PrincipalCommandActor<'a> {
    Supplied(&'a str),
    SubjectSelf(&'a str),
}

impl<'a> PrincipalCommandActor<'a> {
    fn supplied_or_subject_self(actor_ura: Option<&'a str>, principal_ura: &'a str) -> Self {
        actor_ura
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Self::Supplied)
            .unwrap_or(Self::SubjectSelf(principal_ura))
    }

    fn subject_self(principal_ura: &'a str) -> Self {
        Self::SubjectSelf(principal_ura)
    }

    fn actor_ura(self) -> &'a str {
        match self {
            Self::Supplied(actor_ura) | Self::SubjectSelf(actor_ura) => actor_ura.trim(),
        }
    }
}

fn principal_command(
    actor: PrincipalCommandActor<'_>,
    idempotency_key: &str,
    expected_version: Option<u64>,
    proof_kind: ProofKindArg,
    proof_ref: &str,
) -> Value {
    json!({
        "actor_ura": actor.actor_ura(),
        "idempotency_key": idempotency_key,
        "proof": {
            "kind": proof_kind.as_wire(),
            "reference": proof_ref.trim(),
        }
    })
}
EOF
  cat >"$CLI/src/daemon/execution/mission/orchestration.rs" <<'EOF'
struct MissionRunAggregate {
    meta: MissionRunMeta,
}

pub struct MissionRunOpts {
    pub run_timeout: Option<std::time::Duration>,
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

fn find_traditional_agent_target_conflict(ir: &MissionIr) -> anyhow::Result<Option<TraditionalAgentTargetConflict>> {
    let snapshot = AgentAggregateRepository::load_snapshot()?;
    let registered = snapshot.registered_agent_surface_names();
    Ok(None)
}

fn run_mission(
    gateway: Arc<dyn MissionInvocationGateway>,
    opts: MissionRunOpts,
) {
    execute_with_gateway_for_trace_with_timeout(gateway, opts.run_timeout);
}
EOF
  cat >"$CLI/src/daemon/execution/mission/invocation_gateway.rs" <<'EOF'
struct DaemonMissionInvocationGateway {
    parent: AbilityContext,
}

struct PersistedMissionChildTargetResolver;

impl MissionChildTargetResolver for PersistedMissionChildTargetResolver {
    fn callee_ura(&self, request: &MissionInvocationRequest) -> anyhow::Result<String> {
        let snapshot = AgentAggregateRepository::try_load_snapshot()?;
        let agent_name = request.hosted_agent.as_deref().unwrap();
        let agent_ura = snapshot
            .hosted_agent_ura_by_name(agent_name)
            .map_err(|error: HostedAgentNameLookupError| anyhow::anyhow!("{error}"))?;
        Ok(agent_ura.unwrap().to_string())
    }
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

fn route_with_target_context(target: &LocalAbilityTarget) -> anyhow::Result<()> {
    let _context = root_context_for_target(target)?;
    Ok(())
}

enum CostMetadataProjection {
    Undeclared,
}

impl CostMetadataProjection {
    fn value(&self) -> &'static str {
        match self {
            Self::Undeclared => "unknown",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Undeclared => "cost not declared",
        }
    }
}

#[test]
fn mcp_cost_projection_marks_agent_owned_undeclared_rows_unknown() {}

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
  cat >"$CLI/src/daemon/invocation/routing/hub_resolver.rs" <<'EOF'
pub enum HubResolution {
    Static { hub_endpoint: String },
    Offline,
}

pub struct HubResolver<'a> {
    static_peers: &'a SharedFederatedPeers,
}

impl<'a> HubResolver<'a> {
    pub fn new(static_peers: &'a SharedFederatedPeers) -> Self {
        Self { static_peers }
    }

    pub fn resolve(&self, target_realm: &str) -> HubResolution {
        let peers_snapshot = self.static_peers.snapshot();
        if let Some(ura) = peers_snapshot.get(target_realm) {
            return HubResolution::Static {
                hub_endpoint: ura.clone(),
            };
        }

        HubResolution::Offline
    }
}
EOF
  cat >"$CLI/src/daemon/invocation/routing/route_resolver.rs" <<'EOF'
struct LocalHostedAgentPlacements {
    state: HostedPlacementProjectionState,
}

enum HostedPlacementProjectionState {
    Available,
    Unavailable { reason: String },
}

impl LocalHostedAgentPlacements {
    fn load() -> Self {
        match AgentAggregateRepository::try_load_snapshot() {
            Ok(snapshot) => Self::from_projection(snapshot.hosted_agent_placements()),
            Err(error) => Self {
                state: HostedPlacementProjectionState::Unavailable {
                    reason: format!("{error:#}"),
                },
            },
        }
    }

    fn from_projection(projection: AgentHostedPlacementProjection) -> Self {
        Self {
            state: HostedPlacementProjectionState::Available,
        }
    }
}

fn route_selector_from_query(
    query_name: &str,
    ability_name: &str,
) -> Result<Option<RouteSelector>, ResolveRouteFailure> {
    if query_name.contains("@") {
        let _selector = ability_selector_from_descriptor_ref(query_name)?;
    }
    if ability_name.contains("@") {
        return route_selector_from_descriptor_ref(query_name, ability_name).map(Some);
    }
    Ok(None)
}

fn route_selector_from_descriptor_ref(
    owner_ura: &str,
    descriptor_ref: &str,
) -> Result<RouteSelector, ResolveRouteFailure> {
    let selector = ability_selector_from_descriptor_ref(descriptor_ref)?;
    if selector.owner_ura() != owner_ura {
        return Err(ResolveRouteFailure);
    }
    Ok(RouteSelector)
}

fn ability_selector_from_descriptor_ref(
    descriptor_ref: &str,
) -> Result<crate::core::ura::AbilitySelector, ResolveRouteFailure> {
    let descriptor_ref =
        axon_sdk::invocation::canonical_ability_descriptor_ref(descriptor_ref)
            .map_err(|_| ResolveRouteFailure)?;
    let ability_ura =
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
            &descriptor_ref,
        )
        .map_err(|_| ResolveRouteFailure)?;
    crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|_| ResolveRouteFailure)
}

fn resolve_delegation(peer_source: PeerSource, parsed_owner: ParsedOwner, selector: Selector) {
    let resolution = HubResolver::new(peer_source.federated_peers).resolve(&parsed_owner.realm);
    let endpoint = match resolution {
        HubResolution::Static { hub_endpoint } => {
            DelegatedPeerEndpoint::new(hub_endpoint, "federated_peers", None)
        }
        HubResolution::Offline => return,
    };
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/resources/voice.rs" <<'EOF'
use axon_sdk::{
    VoiceCallState,
    VoiceEndReason,
    VoiceEventType,
    VoiceNetworkMetrics,
};

fn register(reg: &mut Catalog) {
    reg.register_rpc_with_owner("voice.list_calls", OwnerKind::RealmAuthority, handler);
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

fn check_handler(args: Value) -> anyhow::Result<Value> {
    let raw_text = r#""owner_ura", "owner_source", "caller_ura""#;
    let owner_source = require_owner_source(args.get("owner_source"))?;
    Ok(json!({ "owner_source": owner_source, "schema": raw_text }))
}

fn require_owner_source(value: Option<&Value>) -> anyhow::Result<&str> {
    value.and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("owner_source required"))
}

#[test]
fn authority_binding_check_requires_explicit_owner_source() {}
EOF
  cat >"$CLI/sdk/go/access_control.go" <<'EOF'
func accessControlRevokeArgs(request AccessControlRevokeRequest) (AccessControlRevokeRequest, map[string]any, error) {
    ownerURA := strings.TrimSpace(request.OwnerURA)
    if ownerURA == "" {
        return AccessControlRevokeRequest{}, nil, invalidAccessControl("owner_ura is required", nil)
    }
    grantID := strings.TrimSpace(request.GrantID)
    if grantID == "" {
        return AccessControlRevokeRequest{}, nil, invalidAccessControl("grant_id is required", nil)
    }
    actorURA := strings.TrimSpace(request.ActorURA)
    if actorURA == "" {
        return AccessControlRevokeRequest{}, nil, invalidAccessControl("actor_ura is required", nil)
    }
    if _, err := ParseURAParts(actorURA); err != nil {
        return AccessControlRevokeRequest{}, nil, invalidAccessControl("actor_ura must be canonical", err)
    }
    request.OwnerURA = ownerURA
    request.ActorURA = actorURA
    args := map[string]any{"owner_ura": ownerURA, "grant_id": grantID, "actor_ura": actorURA}
    return request, args, nil
}

func accessControlCheckArgs(request AccessControlCheckRequest) (map[string]any, error) {
    ownerSource := strings.TrimSpace(request.OwnerSource)
    if ownerSource == "" {
        return nil, invalidAccessControl("owner_source is required", nil)
    }
    return map[string]any{
        "owner_source":                  ownerSource,
    }, nil
}
EOF
  cat >"$CLI/sdk/python/easynet_sdk/access_control.py" <<'EOF'
def _revoke_args(request: AccessControlRevokeRequest) -> tuple[AccessControlRevokeRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    grant_id = _required_text(request.grant_id, "grant_id")
    actor_ura = _required_text(request.actor_ura, "actor_ura")
    parse_ura(actor_ura)
    args: dict[str, object] = {
        "owner_ura": owner_ura,
        "grant_id": grant_id,
        "actor_ura": actor_ura,
    }
    return request, args

def _check_args(request: AccessControlCheckRequest) -> dict[str, object]:
    owner_source = _required_text(request.owner_source, "owner_source")
    return {
        "owner_source": owner_source,
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
    let hosts_realm_authority = hosts_hub_authority;
    if hosts_realm_authority {
        if let Some(provider) = voice_provider_assembly.as_ref() {
            voice_call_ability::register(&mut reg, provider.clone());
        }
    }
	    agent_list_ability::register(&mut reg, || {
	        crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
	    });
	    discover_ability::register_device_aggregate_with_resolver(
	        &mut reg,
	        || {
	            crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
	                .map(|snapshot| snapshot.registered_agent_registry_projection())
	        },
	        Arc::clone(&local_registry_handle),
	        Arc::clone(&discover_federation_resolver),
	    );
	    a2a_bridge_ability::register(
	        &mut reg,
	        || {
	            crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
	                .map(|snapshot| snapshot.registered_agent_registry_projection())
	        },
	        Arc::clone(&local_registry_handle),
	    );
	    let evidence = VoiceAssemblyEvidence {
	        repository_assembled: voice_provider_assembly.is_some(),
	        executable_delivery_evidence: false,
    };
}

fn declare_daemon_native_agent_authorities(
    mut authority_context: AbilityAuthorityContext,
    identity: &PagesIdentity,
) -> anyhow::Result<AbilityAuthorityContext> {
    let Some(user) = identity.user.as_deref() else {
        return Ok(authority_context);
    };
    let realm = identity.realm.as_deref().unwrap_or(crate::core::ura::REALM_EASYNET);
    let declared_roots = [
        ("Pages", pages::management_agent_ura(realm, user)),
        ("Files", files::management_agent_ura(realm, user)),
    ];
    for (_executor, authority_root) in declared_roots {
        authority_context = authority_context.with_declared_agent_authority_root(authority_root)?;
    }
    Ok(authority_context)
}
EOF
  cat >"$CLI/sdk/python/easynet_sdk/_receipt_projection.py" <<'EOF'
def reject_retired_top_level_receipt_alias(raw: object, stage: str) -> None:
    marker = "stage=stage"
    raise ValueError(f"retired receipt alias is not accepted stage={stage}")
EOF
  cat >"$CLI/src/daemon/ability/builtins/device_control/files.rs" <<'EOF'
fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner("fs.read", OwnerKind::Device, handler_read);
    reg.register_rpc_with_owner("fs.write", OwnerKind::Device, handler_write);
    reg.register_rpc_with_owner("fs.stat", OwnerKind::Device, handler_stat);
    reg.register_rpc_with_owner("fs.list", OwnerKind::Device, handler_list);
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/resources/files_store/mod.rs" <<'EOF'
pub(crate) fn management_agent_ura(realm: &str, user: &str) -> String {
    crate::core::ura::agent_ura(realm, user, "files")
}

fn register_files_rpc(
    reg: &mut AxonAbilityCatalog,
    ability: &'static str,
    owner: OwnerKind,
    authority_scope: AuthorityScope,
    manifest: AbilityManifest,
    handler: LocalRpcHandler,
) {
    reg.register_rpc_with_spec_impl_and_authority_scope(
        ability,
        owner,
        authority_scope,
        manifest,
        handler,
        ControlPlaneImplementation::native_daemon(),
    );
}

pub fn register(reg: &mut AxonAbilityCatalog, config: FilesConfig) {
    let owner = OwnerKind::User(config.user.clone());
    register_files_rpc(reg, "files.put", owner.clone(), scope(), manifest(), put_handler);
    register_files_rpc(reg, "files.get", owner.clone(), scope(), manifest(), get_handler);
    register_files_rpc(reg, "files.list", owner, scope(), manifest(), list_handler);
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/integrations/openai_compat.rs" <<'EOF'
fn handle_file_upload_with_context(
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    identity: Option<&OpenAICompatIdentity>,
    args: Value,
) -> anyhow::Result<Value> {
    let files_authority_root =
        crate::daemon::ability::builtins::resources::files_store::management_agent_ura(
            &realm, &user,
        );
    invoke_user_owned_rpc(
        registry.as_ref(),
        &files_authority_root,
        "files.put",
        files_subject,
        store_args,
    )
}

fn extract_chat_reply_text(value: Value) -> anyhow::Result<String> {
    anyhow::bail!("chat-base ability response must be a string or object with string reply, message, or content")
}

fn handle_file_retrieve_with_context(
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    identity: Option<&OpenAICompatIdentity>,
    args: Value,
) -> anyhow::Result<Value> {
    let files_authority_root =
        crate::daemon::ability::builtins::resources::files_store::management_agent_ura(
            &realm, &user,
        );
    invoke_user_owned_rpc(
        registry.as_ref(),
        &files_authority_root,
        "files.get",
        file_subject,
        json!({ "sha256": file_id }),
    )
}

fn deref_to_data_url(
    ura: &str,
    dispatch_handle: &Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
) -> anyhow::Result<String> {
    let (authority_root, ability, args) = (
        crate::daemon::ability::builtins::resources::files_store::management_agent_ura(
            &parsed.realm,
            user,
        ),
        "files.get".to_string(),
        json!({ "ura": ura, "path": path }),
    );
    invoke_user_owned_rpc(registry.as_ref(), &authority_root, ability, subject, args)
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
    let attempt_ledger = InvocationAttemptLedger::open(path)
        .context("refusing to boot without invocation attempt audit ledger")?;
    service = service.with_invocation_attempt_ledger(attempt_ledger);
    if capabilities.owns_upstream_session() {
        register_purge_recovery_on_outbox_ready(outbox, registrar);
    }
    let daemon_route_owner = daemon_ura.as_deref().unwrap();
    service.register_daemon_unary_routes(daemon_route_owner)?;
    service.register_daemon_stream_routes(daemon_route_owner)?;
    if capabilities.hub_runtime {
        service.register_daemon_bidi_routes(daemon_route_owner)?;
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

    async fn register_streams(&self, registrations: Vec<AbilityRegistration>) {
        let _ = stream_env_ability_with_options(handler);
        self.runtime.register_many(registrations).await;
    }

    async fn register_bidis(&self, registrations: Vec<AbilityRegistration>) {
        let options = AbilityOptions::bidi();
        self.runtime.register_many(registrations).await;
    }

    async fn dispatch(&self, route: DaemonUnaryRoute, request: &InvokeRequest, ingress: DaemonRouteIngress) {
        dispatch_rpc_admitted(&self.runtime, route, request, ingress).await;
    }

    async fn open_stream(&self, route: DaemonStreamRoute, request: &InvokeServerStreamRequest) {
        open_stream_admitted(&self.runtime, route, request).await;
    }

    async fn open_bidi(&self, route: DaemonBidiRoute, request: &EnvelopeOpen) {
        open_bidi_external_signed(&self.runtime, route, request).await;
        project_registered_finalized_bidi_receipt(lifecycle).await;
    }
}
EOF
  cat >"$CLI/src/daemon/invocation/streams/stream_dispatcher.rs" <<'EOF'
impl StreamDispatcher {
    pub(crate) async fn dispatch_daemon_route_runtime(
        &self,
        route: DaemonStreamRoute,
        request: &InvokeServerStreamRequest,
    ) {
        let local_system_ingress = true;
        DaemonRouteRuntimeAdapter::new(runtime, cancellations)
            .open_stream(route, request, local_system_ingress)
            .await;
    }
}

pub(crate) struct DaemonStreamRouteProvider;

fn callee_ura_from_envelope(envelope: &Envelope) -> anyhow::Result<String> {
    crate::daemon::invocation::dispatch::invocation_wire::callee_ura_from_envelope(envelope)
}

fn subscribe_directory_v2() {
    invalid_presence_event();
    invalid_presence_snapshot();
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

    pub(crate) fn authorize_identity_write(&self, caller_envelope: Option<&Envelope>, intent: &RegisterPubkeyIntent) {
        let gate = IdentityWriteGate::new(
            self.admission.trust_anchor_snapshot(),
            self.admission.daemon_ura().map(str::to_string),
            self.admission.transport_boundary(),
            self.identity.daemon_realm.clone(),
        );
        gate.authorize_register_pubkey(caller_envelope, intent)?;
    }
}

fn callee_ura_from_envelope(envelope: &Envelope) -> anyhow::Result<String> {
    crate::daemon::invocation::dispatch::invocation_wire::callee_ura_from_envelope(envelope)
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'EOF'
fn missing_invocation_attempt_ledger() -> Status {
    Status::internal("invocation attempt audit ledger is not wired")
}

fn invocation_attempt_audit_status(error: Error) -> Status {
    Status::internal(format!("invocation attempt audit unavailable: {error}"))
}

struct RuntimeAdmissionPlane {
    facade: AdmissionFacade,
}

impl RuntimeAdmissionPlane {
    fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.facade = self.facade.with_transport_boundary(boundary);
        self
    }
}

struct DaemonInvocationService {
    admission_plane: RuntimeAdmissionPlane,
}

impl DaemonInvocationService {
    pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.admission_plane = self.admission_plane.with_transport_boundary(boundary);
        self
    }

    pub(crate) async fn register_daemon_unary_routes(&self, owner_ura: &str) {
        DaemonRouteRuntimeAdapter::new(runtime, cancellations)
            .register(owner_ura, catalog.as_ref(), provider)
            .await;
    }

    pub(crate) async fn register_daemon_stream_routes(&self, owner_ura: &str) {
        DaemonRouteRuntimeAdapter::new(runtime, cancellations)
            .register_streams(owner_ura, catalog.as_ref(), provider)
            .await;
    }

    pub(crate) async fn register_daemon_bidi_routes(&self, owner_ura: &str) {
        DaemonRouteRuntimeAdapter::new(runtime, cancellations)
            .register_bidis(owner_ura, catalog.as_ref(), provider)
            .await;
    }

    async fn dispatch_daemon_unary_route(&self, route: DaemonUnaryRoute, request: &InvokeRequest, ingress: DaemonRouteIngress) {
        self.unary_dispatcher()
            .dispatch_daemon_route_runtime(route, request, ingress)
            .await;
    }

    async fn invoke_stream(&self, inner: InvokeServerStreamRequest) {
        let streams = self.stream_dispatcher();
        match DaemonStreamRoute::from_function(&inner.function_name) {
            Some(route) => streams.dispatch_daemon_route_runtime(route, &inner).await,
            None => streams.dispatch_selected_route(&inner).await,
        }
    }

    async fn invoke_bidi(&self, ability_name: &str, envelope_open: &EnvelopeOpen, up: Streaming<InvokeBidiUp>) {
        let dispatcher = self.bidi_dispatcher();
        match DaemonBidiRoute::from_function(ability_name) {
            Some(route) => dispatcher
                .dispatch_daemon_route_runtime(route, envelope_open, up)
                .await,
            None => dispatcher.dispatch(ability_name, envelope_open, up).await,
        }
    }
}

pub(crate) enum DaemonBidiRoute {
    SessionOpen,
}

impl DaemonBidiRoute {
    pub(crate) fn from_function(function: &str) -> Option<Self> {
        Some(Self::SessionOpen)
    }
}

pub(crate) const DAEMON_INVOCATION_BIDI_ROUTES: &[DaemonBidiRoute] = &[DaemonBidiRoute::SessionOpen];
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/attempt_audit.rs" <<'EOF'
struct InvocationAttemptLedger;
struct InvocationAttemptHandle;
struct InvocationAttemptRecord;

impl InvocationAttemptLedger {
    pub(crate) fn begin(&self) -> anyhow::Result<InvocationAttemptHandle> {
        self.append(&InvocationAttemptRecord)?;
        Ok(InvocationAttemptHandle)
    }

    fn append(&self, record: &InvocationAttemptRecord) -> anyhow::Result<()> {
        let _guard = self.writer.lock().map_err(|_| Error)?;
        let line = serde_json::to_string(record)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    pub(crate) fn list_recent(&self) -> anyhow::Result<Vec<InvocationAttemptRecord>> {
        let record = serde_json::from_str::<InvocationAttemptRecord>(&line)
            .map_err(|err| anyhow::anyhow!("decode invocation attempt ledger row 1: {err}"))?;
        Ok(vec![record])
    }
}
EOF
  mkdir -p "$CLI/src/daemon/invocation/bidi"
  cat >"$CLI/src/daemon/invocation/bidi/bidi_dispatcher.rs" <<'EOF'
use crate::daemon::invocation::dispatch::daemon_invocation_service::{
    DaemonBidiRoute, DAEMON_INVOCATION_BIDI_ROUTES,
};

pub(crate) const RUNTIME_ADMIN_BIDI_ROUTES: &[DaemonBidiRoute] = DAEMON_INVOCATION_BIDI_ROUTES;

pub(crate) struct DaemonBidiRouteProvider {
    session_open: SessionOpenProvider,
}

impl DaemonBidiRouteProvider {
    async fn invoke(&self, route: DaemonBidiRoute, context: Arc<AbilityContext>) {
        match route {
            DaemonBidiRoute::SessionOpen => self.session_open.invoke(context).await,
        }
    }
}

struct SessionOpenPolicy;

struct SessionOpenProvider {
    policy: SessionOpenPolicy,
}

impl SessionOpenProvider {
    async fn invoke(&self, context: Arc<AbilityContext>) {}
}

impl BidiDispatcher {
    async fn dispatch(&self, ability_name: &str) {
        if let Some(route) = DaemonBidiRoute::from_function(ability_name) {
            panic!("exact route must use adapter: {:?}", route);
        }
        self.dispatch_local_bidi_selected_route().await;
    }

    async fn dispatch_daemon_route_runtime(&self, route: DaemonBidiRoute, envelope_open: &EnvelopeOpen, up: Streaming<InvokeBidiUp>) {
        DaemonRouteRuntimeAdapter::new(runtime, cancellations)
            .open_bidi(route, envelope_open, up)
            .await;
    }
}

fn classify_carrier_v1_result(result: DispatchResult) {
    if result.terminal {
        if result.terminal_receipt.is_none() {
            return protocol_failure("CANONICAL_TERMINAL_RECEIPT_REQUIRED");
        }
        return CarrierDispatchEvent::Terminal(result);
    }
    CarrierDispatchEvent::Chunk(result.payload)
}

fn callee_ura_from_envelope(envelope: &Envelope) -> anyhow::Result<String> {
    crate::daemon::invocation::dispatch::invocation_wire::callee_ura_from_envelope(envelope)
}
EOF
  mkdir -p "$CLI/src/daemon/ability/catalog"
  cat >"$CLI/src/daemon/ability/catalog/runtime_admin_contracts.rs" <<'EOF'
fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_control_plane_descriptor_with_owner(
        "session.open",
        &OwnerKind::RealmAuthority,
    );
}
EOF
  mkdir -p "$CLI/src/daemon/invocation/bidi/session_initiator"
  cat >"$CLI/src/daemon/invocation/bidi/session_initiator/envelope.rs" <<'EOF'
fn build_session_envelope_open(caller_ura: &str) {
    let caller = parse_ura(caller_ura).unwrap();
    let hub_ura = crate::core::ura::hub_ura(&caller.realm);
    let descriptor = catalog_descriptor_ref_for_wire(&hub_ura, "session.open", CallMode::Bidi);
    ProtoEnvelope::from_target(caller_ura, &hub_ura, caller_ura, FreshRoot)
        .signed_descriptor_ref_invoke_request_with_signer("session.open", descriptor, vec![], signer);
}
EOF
  mkdir -p "$CLI/src/daemon/axon_bridge"
  cat >"$CLI/src/daemon/axon_bridge/local_runtime_request.rs" <<'EOF'
pub(crate) enum LocalRuntimeIngress {
    ExternalSigned,
}

pub(crate) struct LocalRuntimeRequestFactory;

impl LocalRuntimeRequestFactory {
    fn request_for_local_system() {
        sign_system_canonical(&descriptor_bound_canonical_bytes(&envelope));
    }
}

pub(crate) struct SystemInvocationIssuer;

impl SystemInvocationIssuer {
    pub(crate) fn request_for_descriptor_ref() {
        LocalRuntimeRequestFactory::request_for_local_system();
    }

    pub(crate) fn request_for_complete_envelope() {
        LocalRuntimeRequestFactory::request_for_local_system();
    }
}
EOF
  cat >"$CLI/src/daemon/axon_bridge/wire_descriptor.rs" <<'EOF'
fn descriptor_bound_from_wire_parts(envelope: Envelope) -> Result<(), AxonError> {
    let descriptor_ref = require_descriptor_ref_for_wire(&callee_ura, &ability)?;
    let envelope = wire::try_descriptor_bound_envelope_from_wire_parts(
        envelope,
        descriptor_ref,
        payload,
    )?;
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/axon_bridge/descriptor_bound_dispatch.rs" <<'EOF'
struct LocalSystemAuthority;

struct WireDispatch {
    local_system_authority: Option<LocalSystemAuthority>,
}

pub(crate) fn local_system_from_wire_parts() {
    let local_system_authority = true.then_some(LocalSystemAuthority);
    SystemInvocationIssuer::request_for_complete_envelope();
}

fn request_for_wire_dispatch(local_system_authority: Option<LocalSystemAuthority>) {
    local_system_authority.ok_or_else(|| "trusted-local authority required");
}

fn open_stream_local_explicit_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}

fn open_bidi_local_explicit_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}

pub async fn open_bidi_external_signed() {
    invoke_descriptor_bound_bidi_request_async(request).await;
}

fn dispatch_rpc_local_explicit_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/local_runtime_invoker.rs" <<'EOF'
async fn local_system_request() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}

pub async fn rpc_value_from_handle(handle: InvocationHandle) {
    handle.finalized().await;
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

struct RegisteredInvocationLifecycle;

impl RegisteredInvocationLifecycle {
    pub(crate) async fn finalized(&self) -> Result<FinalizedInvocation> {
        todo!()
    }

    pub(crate) async fn cancel_and_finalize(
        &self,
        reason: impl Into<String>,
    ) -> Result<FinalizedInvocation> {
        todo!()
    }
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

fn hot_agent_authority_snapshot_error(
    agent: &str,
    error: AgentAggregateSnapshotLoadError,
) -> HotAgentAuthorityInventoryError {
    HotAgentAuthorityInventoryError::CounterOverflow
}

fn routeable_mode_registered(ability: &AbilityName, call_mode: DescriptorCallMode) -> bool {
    control_plane_record_for_mode(ability, call_mode).is_some() && self.has_mode(ability, call_mode)
}

fn has_rpc(ability: &AbilityName) -> bool {
    routeable_mode_registered(ability, DescriptorCallMode::Rpc)
}

fn has_stream(ability: &AbilityName) -> bool {
    routeable_mode_registered(ability, DescriptorCallMode::Stream)
}

fn has_bidi(ability: &AbilityName) -> bool {
    routeable_mode_registered(ability, DescriptorCallMode::Bidi)
}

fn unique_handler_slot(key: &RuntimeAbilityKey, slot: HandlerSlot) -> anyhow::Result<HandlerSlot> {
    Ok(slot)
}

fn unique_mode_registered(key: &RuntimeAbilityKey, slot: HandlerSlot) -> anyhow::Result<bool> {
    Ok(control_plane_record_for_authority_mode(
        key.authority_root(),
        slot.call_mode(),
    ).is_some())
}

fn runtime_handlers_for_key(key: &RuntimeAbilityKey) -> Vec<HandlerSlot> {
    vec![unique_handler_slot(key, HandlerSlot::rpc()).unwrap()]
}

fn verify_execution_key_control_plane_modes(key: &RuntimeAbilityKey, slots: &[HandlerSlot]) -> anyhow::Result<()> {
    for slot in slots {
        let _record = control_plane_record_for_authority_mode(
            key.authority_root(),
            slot.call_mode(),
        ).ok_or_else(|| anyhow::anyhow!("missing authority/mode record"))?;
    }
    Ok(())
}

fn list_rpc_names(control_plane: &ControlPlaneRegistry) -> Vec<String> {
    control_plane
        .records()
        .filter(|record| record.call_mode == DescriptorCallMode::Rpc)
        .map(|record| record.name.clone())
        .collect()
}

fn static_runtime_key_validates_exact_authority_mode_record() {}
fn static_runtime_key_rejects_unrelated_authority_record_as_rescue_path() {}
fn dynamic_runtime_key_validates_exact_authority_mode_record() {}
fn ability_name_handler_projection_rejects_multi_authority_same_slot() {}
fn ability_name_handler_projection_does_not_synthesize_cross_authority_runtime_set() {}

// dynamic execution row remains present after adding a second mode

struct PersistedHotAgentAuthority;

impl PersistedHotAgentAuthority {
    fn load(agent: &str) -> Result<(), HotAgentAuthorityInventoryError> {
        let registry_key = crate::core::agent::id::AgentId::parse(agent)
            .map_err(|error| HotAgentAuthorityInventoryError::CounterOverflow)?
            .to_string();
        let snapshot = AgentAggregateRepository::try_load_snapshot()
            .map_err(|error| hot_agent_authority_snapshot_error(agent, error))?;
        if !snapshot.has_registered_agent(&registry_key) {
            return Err(HotAgentAuthorityInventoryError::CounterOverflow);
        }
        let identity = match snapshot.hosted_llm_agent_identity(agent) {
            HostedLlmAgentIdentity::Present(identity) => identity,
            HostedLlmAgentIdentity::Missing => return Err(HotAgentAuthorityInventoryError::CounterOverflow),
            HostedLlmAgentIdentity::Ambiguous => return Err(HotAgentAuthorityInventoryError::CounterOverflow),
        };
        let _host = snapshot.host_device_agent_ura();
        Ok(())
    }
}

struct HotAgentAuthorityInventory;

impl HotAgentAuthorityInventory {
    fn revoke_after_durable_removal(
        &self,
        enrollment: &HotAgentAuthorityEnrollment,
    ) -> Result<(), HotAgentAuthorityInventoryError> {
        let snapshot = AgentAggregateRepository::try_load_snapshot()
            .map_err(|error| hot_agent_authority_snapshot_error(&enrollment.agent, error))?;
        if snapshot.has_registered_agent(&enrollment.agent) {
            return Err(HotAgentAuthorityInventoryError::CounterOverflow);
        }
        if snapshot.has_hosted_llm_agent_identity(&enrollment.agent) {
            return Err(HotAgentAuthorityInventoryError::CounterOverflow);
        }
        Ok(())
    }
}

enum DescriptorCallMode {
    Rpc,
    Stream,
    Bidi,
}

struct AxonAbilityCatalog {
    control_plane: ControlPlane,
    execution_index: ExecutionIndex,
}

impl AxonAbilityCatalog {
    fn control_plane_record_for_mode(&self, ability: &str, call_mode: DescriptorCallMode) -> Result<Option<Record>, ()> {
        Ok(Some(Record))
    }

    fn routeable_mode_registered(&self, ability: &str, call_mode: DescriptorCallMode) -> bool {
        let has_control_plane_record = self
            .control_plane_record_for_mode(ability, call_mode)
            .ok()
            .flatten()
            .is_some();
        has_control_plane_record && self.execution_index.has_mode(ability, call_mode)
    }

    pub fn has_rpc(&self, ability: &str) -> bool {
        self.routeable_mode_registered(ability, DescriptorCallMode::Rpc)
    }

    pub fn has_stream(&self, ability: &str) -> bool {
        self.routeable_mode_registered(ability, DescriptorCallMode::Stream)
    }

    pub fn has_bidi(&self, ability: &str) -> bool {
        self.routeable_mode_registered(ability, DescriptorCallMode::Bidi)
    }

    pub fn list_rpc_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for record in self.control_plane.read().records() {
            if record.descriptor().call_mode() == DescriptorCallMode::Rpc {
                names.push(record.ability().to_string());
            }
        }
        names
    }
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
struct InvocationOutcome;

impl InvocationOutcome {
    /// Read the canonical terminal-result projection.
    fn result(&self) {}

    /// Consume the outcome and return its canonical terminal-result projection.
    fn into_result(self) {}
}

impl RuntimeClient {
    pub async fn request_cancel_signed(
        &self,
        signed: SignedInvocation,
        reason: String,
    ) -> Result<InvocationHandle> {
        let authority = self.cancellation_authority.as_ref().ok_or_else(|| {
            DaemonError::InvalidInvocation(
                "invocation.cancel requires an explicit authority".to_string(),
            )
        })?;
        let prepared = signed.prepare_cancel_command(reason)?;
        let signed_cancel = authority.sign(prepared).await?;
        let response = self.inner.invoke(signed_cancel).await?;
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

    pub(crate) fn accepts_local_self_caller(
        self,
        daemon_ura: Option<&str>,
        caller_ura: &str,
    ) -> bool {
        if !self.admits_local_self() {
            return false;
        }
        daemon_ura.is_some_and(|daemon_ura| daemon_ura == caller_ura)
    }
}

impl AdmissionFacade {
    pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.transport_boundary = boundary;
        self
    }

    pub(crate) fn transport_boundary(&self) -> AdmissionTransportBoundary {
        self.transport_boundary
    }

    fn accepts_local_self_caller(&self, caller_ura: &str) -> bool {
        self.transport_boundary
            .accepts_local_self_caller(self.daemon_ura(), caller_ura)
    }
}

#[test]
fn off_box_facade_does_not_accept_daemon_ura_spoof_as_local_self() {}

#[test]
fn off_box_facade_does_not_accept_local_system_self_admission() {}

EOF
  mkdir -p "$CLI/src/daemon/invocation/dispatch/daemon_invocation_service_tests"
  cat >"$CLI/src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs" <<'EOF'
#[tokio::test]
async fn signed_invocation_cancel_command_replay_is_rejected() {
    let replay = invoke_cancel_twice_through_local_runtime().await;
    assert_eq!(replay.reason, "NONCE_REPLAY");
}
EOF
  cat >"$CLI/src/daemon/invocation/admission/identity_write_gate.rs" <<'EOF'
use crate::daemon::invocation::admission::admission_facade::AdmissionTransportBoundary;

pub(crate) struct IdentityWriteGate {
    daemon_ura: Option<String>,
    transport_boundary: AdmissionTransportBoundary,
}

impl IdentityWriteGate {
    fn is_local_self(&self, caller_ura: &str) -> bool {
        self.transport_boundary
            .accepts_local_self_caller(self.daemon_ura.as_deref(), caller_ura)
    }
}

struct AuthorizedIdentityWriteCaller {
    local_self: bool,
}

#[test]
fn local_self_can_bootstrap_backend_row_without_anchor_entry() {}

#[test]
fn off_box_boundary_rejects_daemon_ura_spoof_without_anchor_entry() {}
EOF
  cat >"$CLI/sdk/python/easynet_sdk/runtime.py" <<'EOF'
class InvocationHandle:
    def __init__(self, subject_ura: str):
        self.subject_ura = subject_ura

def decode_invocation_result(decoded):
    _reject_retired_top_level_receipt_alias(decoded, "invocation result")

class RuntimeClient:
    def resolve_descriptor_ref(self, call_mode: str):
        call_mode = call_mode.strip()
        if not call_mode:
            raise _invalid_runtime_client("descriptor_ref call_mode is required")
        return self._transport.resolve_descriptor_ref({"call_mode": call_mode})

# Product words in comments and private symbols do not create public models.
class _MissionFixture:
    pass
EOF
  cat >"$CLI/docs/spec/ffi-abi-v7.md" <<'EOF'
# EasyNet Generic C ABI v7

## Ownership state machines

- stream cancel and bidi cancel are cancel-request operations at this provider
  boundary. Each registered resource submits at most one independently signed
  canonical invocation.cancel command, memoizes acceptance or rejection, and
  keeps the callback/reader path draining. Duplicate requests never submit a
  second command and must not claim lifecycle terminality without a canonical
  terminal receipt.
- stream close and bidi close are local resource release operations. Bidi
  close-send is a non-terminal local half-close.
EOF
  cat >"$CLI/sdk/go/cabi_runtime.go" <<'EOF'
var _ = ErrDescriptorNotFound

func (s *cabiStreamTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
    return []byte(fmt.Sprintf(`{"stream_id":%q,"cancel_requested":true,"cancelled":false,"state":"CancelRequested","terminal":false}`, streamID)), nil
}

func (b *cabiBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
    return []byte(fmt.Sprintf(`{"session_id":%q,"state":"CancelRequested","terminal":false,"reason":"cancelled"}`, bidiID)), nil
}

func resolveDescriptorRefFromDiagnostics(request map[string]string) error {
    if request["call_mode"] == "" {
        return invalidRuntimePayload("call_mode is required for descriptor_ref resolution", nil)
    }
    return nil
}
EOF
  cat >"$CLI/sdk/python/easynet_sdk/_cabi.py" <<'EOF'
_EXPECTED_DESCRIPTOR_CODE = ErrorCode.DESCRIPTOR_NOT_FOUND

def _resolve_descriptor_ref_from_diagnostics(request):
    call_mode = _required_string(request, "call_mode")
    return call_mode


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
  cat >"$CLI/sdk/go/stream.go" <<'EOF'
func NewStreamEventFromJSON(raw []byte) (StreamEvent, error) {
    if err := rejectRetiredTopLevelReceiptAlias(raw, "stream event"); err != nil {
        return StreamEvent{}, err
    }
    return StreamEvent{}, nil
}

func (s *StreamHandle) Cancel(ctx context.Context, reason string) (StreamCancel, error) {
    cancel := decodeCancel()
    if cancel.state != StreamCancelRequested || cancel.terminal || cancel.cancelled {
        s.state = StreamFailed
        return StreamCancel{}, invalidRuntimePayload("stream cancel transport must return CancelRequested with terminal=false", nil)
    }
    return cancel, nil
}
EOF
  cat >"$CLI/sdk/go/bidi.go" <<'EOF'
func NewBidiFrameFromJSON(raw []byte) (BidiFrame, error) {
    if err := rejectRetiredTopLevelReceiptAlias(raw, "bidi frame"); err != nil {
        return BidiFrame{}, err
    }
    return BidiFrame{}, nil
}

func (s *BidiSession) Cancel(ctx context.Context, reason string) (BidiOutcome, error) {
    outcome := decodeOutcome()
    if outcome.state != BidiCancelRequested || outcome.terminal {
        s.state = BidiFailed
        return BidiOutcome{}, invalidRuntimePayload("bidi cancel transport must return CancelRequested with terminal=false", nil)
    }
    return outcome, nil
}
EOF
  cat >"$CLI/sdk/python/easynet_sdk/stream.py" <<'EOF'
def decode_stream_event(decoded):
    _reject_retired_top_level_receipt_alias(decoded, "stream event")

def cancel(self, reason: str) -> StreamCancel:
    outcome = StreamCancel.from_json(raw)
    if (
        outcome.state != StreamState.CANCEL_REQUESTED
        or outcome.terminal
        or outcome.cancelled
    ):
        self.state = StreamState.FAILED
        raise _invalid_stream(
            "stream cancel transport must return CancelRequested with terminal=false"
        )
    return outcome
EOF
  cat >"$CLI/sdk/python/easynet_sdk/bidi.py" <<'EOF'
def decode_bidi_frame(decoded):
    _reject_retired_top_level_receipt_alias(decoded, "bidi frame")

def cancel(self, reason: str) -> BidiOutcome:
    outcome = BidiOutcome.from_json(raw)
    if outcome.state != BidiState.CANCEL_REQUESTED or outcome.terminal:
        self.state = BidiState.FAILED
        raise _invalid_bidi(
            "bidi cancel transport must return CancelRequested with terminal=false"
        )
    return outcome
EOF
  cat >"$CLI/sdk/go/stream_test.go" <<'EOF'
func TestStreamHandleCancelIsNonTerminalRequest(t *testing.T) {}
func TestStreamHandleRejectsTerminalCancelOutcome(t *testing.T) {}
EOF
  cat >"$CLI/sdk/go/bidi_test.go" <<'EOF'
func TestBidiCancelIsNonTerminalRequest(t *testing.T) {}
func TestBidiCancelRejectsTerminalOutcome(t *testing.T) {}
EOF
  cat >"$CLI/sdk/go/direct_runtime_test.go" <<'EOF'
func TestDirectRuntimeStreamCancelIsExplicitlyUnsupported(t *testing.T) {}
func TestDirectRuntimeBidiCancelIsExplicitlyUnsupported(t *testing.T) {}
EOF
  cat >"$CLI/sdk/python/tests/test_stream.py" <<'EOF'
def test_stream_cancel_is_non_terminal_request() -> None:
    pass

def test_stream_cancel_rejects_terminal_outcome() -> None:
    pass
EOF
  cat >"$CLI/sdk/python/tests/test_bidi.py" <<'EOF'
def test_cancel_is_non_terminal_request() -> None:
    pass

def test_cancel_rejects_terminal_outcome() -> None:
    pass
EOF
  cat >"$CLI/sdk/python/tests/test_direct_runtime.py" <<'EOF'
def test_direct_runtime_stream_cancel_is_explicitly_unsupported() -> None:
    pass

def test_direct_runtime_bidi_cancel_is_explicitly_unsupported() -> None:
    pass
EOF

  cat >"$AXON/sdk/rust/src/invocation/handle.rs" <<'EOF'
struct InvocationCore;

impl InvocationCore {
    async fn emit_with(&self, input: ReceiptDraftInput) {
        let terminal = ExecutionTerminal::new(input);
        self.append_signed_receipt(terminal);
    }

    fn append_signed_receipt(&self, terminal: ExecutionTerminal) {}

    async fn complete_runtime_finalization(&self) {}
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

  cat >"$CLI/src/daemon/ability/catalog/profiles/bootstrap.rs" <<'EOF'
fn build_plan_from_registry() -> anyhow::Result<()> {
    AgentAggregateRepository::load_registered_agent_registry_projection()?;
    Ok(())
}

pub fn llm_sub_agents_from_registry(registry: &AgentRegistry) -> Vec<String> {
    registry
        .agents
        .keys()
        .filter_map(|key| AgentId::parse(key).ok())
        .map(|agent_id| agent_id.name)
        .collect()
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/automation/think.rs" <<'EOF'
pub(crate) struct CatalogEntry;

fn run_think() -> serde_json::Value {
    match collect_owner_catalog("alice") {
        Ok(catalog) => serde_json::json!({"catalog_len": catalog.len()}),
        Err(error) => serde_json::json!({
            "curator": {
                "attempted": true,
                "ok": false,
                "stage": "catalog",
                "error": error,
            }
        }),
    }
}

pub(crate) fn collect_owner_catalog(owner: &str) -> Result<Vec<CatalogEntry>, String> {
    let registry = AgentAggregateRepository::load_registered_agent_registry_projection()
        .map_err(|error| format!("owner ability catalog unavailable: {error}"))?;
    let Some(_entry) = registry.agents.get(owner) else {
        return Ok(Vec::new());
    };
    Ok(Vec::new())
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/invocation_wire.rs" <<'EOF'
pub(crate) fn callee_ura_from_envelope(envelope: &Envelope) -> anyhow::Result<String> {
    let callee_ura = envelope
        .callee
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("invocation tuple must carry callee URA"))?
        .ura
        .as_str();
    crate::core::ura::parse_ura(callee_ura)?;
    Ok(callee_ura.to_string())
}

pub(crate) struct LocalDaemonSystemInvocation {
    derivation_policy: InvocationDerivationPolicy,
}

impl LocalDaemonSystemInvocation {
    pub(crate) fn invoke_request(&self) {
        let _request = InvokeRequest {};
    }

    pub(crate) fn stream_request(&self) {
        let _request = InvokeServerStreamRequest {};
    }

    pub(crate) fn envelope(&self) {
        let _envelope = ProtoEnvelope::from_target(caller, callee, subject, self.derivation_policy);
    }

    pub(crate) fn with_trace_id(self, trace_id: Option<&str>) -> Self {
        self
    }
}

fn callee_ura_from_envelope_extracts_explicit_callee() {}
fn callee_ura_from_envelope_rejects_caller_only_tuple() {}
EOF
  cat >"$CLI/src/support/platform/local_daemon_grpc.rs" <<'EOF'
use crate::daemon::invocation::dispatch::invocation_wire::LocalDaemonSystemInvocation;
use tonic::transport::{Channel, Endpoint, Uri as GrpcEndpointLocator};

fn invoke_with_hosted_agent_delegation(hosted_agent_ura: &str) -> anyhow::Result<()> {
    let _delegation = HostedAgentDelegationRequest::new(hosted_agent_ura)?;
    Ok(())
}

fn local_daemon_system_invocation_from_subject_policy() -> LocalDaemonSystemInvocation {
    LocalDaemonSystemInvocation::from_target()
}

struct LocalDaemonSystemSubjectPolicy;

impl LocalDaemonSystemSubjectPolicy {
    fn resolve(&self) -> anyhow::Result<String> {
        local_daemon_identity_ura()
    }
}

fn local_system_invoke_request_does_not_pre_resolve_descriptor_ref() {}
fn local_system_tuple_plan_requires_explicit_targeted_subject() {}
EOF
  cat >"$CLI/src/daemon/invocation/routing/target.rs" <<'EOF'
fn daemon_system_subject_ura_for_descriptor(target: &LocalAbilityTarget) -> anyhow::Result<String> {
    target.owner_subject()
}

impl LocalAbilityTarget {
    pub(crate) fn daemon_system_subject_ura(&self) -> anyhow::Result<String> {
        daemon_system_subject_ura_for_descriptor(self)
    }
}

pub struct LocalTargetRootInvocation;

pub struct SystemInvocationTargetIssuer;

impl SystemInvocationTargetIssuer {
    pub fn local_root_for_target(target: &LocalAbilityTarget) -> anyhow::Result<LocalTargetRootInvocation> {
        Ok(LocalTargetRootInvocation)
    }

    pub fn local_target_root(target: &LocalAbilityTarget) -> anyhow::Result<LocalTargetRootInvocation> {
        Self::local_root_for_target(target)
    }
}
EOF
  cat >"$CLI/src/support/platform/local_invoke.rs" <<'EOF'
fn invoke_issued_target_root_timeout(invocation: LocalTargetRootInvocation) -> anyhow::Result<()> {
    Ok(())
}

fn root_context_for_target(target: &LocalAbilityTarget) -> anyhow::Result<LocalTargetRootInvocation> {
    SystemInvocationTargetIssuer::local_root_for_target(target)
}

fn local_system_context_for_agent_target_uses_agent_owner_subject() {}
fn local_system_context_for_realm_authority_target_uses_ability_subject() {}

pub fn project_invoke_bidi_down_frame(frame: InvokeBidiDown) -> anyhow::Result<Option<LocalBidiFrame>> {
    project_receipt_payload_json("application/json", &[])?;
    Ok(Some(LocalBidiFrame))
}

fn project_receipt_payload_json(content_type: &str, payload: &[u8]) -> anyhow::Result<serde_json::Value> {
    if !content_type.contains("json") {
        anyhow::bail!("InvokeBidi receipt payload declares non-JSON content_type");
    }
    serde_json::from_slice(payload)
        .map_err(|err| anyhow::anyhow!("InvokeBidi receipt payload is not valid JSON: {err}"))
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/integrations/mcp/bridge.rs" <<'EOF'
fn invoke_mcp(target: &LocalAbilityTarget) -> anyhow::Result<()> {
    let _context = root_context_for_target(target)?;
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/integrations/a2a/bridge.rs" <<'EOF'
fn invoke_a2a(target: &LocalAbilityTarget) -> anyhow::Result<()> {
    let _context = root_context_for_target(target)?;
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/axon_bridge/hot_agent_registrar.rs" <<'EOF'
struct HostedAgentRuntimeBinding {
    agent_ura: String,
}

impl HotAgentRegistrar {
    fn register_agent_replacing(&self, name: &str) -> anyhow::Result<()> {
        let enrollment = catalog.enroll_persisted_hot_agent_authority(name)?;
        let binding = HostedAgentRuntimeBinding {
            agent_ura: enrollment.authority_root().to_string(),
        };
        register(binding)
    }
}
EOF
}

bash -n "$CHECK"
make_good_fixture
expect_pass "canonical fixture"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/mission/executors"
cat >"$CLI/src/daemon/execution/mission/executors/eal.rs" <<'EOF'
use std::time::Duration;

fn run_eal_exec(timeout: Option<Duration>) {
    let _ = timeout.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    run_mission_inproc(source, MissionRunOpts {
        source_label: Some("ability:eal".to_string()),
        trace_path: None,
        invocation_context: None,
    });
}
EOF
expect_fail \
  "EAL executor run timeout fork" \
  "R63_EAL_EXEC_RUN_TIMEOUT_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/catalog_metadata.rs" <<'EOF'
fn registration_hints(owner_ura: &str, registry_name: &str, call_mode: DescriptorCallMode) -> AbilityHints {
    let public_name = crate::core::ura::descriptor_public_ability_name(owner_ura, registry_name);
    AbilityHints {
        destructive: false,
        ..Default::default()
    }
}

fn description_for(name: &str) -> &'static str {
    match name {
        agent_names::AGENT_STOP | agent_names::AGENT_PURGE => agent_lifecycle_ability::stop_agent_description(),
        agent_names::AGENT_PURGE_RECONCILE => agent_lifecycle_ability::purge_reconcile_description(),
        _ => "generic",
    }
}

fn input_schema_for(name: &str) -> Value {
    match name {
        agent_names::AGENT_STOP | agent_names::AGENT_PURGE => agent_lifecycle_ability::stop_agent_input_schema(),
        agent_names::AGENT_PURGE_RECONCILE => agent_lifecycle_ability::purge_reconcile_input_schema(),
        _ => json!({}),
    }
}
EOF
expect_fail \
  "Agent purge public boundary fork" \
  "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK"

make_good_fixture
cat >"$CLI/ability-descriptors/system/agents/agent.purge.ability.toml" <<'EOF'
name = "agent.purge"
description = "Remove an LLM sub-agent registry row by name or Agent URA."
admission_action = "invoke"
hints_json = "{\"read_only\":false,\"destructive\":false,\"idempotent\":false,\"streaming_only\":false,\"bidi_only\":false}"
EOF
expect_fail \
  "Agent purge descriptor boundary fork" \
  "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/agents/lifecycle.rs" <<'EOF'
struct AgentLifecycleProjectionStore;

fn stop_agent_locked(registry: &AgentRegistry, identities: &local_agents::LocalAgentsFile) {
    transaction.persist_registry_projection(&registry);
    transaction.persist_identity_projection(&identities);
}
EOF
expect_fail \
  "Agent purge lifecycle boundary fork" \
  "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK"

make_good_fixture
cat >>"$CLI/src/daemon/ability/builtins/agents/lifecycle.rs" <<'EOF'
fn ensure_identity_bound_purge_supported() -> anyhow::Result<()> {
    Ok(())
}

fn remove_quarantined_directory_identity_bound(
    quarantine: &std::path::Path,
    expected_identity: &AgentRootIdentity,
) -> anyhow::Result<()> {
    PlatformTreeDeletion::remove_quarantined_directory_identity_bound(quarantine, expected_identity)
}
EOF
expect_fail \
  "Agent purge platform deletion owner fork" \
  "R32_AGENT_PURGE_PUBLIC_BOUNDARY_FORK"

make_good_fixture
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
cat >"$AXON/sdk/rust/src/invocation/handle.rs" <<'EOF'
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
cat >"$CLI/docs/AXON-RFC-006-stateful-easynet.tex" <<'EOF'
\subsection{The principal/ caller URI scheme}
\label{sec:caller-uri}
The owner agent is identified by an EasyNet agent URI.
EOF
expect_fail \
  "RFC-006 identity URI terminology" \
  "R4_CURRENT_DOC_IDENTITY_URI_TERMINOLOGY"

make_good_fixture
mkdir -p "$CLI/docs/rfc"
cat >"$CLI/docs/rfc/AXON-RFC-006-stateful-easynet.md" <<'EOF'
TR-INV-12 requires every hub-translated caller to use a principal URI.
EOF
expect_fail \
  "RFC-006 companion identity URI terminology" \
  "R4_CURRENT_DOC_IDENTITY_URI_TERMINOLOGY"

make_good_fixture
mkdir -p "$CLI/docs/rfc"
cat >"$CLI/docs/rfc/AXON-RFC-003-invokebidi-protocol.md" <<'EOF'
membership_gate checks caller URI directory membership.
EOF
expect_fail \
  "RFC-003 identity URI terminology" \
  "R4_CURRENT_DOC_IDENTITY_URI_TERMINOLOGY"

make_good_fixture
cat >"$CLI/docs/PAGES_AND_LLM_API.md" <<'EOF'
INV-2 Capability-URI Key addresses api keys as resources.
EOF
expect_fail \
  "Pages API identity URI terminology" \
  "R4_CURRENT_DOC_IDENTITY_URI_TERMINOLOGY"

make_good_fixture
cat >>"$CLI/sdk/python/easynet_sdk/runtime.py" <<'EOF'
def decode_unary_result(decoded):
    return decoded.get("receipt")
EOF
expect_fail \
  "unary result receipt alias" \
  "R7_UNARY_RESULT_RECEIPT_ALIAS"

make_good_fixture
mkdir -p "$CLI/sdk/python/easynet_sdk/providers/runtime"
cat >"$CLI/sdk/python/easynet_sdk/providers/runtime/direct.py" <<'EOF'
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
mkdir -p "$CLI/sdk/python/easynet_sdk/providers/runtime"
cat >"$CLI/sdk/python/easynet_sdk/providers/runtime/direct.py" <<'EOF'
def emit_stream_event(event, terminal_receipt):
    event["receipt"] = terminal_receipt

def emit_bidi_event(event, receipt):
    event["payload_json"] = {"receipt": receipt}
EOF
expect_fail \
  "direct runtime stream bidi receipt alias" \
  "R11_STREAM_BIDI_RECEIPT_ALIAS"

make_good_fixture
cat >"$CLI/sdk/python/easynet_sdk/stream.py" <<'EOF'
def decode_stream_event(decoded):
    return decoded.get("terminal_receipt")
EOF
expect_fail \
  "stream frame missing retired receipt alias rejection" \
  "R64_SDK_RETIRED_RECEIPT_ALIAS_REJECTION"

make_good_fixture
cat >"$CLI/sdk/go/errors.go" <<'EOF'
func runtimeFailureCode(code string, fallback ErrorCode) ErrorCode {
    if code == "" {
        return fallback
    }
    return ErrProtocolMismatch
}
EOF
expect_fail \
  "Go SDK runtime failure extension code parity" \
  "R65_SDK_RUNTIME_FAILURE_EXTENSION_CODE_PARITY"

make_good_fixture
cat >"$CLI/sdk/python/easynet_sdk/errors.py" <<'EOF'
def canonical_failure_code(code=None):
    if code:
        return ErrorCode.PROTOCOL_MISMATCH
    return ErrorCode.ADMISSION_DENIED
EOF
expect_fail \
  "Python SDK runtime failure extension code parity" \
  "R65_SDK_RUNTIME_FAILURE_EXTENSION_CODE_PARITY"

make_good_fixture
mkdir -p "$CLI/src/ffi/invocation"
cat >"$CLI/src/ffi/invocation/mod.rs" <<'EOF'
fn stream_chunk_json() -> serde_json::Value {
    serde_json::json!({
        "event": "chunk",
        "content_type": "application/json",
    })
}
EOF
expect_fail \
  "C ABI stream callback alias" \
  "R30_SDK_STREAM_BIDI_CALLBACK_ALIAS"

make_good_fixture
cat >"$CLI/sdk/go/cabi_runtime.go" <<'EOF'
func projectCABIOrderedEvent(raw []byte, allocateSequence func(*uint64) uint64, useObservedSequence bool) ([]byte, error) {
    event := map[string]any{}
    if data, ok := event["data_base64"]; ok {
        event["payload_base64"] = data
    }
    if event["event"] == "binary_chunk" {
        event["kind"] = "data"
    }
    return raw, nil
}
EOF
expect_fail \
  "Go C ABI callback repair alias" \
  "R30_SDK_STREAM_BIDI_CALLBACK_ALIAS"

make_good_fixture
cat >"$CLI/sdk/python/easynet_sdk/_cabi.py" <<'EOF'
def _project_cabi_ordered_event(raw, allocate_sequence, use_observed_sequence):
    event = {}
    if "payload_base64" not in event and "data_base64" in event:
        event["payload_base64"] = event.get("data_base64")
    if event.get("event") == "binary_chunk":
        event["kind"] = "data"
    return raw
EOF
expect_fail \
  "Python C ABI callback repair alias" \
  "R30_SDK_STREAM_BIDI_CALLBACK_ALIAS"

make_good_fixture
cat >>"$CLI/src/daemon/ability/builtins/integrations/openai_compat.rs" <<'EOF'
fn legacy_files_dispatch(user: &str) -> String {
    format!("{user}.files.get")
}
EOF
expect_fail \
  "OpenAI legacy files dispatch" \
  "R31_FILE_RESOURCE_OWNERSHIP_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/resources/files_store/mod.rs" <<'EOF'
pub(crate) fn management_agent_ura(realm: &str, user: &str) -> String {
    crate::core::ura::agent_ura(realm, user, "files")
}

pub fn register(reg: &mut AxonAbilityCatalog, config: FilesConfig) {
    let owner = OwnerKind::Device;
    reg.register_rpc_with_owner("files.put", owner, put_handler);
    reg.register_rpc_with_owner("files.get", owner, get_handler);
    reg.register_rpc_with_owner("files.list", owner, list_handler);
}
EOF
expect_fail \
  "Device-owned files resource surface" \
  "R31_FILE_RESOURCE_OWNERSHIP_FORK"

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
cat >"$CLI/src/daemon/ability/builtins/agents/list.rs" <<'EOF'
pub fn register<F>(reg: &mut AxonAbilityCatalog, registry_provider: F)
where
    F: Fn() -> anyhow::Result<AgentRegistry> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> anyhow::Result<AgentRegistry> + Send + Sync> =
        Arc::new(registry_provider);
    reg.register_rpc_with_owner(
        ABILITY_LIST_AGENTS,
        OwnerKind::Device,
        Arc::new(move |_args: Value| list_agents_handler(&provider)),
    );
}

fn list_agents_handler(
    registry_provider: &Arc<dyn Fn() -> anyhow::Result<AgentRegistry> + Send + Sync>,
) -> anyhow::Result<Value> {
    let registry = registry_provider()?;
    let local_agents = crate::daemon::persistence::local_agents::load()?;
    Ok(json!({ "agents": agent_rows(&registry, &local_agents)? }))
}
EOF
expect_fail \
  "agent.list aggregate snapshot fork" \
  "R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/build.rs" <<'EOF'
fn build_registry(shared_stores: RegistrySharedStores, hosts_hub_authority: bool) {
    agent_list_ability::register(&mut reg, || {
        crate::daemon::persistence::agent_registry::load_agents()
            .map_err(|error| anyhow::anyhow!("agent.list: load durable agent registry: {error:#}"))
    });
}
EOF
expect_fail \
  "agent.list catalog snapshot fork" \
  "R33_AGENT_LIST_AGGREGATE_SNAPSHOT_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/receipts"
cat >"$CLI/src/daemon/invocation/receipts/finalization_projection.rs" <<'EOF'
use axon_sdk::invocation::FinalizationCheckpointVerifier;

fn verify_wire(receipt: InvocationReceipt) {
    axon_sdk::invocation::wire::try_receipt_from_wire(receipt);
}
EOF
expect_pass "receipt proof primitives owned by adapter"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/manual_receipt_verify.rs" <<'EOF'
fn verify_wire(receipt: InvocationReceipt) {
    axon_sdk::invocation::wire::try_receipt_from_wire(receipt);
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
python3 - "$CLI/src/daemon/boot/invocation/mod.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
text = text.replace("    service.register_daemon_stream_routes(daemon_route_owner)?;\n", "")
path.write_text(text)
PY
expect_fail \
  "daemon exact stream boot registration owner fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
python3 - "$CLI/src/daemon/boot/invocation/mod.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
text = text.replace("        service.register_daemon_bidi_routes(daemon_route_owner)?;\n", "")
path.write_text(text)
PY
expect_fail \
  "daemon exact bidi boot registration owner fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/bidi/bidi_dispatcher.rs" <<'EOF'
impl BidiDispatcher {
    async fn dispatch(&self, ability_name: &str) {
        match ability_name {
            "session.open" => self.dispatch_self_session_accept().await,
            _ => self.dispatch_local_bidi_selected_route().await,
        }
    }
}
EOF
expect_fail \
  "daemon exact bidi route inventory fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
python3 - "$CLI/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
needle = "        let dispatcher = self.bidi_dispatcher();\n"
text = text.replace(
    needle,
    "        self.admission.verify_envelope_for_bidi(envelope_open)?;\n"
    + needle,
    1,
)
path.write_text(text)
PY
expect_fail \
  "daemon bidi legacy outer admission root fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
python3 - "$CLI/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
needle = "impl DaemonInvocationService {\n"
text = text.replace(
    needle,
    needle + "    fn legacy_unary_root(&self, inner: &InvokeRequest) {\n"
    "        self.admission.verify_invoke(inner);\n"
    "    }\n",
    1,
)
path.write_text(text)
PY
expect_fail \
  "daemon unary legacy outer admission root fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
python3 - "$CLI/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
needle = "    async fn invoke_stream(&self, inner: InvokeServerStreamRequest) {\n"
text = text.replace(
    needle,
    needle + "        self.admission.verify_invoke_stream(&inner)?;\n",
    1,
)
path.write_text(text)
PY
expect_fail \
  "daemon stream legacy outer admission root fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
python3 - "$CLI/src/daemon/invocation/bidi/bidi_dispatcher.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
text = text.replace(
    "struct SessionOpenProvider {\n    policy: SessionOpenPolicy,\n}",
    "struct SessionOpenProvider {\n"
    "    admission: AdmissionFacade,\n"
    "    session_realm: Option<String>,\n"
    "}",
)
path.write_text(text)
PY
expect_fail \
  "session.open provider transport-policy owner fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/runtime_admin_contracts.rs" <<'EOF'
const SESSION_OPEN_TEMPLATE_DEVICE_URA: &str = "easynet:///r/_system/device/session-open-template";

fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_control_plane_descriptor_with_scope(
        "session.open",
        &OwnerKind::Device,
    );
}
EOF
expect_fail \
  "session.open Device owner contract fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
cat >>"$CLI/src/daemon/ability/dispatch.rs" <<'EOF'
impl AxonAbilityCatalog {
    fn register_control_plane_descriptor_with_scope(
        &self,
        ability: &str,
        authority_scope: AuthorityScope,
    ) {
        self.register_control_plane_with_scope(ability, authority_scope);
    }
}
EOF
expect_fail \
  "explicit descriptor authority scope registration fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/bidi/session_initiator/envelope.rs" <<'EOF'
fn build_session_envelope_open(caller_ura: &str) {
    let callee = AgentIdentity {
        ura: caller_ura.to_string(),
    };
    catalog_descriptor_ref_for_wire(caller_ura, "session.open", CallMode::Bidi);
}
EOF
expect_fail \
  "session.open signed tuple owner fork" \
  "R16_DAEMON_ROUTE_RUNTIME_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/support/platform/local_daemon_grpc.rs" <<'EOF'
struct LocalDaemonSystemInvocation {
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
}
EOF
expect_fail \
  "local daemon-system support request owner fork" \
  "R16B_LOCAL_DAEMON_SYSTEM_INVOCATION_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/support/platform/local_daemon_grpc.rs" <<'EOF'
fn rebuild_daemon_system_envelope(caller: String, callee: String, subject: String) {
    let _envelope = crate::daemon::invocation::ProtoEnvelope::targeted(caller, callee, subject);
}
EOF
expect_fail \
  "local daemon-system support envelope owner fork" \
  "R16B_LOCAL_DAEMON_SYSTEM_INVOCATION_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/descriptor_bound_dispatch.rs" <<'EOF'
fn dispatch_rpc_local_explicit_subject() {
    let envelope = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller: system_agent_identity(),
        causal_context: CausalContext::None,
    });
}
EOF
expect_fail \
  "system invocation issuer owner fork" \
  "R16C_SYSTEM_INVOCATION_ISSUER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/descriptor_bound_dispatch.rs" <<'EOF'
pub fn local_system_from_wire_parts() {
    SystemInvocationIssuer::request_for_complete_envelope();
}

fn open_stream_local_explicit_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}
EOF
expect_fail \
  "public unsealed local-system wire ingress fork" \
  "R16C_SYSTEM_INVOCATION_ISSUER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/descriptor_bound_dispatch.rs" <<'EOF'
struct LocalSystemAuthority;

struct WireDispatch {
    local_system_authority: Option<LocalSystemAuthority>,
}

pub(crate) fn local_system_from_wire_parts() {
    let local_system_authority = true.then_some(LocalSystemAuthority);
    SystemInvocationIssuer::request_for_complete_envelope();
}

fn request_for_wire_dispatch(local_system_authority: Option<LocalSystemAuthority>) {
    local_system_authority.ok_or_else(|| "trusted-local authority required");
}

fn open_stream_local_explicit_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}

fn open_bidi_local_explicit_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}

fn dispatch_rpc_local_with_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}
EOF
expect_fail \
  "retired local-system with_subject dispatch vocabulary" \
  "R16C_SYSTEM_INVOCATION_ISSUER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/wire_descriptor.rs" <<'EOF'
enum WireCallerIdentity {
    FromEnvelope,
    LocalSystem,
}

fn descriptor_bound_from_wire_parts(envelope: Envelope) {
    let caller = system_agent_identity();
    let subject = SubjectIdentity::from_callee(&callee);
    let nonce = fresh_nonce();
}
EOF
expect_fail \
  "wire local-system tuple fallback fork" \
  "R16C_SYSTEM_INVOCATION_ISSUER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/manual_envelope.rs" <<'EOF'
fn build_envelope(caller: AgentIdentity, callee: AgentIdentity, subject: SubjectIdentity) {
    let _envelope = Envelope {
        caller: Some(caller),
        callee: Some(callee),
        subject: Some(subject),
        invocation_nonce: vec![1; 16],
        ..Envelope::default()
    };
}
EOF
expect_fail \
  "manual canonical envelope owner fork" \
  "R16D_CANONICAL_ENVELOPE_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/local_runtime_request.rs" <<'EOF'
pub(crate) enum LocalRuntimeIngress {
    LocalSystem { envelope: DescriptorBoundEnvelope, payload: Vec<u8> },
}

pub(crate) struct LocalRuntimeRequestFactory;

impl LocalRuntimeRequestFactory {
    pub(crate) fn request_for_local_system() {
        sign_system_canonical(&descriptor_bound_canonical_bytes(&envelope));
    }
}

pub(crate) struct SystemInvocationIssuer;

impl SystemInvocationIssuer {
    pub(crate) fn request_for_descriptor_ref() {}
    pub(crate) fn request_for_complete_envelope() {}
}
EOF
expect_fail \
  "direct local-system request factory ingress fork" \
  "R16C_SYSTEM_INVOCATION_ISSUER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/local_runtime_invoker.rs" <<'EOF'
async fn local_system_request() {
    let envelope = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller: system_agent_identity(),
        invocation_nonce: fresh_nonce(),
        causal_context: CausalContext::None,
    });
}

pub async fn rpc_value_from_handle(handle: InvocationHandle) {
    handle.finalized().await;
}
EOF
expect_fail \
  "local runtime invoker system issuer fork" \
  "R16C_SYSTEM_INVOCATION_ISSUER_FORK"

make_good_fixture
cat >"$CLI/src/cli/commands/remote_exec.rs" <<'EOF'
fn invoke_remote_exec(target: &str, payload: Value) -> Result<Value> {
    let call = RemoteAbilityInvocationTarget::for_target_owned_selector(target, "process.exec")?;
    remote_invoke::invoke_remote_target(&call, payload, None)
}
EOF
expect_fail \
  "cli remote system ability facade fork" \
  "R23_CLI_REMOTE_SYSTEM_ABILITY_FACADE_FORK"

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
cat >"$CLI/src/cli/commands/start.rs" <<'EOF'
fn bootstrap_local_agent_projection(creds: &Credentials) {
    let plan = build_bootstrap_plan(creds)?;
    let mut file = local_agents::load()?;
    let outcomes = bootstrap::bootstrap_local_agents(&plan, &mut file, &UuidMinter);
    local_agents::save(&file)?;
    Ok(outcomes)
}
EOF
expect_fail \
  "cli start hosted identity projection bypasses lifecycle owner" \
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
cat >"$CLI/src/daemon/execution/mission/dispatch.rs" <<'EOF'
#[test]
#[ignore]
fn agent_send_desugar_e2e() {}
EOF
expect_fail \
  "mission recursion ignored evidence" \
  "R23B_MISSION_RECURSION_IGNORED_EVIDENCE"

make_good_fixture
cat >"$CLI/src/daemon/execution/mission/dispatch.rs" <<'EOF'
fn resolve_model_with_overrides(
    override_model: Option<String>,
    spec_model: Option<String>,
    entry_model: Option<String>,
) -> Option<String> {
    override_model.or(spec_model).or(entry_model)
}

fn send(entry: AgentEntry, spec_model: Option<String>) -> Option<String> {
    resolve_model_with_overrides(None, spec_model, entry.model.clone())
}
EOF
expect_fail \
  "mission model entry fallback" \
  "R66_MISSION_RUNTIME_CONFIG_ENTRY_FALLBACK"

make_good_fixture
cat >"$CLI/src/daemon/execution/mission/dispatch.rs" <<'EOF'
fn resolve_timeout(spec_timeout_secs: Option<u64>, entry_timeout_secs: u64) -> Duration {
    Duration::from_secs(spec_timeout_secs.unwrap_or(entry_timeout_secs))
}

fn send(entry: AgentEntry, spec_timeout_secs: Option<u64>) -> Duration {
    resolve_timeout(spec_timeout_secs, entry.timeout_secs)
}
EOF
expect_fail \
  "mission timeout entry fallback" \
  "R66_MISSION_RUNTIME_CONFIG_ENTRY_FALLBACK"

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
cat >"$CLI/src/daemon/ability/dispatch.rs" <<'EOF'
enum HotAgentAuthorityInventoryError {
    CounterOverflow,
}

struct HotAgentAuthorityEnrollment {
    agent: String,
}

impl PersistedHotAgentAuthority {
    fn load(agent: &str) -> Result<(), HotAgentAuthorityInventoryError> {
        let registry = crate::daemon::persistence::agent_registry::load_agents()?;
        if !registry.agents.contains_key(agent) {
            return Err(HotAgentAuthorityInventoryError::CounterOverflow);
        }
        let identities = crate::daemon::persistence::local_agents::load()?;
        let identity = identities
            .hosted_agents
            .iter()
            .find(|entry| entry.profile == "llm" && entry.name == agent)
            .ok_or(HotAgentAuthorityInventoryError::CounterOverflow)?;
        Ok(())
    }
}

impl HotAgentAuthorityInventory {
    fn revoke_after_durable_removal(
        &self,
        enrollment: &HotAgentAuthorityEnrollment,
    ) -> Result<(), HotAgentAuthorityInventoryError> {
        let registry = crate::daemon::persistence::agent_registry::load_agents()?;
        if registry.agents.contains_key(&enrollment.agent) {
            return Err(HotAgentAuthorityInventoryError::CounterOverflow);
        }
        let identities = crate::daemon::persistence::local_agents::load()?;
        if identities
            .hosted_agents
            .iter()
            .any(|entry| entry.profile == "llm" && entry.name == enrollment.agent)
        {
            return Err(HotAgentAuthorityInventoryError::CounterOverflow);
        }
        Ok(())
    }
}
EOF
expect_fail \
  "hot authority aggregate snapshot fork" \
  "R34_HOT_AUTHORITY_AGGREGATE_SNAPSHOT_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/hot_agent_registrar.rs" <<'EOF'
struct HostedAgentRuntimeBinding {
    agent_ura: String,
}

impl HotAgentRegistrar {
    fn register_agent_replacing(&self, name: &str) -> anyhow::Result<()> {
        let local = crate::daemon::persistence::local_agents::load()?;
        let agent_ura = crate::daemon::persistence::local_agents::lookup_hosted_ura(
            &local,
            "llm",
            name,
        )
        .ok_or_else(|| anyhow::anyhow!("missing"))?;
        let binding = HostedAgentRuntimeBinding { agent_ura };
        register(binding)
    }
}
EOF
expect_fail \
  "hot agent runtime binding aggregate fork" \
  "R34B_HOT_AGENT_RUNTIME_BINDING_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/admission/target_gate.rs" <<'EOF'
struct LocalAgentTargetIndex {
    hosted_agent_targets: HashSet<AgentTargetIdentity>,
    registered_agent_ids: HashSet<String>,
}

impl LocalAgentTargetIndex {
    fn load() -> Self {
        Self {
            hosted_agent_targets: load_hosted_agent_targets(),
            registered_agent_ids: load_registered_agent_ids(),
        }
    }
}

fn load_hosted_agent_targets() -> HashSet<AgentTargetIdentity> {
    crate::daemon::persistence::local_agents::load()
        .map(|file| {
            file.hosted_agents
                .into_iter()
                .filter_map(|entry| parse_agent_target_identity(&entry.agent_ura))
                .collect()
        })
        .unwrap_or_default()
}

fn load_registered_agent_ids() -> HashSet<String> {
    crate::daemon::persistence::agent_registry::load_agents()
        .map(|registry| registry.agents.into_keys().collect())
        .unwrap_or_default()
}
EOF
expect_fail \
  "target gate aggregate snapshot fork" \
  "R35_TARGET_GATE_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/routing/route_resolver.rs" <<'EOF'
struct LocalHostedAgentPlacements {
    by_agent_ura: HashMap<String, HostedAgentPlacement>,
}

impl LocalHostedAgentPlacements {
    fn load() -> Self {
        crate::daemon::persistence::local_agents::load()
            .map(|file| Self::from_file(&file))
            .unwrap_or_default()
    }

    fn from_file(file: &crate::daemon::persistence::local_agents::LocalAgentsFile) -> Self {
        let host_device_ura = file.host_device_agent_ura.trim();
        let by_agent_ura = file
            .hosted_agents
            .iter()
            .map(|entry| {
                (
                    entry.agent_ura.clone(),
                    HostedAgentPlacement {
                        host_device_ura: host_device_ura.to_string(),
                    },
                )
            })
            .collect();
        Self { by_agent_ura }
    }
}
EOF
expect_fail \
  "route resolver aggregate placement fork" \
  "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/health.rs" <<'EOF'
fn scan() -> anyhow::Result<ScanPlan> {
    let mut plan = ScanPlan {
        monitored: Vec::new(),
        unmonitored: Vec::new(),
        live: BTreeSet::new(),
    };
    let registry = crate::daemon::persistence::agent_registry::load_agents()
        .map_err(|error| anyhow::anyhow!("load durable agent registry for health scan: {error:#}"))?;
    let local = crate::daemon::persistence::local_agents::load()
        .map_err(|error| anyhow::anyhow!("load hosted-agent URA index for health scan: {error:#}"))?;
    for (agent_name, entry) in &registry.agents {
        let Some(owner_ura) =
            crate::daemon::persistence::local_agents::lookup_hosted_ura(&local, "llm", agent_name)
        else {
            continue;
        };
        let _ = (owner_ura, entry);
    }
    Ok(plan)
}
EOF
expect_fail \
  "ability health aggregate snapshot fork" \
  "R36_ABILITY_HEALTH_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/routing/route_resolver.rs" <<'EOF'
struct LocalHostedAgentPlacements {
    by_agent_ura: HashMap<String, HostedAgentPlacement>,
}

impl LocalHostedAgentPlacements {
    fn load() -> Self {
        crate::daemon::persistence::local_agents::load()
            .map(|file| Self::from_file(&file))
            .unwrap_or_default()
    }

    fn from_file(file: &crate::daemon::persistence::local_agents::LocalAgentsFile) -> Self {
        let host_device_ura = file.host_device_agent_ura.trim();
        let by_agent_ura = file
            .hosted_agents
            .iter()
            .filter_map(|entry| Some((entry.agent_ura.clone(), HostedAgentPlacement {
                host_device_ura: host_device_ura.to_string(),
                host_node_id: None,
            })))
            .collect();
        Self { by_agent_ura }
    }
}
EOF
expect_fail \
  "route resolver hosted placement aggregate fork" \
  "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/routing/route_resolver.rs" <<'EOF'
struct LocalHostedAgentPlacements {
    state: HostedPlacementProjectionState,
}

enum HostedPlacementProjectionState {
    Available,
    Unavailable { reason: String },
}

impl LocalHostedAgentPlacements {
    fn load() -> Self {
        match AgentAggregateRepository::try_load_snapshot() {
            Ok(snapshot) => Self::from_projection(snapshot.hosted_agent_placements()),
            Err(error) => Self {
                state: HostedPlacementProjectionState::Unavailable {
                    reason: format!("{error:#}"),
                },
            },
        }
    }

    fn from_projection(projection: AgentHostedPlacementProjection) -> Self {
        Self {
            state: HostedPlacementProjectionState::Available,
        }
    }
}

fn route_selector_from_query(query_name: &str, ability_name: &str) -> Option<RouteSelector> {
    if ability_name.trim().is_empty() {
        if let Some(selector) = ability_selector_from_descriptor_ref(query_name) {
            return Some(RouteSelector);
        }
    }
    None
}

fn ability_selector_from_descriptor_ref(
    descriptor_ref: &str,
) -> Option<crate::core::ura::AbilitySelector> {
    let descriptor_ref =
        axon_sdk::invocation::canonical_ability_descriptor_ref(descriptor_ref).ok()?;
    let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
        &descriptor_ref,
    )
    .ok()?;
    crate::core::ura::AbilitySelector::parse(&ability_ura).ok()
}
EOF
expect_fail \
  "route resolver descriptor-ref selector fallback" \
  "R37_ROUTE_RESOLVER_AGENT_PLACEMENT_AGGREGATE_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/mission"
cat >"$CLI/src/daemon/execution/mission/orchestration.rs" <<'EOF'
fn find_traditional_agent_target_conflict(ir: &MissionIr) -> anyhow::Result<Option<TraditionalAgentTargetConflict>> {
    let registry = crate::daemon::persistence::agent_registry::load_agents()?;
    let registered: HashSet<String> = registry.agents.into_keys().collect();
    Ok(None)
}
EOF
cat >"$CLI/src/daemon/execution/mission/invocation_gateway.rs" <<'EOF'
struct PersistedMissionChildTargetResolver;

impl MissionChildTargetResolver for PersistedMissionChildTargetResolver {
    fn callee_ura(&self, request: &MissionInvocationRequest) -> anyhow::Result<String> {
        let local_agents = crate::daemon::persistence::local_agents::load()?;
        let entry = crate::daemon::persistence::local_agents::lookup_hosted_agent_by_name(
            &local_agents,
            request.hosted_agent.as_deref().unwrap(),
        )?;
        Ok(entry.unwrap().agent_ura.clone())
    }
}
EOF
mkdir -p "$CLI/src/support/platform" "$CLI/src/cli/commands"
cat >"$CLI/src/support/platform/local_daemon_grpc.rs" <<'EOF'
fn canonical_hosted_agent_ura_by_name(agent_name: &str) -> anyhow::Result<String> {
    let local_agents = crate::daemon::persistence::local_agents::load()?;
    let entry = crate::daemon::persistence::local_agents::lookup_hosted_agent_by_name(
        &local_agents,
        agent_name,
    )?;
    Ok(entry.unwrap().agent_ura.clone())
}
EOF
cat >"$CLI/src/cli/commands/teach.rs" <<'EOF'
fn resolve_learner_ura(learner: &str) -> anyhow::Result<String> {
    let local = crate::daemon::persistence::local_agents::load()?;
    let entry =
        crate::daemon::persistence::local_agents::lookup_hosted_agent_by_name(&local, learner)?;
    Ok(entry.unwrap().agent_ura.clone())
}
EOF
cat >"$CLI/src/daemon/ability/builtins/governance/teach.rs" <<'EOF'
fn require_owner_authority(owner_agent: &str) -> anyhow::Result<String> {
    let local = crate::daemon::persistence::local_agents::load()?;
    let entry =
        crate::daemon::persistence::local_agents::lookup_hosted_agent_by_name(&local, owner_agent)?;
    Ok(entry.unwrap().agent_ura.clone())
}
EOF
cat >"$CLI/src/daemon/persistence/local_agents.rs" <<'EOF'
fn lookup_hosted_agent_by_name(file: &LocalAgentsFile, name: &str) -> anyhow::Result<Option<&HostedAgentEntry>> {
    Ok(None)
}
EOF
expect_fail \
  "mission child target aggregate fork" \
  "R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/support/platform/local_daemon_grpc.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, HostedAgentNameLookupError,
};

fn resolve_hosted_agent_callee(agent_name: &str) -> anyhow::Result<String> {
    let snapshot = AgentAggregateRepository::try_load_snapshot()?;
    snapshot
        .hosted_agent_ura_by_name(agent_name)
        .map_err(|error: HostedAgentNameLookupError| anyhow::anyhow!("{error}"))?
        .ok_or_else(|| anyhow::anyhow!("unknown hosted Agent"))
}
EOF
expect_fail \
  "local daemon transport hosted Agent name lookup fork" \
  "R38_MISSION_CHILD_TARGET_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/agents/chat.rs" <<'EOF'
fn register(reg: &mut Catalog) {
    reg.register_rpc_with_owner("agents.chat", handler);
}

fn handler(binding: &ChatImplementationBinding) {
    binding.execute_admitted();
}

fn build_discover_handler_for() {
    crate::daemon::persistence::agent_registry::load_agents().unwrap();
}

fn build_invoke_handler_for() {
    crate::daemon::persistence::agent_registry::load_agents().unwrap();
}

fn enumerate_other_agent_specs() {
    crate::daemon::persistence::agent_registry::load_agents().unwrap();
}
EOF
expect_fail \
  "agent chat aggregate provider fork" \
  "R39_AGENT_CHAT_AGGREGATE_PROVIDER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/governance/admin_status.rs" <<'EOF'
fn handler() -> anyhow::Result<()> {
    let local = crate::daemon::persistence::local_agents::load()?;
    let _joined = !local.host_device_agent_ura.is_empty();
    let _count = local.hosted_agents.len();
    Ok(())
}
EOF
expect_fail \
  "governance status aggregate provider fork" \
  "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/governance/invocation_history.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn ledger_resource_ura() -> Option<String> {
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_status().ok()?;
    let parsed = crate::core::ura::parse_ura(hosted_identity.host_device_agent_ura()?).ok()?;
    Some(crate::core::ura::resource_dot_ura(&parsed.realm, "device.dev-1", "billing/invocations"))
}
EOF
expect_fail \
  "invocation history ledger URA projection fallback" \
  "R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/eal/interpreter/dispatch.rs" <<'EOF'
fn load_registry_or_warn() -> AgentRegistry {
    crate::daemon::persistence::agent_registry::load_agents().unwrap_or_default()
}

fn dispatch_step(client: &InvocationClient, child: ChildInvocation) {
    client.invoke_remote(child);
}
EOF
expect_fail \
  "EAL agent dispatch aggregate provider fork" \
  "R41_EAL_AGENT_DISPATCH_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/eal/interpreter/dispatch.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn load_registry_projection_for_dispatch() -> AgentRegistry {
    AgentAggregateRepository::load_registered_agent_registry_projection().unwrap_or_default()
}
EOF
expect_fail \
  "EAL agent dispatch empty-registry fallback" \
  "R41_EAL_AGENT_DISPATCH_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/cli/commands/abilities.rs" <<'EOF'
fn local_agent_ura(agent: &str) -> anyhow::Result<String> {
    let local = crate::daemon::persistence::local_agents::load()?;
    crate::daemon::persistence::local_agents::lookup_hosted_ura(&local, "llm", agent)
        .ok_or_else(|| anyhow::anyhow!("missing"))
}
EOF
cat >"$CLI/src/daemon/ability/builtins/agents/discover.rs" <<'EOF'
struct LocalAgentAbilityOwners {
    local_agents: crate::daemon::persistence::local_agents::LocalAgentsFile,
}

impl LocalAgentAbilityOwners {
    fn load() -> anyhow::Result<Self> {
        Ok(Self {
            local_agents: crate::daemon::persistence::local_agents::load()?,
        })
    }

    fn owner_ura_for(&self, agent_name: &str) -> Option<String> {
        crate::daemon::persistence::local_agents::lookup_hosted_ura(
            &self.local_agents,
            "llm",
            agent_name,
        )
    }
}
EOF
expect_fail \
  "hosted owner lookup aggregate fork" \
  "R42_HOSTED_OWNER_LOOKUP_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/profiles/mod.rs" <<'EOF'
fn load_host_descriptors() -> Vec<AbilityDescriptor> {
    let local = crate::daemon::persistence::local_agents::load().unwrap();
    let host_ura = local.host_device_agent_ura.clone();
    let consent_ura =
        crate::daemon::persistence::local_agents::lookup_hosted_ura(&local, "consent", "default");
    let mcp_ura =
        crate::daemon::persistence::local_agents::lookup_hosted_ura(&local, "mcp", "default");
    let llm_uras: Vec<(String, String)> = local
        .hosted_agents
        .iter()
        .filter(|entry| entry.profile == "llm")
        .map(|entry| (entry.name.clone(), entry.agent_ura.clone()))
        .collect();
    all_descriptors_for_host(&host_ura, consent_ura.as_deref(), mcp_ura.as_deref(), &llm_uras)
}
EOF
expect_fail \
  "host descriptor identity aggregate fork" \
  "R43_HOST_DESCRIPTOR_IDENTITY_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/identity/local_invocation.rs" <<'EOF'
fn persisted_local_device_ura() -> Option<String> {
    let local = crate::daemon::persistence::local_agents::load().ok()?;
    Some(local.host_device_agent_ura.trim().to_string())
}
EOF
cat >"$CLI/src/daemon/resources/context/clipboard_tracker.rs" <<'EOF'
pub fn spawn() {
    let device_ura = crate::daemon::persistence::local_agents::load()
        .map(|file| file.host_device_agent_ura)
        .unwrap_or_default();
    run_loop(&device_ura);
}
EOF
expect_fail \
  "local device URA aggregate fork" \
  "R44_LOCAL_DEVICE_URA_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/persistence/mod.rs" <<'EOF'
pub fn hosted_agent_authority_roots() -> anyhow::Result<Vec<String>> {
    let local = crate::daemon::persistence::local_agents::load()?;
    Ok(local
        .hosted_agents
        .into_iter()
        .map(|entry| entry.agent_ura)
        .collect())
}
EOF
expect_fail \
  "hosted authority roots aggregate fork" \
  "R45_HOSTED_AUTHORITY_ROOTS_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/agents/list.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateSnapshot;

fn list_agents_handler(
    registry_provider: &Arc<dyn Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync>,
) -> anyhow::Result<Value> {
    let snapshot = registry_provider()?;
    Ok(json!({ "agents": agent_rows(&snapshot)? }))
}

fn agent_rows(
    registry: &AgentRegistry,
    local_agents: &crate::daemon::persistence::local_agents::LocalAgentsFile,
) -> anyhow::Result<Vec<Value>> {
    registry
        .agents
        .iter()
        .map(|(name, _entry)| {
            let ura = crate::daemon::persistence::local_agents::lookup_hosted_ura(
                local_agents,
                "llm",
                name,
            );
            Ok(json!({ "name": name, "ura": ura }))
        })
        .collect()
}
EOF
expect_fail \
  "agent list aggregate row fork" \
  "R46_AGENT_LIST_AGGREGATE_ROW_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/dispatch.rs" <<'EOF'
enum DescriptorCallMode {
    Rpc,
    Stream,
    Bidi,
}

struct AxonAbilityCatalog {
    execution_index: ExecutionIndex,
}

impl AxonAbilityCatalog {
    fn runtime_ability_key_for_mode(&self, ability: &str, call_mode: DescriptorCallMode) -> Result<Option<String>, ()> {
        Ok(Some(ability.to_string()))
    }

    pub fn has_rpc(&self, ability: &str) -> bool {
        if let Some(runtime_key) = self
            .runtime_ability_key_for_mode(ability, DescriptorCallMode::Rpc)
            .ok()
            .flatten()
        {
            return self.runtime.ability_options(&runtime_key).is_some();
        }
        self.execution_index.has_rpc(ability)
    }

    pub fn has_stream(&self, ability: &str) -> bool {
        self.execution_index.has_stream(ability)
    }

    pub fn has_bidi(&self, ability: &str) -> bool {
        self.execution_index.has_bidi(ability)
    }

    pub fn list_rpc_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        self.execution_index.extend_rpc_names(&mut names);
        names
    }
}
EOF
expect_fail \
  "ability routeability catalog owner fork" \
  "R25B_ABILITY_ROUTEABILITY_CATALOG_OWNER_FORK"

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
cat >"$CLI/src/daemon/invocation/admission/identity_write_gate.rs" <<'EOF'
pub(crate) struct IdentityWriteGate {
    daemon_ura: Option<String>,
}

impl IdentityWriteGate {
    fn is_loopback(&self, caller_ura: &str) -> bool {
        caller_ura == LOCAL_SYSTEM_AGENT_URA
            || self.daemon_ura.as_deref().is_some_and(|daemon_ura| daemon_ura == caller_ura)
    }
}

struct AuthorizedIdentityWriteCaller {
    loopback: bool,
}
EOF
expect_fail \
  "identity write local self boundary regressed to loopback flag" \
  "R28_IDENTITY_WRITE_LOCAL_SELF_BOUNDARY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/descriptor_binding.rs" <<'EOF'
struct RuntimeBoundAbility {
    runtime_ability_ura: String,
    options: AbilityOptions,
}

impl RuntimeBoundAbility {
    pub(crate) async fn from_selected_route(
        surface: &'static str,
        runtime: &LocalRuntime,
        route: &SelectedInvokeRoute,
    ) -> Result<Self, Status> {
        let runtime_ability_ura = runtime_ability_ura(surface, &route.callee_ura, &route.ability_ura)?;
        let options = runtime.ability_options(&runtime_ability_ura).await.unwrap();
        Ok(Self { runtime_ability_ura, options })
    }

    pub(crate) fn descriptor_ref_for_mode(
        &self,
        surface: &'static str,
        callee_ura: &str,
        mode: CallMode,
        route_ura: Option<&str>,
    ) -> Result<DescriptorBoundAbilityRef, Status> {
        let proof_binding = self.options.proof_for_mode(mode);
        Ok(DescriptorBoundAbilityRef { descriptor_ref: proof_binding.descriptor_version.to_string() })
    }
}
EOF
expect_fail \
  "selected route descriptor proof owner fork" \
  "R29_SELECTED_ROUTE_DESCRIPTOR_PROOF_OWNER_FORK"

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
cat >"$CLI/sdk/go/access_control.go" <<'EOF'
func accessControlRevokeArgs(request AccessControlRevokeRequest) (AccessControlRevokeRequest, map[string]any, error) {
    ownerURA := strings.TrimSpace(request.OwnerURA)
    grantID := strings.TrimSpace(request.GrantID)
    args := map[string]any{"owner_ura": ownerURA, "grant_id": grantID}
    if actor := strings.TrimSpace(request.ActorURA); actor != "" {
        args["actor_ura"] = actor
    }
    return request, args, nil
}
EOF
expect_fail \
  "Go SDK access-control revoke actor URA fork" \
  "R18B_SDK_ACCESS_CONTROL_REVOKE_ACTOR_URA_FORK"

make_good_fixture
cat >"$CLI/sdk/python/easynet_sdk/access_control.py" <<'EOF'
def _revoke_args(request: AccessControlRevokeRequest) -> tuple[AccessControlRevokeRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    grant_id = _required_text(request.grant_id, "grant_id")
    args: dict[str, object] = {
        "owner_ura": owner_ura,
        "grant_id": grant_id,
    }
    _optional(args, "actor_ura", request.actor_ura)
    return request, args
EOF
expect_fail \
  "Python SDK access-control revoke actor URA fork" \
  "R18B_SDK_ACCESS_CONTROL_REVOKE_ACTOR_URA_FORK"

make_good_fixture
cat >"$CLI/src/cli/commands/groups/principal.rs" <<'EOF'
fn principal_command(
    actor_ura: Option<&str>,
    principal_ura: &str,
    idempotency_key: &str,
    expected_version: Option<u64>,
    proof_kind: ProofKindArg,
    proof_ref: &str,
) -> Value {
    let actor = actor_ura
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| principal_ura.trim());
    json!({
        "actor_ura": actor,
        "idempotency_key": idempotency_key,
        "proof": {
            "kind": proof_kind.as_wire(),
            "reference": proof_ref.trim(),
        }
    })
}
EOF
expect_fail \
  "principal command actor fallback fork" \
  "R33_PRINCIPAL_COMMAND_ACTOR_FALLBACK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/routing/hub_resolver.rs" <<'EOF'
pub enum HubResolution {
    Static { hub_endpoint: String },
    DirectoryFallback {
        hub_endpoint: String,
        target_ura: String,
    },
    Offline,
}

pub struct HubResolver<'a> {
    static_peers: &'a SharedFederatedPeers,
    federated_directory: &'a SharedFederatedDirectoryView,
    allow_directory_fallback: bool,
}

impl<'a> HubResolver<'a> {
    pub fn new(
        static_peers: &'a SharedFederatedPeers,
        federated_directory: &'a SharedFederatedDirectoryView,
        allow_directory_fallback: bool,
    ) -> Self {
        Self {
            static_peers,
            federated_directory,
            allow_directory_fallback,
        }
    }

    pub fn resolve(&self, target_realm: &str, target_ura: &str) -> HubResolution {
        if let Some(endpoint) = lookup_in_federated_view(self.federated_directory, target_ura)
            .and_then(|entry| entry.hub_endpoint)
        {
            return HubResolution::DirectoryFallback {
                hub_endpoint: endpoint,
                target_ura: target_ura.to_string(),
            };
        }

        let peers_snapshot = self.static_peers.snapshot();
        if let Some(ura) = peers_snapshot.get(target_realm) {
            return HubResolution::Static {
                hub_endpoint: ura.clone(),
            };
        }

        HubResolution::Offline
    }
}
EOF
expect_fail \
  "hub resolver route authority fork" \
  "R34_HUB_RESOLVER_ROUTE_AUTHORITY_FORK"

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
cat >"$CLI/docs/spec/ffi-abi-v7.md" <<'EOF'
# EasyNet Generic C ABI v7

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
cat >"$CLI/sdk/go/stream.go" <<'EOF'
func (s *StreamHandle) Cancel(ctx context.Context, reason string) (StreamCancel, error) {
    cancel := decodeCancel()
    s.state = StreamCancelled
    return cancel, nil
}
EOF
cat >"$CLI/sdk/go/bidi.go" <<'EOF'
func (s *BidiSession) Cancel(ctx context.Context, reason string) (BidiOutcome, error) {
    outcome := decodeOutcome()
    s.state = BidiCancelled
    return outcome, nil
}
EOF
cat >"$CLI/sdk/python/easynet_sdk/stream.py" <<'EOF'
def cancel(self, reason: str) -> StreamCancel:
    outcome = StreamCancel.from_json(raw)
    self.state = StreamState.CANCELLED
    return outcome
EOF
cat >"$CLI/sdk/python/easynet_sdk/bidi.py" <<'EOF'
def cancel(self, reason: str) -> BidiOutcome:
    outcome = BidiOutcome.from_json(raw)
    self.state = BidiState.CANCELLED
    return outcome
EOF
cat >"$CLI/sdk/go/stream_test.go" <<'EOF'
func TestStreamHandleCancelsNonTerminalStream(t *testing.T) {}
EOF
cat >"$CLI/sdk/go/bidi_test.go" <<'EOF'
func TestBidiCancelIsTerminal(t *testing.T) {}
EOF
cat >"$CLI/sdk/python/tests/test_stream.py" <<'EOF'
def test_stream_cancels_non_terminal_stream() -> None:
    pass
EOF
cat >"$CLI/sdk/python/tests/test_bidi.py" <<'EOF'
def test_cancel_is_terminal() -> None:
    pass
EOF
expect_fail \
  "direct SDK stream bidi cancel terminal authority fork" \
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
        let response = self.inner.invoke(signed_cancel).await?;
        Ok(InvocationHandle::from_response(response))
    }
}
EOF
expect_fail \
  "runtime client default cancellation signer fallback" \
  "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK"

make_good_fixture
cat >>"$CLI/src/daemon/invocation/dispatch/cancellation.rs" <<'EOF'
impl InvocationCancellationRegistry {
    pub fn register(
        &self,
        envelope: &DescriptorBoundEnvelope,
        handle: InvocationHandle,
    ) -> Result<String> {
        todo!()
    }
}
EOF
expect_fail \
  "raw lifecycle registry mutation re-exposed" \
  "R21_UNARY_CANCEL_SIGNED_COMMAND_FORK"

make_good_fixture
cat >>"$CLI/sdk/go/cabi_runtime.go" <<'EOF'
func cabiCallbackBackpressureFailure() []byte {
    return []byte(`{"terminal":true,"transport_terminal":true}`)
}
EOF
expect_fail \
  "Go callback overflow claims canonical terminality" \
  "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK"

make_good_fixture
cat >>"$CLI/sdk/python/easynet_sdk/_cabi.py" <<'EOF'
def _callback_backpressure_failure() -> bytes:
    return _json_bytes({"terminal": True, "transport_terminal": True})
EOF
expect_fail \
  "Python callback overflow claims canonical terminality" \
  "R20_STREAM_BIDI_CANCEL_TERMINAL_AUTHORITY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/bidi/session_initiator/prelude.rs" <<'EOF'
fn run_hosted_agent_advertise_prelude() -> anyhow::Result<()> {
    let local_agents = crate::daemon::persistence::local_agents::load()?;
    let entries = collect_advertise_entries(&realm, &user_segment, &local_agents);
    Ok(())
}

fn collect_advertise_entries(
    realm: &str,
    user_segment: &str,
    local_agents_file: &crate::daemon::persistence::local_agents::LocalAgentsFile,
) -> Vec<String> {
    local_agents_file
        .hosted_agents
        .iter()
        .map(|entry| entry.agent_ura.clone())
        .collect()
}
EOF
expect_fail \
  "hosted advertise prelude aggregate fork" \
  "R47_HOSTED_ADVERTISE_PRELUDE_AGENT_AGGREGATE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/resources/skills/list.rs" <<'EOF'
fn handle(args: Value) -> anyhow::Result<Value> {
    let registry = crate::daemon::persistence::agent_registry::load_agents()?;
    let local_agents = crate::daemon::persistence::local_agents::load().ok();
    let scope = SkillListScope::from_args(&args, local_agents.as_ref())?;
    let hosted_agent_index = HostedAgentUraIndex::from_local_agents(local_agents.as_ref());
    Ok(json!({ "items": [] }))
}

struct HostedAgentUraIndex;

impl HostedAgentUraIndex {
    fn from_local_agents(
        local_agents: Option<&crate::daemon::persistence::local_agents::LocalAgentsFile>,
    ) -> Self {
        if let Some(local_agents) = local_agents {
            let _rows = local_agents
                .hosted_agents
                .iter()
                .map(|entry| (entry.name.clone(), entry.agent_ura.clone()));
        }
        Self
    }
}
EOF
expect_fail \
  "skill list aggregate identity fork" \
  "R48_SKILL_LIST_AGENT_AGGREGATE_IDENTITY_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/resources/skills/publish.rs" <<'EOF'
fn resolve_owner_root_and_type(owner_id: &str) -> anyhow::Result<(PathBuf, agents::AgentType)> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(owner_id).ok_or_else(|| {
        anyhow::anyhow!(
            "owner_agent_id {owner_id:?} is not registered (registered agents: {:?})",
            registry.agents.keys().collect::<Vec<_>>()
        )
    })?;
    Ok((entry.required_root_path(owner_id, "skill.publish")?, entry.agent_type))
}
EOF
expect_fail \
  "skill publish aggregate owner fork" \
  "R49_SKILL_PUBLISH_AGENT_AGGREGATE_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/resources/skills/store.rs" <<'EOF'
fn install_skill(agent: &str) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let _entry = registry.agents.get(agent).ok_or_else(|| anyhow::anyhow!("missing"))?;
    Ok(())
}
EOF
expect_fail \
  "shared skill store aggregate owner fork" \
  "R49_SKILL_PUBLISH_AGENT_AGGREGATE_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/build.rs" <<'EOF'
fn build_registry_with_services_result_inner() {
    discover_ability::register_device_aggregate_with_resolver(
        &mut reg,
        || {
            crate::daemon::persistence::agent_registry::load_agents()
                .map_err(|error| anyhow::anyhow!("load discover agent registry: {error:#}"))
        },
        Arc::clone(&local_registry_handle),
        Arc::clone(&discover_federation_resolver),
    );
    a2a_bridge_ability::register(
        &mut reg,
        || {
            crate::daemon::persistence::agent_registry::load_agents()
                .map_err(|error| anyhow::anyhow!("load A2A agent registry: {error:#}"))
        },
        Arc::clone(&local_registry_handle),
    );
}
EOF
expect_fail \
  "boot discovery aggregate provider fork" \
  "R50_BOOT_DISCOVERY_AGENT_AGGREGATE_PROVIDER_FORK"

make_good_fixture
cat >"$CLI/src/cli/commands/start.rs" <<'EOF'
fn load_and_verify_credentials_with<F>(verify: F) -> anyhow::Result<(Credentials, bool)>
where
    F: Fn(&Credentials) -> CredentialCheck,
{
    let creds = config::load_credentials()?;
    match verify(&creds) {
        CredentialCheck::Valid => Ok((creds, true)),
        CredentialCheck::NetworkUnavailable => anyhow::bail!("hub credential verification unavailable"),
        CredentialCheck::Revoked(msg) => anyhow::bail!(msg),
    }
}
EOF
expect_fail \
  "daemon native join credential verifier fork" \
  "R51_DAEMON_NATIVE_JOIN_CREDENTIAL_VERIFICATION_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/device_control/ability_management/publish.rs" <<'EOF'
fn resolve_owner_root(owner_id: &str) -> anyhow::Result<PathBuf> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(owner_id).ok_or_else(|| anyhow::anyhow!("missing"))?;
    entry.required_root_path(owner_id, "ability.publish")
}
EOF
expect_fail \
  "ability publish aggregate workspace fork" \
  "R52_ABILITY_PUBLISH_AGENT_AGGREGATE_WORKSPACE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/agents/authoring.rs" <<'EOF'
fn put_agent_abilities_handler(name: &str) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(name).ok_or_else(|| anyhow::anyhow!("missing"))?;
    sync(entry)
}
EOF
expect_fail \
  "agent ability authoring registry-only workspace fork" \
  "R53_AGENT_ABILITY_AUTHORING_REGISTRY_ONLY_WORKSPACE_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/profiles/bootstrap.rs" <<'EOF'
fn build_plan_from_registry() -> anyhow::Result<()> {
    crate::daemon::persistence::agent_registry::load_agents()?;
    Ok(())
}
EOF
expect_fail \
  "bootstrap registry projection read owner fork" \
  "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/automation/think.rs" <<'EOF'
fn collect_owner_catalog() -> anyhow::Result<()> {
    crate::daemon::persistence::agent_registry::load_agents()?;
    Ok(())
}
EOF
expect_fail \
  "curator registry projection read owner fork" \
  "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/build.rs" <<'EOF'
fn build_registry_for_daemon_result() -> anyhow::Result<()> {
    crate::daemon::persistence::agent_registry::load_agents()?;
    crate::daemon::persistence::agent_registry::load_agents()?;
    Ok(())
}
EOF
expect_fail \
  "daemon catalog registry projection read owner fork" \
  "R54_AGENT_REGISTRY_PROJECTION_READ_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/governance/teach.rs" <<'EOF'
fn resolve_owner_manifest() -> anyhow::Result<()> {
    crate::daemon::persistence::agent_registry::load_agents()?;
    Ok(())
}

fn recover_forget_transactions() -> anyhow::Result<()> {
    crate::daemon::persistence::agent_registry::load_agents()?;
    Ok(())
}
EOF
expect_fail \
  "teach registry projection read owner fork" \
  "R55_TEACH_REGISTRY_PROJECTION_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/governance/teach.rs" <<'EOF'
fn forget() -> anyhow::Result<()> {
    let snapshot = AgentAggregateRepository::load_snapshot()?;
    let agent = snapshot.registry.agents.get("apprentice");
    Ok(())
}
EOF
expect_fail \
  "teach forget raw runtime registry projection fork" \
  "R55_TEACH_REGISTRY_PROJECTION_OWNER_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/boot/kernel"
cat >"$CLI/src/daemon/boot/kernel/mod.rs" <<'EOF'
fn dispatch(handle: Handle) -> TerminalState {
    SystemInvocationIssuer::request_for_complete_envelope();
    let state = handle.wait().await;
    let terminal = events.iter().rev().find(|e| e.state.is_terminal());
    KernelDispatchTerminal::Failed(format!("{state:?}"))
}
EOF
expect_fail \
  "kernel canonical terminal projection fork" \
  "R56_KERNEL_CANONICAL_TERMINAL_PROJECTION_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/dispatch"
cat >"$CLI/src/daemon/invocation/dispatch/local_runtime_invoker.rs" <<'EOF'
async fn rpc_value_from_handle(handle: InvocationHandle) -> Value {
    let state = handle.wait().await;
    let terminal = events.iter().rev().find(|event| event.state.is_terminal());
    project(state, terminal)
}
EOF
expect_fail \
  "local runtime RPC canonical terminal projection fork" \
  "R57_LOCAL_RUNTIME_RPC_CANONICAL_TERMINAL_PROJECTION_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/receipts" "$CLI/src/daemon/execution/loop_instance"
cat >"$CLI/src/daemon/invocation/receipts/runtime_record.rs" <<'EOF'
pub enum TerminalState {
    Succeeded,
    Failed { reason: String },
    Cancelled,
}

fn project(state: InvocationState) -> TerminalState {
    match state {
        InvocationState::TimedOut => Self::Failed { reason: "timeout".to_string() },
        _ => Self::Succeeded,
    }
}
EOF
cat >"$CLI/src/daemon/execution/loop_instance/mod.rs" <<'EOF'
fn consume(state: TerminalState) {
    match state {
        TerminalState::Succeeded => {}
        TerminalState::Failed { reason } => fail(reason),
        TerminalState::Cancelled => cancel(),
    }
}
EOF
expect_fail \
  "terminal timeout projection fork" \
  "R58_TERMINAL_TIMEOUT_PROJECTION_FORK"

make_good_fixture
mkdir -p "$CLI/sdk/go" "$CLI/sdk/python/easynet_sdk"
cat >"$CLI/sdk/go/runtime_ability.go" <<'EOF'
func build(addressing Addressing) string {
    return addressing.OwnerAbilityDescriptorRef()
}
EOF
cat >"$CLI/sdk/python/easynet_sdk/runtime_ability.py" <<'EOF'
class RuntimeAbilityClient:
    def build(self):
        return self._addressing.owner_ability_descriptor_ref()

    def open_stream(self):
        return self.build()
EOF
expect_fail \
  "SDK runtime descriptor owner fork" \
  "R59_SDK_RUNTIME_DESCRIPTOR_OWNER_FORK"

make_good_fixture
mkdir -p "$CLI/sdk/python/easynet_sdk"
cat >"$CLI/sdk/python/easynet_sdk/runtime.py" <<'EOF'
class RuntimeClient:
    def resolve_descriptor_ref(self, call_mode="rpc"):
        return self._transport.resolve_descriptor_ref({"call_mode": call_mode})
EOF
expect_fail \
  "SDK descriptor call mode normalization fork" \
  "R60_SDK_DESCRIPTOR_CALL_MODE_NORMALIZATION_FORK"

make_good_fixture
mkdir -p "$CLI/sdk/go"
cat >"$CLI/sdk/go/direct_runtime.go" <<'EOF'
func (t *directRuntimeTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
    return directRuntimePrepare(ctx, t.addressing, draftJSON, optionsJSON)
}

func (t *directRuntimeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
    snapshot := t.storeDirectHandle("Completed", true, nil)
    return directRuntimeHandleSnapshotJSON(snapshot)
}
EOF
expect_fail \
  "direct runtime handle owner fork" \
  "R61_DIRECT_RUNTIME_HANDLE_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/bidi/session_wire.rs" <<'EOF'
enum SessionDispatch {
    BidiOpen {
        call_id: u64,
    },
    Result {
        call_id: u64,
        terminal: bool,
    },
}
EOF
expect_fail \
  "JSON session invocation carrier fork" \
  "R64_SESSION_CANONICAL_CARRIER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/invocation/dispatch/local_session_dispatcher.rs" <<'EOF'
fn carrier_v1_control_failure(call_id: u64) -> DispatchResult {
    DispatchResult {
        call_id,
        terminal: true,
        ..Default::default()
    }
}

async fn send_bidi_terminal(
    outbound: &SessionUpSender,
    call_id: u64,
    failure: Error,
) {
    outbound.send(DispatchResult {
        call_id,
        terminal: true,
        failure: Some(failure),
        ..Default::default()
    });
}

fn callee_ura_from_envelope(envelope: &Envelope) -> anyhow::Result<String> {
    crate::daemon::invocation::dispatch::invocation_wire::callee_ura_from_envelope(envelope)
}
EOF
expect_fail \
  "receiptless session terminal fork" \
  "R64_SESSION_CANONICAL_CARRIER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/runtime_factory.rs" <<'EOF'
fn ledger_invocation_ura() -> String {
    crate::core::ura::invocation_history_resource_ura(
        "_system",
        "authority.invocations",
        "inv_123",
    )
}

fn ledger_route_ura() {
    panic!("LedgerSink cannot derive ability URA from binding callee=`y` caller=`z` ability=`a`")
}
EOF
expect_fail \
  "LedgerSink invocation system fallback" \
  "R65_LEDGER_SINK_SYSTEM_FALLBACK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/runtime_factory.rs" <<'EOF'
fn ledger_invocation_ura() {
    panic!("LedgerSink cannot derive invocation record URA from binding subject=`x` callee=`y` caller=`z` invocation_id=`i`")
}

fn ledger_route_ura(ability_name: &str) -> String {
    crate::core::ura::hub_ability_ura("_system", &format!("system.{ability_name}"))
}
EOF
expect_fail \
  "LedgerSink route system fallback" \
  "R65_LEDGER_SINK_SYSTEM_FALLBACK"

make_good_fixture
mkdir -p "$CLI/src/daemon/trust"
cat >"$CLI/src/daemon/trust/anchor.rs" <<'EOF'
use std::collections::HashMap;

struct TrustedAgent {
    public_key_b64: String,
}

struct RealmTrustAnchor {
    by_ura: HashMap<String, TrustedAgent>,
    users: HashMap<String, Vec<TrustedAgent>>,
}

impl RealmTrustAnchor {
    pub fn lookup(&self, agent_ura: &str) -> Option<&TrustedAgent> {
        if let Some(entry) = self.by_ura.get(agent_ura) {
            return Some(entry);
        }
        self.users
            .get(agent_ura)
            .and_then(|bucket| bucket.iter().next())
    }
}
EOF
expect_fail \
  "trust anchor user bucket lookup fork" \
  "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK"

make_good_fixture
mkdir -p "$CLI/src/daemon/trust"
cat >"$CLI/src/daemon/trust/key_resolver.rs" <<'EOF'
struct RealmTrustAnchorKeyResolver;
struct VerifyingKey;
struct AxonError;

impl RealmTrustAnchorKeyResolver {
    fn resolve_all(&self, agent_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
        let user_rows = vec![];
        let keys: Vec<VerifyingKey> = user_rows
            .iter()
            .filter_map(|row| decode_pubkey(&row.public_key_b64, agent_ura).ok())
            .collect();
        Ok(keys)
    }
}

fn decode_pubkey(public_key_b64: &str, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
    todo!()
}
EOF
expect_fail \
  "trust key resolver corrupt user key skip" \
  "R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK"

make_good_fixture
cat >"$CLI/src/daemon/ability/builtins/governance/api_key.rs" <<'EOF'
struct ApiKeyStore;

impl Default for ApiKeyStore {
    fn default() -> Self {
        Self
    }
}

fn load_store() -> ApiKeyStore {
    let text = std::fs::read_to_string("api_keys.toml").unwrap_or_default();
    toml::from_str(&text).unwrap_or_default()
}
EOF
expect_fail \
  "api key store parse fallback" \
  "R68_API_KEY_STORE_PARSE_FALLBACK"

make_good_fixture
cat >"$CLI/src/daemon/persistence/context_store.rs" <<'EOF'
struct ClipEntry;
struct ClipListEntry;

fn clipboard_log_path() -> std::path::PathBuf {
    "clipboard.jsonl".into()
}

fn list_clips(limit: usize) -> Vec<ClipEntry> {
    let Ok(content) = std::fs::read_to_string(clipboard_log_path()) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn list_clip_summaries(limit: usize) -> Vec<ClipListEntry> {
    Vec::new()
}

fn remove_clip(id: &str) -> anyhow::Result<ClipEntry> {
    let content = std::fs::read_to_string(clipboard_log_path()).unwrap_or_default();
    anyhow::bail!("context clipboard: no clip {id}")
}
EOF
expect_fail \
  "context clipboard history fallback" \
  "R69_CONTEXT_CLIPBOARD_HISTORY_FALLBACK"

make_good_fixture
mkdir -p "$CLI/sdk/node"
cat >"$CLI/sdk/node/index.js" <<'EOF'
const ERROR_CODES = new Set([
  "INVALID_ARGUMENT",
  "AUTHORITY_DENIED",
]);

export class InvocationDraft {
  constructor(fields) {
    this.metadata = fields.metadata ?? {};
    validateAuthorityMetadata(this.metadata);
  }
}

export class SessionAuthority {
  static fromMetadata(value) {
    return value;
  }
}

function validateAuthorityMetadata(metadata) {
  const value = metadata ?? {};
  const session = authorityMetadataValue(value, "x-easynet-session-authority");
  if (session) {
    return;
  }
}

function authorityMetadataValue(metadata, key) {
  return metadata[key] ?? "";
}
EOF
expect_fail \
  "node authority binding preflight fallback" \
  "R70_NODE_AUTHORITY_BINDING_PREFLIGHT"

make_good_fixture
mkdir -p "$CLI/sdk/node/test"
cat >"$CLI/sdk/node/test/types.test.ts" <<'EOF'
import {
  AuthorityMetadata,
  InvocationBuilder,
} from "../index.js";

// @ts-expect-error Product profiles are not part of the generic runtime SDK.
import { AdminClient } from "../index.js";

new InvocationBuilder()
  .withAuthorityMetadata(new AuthorityMetadata({
    kind: "delegation",
    key: "x-easynet-delegation",
    value: "opaque-authority",
  }));

void AdminClient;
EOF
expect_fail \
  "node product-neutral types test fallback" \
  "R71_NODE_PRODUCT_NEUTRAL_TYPES_TEST"

make_good_fixture
mkdir -p "$CLI/src/daemon/plugins/sidecar"
cat >"$CLI/src/daemon/plugins/sidecar/io.rs" <<'EOF'
use std::io::{BufReader, Read};
use std::process::ChildStderr;

pub(super) fn spawn_stderr_reader(stderr: ChildStderr) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut stderr = String::new();
        let _ = reader.read_to_string(&mut stderr);
        stderr
    })
}

pub(super) fn collect_stderr(handle: Option<std::thread::JoinHandle<String>>) -> String {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}
EOF
expect_fail \
  "plugin sidecar stderr diagnostic fallback" \
  "R72_PLUGIN_SIDECAR_STDERR_DIAGNOSTICS"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/admission"
cat >"$CLI/src/daemon/invocation/admission/device_trust_sync.rs" <<'EOF'
struct ResolvedCallerTrust {
    public_keys_b64: Vec<String>,
}

fn parse_resolved_caller_trust(result_bytes: &[u8]) -> anyhow::Result<ResolvedCallerTrust> {
    let response: serde_json::Value = serde_json::from_slice(result_bytes)?;
    let mut keys: Vec<String> = response
        .get("public_keys_b64")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if keys.is_empty() {
        if let Some(pk) = response.get("public_key_b64").and_then(|v| v.as_str()) {
            keys.push(pk.to_string());
        }
    }
    Ok(ResolvedCallerTrust {
        public_keys_b64: keys,
    })
}
EOF
expect_fail \
  "device trust sync resolve_key legacy response repair" \
  "R73_DEVICE_TRUST_SYNC_RESOLVE_KEY_SCHEMA"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/bidi/session_initiator"
cat >"$CLI/src/daemon/invocation/bidi/session_initiator/prelude.rs" <<'EOF'
fn resolved_public_keys(result: &[u8]) -> Vec<String> {
    let parsed = serde_json::from_slice::<serde_json::Value>(result).ok();
    let mut pubkeys: Vec<String> = parsed
        .as_ref()
        .and_then(|v| v.get("public_keys_b64"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| {
                    let key = k.as_str()?.trim();
                    (!key.is_empty()).then(|| key.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    if pubkeys.is_empty() {
        if let Some(pk) = parsed
            .as_ref()
            .and_then(|v| v.get("public_key_b64"))
            .and_then(|pk| pk.as_str())
        {
            pubkeys.push(pk.to_string());
        }
    }
    pubkeys
}

async fn sync_paired_user_trust_prelude() -> anyhow::Result<()> {
    let args = serde_json::to_vec(&serde_json::json!({ "agent_ura": user_ura }))?;
    let response = invoke_prelude_unary(args).await?;
    let pubkeys = resolved_public_keys(&response.result);
    Ok(())
}
EOF
expect_fail \
  "session prelude resolve_key legacy response repair" \
  "R93_SESSION_PRELUDE_RESOLVE_KEY_SCHEMA"

make_good_fixture
mkdir -p "$CLI/src/daemon/resources/pages"
cat >"$CLI/src/daemon/resources/pages/pages_serve_ability.rs" <<'EOF'
fn retired_module() {}
EOF
expect_fail \
  "pages serve pseudo ability module retired" \
  "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA"

make_good_fixture
cat >"$CLI/src/daemon/resources/pages/pages_http_projection.rs" <<'EOF'
use serde_json::Value;

struct ServedBytes {
    status: u16,
    bytes: Vec<u8>,
    content_type: String,
    force_attachment: bool,
    sha256: String,
}

fn bytes_from_value(value: Value) -> ServedBytes {
    use base64::Engine;
    let b64 = value.get("bytes_b64").and_then(Value::as_str).unwrap_or("");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap_or_default();
    let content_type = value
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_string();
    let force_attachment = value
        .get("force_attachment")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sha256 = value
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    ServedBytes {
        status: 200,
        bytes,
        content_type,
        force_attachment,
        sha256,
    }
}
EOF
expect_fail \
  "pages serve fetch projection fallback" \
  "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA"

make_good_fixture
cat >"$CLI/src/daemon/resources/pages/pages_http_projection.rs" <<'EOF'
fn projection() {
    let _ = "canonical_invoke";
}

fn bytes_from_value(value: Value) -> anyhow::Result<ServedBytes> {
    let b64 = required_non_empty_string(&value, "bytes_b64")?;
    let content_type = required_non_empty_string(&value, "content_type")?;
    let sha256 = required_non_empty_string(&value, "sha256")?;
    if sha256 != actual_sha256 {}
}
EOF
expect_fail \
  "pages http projection rejects invocation vocabulary" \
  "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA"

make_good_fixture
cat >"$CLI/src/daemon/resources/pages/pages_listener.rs" <<'EOF'
use super::pages_serve_ability::{serve_bytes, ServedBytes};
EOF
expect_fail \
  "pages listener rejects retired pseudo ability import" \
  "R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA"

make_good_fixture
cat >"$CLI/src/daemon/invocation/admission/authority_metadata.rs" <<'EOF'
use std::time::{SystemTime, UNIX_EPOCH};

fn project_admitted_session_authority() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
EOF
expect_fail \
  "authority metadata clock fallback" \
  "R78_AUTHORITY_METADATA_CLOCK_FAIL_CLOSED"

make_good_fixture
cat >"$CLI/src/daemon/invocation/admission/authority_metadata.rs" <<'EOF'
const REASON_AUTHORITY_FORMAT_INVALID: &str = "AUTHORITY_FORMAT_INVALID";

struct DelegationPayload {
    issuer_ura: String,
    subject_ura: String,
    caller_ura: String,
    audience: String,
    scopes: Vec<String>,
}

struct SessionAuthorityPayload {
    issuer_ura: String,
    session_id: String,
    session_owner_user_id: String,
    creator_principal_id: String,
    callee_ura: String,
    subject_ura: String,
    audience: String,
    scopes: Vec<String>,
    allowed_actions: Vec<String>,
    allowed_followup_abilities: Vec<String>,
}

fn validate_delegation_payload_shape(payload: &DelegationPayload) -> Result<(), Error> {
    if payload.issuer_ura.trim().is_empty()
        || payload.subject_ura.trim().is_empty()
        || payload.caller_ura.trim().is_empty()
        || payload.audience.trim().is_empty()
        || payload.scopes.is_empty()
    {
        return Err(Error);
    }
    Ok(())
}

fn validate_session_authority_payload_shape(payload: &SessionAuthorityPayload) -> Result<(), Error> {
    if payload.issuer_ura.trim().is_empty()
        || payload.session_id.trim().is_empty()
        || payload.session_owner_user_id.trim().is_empty()
        || payload.creator_principal_id.trim().is_empty()
        || payload.callee_ura.trim().is_empty()
        || payload.subject_ura.trim().is_empty()
        || payload.audience.trim().is_empty()
        || payload.scopes.is_empty()
        || payload.allowed_actions.is_empty()
        || payload.allowed_followup_abilities.is_empty()
    {
        return Err(Error);
    }
    Ok(())
}
EOF
expect_fail \
  "authority metadata all-zero principal fallback" \
  "R90_AUTHORITY_METADATA_REJECTS_ALL_ZERO_PRINCIPAL"

make_good_fixture
mkdir -p "$CLI/sdk/go" "$CLI/sdk/python/easynet_sdk"
cat >"$CLI/sdk/go/cabi_runtime.go" <<'EOF'
var _ = ErrDescriptorNotFound

type cabiStreamTransport struct{}
func (s *cabiStreamTransport) Cancel() []byte {
    return []byte(`{"state":"CancelRequested","terminal":false}`)
}

type cabiBidiTransport struct{}
func (b *cabiBidiTransport) Cancel() []byte {
    return []byte(`{"state":"CancelRequested","terminal":false}`)
}

func resolveDescriptorRefFromDiagnostics() error {
    _ = invalidRuntimePayload("call_mode is required for descriptor_ref resolution", nil)
    return &SDKError{
        Code:      ErrNotFound,
        Stage:     "cabi",
        Retry:     RetryNever,
        Retryable: RetryableForHint(RetryNever),
        Message:   "descriptor_ref not found",
    }
}
EOF
cat >"$CLI/sdk/python/easynet_sdk/_cabi.py" <<'EOF'
_EXPECTED_DESCRIPTOR_CODE = ErrorCode.DESCRIPTOR_NOT_FOUND

class _CABIStreamTransport:
    def cancel(self):
        return {"state": "CancelRequested", "terminal": False}

class _CABIBidiTransport:
    def cancel(self):
        return {"state": "CancelRequested", "terminal": False}

def _resolve_descriptor_ref_from_diagnostics():
    _required_string(request, "call_mode")
    raise SDKError(
        code=ErrorCode.NOT_FOUND,
        stage="cabi",
        retry=RetryHint.NEVER,
        message="descriptor_ref not found",
    )
EOF
expect_fail \
  "C ABI descriptor resolve generic not-found fallback" \
  "R91_CABI_DESCRIPTOR_RESOLVE_NOT_FOUND_TYPED"

make_good_fixture
mkdir -p "$CLI/src/ffi/invocation"
cat >"$CLI/src/ffi/invocation/mod.rs" <<'EOF'
fn runtime_system_descriptor_catalog_entries(owner_ura: &str) -> std::result::Result<Vec<serde_json::Value>, String> {
    Ok(catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .filter_map(|row| {
            row.descriptor
                .rebind_owner_ura(owner_ura)
                .ok()
                .and_then(descriptor_catalog_entry_from_descriptor)
        })
        .collect())
}

fn runtime_meta_descriptor_catalog_entries() -> std::result::Result<Vec<serde_json::Value>, String> {
    Ok(abilities
        .iter()
        .filter_map(descriptor_catalog_entry_from_value)
        .collect())
}

fn descriptor_catalog_entry_from_descriptor(
    descriptor: AbilityDescriptor,
) -> Option<serde_json::Value> {
    let ability_ura = descriptor.canonical_ability_ura()?;
    Some(serde_json::json!({"ability_ura": ability_ura}))
}

fn descriptor_catalog_entry_from_value(value: &serde_json::Value) -> Option<serde_json::Value> {
    let ability_ura = value.get("ability_ura")?.as_str()?.trim();
    let descriptor_hash = value.get("descriptor_hash")?.as_str()?.trim();
    if descriptor_hash.len() != 64 {
        return None;
    }
    Some(serde_json::json!({"ability_ura": ability_ura}))
}
EOF
expect_fail \
  "FFI descriptor catalog malformed provider rows skipped" \
  "R94_FFI_DESCRIPTOR_CATALOG_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/ffi/invocation"
cat >"$CLI/src/ffi/invocation/mod.rs" <<'EOF'
fn runtime_resolve_descriptor_ref_json(session: &ClientSession, request_json: &str) -> anyhow::Result<Value> {
    let object = serde_json::from_str::<Value>(request_json)?;
    let runtime_owner_ura = runtime_owner_ura_from_session(session).ok();
    let caller_ura = object
        .get("caller_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .map(Ok)
        .unwrap_or_else(|| {
            runtime_owner_ura
                .clone()
                .ok_or_else(|| "runtime owner URA is unavailable".to_string())
                .map_err(anyhow::Error::msg)
        })?;
    RemoteSystemInvocationIssuer::root_plan(&target, caller_ura, subject, args, timeout)?;
    Ok(Value::Null)
}
EOF
expect_fail \
  "FFI descriptor remote probe caller_ura defaulted from runtime owner" \
  "R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG"

make_good_fixture
mkdir -p "$CLI/sdk/go" "$CLI/sdk/python/easynet_sdk"
cat >"$CLI/sdk/go/authorized_runtime_session.go" <<'EOF'
type SessionHistoryOperations struct {
    session *AuthorizedRuntimeSession
}

func (o *SessionHistoryOperations) List(ctx context.Context, request ReceiptListRequest) (ReceiptHistoryPage, error) {
    if err := validateSessionHistoryRuntimeCall(request.Call); err != nil {
        return ReceiptHistoryPage{}, err
    }
    return o.session.receipts.List(ctx, request)
}

func validateSessionHistoryRuntimeCall(call RuntimeCallContext) error {
    return validateSessionHistoryAuthorityBinding(call.Authority, call)
}
EOF
cat >"$CLI/sdk/python/easynet_sdk/authorized_runtime_session.py" <<'EOF'
class SessionHistoryOperations:
    def __init__(self, session):
        self._session = session

    def list(self, request):
        _validate_session_history_call(request.call)
        return self._session._receipts.list(request)

def _validate_session_history_call(call):
    _validate_session_history_authority_binding(call.authority, call)
EOF
expect_fail \
  "SDK history filter tuple binding missing" \
  "R96_SDK_HISTORY_FILTER_TUPLE_BINDING"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/dispatch" "$CLI/src/daemon/boot/invocation"
cat >"$CLI/src/daemon/invocation/dispatch/attempt_audit.rs" <<'EOF'
struct InvocationAttemptLedger;
struct InvocationAttemptHandle;
struct InvocationAttemptRecord;

impl InvocationAttemptLedger {
    pub(crate) fn begin(&self) -> InvocationAttemptHandle {
        self.append(&InvocationAttemptRecord);
        InvocationAttemptHandle
    }

    fn append(&self, record: &InvocationAttemptRecord) {
        let Ok(_guard) = self.writer.lock() else {
            return;
        };
        if let Ok(line) = serde_json::to_string(record) {
            let _ = writeln!(file, "{line}");
        }
    }

    pub(crate) fn list_recent(&self) -> Vec<InvocationAttemptRecord> {
        let mut records = Vec::new();
        if let Ok(record) = serde_json::from_str::<InvocationAttemptRecord>(&line) {
            records.push(record);
        }
        records
    }
}

impl InvocationAttemptHandle {
    pub(crate) fn disabled() -> Self {
        InvocationAttemptHandle
    }
}
EOF
cat >"$CLI/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'EOF'
impl DaemonInvocationService {
    fn begin_invoke_attempt(&self) -> InvocationAttemptHandle {
        self.runtime
            .attempt_ledger
            .as_ref()
            .map(|ledger| ledger.begin())
            .unwrap_or_else(InvocationAttemptHandle::disabled)
    }
}
EOF
cat >"$CLI/src/daemon/boot/invocation/mod.rs" <<'EOF'
fn start_daemon_invocation_transport() {
    match InvocationAttemptLedger::open(path) {
        Ok(ledger) => service = service.with_invocation_attempt_ledger(ledger),
        Err(err) => {
            op_event!(kind = invocation_attempt_ledger_disabled, error = err);
        }
    }
}
EOF
expect_fail \
  "invocation attempt audit disabled compatibility" \
  "R92_INVOCATION_ATTEMPT_AUDIT_FAIL_CLOSED"

make_good_fixture
cat >"$CLI/src/daemon/identity/receipt_signing.rs" <<'EOF'
struct Provider;

impl Provider {
    async fn resolve(&self, caller_ura: &str) -> Result<Option<Authority>, Error> {
        let signer = self.signer_capabilities.get(caller_ura).unwrap();
        let caller = self
            .self_signed
            .get(caller_ura)
            .map(|authority| authority.callee_identity().clone())
            .or_else(|| strict_identity(caller_ura).ok())
            .ok_or_else(|| Error)?;
        Ok(Some(Authority { caller, signer }))
    }
}
EOF
expect_fail \
  "invocation signer raw key authority fallback" \
  "R79_INVOCATION_SIGNER_CUSTODY_AUTHORITY"

make_good_fixture
cat >"$CLI/src/daemon/identity/self_identity.rs" <<'EOF'
pub struct RuntimeSigningIdentity;

impl RuntimeSigningIdentity {
    pub fn load(
        owner_ura: impl Into<String>,
        provider: Arc<dyn SelfIdentity>,
    ) -> Result<Self, SelfIdentityError> {
        let owner_ura = owner_ura.into();
        let owner_ura = owner_ura.trim();
        if owner_ura.is_empty() {
            return Err(SelfIdentityError::InvalidOwner);
        }
        let public_key = provider.public_key(owner_ura)?;
        Ok(Self::from_public_projection(owner_ura, public_key, provider))
    }
}
EOF
expect_fail \
  "runtime owner signer accepts user ura fallback" \
  "R79_INVOCATION_SIGNER_CUSTODY_AUTHORITY"

make_good_fixture
cat >"$CLI/src/daemon/ability/catalog/build.rs" <<'EOF'
pub struct RegistryBuildConfig {
    pub authority_context: Option<AbilityAuthorityContext>,
}

pub struct RegistryDaemonBuildConfig {
    pub authority_context: Option<AbilityAuthorityContext>,
}

fn build_registry_with_services_result_inner(config: RegistryBuildConfig) {
    let RegistryBuildConfig { authority_context } = config;
    let authority_context = authority_context.unwrap_or_default();
    assemble(authority_context);
}

fn build_absent() -> RegistryBuildConfig {
    RegistryBuildConfig {
        authority_context: None,
    }
}
EOF
expect_fail \
  "catalog authority context optional fallback" \
  "R80_CATALOG_AUTHORITY_CONTEXT_REQUIRED"

make_good_fixture
mkdir -p "$CLI/src/daemon/ability/catalog" "$CLI/src/daemon/invocation/dispatch"
cat >"$CLI/src/daemon/ability/catalog/publication.rs" <<'EOF'
use serde_json::Value;

pub(crate) struct LocalAbilityPublicationSnapshot;

impl LocalAbilityPublicationSnapshot {
    #[must_use]
    pub(crate) fn owner_projection_values(&self, owner_ura: &str) -> Vec<Value> {
        self.descriptors_by_owner
            .get(owner_ura)
            .into_iter()
            .flatten()
            .filter_map(|descriptor| summary_from_descriptor(descriptor).ok())
            .filter_map(|summary| serde_json::to_value(summary).ok())
            .collect()
    }
}
EOF
cat >"$CLI/src/daemon/invocation/dispatch/federation_wrappers.rs" <<'EOF'
use serde_json::Value;

pub(crate) fn handle_resolve_at() -> ResolveResponse {
    ResolveResponse { agents: Vec::new() }
}

fn resolved_owner_projection_values() -> Vec<serde_json::Value> {
    let mut by_public_name = std::collections::BTreeMap::<String, serde_json::Value>::new();
    let mut order = Vec::new();
    let mut push = |summary: serde_json::Value| {
        let Some(key) = summary_from_value(&summary).and_then(|parsed| summary_public_name(&parsed)) else {
            return;
        };
        if by_public_name.insert(key.clone(), summary).is_none() {
            order.push(key);
        }
    };
    Vec::new()
}
EOF
expect_fail \
  "ability publication projection silent drop" \
  "R81_ABILITY_PUBLICATION_PROJECTION_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/boot/lifecycle" "$CLI/src/daemon/plugins/companion" "$CLI/src/daemon/plugins"
cat >"$CLI/src/daemon/boot/lifecycle/status.rs" <<'EOF'
use serde_json::Value;

pub struct RuntimeStatusReport {
    desktop_companions: Vec<Value>,
}

pub(super) fn desktop_companion_statuses() -> Vec<Value> {
    let Ok(state) = crate::daemon::plugins::default_state() else {
        return Vec::new();
    };
    let manager = crate::daemon::plugins::DesktopCompanionManager::current();
    state
        .index()
        .packages()
        .iter()
        .filter_map(|package| manager.status_json(package).ok())
        .collect()
}
EOF
cat >"$CLI/src/daemon/plugins/surface.rs" <<'EOF'
pub struct PluginPackageSurfaceRecord {
    pub companion: Option<serde_json::Value>,
}

pub struct PluginSurfaceProjector;

impl PluginSurfaceProjector {
    pub fn project_packages_with_daemon(index: &PluginPackageIndex) -> Vec<PluginPackageSurfaceRecord> {
        let companion_manager = DesktopCompanionManager::current();
        index
            .packages()
            .iter()
            .map(|package| PluginPackageSurfaceRecord {
                companion: companion_manager.status_json(package).ok(),
            })
            .collect()
    }
}
EOF
cat >"$CLI/src/daemon/plugins/companion/mod.rs" <<'EOF'
pub struct DesktopCompanionManager;

impl DesktopCompanionManager {
    pub fn status_json(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let status = self.status_for_package(package)?;
        serde_json::to_value(status)
            .ok()
            .and_then(|value| project_status(&value).ok())
            .ok_or_else(|| PluginHostError::InvalidCompanionManifest {
                id: package.id().as_str().to_string(),
                reason: "companion status projection failed".to_string(),
            })
    }
}
EOF
expect_fail \
  "desktop companion status projection silent drop" \
  "R82_DESKTOP_COMPANION_STATUS_PROJECTION_ERRORS"

make_good_fixture
mkdir -p "$CLI/src/daemon/ability/builtins/automation"
cat >"$CLI/src/daemon/ability/builtins/automation/think.rs" <<'EOF'
pub(crate) struct CatalogEntry;

fn run_think() -> serde_json::Value {
    let catalog = collect_owner_catalog("alice");
    serde_json::json!({
        "curator": {
            "attempted": true,
            "ok": true,
            "catalog_len": catalog.len(),
        }
    })
}

/// Catalog gathering is best-effort: an unreadable agent dir returns an empty
/// list, and validation downstream emits a clear "no catalog available".
pub(crate) fn collect_owner_catalog(owner: &str) -> Vec<CatalogEntry> {
    let registry = match crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_registered_agent_registry_projection() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let entry = match registry.agents.get(owner) {
        Some(e) => e.clone(),
        None => return Vec::new(),
    };
    Vec::new()
}
EOF
expect_fail \
  "curator catalog registry projection fallback" \
  "R83_CURATOR_CATALOG_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/schedule" "$CLI/src/bin"
cat >"$CLI/src/daemon/execution/schedule/mod.rs" <<'EOF'
pub struct DueFire;
pub struct ScheduleId;
pub struct ScheduleService {
    cache: Cache,
}

impl ScheduleService {
    pub fn due(&self) -> Vec<DueFire> {
        let cache = match self.cache.read() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        for entry in cache.values() {
            let _cron = match parse_cron(&entry.cron_expr) {
                Ok(cron) => cron,
                Err(_) => continue,
            };
        }
        Vec::new()
    }
}
EOF
cat >"$CLI/src/bin/easynet-daemon.rs" <<'EOF'
fn spawn_schedule_tick(schedule: ScheduleService) {
    let due = schedule.due();
    if due.is_empty() {
        return;
    }
}
EOF
expect_fail \
  "schedule due selection silent empty fallback" \
  "R84_SCHEDULE_DUE_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/schedule" "$CLI/src/bin" "$CLI/src/daemon/ability/builtins/automation"
cat >"$CLI/src/daemon/execution/schedule/mod.rs" <<'EOF'
pub struct DueFire;
pub struct ScheduleEntry;
pub struct ScheduleId;
pub struct ScheduleService {
    cache: Cache,
}

impl ScheduleService {
    pub fn list(&self) -> Vec<ScheduleEntry> {
        match self.cache.read() {
            Ok(g) => g.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn due(&self) -> Result<Vec<DueFire>> {
        let cache = self
            .cache
            .read()
            .map_err(|_| anyhow::anyhow!("schedule due cache lock poisoned"))?;
        for entry in cache.values() {
            let _cron = parse_cron(&entry.cron_expr)
                .map_err(|err| anyhow::anyhow!("schedule {} has invalid cron: {err:#}", entry.id))?;
        }
        Ok(Vec::new())
    }
}
EOF
cat >"$CLI/src/bin/easynet-daemon.rs" <<'EOF'
fn spawn_schedule_tick(schedule: ScheduleService) {
    let due = match schedule.due() {
        Ok(due) => due,
        Err(err) => {
            eprintln!("due selection failed: {err:#}");
            return;
        }
    };
    let schedules = match schedule.list() {
        Ok(schedules) => schedules,
        Err(err) => {
            eprintln!("schedule snapshot failed: {err:#}");
            return;
        }
    };
}
EOF
cat >"$CLI/src/daemon/ability/builtins/automation/schedule.rs" <<'EOF'
fn list_handler(svc: &ScheduleService, _args: Value) -> anyhow::Result<Value> {
    let entries = svc.list();
    let arr: Vec<Value> = entries
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "schedules": arr }))
}
EOF
expect_fail \
  "schedule list snapshot silent empty fallback" \
  "R84_SCHEDULE_DUE_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/schedule" "$CLI/src/bin" "$CLI/src/daemon/ability/builtins/automation" "$CLI/src/daemon/ability/builtins/resources/context/loaders"
cat >"$CLI/src/daemon/execution/schedule/mod.rs" <<'EOF'
pub struct DueFire;
pub struct ScheduleEntry;
pub struct ScheduleId;
pub struct ScheduleService {
    cache: Cache,
}

impl ScheduleService {
    pub fn list(&self) -> Result<Vec<ScheduleEntry>> {
        let cache = self
            .cache
            .read()
            .map_err(|_| anyhow::anyhow!("schedule list cache lock poisoned"))?;
        Ok(cache.values().cloned().collect())
    }

    pub fn next_fire_after(&self, id: &ScheduleId, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
        Ok(None)
    }

    pub fn next_fire_for_entry(entry: &ScheduleEntry, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
        let cron = parse_entry_cron(entry)?;
        Ok(cron.after(&now).next())
    }

    pub fn due(&self) -> Result<Vec<DueFire>> {
        let cache = self
            .cache
            .read()
            .map_err(|_| anyhow::anyhow!("schedule due cache lock poisoned"))?;
        for entry in cache.values() {
            let _cron = parse_entry_cron(entry)?;
        }
        Ok(Vec::new())
    }
}

fn parse_entry_cron(entry: &ScheduleEntry) -> Result<CronSchedule> {
    parse_cron(&entry.cron_expr)
        .map_err(|err| anyhow::anyhow!("schedule {} has invalid cron: {err:#}", entry.id))
}
EOF
cat >"$CLI/src/bin/easynet-daemon.rs" <<'EOF'
fn spawn_schedule_tick(schedule: ScheduleService) {
    let due = match schedule.due() {
        Ok(due) => due,
        Err(err) => {
            eprintln!("due selection failed: {err:#}");
            return;
        }
    };
    let schedules = match schedule.list() {
        Ok(schedules) => schedules,
        Err(err) => {
            eprintln!("schedule snapshot failed: {err:#}");
            return;
        }
    };
}
EOF
cat >"$CLI/src/daemon/ability/builtins/automation/schedule.rs" <<'EOF'
fn list_handler(svc: &ScheduleService, _args: Value) -> anyhow::Result<Value> {
    let entries = svc.list()?;
    let arr: Vec<Value> = entries
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?;
    Ok(json!({ "schedules": arr }))
}
EOF
cat >"$CLI/src/daemon/ability/builtins/resources/context/loaders/schedule.rs" <<'EOF'
impl ContextLoader for ScheduleLoader {
    fn load(&self, agent_name: &str, _session_id: &str) -> anyhow::Result<Option<String>> {
        let now = Utc::now();
        for entry in self.svc.list()? {
            let next = match self.svc.next_fire_after(&entry.id, now) {
                Ok(Some(t)) => t,
                Ok(None) | Err(_) => continue,
            };
        }
        Ok(None)
    }
}
EOF
expect_fail \
  "schedule context loader next-fire fallback" \
  "R84_SCHEDULE_DUE_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/session" "$CLI/src/daemon/ability/builtins/device_control" "$CLI/src/daemon/boot/kernel"
cat >"$CLI/src/daemon/execution/session/mod.rs" <<'EOF'
pub struct Session;
pub struct SessionId;
pub struct SessionService {
    sessions: Lock,
}

impl SessionService {
    pub fn list_active(&self) -> Vec<Session> {
        match self.sessions.read() {
            Ok(g) => g.values().map(|e| e.meta.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn get(&self, id: &SessionId) -> Option<Session> {
        self.sessions
            .read()
            .ok()
            .and_then(|g| g.get(id).map(|e| e.meta.clone()))
    }
}
EOF
cat >"$CLI/src/daemon/ability/builtins/device_control/session.rs" <<'EOF'
fn list_handler(svc: &SessionService, args: Value) -> anyhow::Result<Value> {
    let sessions = svc.list_active();
    let json_sessions: Vec<Value> = sessions
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "sessions": json_sessions }))
}

fn attach_handler(svc: &SessionService, args: Value) -> anyhow::Result<StreamSource> {
    let id = SessionId::new("live");
    if svc.get(&id).is_none() {
        return Ok(StreamSource::Snapshot(Vec::new()));
    }
    Ok(StreamSource::Snapshot(Vec::new()))
}
EOF
cat >"$CLI/src/daemon/boot/kernel/mod.rs" <<'EOF'
fn list_active_sessions(&self) -> anyhow::Result<Vec<Session>> {
    Ok(self.session.list_active())
}

fn get_session(&self, id: &SessionId) -> anyhow::Result<Option<Session>> {
    Ok(self.session.get(id))
}
EOF
expect_fail \
  "session index silent empty fallback" \
  "R85_SESSION_INDEX_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/mission/discuss" "$CLI/src/daemon/boot/kernel"
cat >"$CLI/src/daemon/execution/mission/discuss/mod.rs" <<'EOF'
pub struct DiscussRoom;
pub struct DiscussService {
    rooms: Lock,
}

impl DiscussService {
    pub fn list(&self) -> Vec<DiscussRoom> {
        match self.rooms.read() {
            Ok(g) => g.values().map(|s| s.meta.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }
}
EOF
cat >"$CLI/src/daemon/boot/kernel/mod.rs" <<'EOF'
fn list_discuss_rooms(&self) -> anyhow::Result<Vec<DiscussRoom>> {
    Ok((*self.discuss).list())
}
EOF
expect_fail \
  "discuss room registry silent empty fallback" \
  "R86_DISCUSS_ROOM_REGISTRY_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/loop_instance" "$CLI/src/daemon/ability/builtins/automation" "$CLI/src/daemon/boot/kernel"
cat >"$CLI/src/daemon/execution/loop_instance/mod.rs" <<'EOF'
pub struct LoopId;
pub struct LoopInstance {
    state: LoopState,
}
pub enum LoopState {
    Running,
    Cancelled,
}
pub struct LoopService {
    cache: Lock,
}

impl LoopService {
    pub fn status(&self, id: &LoopId) -> Option<LoopInstance> {
        self.cache.read().ok().and_then(|g| g.get(id).cloned())
    }

    pub fn list(&self) -> Vec<LoopInstance> {
        match self.cache.read() {
            Ok(g) => g.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn resume_inflight(&self) -> anyhow::Result<()> {
        for inst in self.list() {
            start(inst);
        }
        Ok(())
    }

    pub fn subscribe(&self, id: &LoopId) -> anyhow::Result<StreamSource> {
        let _inst = self
            .status(id)
            .ok_or_else(|| anyhow::anyhow!("loop not found"))?;
        Ok(StreamSource::Snapshot(Vec::new()))
    }
}

impl std::fmt::Debug for LoopService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.cache.read().ok().map(|g| g.len()).unwrap_or(0);
        write!(f, "LoopService {{ loops: {n} }}")
    }
}
EOF
cat >"$CLI/src/daemon/ability/builtins/automation/loop_ability.rs" <<'EOF'
fn status_handler(svc: &LoopService, args: Value) -> anyhow::Result<Value> {
    let id = "loop-1";
    match svc.status(&LoopId::new(id)) {
        Some(inst) => Ok(serde_json::to_value(inst)?),
        None => anyhow::bail!("loop.status: loop {id} not found"),
    }
}
EOF
cat >"$CLI/src/daemon/boot/kernel/mod.rs" <<'EOF'
fn loop_status(&self, id: &LoopId) -> anyhow::Result<Option<LoopInstance>> {
    Ok(self.loop_svc.status(id))
}
EOF
expect_fail \
  "loop cache silent empty unknown fallback" \
  "R87_LOOP_CACHE_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/ability/builtins/agents"
cat >"$CLI/src/daemon/ability/builtins/agents/chat.rs" <<'EOF'
fn invoke_direct_with_progress(agent_name: &str) -> anyhow::Result<Value> {
    let other_specs = enumerate_other_agent_specs(agent_name);
    let cross_agent_hint = format_cross_agent_hint(&other_specs);
    Ok(json!({"hint": cross_agent_hint}))
}

fn stream_handler(agent_name: &str) -> anyhow::Result<StreamSource> {
    let other_specs = enumerate_other_agent_specs(agent_name);
    let cross_agent_hint = format_cross_agent_hint(&other_specs);
    Ok(StreamSource::Snapshot(vec![json!({"hint": cross_agent_hint})]))
}

fn enumerate_other_agent_specs(self_agent_name: &str) -> Vec<AgentAbilitySpec> {
    let snapshot = match AgentAggregateRepository::load_snapshot() {
        Ok(snapshot) => snapshot,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (other_name, other_entry) in snapshot.registered_agents() {
        if other_name == self_agent_name {
            continue;
        }
        out.push(abilities_for(other_name, other_entry));
    }
    out
}
EOF
expect_fail \
  "chat cross-agent registry silent empty fallback" \
  "R88_CHAT_CROSS_AGENT_REGISTRY_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/execution/permission" "$CLI/src/daemon/ability/builtins/governance" "$CLI/src/daemon/boot/kernel"
cat >"$CLI/src/daemon/execution/permission/mod.rs" <<'EOF'
pub struct PermissionRequest;
pub struct SubscriberBroker {
    pending: Lock,
}

impl SubscriberBroker {
    pub fn pending_snapshot(&self) -> Vec<PermissionRequest> {
        self.pending
            .read()
            .ok()
            .map(|g| g.values().map(|p| p.request.clone()).collect())
            .unwrap_or_default()
    }
}

pub struct PermissionService {
    subscriber: Option<SubscriberBroker>,
}

impl PermissionService {
    pub fn pending(&self) -> Vec<PermissionRequest> {
        self.subscriber
            .as_ref()
            .map(|s| s.pending_snapshot())
            .unwrap_or_default()
    }
}
EOF
cat >"$CLI/src/daemon/ability/builtins/governance/consent.rs" <<'EOF'
fn subscribe_handler(svc: &PermissionService) -> anyhow::Result<StreamSource> {
    let snapshot: Vec<Value> = svc
        .pending()
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
    Ok(StreamSource::Snapshot(snapshot))
}

fn list_pending_handler(svc: &PermissionService) -> anyhow::Result<Value> {
    let snapshot: Vec<Value> = svc
        .pending()
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "requests": snapshot }))
}
EOF
cat >"$CLI/src/daemon/boot/kernel/mod.rs" <<'EOF'
fn pending_permission_requests(&self) -> anyhow::Result<Vec<PermissionRequest>> {
    Ok(self.permission.pending())
}
EOF
expect_fail \
  "permission pending queue silent empty fallback" \
  "R89_PERMISSION_PENDING_QUEUE_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/plugins" "$CLI/src/daemon/boot/invocation" "$CLI/src/daemon/ability/wire"
cat >"$CLI/src/daemon/plugins/runtime_manager.rs" <<'EOF'
use crate::daemon::ability::wire::AbilityWireRegistry;

pub struct PluginRuntimeManager;

impl PluginRuntimeManager {
    pub fn new() -> Self {
        let loaded: Result<(), String> = Err("broken package state".to_string());
        let _wire_registry = loaded
            .as_ref()
            .map(|_| AbilityWireRegistry::core())
            .unwrap_or_else(|_| AbilityWireRegistry::core());
        Self
    }
}
EOF
cat >"$CLI/src/daemon/boot/invocation/mod.rs" <<'EOF'
fn attach_wire_registry() {
    let _registry = match crate::daemon::ability::wire::AbilityWireRegistry::load_default_profile() {
        Ok(registry) => registry,
        Err(_err) => {
            let _kind = "ability_wire_registry_load_failed";
            let _message = "daemon will use core bidi wire profiles only";
            crate::daemon::ability::wire::AbilityWireRegistry::core()
        }
    };
}
EOF
cat >"$CLI/src/daemon/ability/wire/mod.rs" <<'EOF'
pub struct AbilityWireRegistry;

impl AbilityWireRegistry {
    pub fn load_default_profile() -> Result<Self, String> {
        Ok(Self)
    }

    pub fn bidi_wire_kind_for(&self, _ability: &str) -> Option<()> {
        None
    }
}

pub fn bidi_wire_kind_for(ability: &str) -> Option<()> {
    AbilityWireRegistry::load_default_profile()
        .ok()
        .and_then(|registry| registry.bidi_wire_kind_for(ability))
}
EOF
expect_fail \
  "plugin wire profile core-only fallback" \
  "R75_PLUGIN_WIRE_PROFILE_FAIL_CLOSED"

make_good_fixture
mkdir -p "$CLI/src/daemon/invocation/admission"
cat >"$CLI/src/daemon/invocation/admission/peer_envelope_signer.rs" <<'EOF'
use axon_sdk::pb::axon::v1::Envelope;

pub(crate) struct PeerInvokeRequest<'a> {
    caller_envelope: Option<&'a Envelope>,
}

pub(crate) fn build_peer_envelope(
    caller_envelope: Option<&Envelope>,
    target_ura: &str,
) -> Envelope {
    let mut forwarded = caller_envelope.cloned().unwrap_or_default();
    let subject_ura = caller_envelope
        .and_then(|env| env.caller.as_ref())
        .map(|caller| caller.ura.trim().to_string())
        .filter(|ura| !ura.is_empty())
        .unwrap_or_else(|| target_ura.trim().to_string());
    forwarded.subject = Some(axon_sdk::pb::axon::v1::SubjectIdentity {
        ura: subject_ura,
        profile: "ura-v1".to_string(),
    });
    forwarded
}
EOF
expect_fail \
  "peer envelope target subject fallback" \
  "R76_PEER_ENVELOPE_EXPLICIT_SUBJECT"

make_good_fixture
mkdir -p "$CLI/src/support/platform" "$CLI/src/daemon/invocation/routing"
cat >"$CLI/src/support/platform/local_invoke.rs" <<'EOF'
pub struct LocalBidiFrame;

pub fn project_invoke_bidi_down_frame(
    frame: axon_sdk::pb::axon::v1::InvokeBidiDown,
) -> anyhow::Result<Option<LocalBidiFrame>> {
    use axon_sdk::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    match frame.payload {
        Some(DownPayload::Receipt(receipt)) => {
            let _payload = serde_json::from_slice::<serde_json::Value>(&receipt.payload)
                .unwrap_or_else(|_| serde_json::json!({"data_b64": B64.encode(&receipt.payload)}));
            Ok(Some(LocalBidiFrame))
        }
        _ => Ok(None),
    }
}
EOF
cat >"$CLI/src/daemon/invocation/routing/remote_invoke.rs" <<'EOF'
fn drain(frame: axon_sdk::pb::axon::v1::InvokeBidiDown) {
    let _ = frame;
}
EOF
expect_fail \
  "bidi receipt payload data_b64 fallback" \
  "R77_BIDI_RECEIPT_PAYLOAD_PROJECTION"

make_good_fixture
cat >"$CLI/src/cli/commands/groups/device.rs" <<'EOF'
fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let creds = crate::daemon::persistence::config::load_credentials().ok();
    let local_node = creds
        .as_ref()
        .map(|c| c.node_id.clone())
        .unwrap_or_default();
    let local_tenant = creds.as_ref().map(|c| c.realm.clone()).unwrap_or_default();
    let local_ura = if !local_tenant.is_empty() && !local_node.is_empty() {
        crate::core::ura::device_ura(&local_tenant, &local_node)
    } else {
        String::new()
    };
    if !local_ura.is_empty() {
        invoke_revoke(args.node_id.as_str(), "reason", local_ura.as_str())?;
    }
    Ok(())
}

fn describe_target(node_id: &str) -> anyhow::Result<Value> {
    let creds = crate::daemon::persistence::config::load_credentials().ok();
    let local_node = creds
        .as_ref()
        .map(|c| c.node_id.clone())
        .unwrap_or_default();
    classify_device_show_target(node_id, &local_node)
}

fn classify_device_show_target(raw: &str, local_node_id: &str) -> anyhow::Result<DeviceShowTarget> {
    if raw == local_node_id {
        return Ok(DeviceShowTarget::Local);
    }
    Ok(DeviceShowTarget::RemoteDevice(raw.to_string()))
}
EOF
expect_fail \
  "device product ingress local identity fallback" \
  "R94_DEVICE_PRODUCT_INGRESS_REQUIRES_LOCAL_IDENTITY"

make_good_fixture
cat >"$CLI/src/daemon/resources/pages/pages_listener.rs" <<'EOF'
fn handle_api_body(body_bytes: bytes::Bytes) -> serde_json::Value {
    if body_bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null)
    }
}
EOF
expect_fail \
  "Pages API malformed JSON null fallback" \
  "R95_PAGES_API_BODY_FAIL_CLOSED"

make_good_fixture
cat >"$CLI/src/daemon/boot/lifecycle/projection.rs" <<'EOF'
use crate::daemon::persistence::config;

pub struct RuntimeProjectionStore;
pub struct RuntimeSessionProjection;

impl RuntimeProjectionStore {
    pub fn load(&self) -> Option<RuntimeSessionProjection> {
        RuntimeSessionProjection::load_current()
    }
}

impl RuntimeSessionProjection {
    pub fn from_state(_state: config::RuntimeState) -> Self {
        Self
    }

    pub fn load_current() -> Option<Self> {
        config::load().ok().map(Self::from_state)
    }
}
EOF
expect_fail \
  "runtime projection load failure hidden as missing projection" \
  "R96_RUNTIME_PROJECTION_LOAD_FAIL_CLOSED"

make_good_fixture
cat >"$CLI/src/cli/commands/reset.rs" <<'EOF'
fn run(args: ResetArgs) -> anyhow::Result<()> {
    let runtime_state = crate::daemon::persistence::config::load().ok();
    if !args.force {
        if let Some(ref state) = runtime_state {
            if state.pid.is_some() {
                anyhow::bail!("runtime is currently running");
            }
        }
    }
    if let Some(ref state) = runtime_state {
        if state.pid.is_none() {
            crate::daemon::persistence::config::remove().ok();
        }
    }
    crate::daemon::persistence::config::delete_credentials()?;
    Ok(())
}
EOF
expect_fail \
  "device reset runtime projection fallback" \
  "R97_RESET_RUNTIME_PROJECTION_FAIL_CLOSED"

make_good_fixture
cat >"$CLI/src/cli/commands/groups/mcp.rs" <<'EOF'
fn run_status() -> anyhow::Result<()> {
    let state = crate::daemon::persistence::config::load().ok();
    if state.is_none() {
        output::warn("runtime not running");
    }
    Ok(())
}
EOF
expect_fail \
  "MCP status runtime projection fallback" \
  "R98_MCP_STATUS_RUNTIME_PROJECTION_FAIL_CLOSED"

make_good_fixture
cat >"$CLI/src/cli/presentation/banner.rs" <<'EOF'
fn write_runtime_status() {
    let runtime_state = crate::daemon::persistence::config::load().ok();
    if runtime_state.is_none() {
        let _message = "not running";
    }
}
EOF
expect_fail \
  "banner runtime projection fallback" \
  "R99_BANNER_RUNTIME_PROJECTION_FAIL_CLOSED"

make_good_fixture
expect_pass "fixture restored after all negative cases"

printf 'test_check_architecture_convergence.sh: all cases passed\n'
