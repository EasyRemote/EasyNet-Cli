// EasyNet CLI — `meta.teach` / `meta.acquire` / `meta.forget`
// =============================================================
//
// File: src/runtime/agents/teach_ability.rs
// Description: GET route B (seven-axes T3.3, spec §2.5) — owner-granted
//              descriptor transfer. `meta.teach` records an explicit grant,
//              `meta.acquire` imports the granted declaration-only manifest,
//              and `meta.forget` removes that imported descriptor copy. This
//              module does NOT transfer executable code.
//
// v1 scope (spec D6 ruling): same-device, manifest-only. Acquire copies a
// declaration-only manifest into the learner agent's workspace so discover can
// project a learner-owned descriptor URA. It is intentionally not invokable.
//
// Execution posture: the grant default is `execution_mode =
// "sandbox_first"` (capability.proto:238). This surface does not interpret
// that value as permission to transfer executable code; acquire is
// deliberately fail-closed for any granted descriptor that carries `[exec]`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::core::ability_spec::{AbilityExec, AbilityManifest};
use crate::persistence::teach_grants::{
    AcquireStagedGrant, AcquiringArtifactRecoveryState, AcquiringArtifactTxn,
    DescriptorImportRecord, TeachGrant, TeachGrantAdmissionSnapshot,
    TeachGrantAdmissionSnapshotDraft, TeachGrantAuthoritySnapshot, TeachGrantDraft,
    TeachGrantStore, EXECUTION_MODE_DEFAULT,
};
use crate::runtime::ability::HostedAgentAuthority;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, EnvelopeContext, OwnerKind};
use crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell;
use crate::runtime::axon_bridge::hot_agent_registrar::{
    block_on_hot_registrar, HotAgentRuntimeSyncOutcome,
};
use crate::ura::AbilitySelector;

pub const TEACH: &str = "meta.teach";
pub const ACQUIRE: &str = "meta.acquire";
pub const FORGET: &str = "meta.forget";
const TRANSFER_KIND_DISCOVERY_ONLY_MANIFEST: &str = "discovery_only_manifest";
const INVOCATION_STATUS_NOT_INVOKABLE_WITHOUT_EXEC_BINDING: &str =
    "not_invokable_without_exec_binding";
const TEACH_MANIFEST_VERB: &str = "teach";
const ACQUIRE_MANIFEST_VERB: &str = "acquire";
const FORGET_MANIFEST_VERB: &str = "forget";
static IMPORT_STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

trait TeachClock {
    fn now_rfc3339(&self) -> String;
}

#[derive(Debug, Clone, Copy)]
struct SystemTeachClock;

impl TeachClock for SystemTeachClock {
    fn now_rfc3339(&self) -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

#[derive(Debug, Clone)]
struct StagedDescriptorImportManifest {
    temp: PathBuf,
    dest: PathBuf,
    content_hash: [u8; 32],
}

impl AcquiringArtifactTxn for StagedDescriptorImportManifest {
    fn committed_artifact_path(&self) -> String {
        self.dest.to_string_lossy().into_owned()
    }

    fn staging_artifact_path(&self) -> Option<String> {
        Some(self.temp.to_string_lossy().into_owned())
    }

    fn content_hash(&self) -> String {
        sha256_hex(self.content_hash)
    }

    fn commit(&self) -> anyhow::Result<()> {
        commit_descriptor_import_manifest(self)
    }

    fn rollback(&self) -> anyhow::Result<()> {
        rollback_descriptor_import_manifest(self)
    }
}

#[derive(Debug, Clone)]
struct StagedDescriptorRemovalManifest {
    temp: Option<PathBuf>,
}

pub fn teach_description() -> &'static str {
    "Grant one local agent permission to import a specific advertised ability descriptor. \
     The grant is explicit, same-device, and does not install executable code."
}

pub fn acquire_description() -> &'static str {
    "Import a previously granted declaration-only ability manifest into a learner \
     agent workspace. The imported descriptor is discoverable but not invokable."
}

pub fn forget_description() -> &'static str {
    "Remove an imported declaration-only descriptor from an agent workspace. Native authored \
     abilities are not removed through this surface."
}

pub fn teach_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["ability", "learner_ura"],
        "properties": {
            "ability": {
                "type": "string",
                "description": "Owner-local ability descriptor name to grant, for example mentor.quote."
            },
            "learner_ura": {
                "type": "string",
                "description": "Agent URA allowed to import the descriptor."
            }
        },
        "additionalProperties": false
    })
}

pub fn acquire_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["ability_ura", "learner"],
        "properties": {
            "ability_ura": {
                "type": "string",
                "description": "Canonical Ability URA whose declaration manifest was granted to the learner."
            },
            "learner": {
                "type": "string",
                "description": "Local learner agent name whose workspace receives the descriptor copy."
            }
        },
        "additionalProperties": false
    })
}

pub fn forget_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["ability", "agent"],
        "properties": {
            "ability": {
                "type": "string",
                "description": "Public imported descriptor name to remove from the agent workspace."
            },
            "agent": {
                "type": "string",
                "description": "Local agent name that previously imported the descriptor."
            }
        },
        "additionalProperties": false
    })
}

fn teach_manifest() -> AbilityManifest {
    AbilityManifest::new(
        TEACH_MANIFEST_VERB,
        teach_description(),
        teach_input_schema(),
    )
    .expect("meta.teach manifest is static and valid")
}

fn acquire_manifest() -> AbilityManifest {
    AbilityManifest::new(
        ACQUIRE_MANIFEST_VERB,
        acquire_description(),
        acquire_input_schema(),
    )
    .expect("meta.acquire manifest is static and valid")
}

fn forget_manifest() -> AbilityManifest {
    AbilityManifest::new(
        FORGET_MANIFEST_VERB,
        forget_description(),
        forget_input_schema(),
    )
    .expect("meta.forget manifest is static and valid")
}

pub fn register(reg: &mut AxonAbilityCatalog, hot_registrar: Arc<SharedHotRegistrarCell>) {
    reg.register_rpc_with_envelope_and_spec(
        TEACH,
        OwnerKind::Device,
        teach_manifest(),
        Arc::new(teach_handler),
    );
    let registrar_for_acquire = Arc::clone(&hot_registrar);
    reg.register_rpc_with_envelope_and_spec(
        ACQUIRE,
        OwnerKind::Device,
        acquire_manifest(),
        Arc::new(move |env, args| {
            acquire_handler_with_hot_registrar(env, args, Some(&registrar_for_acquire))
        }),
    );
    let registrar_for_forget = Arc::clone(&hot_registrar);
    reg.register_rpc_with_envelope_and_spec(
        FORGET,
        OwnerKind::Device,
        forget_manifest(),
        Arc::new(move |env, args| {
            forget_handler_with_hot_registrar(env, args, Some(&registrar_for_forget))
        }),
    );
}

/// Where an ability lives on this device: its owning agent and the
/// flat-layout manifest path.
#[derive(Debug, Clone)]
struct AbilityHome {
    owner_agent: String,
    public_name: String,
    manifest_path: PathBuf,
}

#[derive(Debug, Clone)]
struct GrantedDescriptorSnapshot {
    bytes: Vec<u8>,
    manifest: AbilityManifest,
    manifest_hash: String,
}

impl GrantedDescriptorSnapshot {
    fn from_home(home: &AbilityHome) -> anyhow::Result<Self> {
        Self::from_path(&home.manifest_path, &home.public_name)
    }

    fn from_path(path: &Path, expected_name: &str) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("read source manifest {}: {e}", path.display()))?;
        let toml = std::str::from_utf8(&bytes).map_err(|e| {
            anyhow::anyhow!("source manifest {} is not UTF-8 TOML: {e}", path.display())
        })?;
        let manifest = AbilityManifest::from_toml_str(toml)
            .map_err(|e| anyhow::anyhow!("parse source manifest {}: {e}", path.display()))?;
        if manifest.name() != expected_name {
            anyhow::bail!(
                "source manifest {} declares name {:?}, expected {:?}; teach/acquire requires \
                 filename, ability URA, and manifest identity to match",
                path.display(),
                manifest.name(),
                expected_name
            );
        }
        let manifest_hash = sha256_hex(sha256_bytes(&bytes));
        Ok(Self {
            bytes,
            manifest,
            manifest_hash,
        })
    }

    fn require_hash(&self, expected_hash: &str, source: &Path) -> anyhow::Result<()> {
        if self.manifest_hash != expected_hash {
            anyhow::bail!(
                "{ACQUIRE} rejected source manifest {}: teach grant pinned {}, current disk hash {}",
                source.display(),
                expected_hash,
                self.manifest_hash
            );
        }
        Ok(())
    }

    fn transferable_bytes<'a>(
        &'a self,
        execution_mode: &str,
        source: &Path,
    ) -> anyhow::Result<&'a [u8]> {
        TransferExecutionPosture::for_manifest(&self.manifest, execution_mode)
            .enforce(ACQUIRE, source)?;
        Ok(&self.bytes)
    }
}

struct OwnerAuthority {
    owner_ura: String,
    granted_by: String,
    admission_authority: TeachGrantAuthoritySnapshot,
}

/// Resolve an owner-local registry name (`<agent>.<ability>`) to the
/// owning agent's on-disk manifest. Precise refusals at every step —
/// a transfer surface must never guess.
fn resolve_owner_manifest(registry_name: &str) -> anyhow::Result<AbilityHome> {
    let Some((agent, public_name)) = registry_name.split_once('.') else {
        anyhow::bail!(
            "ability must use the owner-local `<agent>.<name>` form; got {registry_name:?}"
        );
    };
    let agents = crate::registry::agents::load_agents()?;
    let Some(entry) = agents.agents.get(agent) else {
        anyhow::bail!("no agent {agent:?} on this device (see `easynet agent list`)");
    };
    let Some(root) = entry.root_path.as_ref() else {
        anyhow::bail!("agent {agent:?} has no workspace root; cannot locate its abilities");
    };
    let manifest_path = root
        .join("abilities")
        .join(format!("{public_name}.ability.toml"));
    if !manifest_path.exists() {
        anyhow::bail!(
            "agent {agent:?} publishes no ability {public_name:?} \
             (expected manifest at {})",
            manifest_path.display()
        );
    }
    Ok(AbilityHome {
        owner_agent: agent.to_string(),
        public_name: public_name.to_string(),
        manifest_path,
    })
}

fn required_str<'a>(args: &'a Value, field: &str, surface: &str) -> anyhow::Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{surface} requires `{field}`"))
}

fn hosted_agent_by_name<'a>(
    local: &'a crate::persistence::local_agents::LocalAgentsFile,
    agent_name: &str,
    surface: &str,
) -> anyhow::Result<&'a crate::persistence::local_agents::HostedAgentEntry> {
    crate::persistence::local_agents::lookup_hosted_agent_by_name(local, agent_name)?.ok_or_else(
        || {
            anyhow::anyhow!(
                "{surface} cannot resolve agent {agent_name:?}: no persisted local Agent URA"
            )
        },
    )
}

fn hosted_agent_by_ura<'a>(
    local: &'a crate::persistence::local_agents::LocalAgentsFile,
    agent_ura: &str,
    surface: &str,
) -> anyhow::Result<&'a crate::persistence::local_agents::HostedAgentEntry> {
    local
        .hosted_agents
        .iter()
        .find(|entry| entry.agent_ura == agent_ura)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{surface} cannot teach to {agent_ura:?}: learner must be a local hosted Agent \
                 on this device"
            )
        })
}

/// Validate that the current invocation caller may confer an
/// ability advertised by `owner_agent`.
///
/// Clean v1 rule: the caller must be the advertising Agent URA, or the
/// invocation must carry signed hosted-agent delegation metadata issued through
/// that Agent's persisted host authority.
fn require_owner_authority(
    env: &EnvelopeContext,
    owner_agent: &str,
) -> anyhow::Result<OwnerAuthority> {
    let caller = env.caller();
    let local = crate::persistence::local_agents::load()?;
    let owner_entry = hosted_agent_by_name(&local, owner_agent, TEACH)?;

    if let Some(authority) = authorized_hosted_agent_delegation(
        env,
        &owner_entry.agent_ura,
        &owner_entry.signing_authority,
        TEACH,
    )? {
        return Ok(OwnerAuthority {
            owner_ura: owner_entry.agent_ura.clone(),
            granted_by: authority.host_device_ura().to_string(),
            admission_authority: TeachGrantAuthoritySnapshot::hosted_agent_delegation(
                authority.agent_ura(),
                authority.host_device_ura(),
                authority.ability(),
            ),
        });
    }

    if caller == owner_entry.agent_ura {
        return Ok(OwnerAuthority {
            owner_ura: owner_entry.agent_ura.clone(),
            granted_by: caller.to_string(),
            admission_authority: TeachGrantAuthoritySnapshot::direct_owner(caller),
        });
    }

    anyhow::bail!(
        "{TEACH} caller {caller:?} cannot teach abilities advertised by \
         {owner_agent:?}; expected advertising agent {} or signed hosted-agent delegation from \
         its host device authority",
        owner_entry.agent_ura
    );
}

fn authorized_hosted_agent_delegation(
    env: &EnvelopeContext,
    expected_agent_ura: &str,
    persisted_signing_authority: &str,
    surface: &str,
) -> anyhow::Result<Option<HostedAgentAuthority>> {
    let Some(delegation) = env.hosted_agent_delegation() else {
        return Ok(None);
    };
    delegation
        .authorize(expected_agent_ura, persisted_signing_authority, surface)
        .map(Some)
}

fn require_hosted_agent_authority(
    env: &EnvelopeContext,
    local: &crate::persistence::local_agents::LocalAgentsFile,
    agent_name: &str,
    agent_ura: &str,
    surface: &str,
) -> anyhow::Result<String> {
    let caller = env.caller();
    if caller == agent_ura {
        return Ok(caller.to_string());
    }

    let entry = hosted_agent_by_name(local, agent_name, surface)?;
    if entry.agent_ura != agent_ura {
        anyhow::bail!(
            "{surface} cannot authorize agent {agent_name:?}: persisted URA drifted \
             (expected {}, got {})",
            entry.agent_ura,
            agent_ura
        );
    }

    if let Some(authority) =
        authorized_hosted_agent_delegation(env, agent_ura, &entry.signing_authority, surface)?
    {
        return Ok(authority.host_device_ura().to_string());
    }

    anyhow::bail!(
        "{surface} caller {caller:?} cannot mutate abilities for agent {agent_name:?}; \
         expected the learner Agent URA {agent_ura} or signed hosted-agent delegation from its \
         host device authority"
    );
}

/// `meta.teach { ability, learner_ura }` — the owner grants one descriptor
/// import right to ONE learner. Idempotent per (ability, learner): re-granting
/// refreshes the descriptor grant.
fn teach_handler(env: EnvelopeContext, args: Value) -> anyhow::Result<Value> {
    teach_handler_with_clock(env, args, &SystemTeachClock)
}

fn teach_handler_with_clock(
    env: EnvelopeContext,
    args: Value,
    clock: &dyn TeachClock,
) -> anyhow::Result<Value> {
    let ability = required_str(&args, "ability", TEACH)?;
    let learner_ura = required_str(&args, "learner_ura", TEACH)?;
    let parsed = crate::ura::parse_ura(learner_ura)
        .map_err(|e| anyhow::anyhow!("invalid learner_ura {learner_ura:?}: {e}"))?;
    if parsed.kind != crate::ura::URAKind::Agent {
        anyhow::bail!("learner_ura must be an Agent URA; descriptor grants target agents");
    }
    let local = crate::persistence::local_agents::load()?;
    hosted_agent_by_ura(&local, learner_ura, TEACH)?;

    let home = resolve_owner_manifest(ability)?;
    let snapshot = GrantedDescriptorSnapshot::from_home(&home)?;
    let authority = require_owner_authority(&env, &home.owner_agent)?;
    let ability_ura = crate::ura::owner_ability_ura(&authority.owner_ura, &home.public_name)
        .ok_or_else(|| anyhow::anyhow!("could not mint the granted descriptor URA"))?;
    let granted_at = clock.now_rfc3339();
    let admission_snapshot = teach_grant_admission_snapshot(
        &env,
        authority.admission_authority.clone(),
        GrantedDescriptorFacts {
            granted_by_ura: &authority.granted_by,
            granted_ability: ability,
            granted_ability_ura: &ability_ura,
            owner_ura: &authority.owner_ura,
            learner_ura,
            manifest_hash: &snapshot.manifest_hash,
        },
    )?;
    TeachGrantStore::open_default().grant(TeachGrant::from_draft(TeachGrantDraft {
        ability: ability.to_string(),
        ability_ura: ability_ura.clone(),
        owner_ura: authority.owner_ura.clone(),
        granted_by_ura: authority.granted_by.clone(),
        owner_agent: home.owner_agent.clone(),
        learner_ura: learner_ura.to_string(),
        manifest_hash: snapshot.manifest_hash.clone(),
        execution_mode: EXECUTION_MODE_DEFAULT.to_string(),
        granted_at,
        admission_snapshot,
    }))?;

    Ok(json!({
        "granted_descriptor": ability,
        "ability_ura": ability_ura,
        "owner_agent": home.owner_agent,
        "owner_ura": authority.owner_ura,
        "granted_by": authority.granted_by,
        "learner_ura": learner_ura,
        "manifest_hash": snapshot.manifest_hash,
        "execution_mode": EXECUTION_MODE_DEFAULT,
        "transfer_kind": TRANSFER_KIND_DISCOVERY_ONLY_MANIFEST,
        "invokable_after_acquire": false,
    }))
}

/// The grant-identity facts an admission snapshot binds, distinct from the
/// envelope tuple the snapshot projects from `env`.
struct GrantedDescriptorFacts<'a> {
    granted_by_ura: &'a str,
    granted_ability: &'a str,
    granted_ability_ura: &'a str,
    owner_ura: &'a str,
    learner_ura: &'a str,
    manifest_hash: &'a str,
}

fn teach_grant_admission_snapshot(
    env: &EnvelopeContext,
    authority: TeachGrantAuthoritySnapshot,
    facts: GrantedDescriptorFacts<'_>,
) -> anyhow::Result<TeachGrantAdmissionSnapshot> {
    TeachGrantAdmissionSnapshot::from_draft(TeachGrantAdmissionSnapshotDraft {
        invocation_id: env.invocation_id().to_string(),
        caller_ura: env.caller().to_string(),
        callee_ura: env.callee().to_string(),
        subject_ura: env.subject().to_string(),
        envelope_ability: env.ability().to_string(),
        invocation_nonce_hex: hex::encode(env.invocation_nonce()),
        causal_context: env.causal_context().clone(),
        authority,
        granted_ability: facts.granted_ability.to_string(),
        granted_ability_ura: facts.granted_ability_ura.to_string(),
        owner_ura: facts.owner_ura.to_string(),
        granted_by_ura: facts.granted_by_ura.to_string(),
        learner_ura: facts.learner_ura.to_string(),
        manifest_hash: facts.manifest_hash.to_string(),
    })
}

fn stage_path_for(dest: &Path, operation: &str) -> anyhow::Result<PathBuf> {
    let file_name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("imported descriptor manifest path has no file name"))?;
    if operation == "import" {
        let seq = IMPORT_STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        return Ok(dest.with_file_name(format!(
            ".{file_name}.{operation}.{}.{}.staging",
            std::process::id(),
            seq
        )));
    }
    Ok(dest.with_file_name(format!(".{file_name}.{operation}.staging")))
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn sha256_hex(bytes: [u8; 32]) -> String {
    format!("sha256:{}", hex::encode(bytes))
}

fn sync_parent_dir(path: &Path) -> anyhow::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let dir = std::fs::File::open(parent)
        .map_err(|e| anyhow::anyhow!("open parent dir {}: {e}", parent.display()))?;
    dir.sync_all()
        .map_err(|e| anyhow::anyhow!("fsync parent dir {}: {e}", parent.display()))
}

fn remove_file_if_exists(path: &Path, action: &str) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            sync_parent_dir(path)?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("{action} {}: {e}", path.display())),
    }
}

fn write_staged_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = std::fs::File::create(path)
        .map_err(|e| anyhow::anyhow!("create staged manifest {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| anyhow::anyhow!("write staged manifest {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| anyhow::anyhow!("fsync staged manifest {}: {e}", path.display()))?;
    sync_parent_dir(path)
}

fn stage_descriptor_import_manifest(
    snapshot: &GrantedDescriptorSnapshot,
    source: &Path,
    dest_dir: &Path,
    dest: &Path,
    learner: &str,
    public_name: &str,
    execution_mode: &str,
) -> anyhow::Result<StagedDescriptorImportManifest> {
    if dest.exists() {
        anyhow::bail!(
            "agent {learner:?} already has an ability named {public_name:?}; \
             forget it first or rename; descriptor import never overwrites"
        );
    }
    let bytes = snapshot.transferable_bytes(execution_mode, source)?;
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| anyhow::anyhow!("create {}: {e}", dest_dir.display()))?;
    let temp = stage_path_for(dest, "import")?;
    remove_file_if_exists(&temp, "remove stale descriptor-import staging manifest")?;
    write_staged_file(&temp, bytes)?;
    Ok(StagedDescriptorImportManifest {
        temp,
        dest: dest.to_path_buf(),
        content_hash: sha256_bytes(bytes),
    })
}

fn commit_descriptor_import_manifest(
    staged: &StagedDescriptorImportManifest,
) -> anyhow::Result<()> {
    std::fs::hard_link(&staged.temp, &staged.dest).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!(
                "commit descriptor-import manifest {}: destination already exists",
                staged.dest.display()
            )
        } else {
            anyhow::anyhow!(
                "commit descriptor-import manifest {} -> {} without overwrite: {e}",
                staged.temp.display(),
                staged.dest.display()
            )
        }
    })?;
    sync_parent_dir(&staged.dest)?;
    let _ = remove_file_if_exists(
        &staged.temp,
        "remove committed descriptor-import staging manifest",
    );
    Ok(())
}

fn rollback_descriptor_import_manifest(
    staged: &StagedDescriptorImportManifest,
) -> anyhow::Result<()> {
    if staged.dest.exists() {
        let bytes = std::fs::read(&staged.dest).map_err(|e| {
            anyhow::anyhow!(
                "read committed descriptor-import manifest {} before rollback: {e}",
                staged.dest.display()
            )
        })?;
        if sha256_bytes(&bytes) != staged.content_hash {
            anyhow::bail!(
                "refusing to rollback descriptor-import manifest {} because its content no longer \
                 matches this transaction's staged bytes",
                staged.dest.display()
            );
        }
        return remove_file_if_exists(&staged.dest, "remove committed descriptor-import manifest");
    }
    remove_file_if_exists(&staged.temp, "remove staged descriptor-import manifest")
}

pub fn recover_descriptor_import_transactions() -> anyhow::Result<usize> {
    TeachGrantStore::open_default().recover_acquiring(|record| {
        let path = record.acquiring_manifest_path().ok_or_else(|| {
            anyhow::anyhow!("recover acquiring descriptor-import manifest: missing committed path")
        })?;
        let expected_hash = record.acquiring_manifest_hash().ok_or_else(|| {
            anyhow::anyhow!(
                "recover acquiring descriptor-import manifest {path}: missing content hash"
            )
        })?;
        let path = Path::new(path);
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let staging = record.acquiring_staging_manifest_path().ok_or_else(|| {
                    anyhow::anyhow!(
                        "recover acquiring descriptor-import manifest {}: committed file is absent and \
                         acquiring row has no staging path to clean",
                        path.display()
                    )
                })?;
                remove_file_if_exists(
                    Path::new(staging),
                    "remove stale acquiring descriptor-import staging manifest",
                )?;
                return Ok(AcquiringArtifactRecoveryState::NotCommitted);
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "recover acquiring descriptor-import manifest {}: read failed: {err}",
                    path.display()
                ));
            }
        };
        let actual_hash = sha256_hex(sha256_bytes(&bytes));
        if actual_hash != expected_hash {
            anyhow::bail!(
                "recover acquiring descriptor-import manifest {}: hash mismatch (ledger {}, disk {})",
                path.display(),
                expected_hash,
                actual_hash
            );
        }
        if let Some(staging) = record.acquiring_staging_manifest_path() {
            remove_file_if_exists(
                Path::new(staging),
                "remove stale committed acquiring descriptor-import staging manifest",
            )?;
        }
        Ok(AcquiringArtifactRecoveryState::Committed)
    })
}

fn stage_forget_manifest(
    path: Option<&Path>,
    expected_hash: Option<&str>,
) -> anyhow::Result<StagedDescriptorRemovalManifest> {
    let Some(dest) = path else {
        return Ok(StagedDescriptorRemovalManifest { temp: None });
    };
    let temp = stage_path_for(dest, "forget")?;
    if temp.exists() {
        if !dest.exists() {
            return Ok(StagedDescriptorRemovalManifest { temp: Some(temp) });
        }
        remove_file_if_exists(&temp, "remove stale forgotten staging manifest")?;
    }
    if !dest.exists() {
        return Ok(StagedDescriptorRemovalManifest { temp: None });
    }
    if let Some(expected_hash) = expected_hash {
        let bytes = std::fs::read(dest).map_err(|e| {
            anyhow::anyhow!(
                "read imported descriptor manifest {} before forget hash check: {e}",
                dest.display()
            )
        })?;
        let actual_hash = sha256_hex(sha256_bytes(&bytes));
        if actual_hash != expected_hash {
            anyhow::bail!(
                "{FORGET} refuses to remove imported descriptor manifest {} because ledger hash {} does not \
                 match disk hash {}; inspect the file before retrying",
                dest.display(),
                expected_hash,
                actual_hash
            );
        }
    }
    std::fs::rename(dest, &temp).map_err(|e| {
        anyhow::anyhow!(
            "stage imported descriptor manifest removal {} -> {}: {e}",
            dest.display(),
            temp.display()
        )
    })?;
    if let Err(sync_err) = sync_parent_dir(dest) {
        let restore = std::fs::rename(&temp, dest)
            .map_err(|e| {
                anyhow::anyhow!(
                    "restore imported descriptor manifest after failed forget fsync {} -> {}: {e}",
                    temp.display(),
                    dest.display()
                )
            })
            .and_then(|()| sync_parent_dir(dest));
        return Err(append_cleanup_error(
            sync_err,
            restore,
            "restore imported descriptor manifest after failed forget staging fsync",
        ));
    }
    Ok(StagedDescriptorRemovalManifest { temp: Some(temp) })
}

fn commit_forget_manifest(staged: &StagedDescriptorRemovalManifest) -> anyhow::Result<()> {
    let Some(temp) = staged.temp.as_ref() else {
        return Ok(());
    };
    remove_file_if_exists(temp, "remove staged forgotten manifest")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TransferExecutionPosture {
    DiscoveryOnly,
    ExecutableBlocked {
        exec_kind: &'static str,
        execution_mode: String,
    },
}

impl TransferExecutionPosture {
    fn for_manifest(manifest: &AbilityManifest, execution_mode: &str) -> Self {
        match manifest.exec() {
            None => Self::DiscoveryOnly,
            Some(exec) => Self::ExecutableBlocked {
                exec_kind: ability_exec_kind(exec),
                execution_mode: execution_mode.to_string(),
            },
        }
    }

    fn enforce(self, surface: &str, source: &Path) -> anyhow::Result<()> {
        match self {
            Self::DiscoveryOnly => Ok(()),
            Self::ExecutableBlocked {
                exec_kind,
                execution_mode,
            } => anyhow::bail!(
                "{surface} refuses executable transferred ability {}: manifest declares \
                 [exec] kind {exec_kind:?}, grant execution_mode={execution_mode:?}, and \
                 this product surface admits discovery-only manifest transfer only. Remove \
                 [exec] before teaching or deploy executable code through the device ability \
                 deployment path.",
                source.display()
            ),
        }
    }
}

fn ability_exec_kind(exec: &AbilityExec) -> &'static str {
    match exec {
        AbilityExec::Shell(_) => "shell",
        AbilityExec::Http(_) => "http",
        AbilityExec::Eal(_) => "eal",
        AbilityExec::Mcp(_) => "mcp",
        AbilityExec::HostStream(_) => "host_stream",
    }
}

/// `meta.acquire { ability_ura, learner }` imports a granted declaration-only
/// manifest copy into one local learner workspace. It mints a learner-owned
/// descriptor URA for discovery, but it does not install an executable runtime
/// binding.
fn acquire_handler_with_hot_registrar(
    env: EnvelopeContext,
    args: Value,
    hot_registrar: Option<&SharedHotRegistrarCell>,
) -> anyhow::Result<Value> {
    AcquireWorkflow::new(env, args, hot_registrar, &SystemTeachClock).run()
}

struct AcquireWorkflow<'a> {
    env: EnvelopeContext,
    args: Value,
    hot_registrar: Option<&'a SharedHotRegistrarCell>,
    clock: &'a dyn TeachClock,
}

impl<'a> AcquireWorkflow<'a> {
    fn new(
        env: EnvelopeContext,
        args: Value,
        hot_registrar: Option<&'a SharedHotRegistrarCell>,
        clock: &'a dyn TeachClock,
    ) -> Self {
        Self {
            env,
            args,
            hot_registrar,
            clock,
        }
    }

    fn run(self) -> anyhow::Result<Value> {
        let Self {
            env,
            args,
            hot_registrar,
            clock,
        } = self;
        let request = AcquireRequest::from_args(&args)?;
        let authorized = AuthorizedAcquire::from_request(env, request)?;
        let admitted = authorized.admit_transfer()?;
        let committed = admitted.commit(clock)?;
        committed.refresh_runtime(hot_registrar)?.into_response()
    }
}

#[derive(Debug, Clone)]
struct AcquireRequest {
    ability_ura: String,
    learner: String,
    registry_name: String,
    public_name: String,
    owner_ura: String,
}

impl AcquireRequest {
    fn from_args(args: &Value) -> anyhow::Result<Self> {
        let ability_ura_arg = required_str(args, "ability_ura", ACQUIRE)?;
        let learner = required_str(args, "learner", ACQUIRE)?.to_string();
        let selector = AbilitySelector::parse(ability_ura_arg)?;
        Ok(Self {
            ability_ura: selector.ability_ura().to_string(),
            learner,
            registry_name: selector.local_registry_ability().to_string(),
            public_name: selector.public_name().to_string(),
            owner_ura: selector.owner_ura().to_string(),
        })
    }
}

#[derive(Debug, Clone)]
struct AuthorizedAcquire {
    request: AcquireRequest,
    learner_entry_for_runtime: crate::registry::agents::AgentEntry,
    learner_ura: String,
    mutated_by: String,
    owner_home: AbilityHome,
    dest_dir: PathBuf,
    dest: PathBuf,
}

impl AuthorizedAcquire {
    fn from_request(env: EnvelopeContext, request: AcquireRequest) -> anyhow::Result<Self> {
        if env.subject() != request.ability_ura {
            anyhow::bail!(
                "{ACQUIRE} subject {:?} must be the source descriptor {:?}",
                env.subject(),
                request.ability_ura
            );
        }
        let agents = crate::registry::agents::load_agents()?;
        let learner_entry_for_runtime =
            agents
                .agents
                .get(&request.learner)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no agent {:?} on this device to learn into",
                        request.learner
                    )
                })?;
        let learner_root = learner_entry_for_runtime
            .root_path
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "agent {:?} has no workspace root to learn into",
                    request.learner
                )
            })?;

        let local = crate::persistence::local_agents::load()?;
        let learner_ura = hosted_agent_by_name(&local, &request.learner, ACQUIRE)?
            .agent_ura
            .clone();
        let mutated_by =
            require_hosted_agent_authority(&env, &local, &request.learner, &learner_ura, ACQUIRE)?;

        let owner_home = resolve_owner_manifest(&request.registry_name)?;
        let owner_entry = hosted_agent_by_name(&local, &owner_home.owner_agent, ACQUIRE)?;
        if owner_entry.agent_ura != request.owner_ura {
            anyhow::bail!(
                "{ACQUIRE} rejected {:?}: local ability {:?} belongs to owner {}, not {}",
                request.ability_ura,
                request.registry_name,
                owner_entry.agent_ura,
                request.owner_ura
            );
        }

        let dest_dir = learner_root.join("abilities");
        let dest = dest_dir.join(format!("{}.ability.toml", request.public_name));
        if dest.exists() {
            anyhow::bail!(
                "agent {:?} already has an ability descriptor named {:?}; forget it first or rename; \
                 descriptor import never overwrites",
                request.learner,
                request.public_name
            );
        }

        Ok(Self {
            request,
            learner_entry_for_runtime,
            learner_ura,
            mutated_by,
            owner_home,
            dest_dir,
            dest,
        })
    }

    fn admit_transfer(self) -> anyhow::Result<AdmittedAcquire> {
        recover_descriptor_import_transactions()?;
        let store = TeachGrantStore::open_default();
        let grant = store
            .grant_for(
                &self.request.registry_name,
                &self.request.ability_ura,
                &self.request.owner_ura,
                &self.learner_ura,
            )?
            .ok_or_else(|| {
                no_teach_grant_error(
                    &self.request.registry_name,
                    &self.request.ability_ura,
                    &self.request.owner_ura,
                    &self.learner_ura,
                )
            })?;
        Ok(AdmittedAcquire { plan: self, grant })
    }
}

#[derive(Debug, Clone)]
struct AdmittedAcquire {
    plan: AuthorizedAcquire,
    grant: TeachGrant,
}

impl AdmittedAcquire {
    fn commit(self, clock: &dyn TeachClock) -> anyhow::Result<CommittedAcquire> {
        let Self { plan, grant } = self;
        let snapshot = GrantedDescriptorSnapshot::from_home(&plan.owner_home)?;
        snapshot.require_hash(grant.manifest_hash(), &plan.owner_home.manifest_path)?;
        let staged = stage_descriptor_import_manifest(
            &snapshot,
            &plan.owner_home.manifest_path,
            &plan.dest_dir,
            &plan.dest,
            &plan.request.learner,
            &plan.request.public_name,
            grant.execution_mode(),
        )?;
        let import_record = DescriptorImportRecord::new(
            &plan.request.public_name,
            &plan.request.learner,
            &plan.request.ability_ura,
            grant.manifest_hash(),
            clock.now_rfc3339(),
        );
        let acquired = TeachGrantStore::open_default().acquire_staged(AcquireStagedGrant::new(
            plan.request.registry_name.clone(),
            plan.request.ability_ura.clone(),
            plan.request.owner_ura.clone(),
            plan.learner_ura.clone(),
            grant,
            import_record,
            staged,
        )?)?;
        Ok(CommittedAcquire { plan, acquired })
    }
}

#[derive(Debug)]
struct CommittedAcquire {
    plan: AuthorizedAcquire,
    acquired: crate::persistence::teach_grants::AcquiredTeachGrant,
}

impl CommittedAcquire {
    fn refresh_runtime(
        self,
        hot_registrar: Option<&SharedHotRegistrarCell>,
    ) -> anyhow::Result<RuntimeSyncedAcquire> {
        let new_descriptor_ura =
            crate::ura::owner_ability_ura(&self.plan.learner_ura, &self.plan.request.public_name)
                .ok_or_else(|| anyhow::anyhow!("could not mint the learner's ability URA"))?;
        let runtime_sync = sync_learner_runtime_after_acquire(
            hot_registrar,
            &self.plan.request.learner,
            &self.plan.learner_entry_for_runtime,
        );
        Ok(RuntimeSyncedAcquire {
            plan: self.plan,
            acquired: self.acquired,
            new_descriptor_ura,
            runtime_sync,
        })
    }
}

#[derive(Debug)]
struct RuntimeSyncedAcquire {
    plan: AuthorizedAcquire,
    acquired: crate::persistence::teach_grants::AcquiredTeachGrant,
    new_descriptor_ura: String,
    runtime_sync: RuntimeSyncOutcome,
}

impl RuntimeSyncedAcquire {
    fn into_response(self) -> anyhow::Result<Value> {
        let transaction_status = self.runtime_sync.transaction_status().as_str();
        Ok(json!({
            "acquired_descriptor": self.plan.request.public_name,
            "new_descriptor_ura": self.new_descriptor_ura,
            "source_descriptor_ura": self.plan.request.ability_ura,
            "execution_mode": self.acquired.grant().execution_mode(),
            "manifest_hash": self.acquired.import_record().manifest_hash(),
            "transfer_kind": TRANSFER_KIND_DISCOVERY_ONLY_MANIFEST,
            "invokable": false,
            "invocation_status": INVOCATION_STATUS_NOT_INVOKABLE_WITHOUT_EXEC_BINDING,
            "descriptor_transaction_status": transaction_status,
            "mutated_by": self.plan.mutated_by,
            "runtime_sync": self.runtime_sync.into_value(),
        }))
    }
}

#[derive(Debug, Clone)]
struct ForgetRequest {
    ability: String,
    agent: String,
}

impl ForgetRequest {
    fn from_args(args: &Value) -> anyhow::Result<Self> {
        Ok(Self {
            ability: required_str(args, "ability", FORGET)?.to_string(),
            agent: required_str(args, "agent", FORGET)?.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
struct AuthorizedForget {
    request: ForgetRequest,
    agent_entry_for_runtime: Option<crate::registry::agents::AgentEntry>,
    mutated_by: String,
    manifest: Option<PathBuf>,
}

impl AuthorizedForget {
    fn from_request(env: EnvelopeContext, request: ForgetRequest) -> anyhow::Result<Self> {
        let agents = crate::registry::agents::load_agents()?;
        let agent_entry_for_runtime = agents.agents.get(&request.agent).cloned();
        let local = crate::persistence::local_agents::load()?;
        let agent_ura = hosted_agent_by_name(&local, &request.agent, FORGET)?
            .agent_ura
            .clone();
        let expected_subject = crate::ura::owner_ability_ura(&agent_ura, &request.ability)
            .ok_or_else(|| {
                anyhow::anyhow!("{FORGET} could not mint imported descriptor subject")
            })?;
        if env.subject() != expected_subject {
            anyhow::bail!(
                "{FORGET} subject {:?} must be the imported descriptor {:?}",
                env.subject(),
                expected_subject
            );
        }
        let mutated_by =
            require_hosted_agent_authority(&env, &local, &request.agent, &agent_ura, FORGET)?;
        let manifest = agents
            .agents
            .get(&request.agent)
            .and_then(|e| e.root_path.as_ref())
            .map(|root| {
                root.join("abilities")
                    .join(format!("{}.ability.toml", request.ability))
            });
        Ok(Self {
            request,
            agent_entry_for_runtime,
            mutated_by,
            manifest,
        })
    }

    fn stage_removal(self) -> anyhow::Result<StagedForget> {
        recover_descriptor_import_transactions()?;
        let manifest = self.manifest.clone();
        let store = TeachGrantStore::open_default();
        let staged = store.stage_forget(&self.request.agent, &self.request.ability, |record| {
            stage_forget_manifest(manifest.as_deref(), Some(record.manifest_hash()))
        })?;
        Ok(StagedForget { plan: self, staged })
    }
}

#[derive(Debug)]
struct StagedForget {
    plan: AuthorizedForget,
    staged: crate::persistence::teach_grants::StagedDescriptorImportRemoval<
        StagedDescriptorRemovalManifest,
    >,
}

impl StagedForget {
    fn commit_artifact(self) -> anyhow::Result<RuntimePendingForget> {
        let resumed = self.staged.resumed();
        let pending = TeachGrantStore::open_default()
            .commit_forget_artifact(&self.staged, commit_forget_manifest)?;
        Ok(RuntimePendingForget {
            plan: self.plan,
            pending,
            resumed,
        })
    }
}

#[derive(Debug)]
struct RuntimePendingForget {
    plan: AuthorizedForget,
    pending: crate::persistence::teach_grants::RuntimePendingDescriptorImportRemoval,
    resumed: bool,
}

impl RuntimePendingForget {
    fn refresh_runtime(
        self,
        hot_registrar: Option<&SharedHotRegistrarCell>,
    ) -> RuntimeSyncedForget {
        let runtime_sync = match self.plan.agent_entry_for_runtime.as_ref() {
            Some(entry) => {
                sync_learner_runtime_after_forget(hot_registrar, &self.plan.request.agent, entry)
            }
            None => {
                let report = RuntimeSyncReport::not_ready("agent_registry_entry_missing");
                RuntimeSyncOutcome::after_durable_commit(report)
            }
        };
        RuntimeSyncedForget {
            plan: self.plan,
            pending: self.pending,
            resumed: self.resumed,
            runtime_sync,
        }
    }
}

#[derive(Debug)]
struct RuntimeSyncedForget {
    plan: AuthorizedForget,
    pending: crate::persistence::teach_grants::RuntimePendingDescriptorImportRemoval,
    resumed: bool,
    runtime_sync: RuntimeSyncOutcome,
}

impl RuntimeSyncedForget {
    fn finalize(self) -> anyhow::Result<Value> {
        let runtime_sync = self.runtime_sync.require_committed(
            FORGET,
            "descriptor artifact removal is committed and the ledger tombstone remains in \
             Forgetting state; retry meta.forget after the agent runtime is wired",
        )?;
        let committed = TeachGrantStore::open_default().finish_forget(&self.pending)?;
        let record = committed.record();
        Ok(json!({
            "removed_descriptor": self.plan.request.ability,
            "agent": self.plan.request.agent,
            "source_descriptor_ura": record.source_descriptor_ura(),
            "descriptor_transaction_status": runtime_sync.transaction_status().as_str(),
            "mutated_by": self.plan.mutated_by,
            "runtime_sync": runtime_sync.into_value(),
            "resumed": self.resumed,
        }))
    }
}

fn no_teach_grant_error(
    registry_name: &str,
    ability_ura: &str,
    owner_ura: &str,
    learner_ura: &str,
) -> anyhow::Error {
    anyhow::anyhow!(
        "descriptor grant missing (allow_transferred_code=false): owner {owner_ura:?} has not \
         granted descriptor {ability_ura:?} ({registry_name:?}) to learner {learner_ura}"
    )
}

fn append_cleanup_error(
    primary: anyhow::Error,
    cleanup: anyhow::Result<()>,
    cleanup_action: &'static str,
) -> anyhow::Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup_err) => {
            anyhow::anyhow!("{primary}; additionally failed to {cleanup_action}: {cleanup_err}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeSyncReport {
    attempted: bool,
    ok: bool,
    registered: usize,
    replaced: usize,
    failed: usize,
    removed: usize,
    runtime_not_ready: bool,
    catalog_not_ready: bool,
    rejected_reserved_owner: bool,
    reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescriptorTransactionStatus {
    Committed,
    CommittedRuntimeDegraded,
}

impl DescriptorTransactionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::CommittedRuntimeDegraded => "committed_runtime_degraded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeSyncOutcome {
    report: RuntimeSyncReport,
    transaction_status: DescriptorTransactionStatus,
}

impl RuntimeSyncOutcome {
    fn after_durable_commit(report: RuntimeSyncReport) -> Self {
        let transaction_status = if report.ok {
            DescriptorTransactionStatus::Committed
        } else {
            DescriptorTransactionStatus::CommittedRuntimeDegraded
        };
        Self {
            report,
            transaction_status,
        }
    }

    fn transaction_status(&self) -> DescriptorTransactionStatus {
        self.transaction_status
    }

    fn require_committed(self, surface: &str, recovery_hint: &str) -> anyhow::Result<Self> {
        if self.transaction_status == DescriptorTransactionStatus::Committed {
            return Ok(self);
        }
        anyhow::bail!(
            "{surface} durable transaction reached runtime-sync with status `{}` ({}). {recovery_hint}",
            self.transaction_status.as_str(),
            self.report.failure_summary()
        )
    }

    fn into_value(self) -> Value {
        let mut value = self.report.into_value();
        value["status"] = Value::String(self.transaction_status.as_str().to_string());
        value
    }
}

impl RuntimeSyncReport {
    fn not_ready(reason: &'static str) -> Self {
        Self {
            attempted: false,
            ok: false,
            registered: 0,
            replaced: 0,
            failed: 0,
            removed: 0,
            runtime_not_ready: true,
            catalog_not_ready: false,
            rejected_reserved_owner: false,
            reason: Some(reason),
        }
    }

    fn from_outcome(outcome: HotAgentRuntimeSyncOutcome) -> Self {
        let ok = !outcome.runtime_not_ready
            && !outcome.catalog_not_ready
            && !outcome.rejected_reserved_owner
            && outcome.failed == 0;
        Self {
            attempted: true,
            ok,
            registered: outcome.registered,
            replaced: outcome.replaced,
            failed: outcome.failed,
            removed: outcome.removed,
            runtime_not_ready: outcome.runtime_not_ready,
            catalog_not_ready: outcome.catalog_not_ready,
            rejected_reserved_owner: outcome.rejected_reserved_owner,
            reason: None,
        }
    }

    fn failure_summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(reason) = self.reason {
            parts.push(format!("reason={reason}"));
        }
        parts.push(format!("attempted={}", self.attempted));
        parts.push(format!("runtime_not_ready={}", self.runtime_not_ready));
        parts.push(format!("catalog_not_ready={}", self.catalog_not_ready));
        parts.push(format!(
            "rejected_reserved_owner={}",
            self.rejected_reserved_owner
        ));
        parts.push(format!("failed={}", self.failed));
        parts.join(", ")
    }

    fn into_value(self) -> Value {
        let mut value = json!({
            "attempted": self.attempted,
            "ok": self.ok,
            "registered": self.registered,
            "replaced": self.replaced,
            "failed": self.failed,
            "removed": self.removed,
            "runtime_not_ready": self.runtime_not_ready,
            "catalog_not_ready": self.catalog_not_ready,
            "rejected_reserved_owner": self.rejected_reserved_owner,
        });
        if let Some(reason) = self.reason {
            value["reason"] = Value::String(reason.to_string());
        }
        value
    }
}

fn collect_learner_runtime_sync(
    hot_registrar: Option<&SharedHotRegistrarCell>,
    learner: &str,
    entry: &crate::registry::agents::AgentEntry,
) -> RuntimeSyncReport {
    let Some(cell) = hot_registrar else {
        return RuntimeSyncReport::not_ready("hot_registrar_not_provided");
    };
    let Some(registrar) = cell.get().cloned() else {
        return RuntimeSyncReport::not_ready("hot_registrar_not_wired");
    };
    let learner_name = learner.to_string();
    let entry_for_runtime = entry.clone();
    let Some(outcome) = block_on_hot_registrar(async move {
        registrar
            .register_agent(&learner_name, &entry_for_runtime)
            .await
    }) else {
        return RuntimeSyncReport::not_ready("no_tokio_runtime_for_hot_registrar");
    };
    RuntimeSyncReport::from_outcome(outcome)
}

fn sync_learner_runtime_after_acquire(
    hot_registrar: Option<&SharedHotRegistrarCell>,
    learner: &str,
    entry: &crate::registry::agents::AgentEntry,
) -> RuntimeSyncOutcome {
    let report = collect_learner_runtime_sync(hot_registrar, learner, entry);
    RuntimeSyncOutcome::after_durable_commit(report)
}

/// `meta.forget { ability, agent }` — drop an imported descriptor. The
/// descriptor-import ledger is the authority: a native ability never matches
/// it, so forget can never silently delete what an agent authored.
struct ForgetWorkflow<'a> {
    env: EnvelopeContext,
    args: Value,
    hot_registrar: Option<&'a SharedHotRegistrarCell>,
}

impl<'a> ForgetWorkflow<'a> {
    fn new(
        env: EnvelopeContext,
        args: Value,
        hot_registrar: Option<&'a SharedHotRegistrarCell>,
    ) -> Self {
        Self {
            env,
            args,
            hot_registrar,
        }
    }

    fn run(self) -> anyhow::Result<Value> {
        let request = ForgetRequest::from_args(&self.args)?;
        let authorized = AuthorizedForget::from_request(self.env, request)?;
        let staged = authorized.stage_removal()?;
        let runtime_pending = staged.commit_artifact()?;
        runtime_pending
            .refresh_runtime(self.hot_registrar)
            .finalize()
    }
}

fn forget_handler_with_hot_registrar(
    env: EnvelopeContext,
    args: Value,
    hot_registrar: Option<&SharedHotRegistrarCell>,
) -> anyhow::Result<Value> {
    ForgetWorkflow::new(env, args, hot_registrar).run()
}

fn sync_learner_runtime_after_forget(
    hot_registrar: Option<&SharedHotRegistrarCell>,
    learner: &str,
    entry: &crate::registry::agents::AgentEntry,
) -> RuntimeSyncOutcome {
    let report = collect_learner_runtime_sync(hot_registrar, learner, entry);
    RuntimeSyncOutcome::after_durable_commit(report)
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;
    use crate::registry::agents::{AgentRegistry, AgentType};

    /// Two agents on disk — mentor publishes executable `quote`,
    /// apprentice publishes nothing — both with minted URAs, all
    /// through the product files.
    fn seed() -> (String, String, String) {
        seed_with_mentor_manifest(
            "name = \"quote\"\n\
             description = \"emit a quotable line\"\n\n\
             [input_schema]\n\
             type = \"object\"\n\n\
             [exec]\n\
             kind = \"shell\"\n\
             argv = [\"/bin/echo\", \"quote\"]\n",
        )
    }

    /// Same fixture as `seed`, but the mentor manifest intentionally
    /// has no `[exec]`. These abilities are metadata/discovery rows;
    /// they must not be synthesized into runtime handlers by hot
    /// registration.
    fn seed_declaration_only() -> (String, String, String) {
        seed_with_mentor_manifest(
            "name = \"quote\"\n\
             description = \"emit a quotable line\"\n\n\
             [input_schema]\n\
             type = \"object\"\n",
        )
    }

    fn seed_with_mentor_manifest(mentor_manifest: &str) -> (String, String, String) {
        let home = std::env::var("HOME").expect("HomeGuard sets HOME");
        let mut registry = AgentRegistry::default();
        for name in ["mentor", "apprentice"] {
            let root = std::path::Path::new(&home).join(format!("agents/{name}"));
            std::fs::create_dir_all(root.join("abilities")).expect("agent root");
            std::fs::write(
                root.join("agent.toml"),
                format!("name = \"{name}\"\nruntime = \"claude-code\"\n"),
            )
            .expect("agent.toml");
            let mut entry = crate::registry::agents::AgentEntry::new(AgentType::ClaudeCode, None);
            entry.root_path = Some(root);
            registry.agents.insert(name.to_string(), entry);
        }
        crate::registry::agents::save_agents(&registry).expect("save agents");

        std::fs::write(
            std::path::Path::new(&home).join("agents/mentor/abilities/quote.ability.toml"),
            mentor_manifest,
        )
        .expect("mentor manifest");

        let mut local = crate::persistence::local_agents::LocalAgentsFile {
            host_device_agent_ura: crate::ura::device_ura("test", "local"),
            ..Default::default()
        };
        let mentor_ura = crate::ura::agent_ura("localhost", "dev", "mentor");
        let apprentice_ura = crate::ura::agent_ura("localhost", "dev", "apprentice");
        crate::persistence::local_agents::upsert_hosted_agent(
            &mut local,
            "llm",
            "mentor",
            &mentor_ura,
        );
        crate::persistence::local_agents::upsert_hosted_agent(
            &mut local,
            "llm",
            "apprentice",
            &apprentice_ura,
        );
        crate::persistence::local_agents::save(&local).expect("save local agents");

        let source_descriptor_ura =
            crate::ura::owner_ability_ura(&mentor_ura, "quote").expect("mentor ability URA");
        (source_descriptor_ura, apprentice_ura, mentor_ura)
    }

    fn subject_for(owner_ura: &str, public_name: &str) -> String {
        crate::ura::owner_ability_ura(owner_ura, public_name).expect("test ability subject")
    }

    fn caller_env_with_subject(
        caller: impl Into<String>,
        ability: impl Into<String>,
        subject: impl Into<String>,
    ) -> EnvelopeContext {
        EnvelopeContext::for_test_ability(caller, ability, subject)
    }

    fn teach_env(caller: impl Into<String>, owner_ura: &str) -> EnvelopeContext {
        teach_env_for(caller, owner_ura, "quote")
    }

    fn teach_env_for(
        caller: impl Into<String>,
        owner_ura: &str,
        public_name: &str,
    ) -> EnvelopeContext {
        caller_env_with_subject(caller, TEACH, subject_for(owner_ura, public_name))
    }

    fn teach_env_with_hosted_delegation(
        caller: impl Into<String>,
        owner_ura: &str,
        hosted_agent_ura: &str,
    ) -> EnvelopeContext {
        use ed25519_dalek::Signer as _;

        let env = teach_env(caller, owner_ura);
        let signer = ed25519_dalek::SigningKey::from_bytes(&[0x44; 32]);
        let nonce_hex = hex::encode(env.invocation_nonce());
        let descriptor_ref = format!(
            "{}@1.0.0",
            crate::ura::owner_ability_ura(env.callee(), TEACH).expect("test teach descriptor ref")
        );
        let envelope = crate::runtime::ability::HostedAgentDelegationEnvelopeBinding::new(
            env.caller(),
            env.callee(),
            env.subject(),
            env.invocation_id(),
            nonce_hex.as_str(),
            descriptor_ref.as_str(),
        )
        .expect("test hosted-agent delegation envelope");
        let claims = crate::runtime::ability::HostedAgentDelegationClaims::new(
            hosted_agent_ura,
            "host_device",
            envelope.clone(),
        )
        .expect("test hosted-agent delegation claims");
        let signature = signer.sign(&claims.signing_payload_bytes(env.caller()));
        let raw = claims
            .signed_metadata_value(env.caller(), &signature)
            .expect("test hosted-agent delegation token");
        let delegation =
            crate::runtime::ability::HostedAgentDelegationContext::from_signed_metadata(
                &raw,
                &envelope,
                signer.verifying_key(),
            )
            .expect("test hosted-agent delegation context");
        env.with_hosted_agent_delegation(Some(delegation))
    }

    fn acquire_env(caller: impl Into<String>, source_descriptor_ura: &str) -> EnvelopeContext {
        caller_env_with_subject(caller, ACQUIRE, source_descriptor_ura)
    }

    fn forget_env(caller: impl Into<String>, learner_ura: &str) -> EnvelopeContext {
        caller_env_with_subject(caller, FORGET, subject_for(learner_ura, "quote"))
    }

    fn hot_registrar_cell_with_runtime(
        runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ) -> SharedHotRegistrarCell {
        let dispatch_handle = Arc::new(std::sync::OnceLock::new());
        let registrar =
            crate::runtime::axon_bridge::hot_agent_registrar::HotAgentRegistrar::new_pending(
                Arc::new(Vec::new()),
                Arc::clone(&dispatch_handle),
                Arc::new(
                    crate::runtime::agents::discover_ability::BridgeDiscoverFederationResolver,
                ),
            );
        registrar.set_runtime(Arc::clone(&runtime));
        let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime(runtime));
        dispatch_handle
            .set(catalog)
            .expect("test catalog wired exactly once");
        let hot_cell = SharedHotRegistrarCell::new();
        assert!(hot_cell.set(registrar).is_ok());
        hot_cell
    }

    struct FixedTeachClock(&'static str);

    impl TeachClock for FixedTeachClock {
        fn now_rfc3339(&self) -> String {
            self.0.to_string()
        }
    }

    fn mark_first_import_as_acquiring_for_test(manifest_path: &Path) {
        let grants_path = crate::persistence::teach_grants::path();
        let mut grants: Value =
            serde_json::from_slice(&std::fs::read(&grants_path).expect("teach grants file"))
                .expect("teach grants json");
        let import = grants["imports"]
            .as_array_mut()
            .and_then(|imports| imports.first_mut())
            .expect("one import row");
        let manifest_hash = import["manifest_hash"].clone();
        import["state"] = json!("acquiring");
        import["acquiring_manifest_path"] = json!(manifest_path.to_string_lossy().to_string());
        import["acquiring_staging_manifest_path"] = Value::Null;
        import["acquiring_manifest_hash"] = manifest_hash;
        std::fs::write(
            grants_path,
            serde_json::to_vec_pretty(&grants).expect("serialize grants"),
        )
        .expect("write acquiring grants fixture");
    }

    #[test]
    fn teach_ledger_uses_the_explicit_clock() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        let ts = "2026-06-23T01:02:03Z";

        teach_handler_with_clock(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
            &FixedTeachClock(ts),
        )
        .expect("owner teaches");

        let file = std::fs::read_to_string(crate::persistence::teach_grants::path())
            .expect("teach grants file");
        let parsed: Value = serde_json::from_str(&file).expect("teach grants json");
        assert_eq!(parsed["grants"][0]["granted_at"], ts);
        assert_eq!(
            parsed["grants"][0]["ability_ura"],
            crate::ura::owner_ability_ura(&mentor_ura, "quote").unwrap()
        );
        assert_eq!(parsed["grants"][0]["owner_ura"], mentor_ura);
    }

    #[test]
    fn teach_rejects_non_local_hosted_learner_ura() {
        let _g = HomeGuard::new();
        let (_, _, mentor_ura) = seed();
        let remote_agent = crate::ura::agent_ura("remote-realm", "remote-dev", "apprentice");

        let err = teach_handler_with_clock(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": remote_agent }),
            &FixedTeachClock("2026-06-23T01:02:03Z"),
        )
        .expect_err("teach must only grant to hosted local agents");

        assert!(
            err.to_string()
                .contains("learner must be a local hosted Agent"),
            "unexpected error: {err}"
        );
        assert!(
            !crate::persistence::teach_grants::path().exists(),
            "rejected teach must not create a grant file"
        );
    }

    #[test]
    fn acquire_without_grant_is_the_default_refusal() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, _) = seed();
        let err = acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura, &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect_err("no grant must refuse");
        assert!(
            format!("{err}").contains("allow_transferred_code=false"),
            "the refusal must name the gate: {err}"
        );
    }

    #[test]
    fn acquire_rejects_same_registry_key_from_a_different_owner_ura() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let forged_owner = crate::ura::agent_ura("localhost", "other-user", "mentor");
        let forged_ability =
            crate::ura::owner_ability_ura(&forged_owner, "quote").expect("forged ability URA");
        let err = acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura, &forged_ability),
            json!({ "ability_ura": forged_ability, "learner": "apprentice" }),
            None,
        )
        .expect_err("same registry key with different owner URA must refuse");

        assert!(
            format!("{err}").contains("belongs to owner"),
            "error should identify owner mismatch: {err}"
        );
        let home = std::env::var("HOME").unwrap();
        assert!(
            !std::path::Path::new(&home)
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "refused acquire must not mint the learner copy"
        );
    }

    #[test]
    fn teach_then_acquire_mints_the_learner_copy() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();

        let teach = teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("owner teaches");
        assert_eq!(teach["owner_ura"], teach["granted_by"]);
        let resp = acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect("learner acquires");

        // The learner owns a NEW descriptor under its own URA.
        let new_descriptor_ura = resp["new_descriptor_ura"]
            .as_str()
            .expect("new_descriptor_ura");
        let sel = AbilitySelector::parse(new_descriptor_ura).expect("new URA round-trips");
        assert_eq!(sel.owner_kind(), "agent");
        assert_eq!(sel.owner_ura(), apprentice_ura, "owner is the learner now");
        assert_eq!(resp["execution_mode"], EXECUTION_MODE_DEFAULT);
        assert_eq!(resp["transfer_kind"], TRANSFER_KIND_DISCOVERY_ONLY_MANIFEST);
        assert_eq!(resp["invokable"], false);

        // …the copy is on disk, and the original is untouched.
        let home = std::env::var("HOME").unwrap();
        assert!(std::path::Path::new(&home)
            .join("agents/apprentice/abilities/quote.ability.toml")
            .exists());
        assert!(std::path::Path::new(&home)
            .join("agents/mentor/abilities/quote.ability.toml")
            .exists());
    }

    #[test]
    fn acquire_rejects_source_manifest_changed_after_teach_grant() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let home = std::env::var("HOME").unwrap();
        let mentor_manifest =
            std::path::Path::new(&home).join("agents/mentor/abilities/quote.ability.toml");
        std::fs::write(
            &mentor_manifest,
            "name = \"quote\"\n\
             description = \"mutated after the grant\"\n\n\
             [input_schema]\n\
             type = \"object\"\n",
        )
        .expect("mutate source manifest");

        let err = acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect_err("acquire must be bound to the descriptor granted by the owner");
        assert!(format!("{err}").contains("teach grant pinned"), "{err}");
        assert!(
            !std::path::Path::new(&home)
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "hash mismatch must not mint a learner copy"
        );
        assert!(
            TeachGrantStore::open_default()
                .grant_for(
                    "mentor.quote",
                    &source_descriptor_ura,
                    &mentor_ura,
                    &apprentice_ura
                )
                .unwrap()
                .is_some(),
            "hash mismatch must not consume the grant"
        );
    }

    #[test]
    fn acquire_blocks_executable_transfer_before_staging() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let err = acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect_err("executable transfers are outside descriptor-import scope");

        assert!(
            format!("{err}").contains("refuses executable transferred ability"),
            "{err}"
        );
        assert!(
            format!("{err}").contains("discovery-only manifest transfer only"),
            "{err}"
        );
        let home = std::env::var("HOME").unwrap();
        assert!(
            !std::path::Path::new(&home)
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "blocked acquire must not stage or commit an imported descriptor manifest"
        );
        assert!(
            TeachGrantStore::open_default()
                .grant_for(
                    "mentor.quote",
                    &source_descriptor_ura,
                    &mentor_ura,
                    &apprentice_ura
                )
                .unwrap()
                .is_some(),
            "blocked acquire must not consume the teach grant"
        );
    }

    #[test]
    fn acquire_with_hot_registrar_does_not_mount_declaration_only_copy() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let runtime = easynet_axon::invocation::LocalRuntime::new();
        let hot_cell = hot_registrar_cell_with_runtime(Arc::clone(&runtime));

        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let resp = tokio_runtime
            .block_on(async {
                acquire_handler_with_hot_registrar(
                    acquire_env(apprentice_ura, &source_descriptor_ura),
                    json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("learner acquires metadata-only ability");

        assert_eq!(resp["runtime_sync"]["attempted"], true);
        assert_eq!(resp["runtime_sync"]["failed"], 0);
        assert_eq!(resp["descriptor_transaction_status"], "committed");
        assert_eq!(resp["runtime_sync"]["status"], "committed");
        let runtime_key = crate::ura::owner_ability_ura(
            &crate::runtime::local_invocation_identity::local_device_ura(),
            "apprentice.quote",
        )
        .expect("runtime key");
        let live = crate::support::async_bridge::run_blocking(
            runtime.has_ability(&runtime_key),
            crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
        );
        assert!(
            !live,
            "manifest without [exec] must remain discovery-only, not a runtime fallback"
        );
    }

    #[test]
    fn forget_with_hot_registrar_removes_the_discovery_only_learner_copy() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let runtime = easynet_axon::invocation::LocalRuntime::new();
        let hot_cell = hot_registrar_cell_with_runtime(Arc::clone(&runtime));
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        tokio_runtime
            .block_on(async {
                acquire_handler_with_hot_registrar(
                    acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
                    json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("learner acquires");
        assert!(
            std::path::Path::new(&std::env::var("HOME").unwrap())
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "precondition: acquire persisted the descriptor-import ledger row"
        );
        let runtime_key = crate::ura::owner_ability_ura(
            &crate::runtime::local_invocation_identity::local_device_ura(),
            "apprentice.quote",
        )
        .expect("runtime key");
        assert!(
            !crate::support::async_bridge::run_blocking(
                runtime.has_ability(&runtime_key),
                crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
            ),
            "declaration-only acquired ability must not be a live runtime handler"
        );

        let resp = tokio_runtime
            .block_on(async {
                forget_handler_with_hot_registrar(
                    forget_env(apprentice_ura.clone(), &apprentice_ura),
                    json!({ "ability": "quote", "agent": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("forget");
        assert_eq!(resp["runtime_sync"]["attempted"], true);
        assert_eq!(resp["descriptor_transaction_status"], "committed");
        assert_eq!(resp["runtime_sync"]["status"], "committed");
        assert!(
            !crate::support::async_bridge::run_blocking(
                runtime.has_ability(&runtime_key),
                crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
            ),
            "forget must reconcile stale live LocalRuntime rows"
        );
    }

    #[test]
    fn acquire_runtime_sync_unavailable_returns_committed_degraded() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let unwired_cell = SharedHotRegistrarCell::new();
        let resp = acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            Some(&unwired_cell),
        )
        .expect("durable acquire commits even when live runtime sync is degraded");

        assert_eq!(
            resp["descriptor_transaction_status"],
            "committed_runtime_degraded"
        );
        assert_eq!(resp["runtime_sync"]["status"], "committed_runtime_degraded");
        assert_eq!(resp["runtime_sync"]["reason"], "hot_registrar_not_wired");
        assert!(
            std::path::Path::new(&std::env::var("HOME").unwrap())
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "acquire artifact remains committed when runtime sync is degraded"
        );
    }

    #[test]
    fn forget_runtime_sync_unavailable_returns_error_and_keeps_tombstone() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");
        acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect("learner acquires discovery-only manifest");

        let unwired_cell = SharedHotRegistrarCell::new();
        let err = forget_handler_with_hot_registrar(
            forget_env(apprentice_ura.clone(), &apprentice_ura),
            json!({ "ability": "quote", "agent": "apprentice" }),
            Some(&unwired_cell),
        )
        .expect_err("forget must fail until live runtime cleanup converges");
        let err = err.to_string();
        assert!(
            err.contains("committed_runtime_degraded")
                && err.contains("hot_registrar_not_wired")
                && err.contains("Forgetting state"),
            "forget must return an actionable convergence error: {err}"
        );

        let grants: Value = serde_json::from_slice(
            &std::fs::read(crate::persistence::teach_grants::path()).unwrap(),
        )
        .unwrap();
        let imports = grants["imports"].as_array().unwrap();
        // Compensating-transaction invariant (symmetric with acquire): a
        // degraded runtime sync does NOT retire the ledger tombstone — the
        // row stays in `forgetting` so boot recovery can re-drive the live
        // cleanup. Dropping it here would lose the only retry anchor while
        // a stale ability may still be advertised in the live runtime.
        assert_eq!(
            imports.len(),
            1,
            "tombstone must survive a degraded sync: {grants}"
        );
        assert_eq!(imports[0]["state"], "forgetting", "{grants}");
        assert!(
            !std::path::Path::new(&std::env::var("HOME").unwrap())
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "forget artifact step remains complete when runtime sync is degraded"
        );

        let runtime = easynet_axon::invocation::LocalRuntime::new();
        let hot_cell = hot_registrar_cell_with_runtime(Arc::clone(&runtime));
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let retry = tokio_runtime
            .block_on(async {
                forget_handler_with_hot_registrar(
                    forget_env(apprentice_ura.clone(), &apprentice_ura),
                    json!({ "ability": "quote", "agent": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("retry finishes the forgetting tombstone once runtime sync converges");
        assert_eq!(retry["descriptor_transaction_status"], "committed");
        assert_eq!(retry["runtime_sync"]["status"], "committed");
        assert_eq!(retry["resumed"], true);
        let grants: Value = serde_json::from_slice(
            &std::fs::read(crate::persistence::teach_grants::path()).unwrap(),
        )
        .unwrap();
        assert!(
            grants["imports"].as_array().unwrap().is_empty(),
            "successful retry must retire the tombstone: {grants}"
        );
    }

    #[test]
    fn forget_recovers_committed_acquiring_import_before_removal() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");
        acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect("acquire");

        let imported_manifest = std::path::Path::new(&std::env::var("HOME").unwrap())
            .join("agents/apprentice/abilities/quote.ability.toml");
        mark_first_import_as_acquiring_for_test(&imported_manifest);

        // Wire a live registrar so runtime sync converges (Committed):
        // only then does forget retire the tombstone. Without it the
        // compensating transaction correctly keeps the row degraded (see
        // forget_runtime_sync_unavailable_returns_error_and_keeps_tombstone).
        let runtime = easynet_axon::invocation::LocalRuntime::new();
        let hot_cell = hot_registrar_cell_with_runtime(Arc::clone(&runtime));
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let resp = tokio_runtime
            .block_on(async {
                forget_handler_with_hot_registrar(
                    forget_env(apprentice_ura.clone(), &apprentice_ura),
                    json!({ "ability": "quote", "agent": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("forget must recover the committed acquire before removing");
        assert_eq!(resp["removed_descriptor"], "quote");
        assert_eq!(resp["descriptor_transaction_status"], "committed");
        assert!(
            !imported_manifest.exists(),
            "forget must remove the recovered imported descriptor"
        );
        let grants: Value = serde_json::from_slice(
            &std::fs::read(crate::persistence::teach_grants::path()).unwrap(),
        )
        .unwrap();
        assert!(grants["imports"].as_array().unwrap().is_empty());
    }

    #[test]
    fn descriptor_import_never_overwrites_an_existing_ability() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");
        let home = std::env::var("HOME").unwrap();
        std::fs::write(
            std::path::Path::new(&home).join("agents/apprentice/abilities/quote.ability.toml"),
            "name = \"quote\"\ndescription = \"the apprentice's own quote\"\n\n[input_schema]\ntype = \"object\"\n",
        )
        .expect("native manifest");
        let err = acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect_err("must refuse to clobber");
        assert!(format!("{err}").contains("never overwrites"), "{err}");
    }

    #[test]
    fn descriptor_import_commit_refuses_destination_created_after_stage() {
        let _g = HomeGuard::new();
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let dir = home.join("atomic-import");
        std::fs::create_dir_all(&dir).expect("dir");
        let temp = dir.join(".quote.ability.toml.import.staging");
        let dest = dir.join("quote.ability.toml");
        let staged_bytes = b"name = \"quote\"\ndescription = \"imported\"\n";
        std::fs::write(&temp, staged_bytes).expect("stage");
        std::fs::write(&dest, b"native ability").expect("racing writer");
        let staged = StagedDescriptorImportManifest {
            temp: temp.clone(),
            dest: dest.clone(),
            content_hash: sha256_bytes(staged_bytes),
        };

        let err = staged
            .commit()
            .expect_err("commit must not replace a concurrently-created native ability");

        assert!(
            err.to_string().contains("destination already exists"),
            "{err}"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"native ability");
        assert!(
            temp.exists(),
            "failed no-replace commit leaves the staged artifact available for rollback"
        );
    }

    #[test]
    fn forget_removes_only_imported_descriptors() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");
        acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect("acquire");

        // A native ability is not forgettable…
        let err = forget_handler_with_hot_registrar(
            forget_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "quote", "agent": "mentor" }),
            None,
        )
        .expect_err("mentor never LEARNED quote");
        assert!(format!("{err}").contains("no imported descriptor"), "{err}");

        // ...the imported descriptor copy is. Wire a live registrar so
        // runtime sync converges (Committed) and the forget finalizes;
        // require-forget-runtime-convergence keeps the tombstone otherwise.
        let runtime = easynet_axon::invocation::LocalRuntime::new();
        let hot_cell = hot_registrar_cell_with_runtime(Arc::clone(&runtime));
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        tokio_runtime
            .block_on(async {
                forget_handler_with_hot_registrar(
                    forget_env(apprentice_ura.clone(), &apprentice_ura),
                    json!({ "ability": "quote", "agent": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("forget");
        let home = std::env::var("HOME").unwrap();
        assert!(!std::path::Path::new(&home)
            .join("agents/apprentice/abilities/quote.ability.toml")
            .exists());
        assert!(
            std::path::Path::new(&home)
                .join("agents/mentor/abilities/quote.ability.toml")
                .exists(),
            "the original survives a learner's forget"
        );
    }

    #[test]
    fn forget_rejects_imported_descriptor_manifest_changed_after_acquire() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");
        acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect("acquire");

        let home = std::env::var("HOME").unwrap();
        let imported_descriptor_manifest =
            std::path::Path::new(&home).join("agents/apprentice/abilities/quote.ability.toml");
        std::fs::write(
            &imported_descriptor_manifest,
            "name = \"quote\"\n\
             description = \"mutated learner copy\"\n\n\
             [input_schema]\n\
             type = \"object\"\n",
        )
        .expect("mutate imported descriptor manifest");

        let err = forget_handler_with_hot_registrar(
            forget_env(apprentice_ura.clone(), &apprentice_ura),
            json!({ "ability": "quote", "agent": "apprentice" }),
            None,
        )
        .expect_err("forget must not delete a drifted imported descriptor manifest");
        assert!(
            format!("{err}").contains("refuses to remove imported descriptor manifest"),
            "{err}"
        );
        assert!(
            imported_descriptor_manifest.exists(),
            "refused forget must leave the drifted file for operator inspection"
        );
    }

    #[test]
    fn acquire_refuses_callers_that_do_not_authorize_the_learner() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");

        let stranger = crate::ura::agent_ura("localhost", "dev", "stranger");
        let err = acquire_handler_with_hot_registrar(
            acquire_env(stranger, &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect_err("stranger must not install into apprentice workspace");
        assert!(
            format!("{err}").contains("cannot mutate abilities for agent"),
            "{err}"
        );
    }

    #[test]
    fn forget_refuses_callers_that_do_not_authorize_the_learner() {
        let _g = HomeGuard::new();
        let (source_descriptor_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");
        acquire_handler_with_hot_registrar(
            acquire_env(apprentice_ura.clone(), &source_descriptor_ura),
            json!({ "ability_ura": source_descriptor_ura, "learner": "apprentice" }),
            None,
        )
        .expect("acquire");

        let stranger = crate::ura::agent_ura("localhost", "dev", "stranger");
        let err = forget_handler_with_hot_registrar(
            forget_env(stranger, &apprentice_ura),
            json!({ "ability": "quote", "agent": "apprentice" }),
            None,
        )
        .expect_err("stranger must not delete apprentice workspace");
        assert!(
            format!("{err}").contains("cannot mutate abilities for agent"),
            "{err}"
        );
    }

    #[test]
    fn teach_refuses_unknown_abilities_precisely() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        let err = teach_handler(
            teach_env_for(mentor_ura.clone(), &mentor_ura, "ghost"),
            json!({ "ability": "mentor.ghost", "learner_ura": apprentice_ura }),
        )
        .expect_err("no such manifest");
        assert!(format!("{err}").contains("publishes no ability"), "{err}");
    }

    #[test]
    fn teach_rejects_manifest_name_that_does_not_match_owner_registry_key() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed_with_mentor_manifest(
            "name = \"renamed\"\n\
             description = \"wrong identity\"\n\n\
             [input_schema]\n\
             type = \"object\"\n",
        );
        let err = teach_handler(
            teach_env(mentor_ura.clone(), &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect_err("manifest identity must match the owner-local registry key");
        assert!(
            format!("{err}").contains("filename, ability URA, and manifest identity to match"),
            "{err}"
        );
        assert!(
            !crate::persistence::teach_grants::path().exists(),
            "rejected teach must not create a grant file"
        );
    }

    #[test]
    fn teach_refuses_callers_that_are_not_the_owner_or_host_authority() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        let stranger = crate::ura::agent_ura("localhost", "dev", "stranger");

        let err = teach_handler(
            teach_env(stranger, &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect_err("unauthorized caller must not write a grant");
        assert!(
            format!("{err}").contains("cannot teach abilities advertised by"),
            "{err}"
        );
    }

    #[test]
    fn teach_refuses_unsigned_host_device_authority_for_a_local_owner() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        let host = crate::ura::device_ura("test", "local");

        let err = teach_handler(
            teach_env(host, &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect_err("unsigned host device authority must not write grants");
        assert!(
            format!("{err}").contains("signed hosted-agent delegation"),
            "{err}"
        );
    }

    #[test]
    fn teach_allows_signed_hosted_agent_delegation_for_a_local_owner() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        let host = crate::ura::device_ura("test", "local");

        let resp = teach_handler(
            teach_env_with_hosted_delegation(host.clone(), &mentor_ura, &mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("signed host device delegation authorizes the local owner");
        assert_eq!(resp["granted_by"], host);
    }
}
