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
    "$CLI/src/eal/interpreter" \
    "$CLI/src/daemon/ability/builtins/automation" \
    "$CLI/src/daemon/ability/builtins/agents" \
    "$CLI/src/daemon/ability/builtins/device_control" \
    "$CLI/src/daemon/ability/builtins/device_control/ability_management" \
    "$CLI/src/daemon/ability/builtins/integrations" \
    "$CLI/src/daemon/execution/mission" \
    "$CLI/src/daemon/execution/mcp" \
    "$CLI/src/daemon/ability/builtins/governance" \
    "$CLI/src/daemon/ability/builtins/resources" \
    "$CLI/src/daemon/ability/builtins/resources/files_store" \
    "$CLI/src/daemon/ability/builtins/resources/skills" \
    "$CLI/src/daemon/ability/catalog/profiles" \
    "$CLI/src/daemon/axon_bridge" \
    "$CLI/src/daemon/boot/invocation" \
    "$CLI/src/daemon/identity" \
    "$CLI/src/daemon/invocation/admission" \
    "$CLI/src/daemon/invocation/bidi/session_initiator" \
    "$CLI/src/daemon/invocation/dispatch" \
    "$CLI/src/daemon/invocation/routing" \
    "$CLI/src/daemon/invocation/streams" \
    "$CLI/src/daemon/persistence" \
    "$CLI/src/daemon/resources/context" \
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
    AgentAggregateRepository::load_snapshot()
        .map(|snapshot| snapshot.registered_agent_registry_projection());
}

fn dispatch_step(gateway: &dyn MissionInvocationGateway, request: MissionInvocationRequest) {
    gateway.invoke_step(request);
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
  cat >"$CLI/src/cli/commands/abilities.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn local_agent_ura(agent: &str) -> anyhow::Result<String> {
    let snapshot = AgentAggregateRepository::load_hosted_identity_snapshot()?;
    snapshot.hosted_llm_agent_ura(agent).map(str::to_string).ok_or_else(|| anyhow::anyhow!("missing"))
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
fn carrier_v1_control_failure(call_id: u64) -> DispatchResult {
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

fn enumerate_other_agent_specs() {
    let snapshot = AgentAggregateRepository::load_snapshot().unwrap();
    snapshot.registered_agents();
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
    transaction.persist_registry_projection(&registry);
    transaction.persist_identity_projection(&identities);
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
    AgentAggregateRepository::load_hosted_identity_status()
        .map(|status| status.hosted_agent_count())
        .unwrap_or_default()
}
EOF
  cat >"$CLI/src/daemon/ability/builtins/governance/invocation_history.rs" <<'EOF'
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

fn ledger_resource_ura() -> Option<String> {
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_status().ok()?;
    hosted_identity.host_device_agent_ura().map(str::to_string)
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
        .map(|(name, _entry)| {
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

fn find_implicit_agent_fallback(ir: &MissionIr) -> anyhow::Result<Option<ImplicitAgentFallback>> {
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
        let peers_snapshot = self.static_peers.snapshot();
        if let Some(ura) = peers_snapshot.get(target_realm) {
            return HubResolution::Static {
                hub_endpoint: ura.clone(),
            };
        }

        if self.allow_directory_fallback {
            if let Some(endpoint) = lookup_in_federated_view(self.federated_directory, target_ura)
                .and_then(|entry| entry.hub_endpoint)
            {
                return HubResolution::DirectoryFallback {
                    hub_endpoint: endpoint,
                    target_ura: target_ura.to_string(),
                };
            }
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

fn resolve_delegation(peer_source: PeerSource, parsed_owner: ParsedOwner, selector: Selector) {
    let resolution = HubResolver::new(
        peer_source.federated_peers,
        peer_source.federated_directory,
        peer_source.allow_directory_auto_route,
    )
    .resolve(&parsed_owner.realm, &selector.owner_ura);
    let endpoint = match resolution {
        HubResolution::Static { hub_endpoint } => {
            DelegatedPeerEndpoint::new(hub_endpoint, "federated_peers", None)
        }
        HubResolution::DirectoryFallback {
            hub_endpoint,
            target_ura,
        } => DelegatedPeerEndpoint::new(
            hub_endpoint,
            "federated_directory",
            Some(target_ura.as_str()),
        ),
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
EOF
  mkdir -p "$CLI/src/daemon/ability/catalog"
  cat >"$CLI/src/daemon/ability/catalog/runtime_admin_contracts.rs" <<'EOF'
fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_control_plane_descriptor_with_owner(
        "session.open",
        &OwnerKind::Hub,
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
  cat >"$CLI/src/daemon/axon_bridge/dispatch_shim.rs" <<'EOF'
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

fn open_stream_local_with_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}

fn open_bidi_local_with_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}

pub async fn open_bidi_external_signed() {
    invoke_descriptor_bound_bidi_request_async(request).await;
}

fn dispatch_rpc_local_with_subject() {
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

struct PersistedHotAgentAuthority;

impl PersistedHotAgentAuthority {
    fn load(agent: &str) -> Result<(), HotAgentAuthorityInventoryError> {
        let snapshot = AgentAggregateRepository::try_load_snapshot()
            .map_err(|error| hot_agent_authority_snapshot_error(agent, error))?;
        if !snapshot.has_registered_agent(agent) {
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
    def resolve_descriptor_ref(self, call_mode="rpc"):
        call_mode = call_mode.strip() or "rpc"
        return self._transport.resolve_descriptor_ref({"call_mode": call_mode})

# Product words in comments and private symbols do not create public models.
class _MissionFixture:
    pass
EOF
  cat >"$CLI/docs/spec/ffi-abi-v5.md" <<'EOF'
# EasyNet Generic C ABI v5

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
EOF
  cat >"$CLI/src/daemon/ability/builtins/automation/think.rs" <<'EOF'
fn collect_owner_catalog() -> anyhow::Result<()> {
    AgentAggregateRepository::load_registered_agent_registry_projection()?;
    Ok(())
}
EOF
  cat >"$CLI/src/daemon/invocation/dispatch/invocation_wire.rs" <<'EOF'
pub(crate) struct LocalDaemonLoopbackInvocation {
    derivation_policy: InvocationDerivationPolicy,
}

impl LocalDaemonLoopbackInvocation {
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
EOF
  cat >"$CLI/src/support/platform/local_daemon_grpc.rs" <<'EOF'
use crate::daemon::invocation::dispatch::invocation_wire::LocalDaemonLoopbackInvocation;

fn invoke_with_hosted_agent_delegation(hosted_agent_ura: &str) -> anyhow::Result<()> {
    let _delegation = HostedAgentDelegationRequest::new(hosted_agent_ura)?;
    Ok(())
}

fn local_daemon_loopback_invocation_from_subject_policy() -> LocalDaemonLoopbackInvocation {
    LocalDaemonLoopbackInvocation::from_target()
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
struct LocalDaemonLoopbackInvocation {
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
}
EOF
expect_fail \
  "local loopback support request owner fork" \
  "R16B_LOCAL_LOOPBACK_INVOCATION_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/support/platform/local_daemon_grpc.rs" <<'EOF'
fn rebuild_loopback_envelope(caller: String, callee: String, subject: String) {
    let _envelope = crate::daemon::invocation::ProtoEnvelope::targeted(caller, callee, subject);
}
EOF
expect_fail \
  "local loopback support envelope owner fork" \
  "R16B_LOCAL_LOOPBACK_INVOCATION_OWNER_FORK"

make_good_fixture
cat >"$CLI/src/daemon/axon_bridge/dispatch_shim.rs" <<'EOF'
fn dispatch_rpc_local_with_subject() {
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
cat >"$CLI/src/daemon/axon_bridge/dispatch_shim.rs" <<'EOF'
pub fn local_system_from_wire_parts() {
    SystemInvocationIssuer::request_for_complete_envelope();
}

fn open_stream_local_with_subject() {
    SystemInvocationIssuer::request_for_descriptor_ref();
}
EOF
expect_fail \
  "public unsealed local-system wire ingress fork" \
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
  "R66_MISSION_MODEL_ENTRY_FALLBACK"

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
mkdir -p "$CLI/src/daemon/execution/mission"
cat >"$CLI/src/daemon/execution/mission/orchestration.rs" <<'EOF'
fn find_implicit_agent_fallback(ir: &MissionIr) -> anyhow::Result<Option<ImplicitAgentFallback>> {
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
func (t *DirectRuntimeTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
    return directRuntimePrepare(ctx, t.addressing, draftJSON, optionsJSON)
}

func (t *DirectRuntimeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
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
expect_pass "fixture restored after all negative cases"

printf 'test_check_architecture_convergence.sh: all cases passed\n'
