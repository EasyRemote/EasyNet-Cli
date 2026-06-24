// EasyNet CLI — `meta.teach` / `meta.acquire` / `meta.forget`
// =============================================================
//
// File: src/runtime/agents/teach_ability.rs
// Description: GET route B (seven-axes T3.3, spec §2.5) — capability
//              transfer as the ONTOLOGY defines it: `learn` is the
//              reflexive `meta.acquire` (ONTOLOGY_AGENT_ABILITY:220),
//              `forget` its inverse, and `teach` the cross-owner half
//              that makes either possible. The initiative is always
//              the owner's: no grant in the teach directory means
//              `allow_transferred_code = false` (the InstallPolicy
//              default, capability.proto:235-239) and `meta.acquire`
//              refuses.
//
// v1 scope (spec D6 ruling): same-device, manifest-only. A learn
// copies the taught manifest into the learner agent's workspace —
// the learner's hosted URA (RFC-005 §1.4 mint) then makes the copy a
// NEW ability under the learner's own identity: discover projects
// it, `exactly one owner` holds for both copies, two receipt chains
// account separately. Cross-device transfer rides PayloadTransfer in
// a follow-up; `--with-assets` likewise.
//
// Execution posture: the grant default is `execution_mode =
// "sandbox_first"` (capability.proto:238). Until the runtime boundary
// can prove that posture for transferred code, acquire is deliberately
// fail-closed for any taught manifest that carries `[exec]`; only
// discovery-only manifest transfer is admitted.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::core::ability_spec::{AbilityExec, AbilityManifest};
use crate::persistence::teach_grants::{
    AcquiringArtifactRecoveryState, AcquiringArtifactTxn, LearnedRecord, TeachGrant,
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
static LEARN_STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
struct StagedLearnedManifest {
    temp: PathBuf,
    dest: PathBuf,
    content_hash: [u8; 32],
}

impl AcquiringArtifactTxn for StagedLearnedManifest {
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
        commit_learned_manifest(self)
    }

    fn rollback(&self) -> anyhow::Result<()> {
        rollback_learned_manifest(self)
    }
}

#[derive(Debug, Clone)]
struct StagedForgottenManifest {
    temp: Option<PathBuf>,
    dest: Option<PathBuf>,
}

pub fn teach_description() -> &'static str {
    "Grant one local agent permission to acquire a specific advertised ability. \
     The grant is explicit, same-device, and does not execute transferred code."
}

pub fn acquire_description() -> &'static str {
    "Acquire a previously taught ability into a learner agent workspace, minting \
     a separate learner-owned copy under the learner's authority projection."
}

pub fn forget_description() -> &'static str {
    "Remove a learned ability copy from an agent workspace. Native authored \
     abilities are not removed through this surface."
}

pub fn teach_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["ability", "learner_ura"],
        "properties": {
            "ability": {
                "type": "string",
                "description": "Owner-local ability name to teach, for example mentor.quote."
            },
            "learner_ura": {
                "type": "string",
                "description": "Agent URA allowed to acquire the ability."
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
                "description": "Canonical ability URA previously granted to the learner."
            },
            "learner": {
                "type": "string",
                "description": "Local learner agent name whose workspace receives the learned copy."
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
                "description": "Public learned ability name to remove from the agent workspace."
            },
            "agent": {
                "type": "string",
                "description": "Local agent name that previously learned the ability."
            }
        },
        "additionalProperties": false
    })
}

pub fn register(reg: &mut AxonAbilityCatalog, hot_registrar: Arc<SharedHotRegistrarCell>) {
    reg.register_rpc_with_envelope_and_owner(TEACH, OwnerKind::Device, Arc::new(teach_handler));
    let registrar_for_acquire = Arc::clone(&hot_registrar);
    reg.register_rpc_with_envelope_and_owner(
        ACQUIRE,
        OwnerKind::Device,
        Arc::new(move |env, args| {
            acquire_handler_with_hot_registrar(env, args, Some(&registrar_for_acquire))
        }),
    );
    let registrar_for_forget = Arc::clone(&hot_registrar);
    reg.register_rpc_with_envelope_and_owner(
        FORGET,
        OwnerKind::Device,
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
struct TaughtManifestSnapshot {
    bytes: Vec<u8>,
    manifest: AbilityManifest,
    manifest_hash: String,
}

impl TaughtManifestSnapshot {
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
/// Clean v1 rule: the caller must be either the advertising Agent URA or
/// the host device named by that Agent's persisted signing authority.
/// This keeps local operator UX working without allowing an arbitrary
/// admitted caller to write grants for someone else's ability.
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
        });
    }

    if caller == owner_entry.agent_ura {
        return Ok(OwnerAuthority {
            owner_ura: owner_entry.agent_ura.clone(),
            granted_by: caller.to_string(),
        });
    }

    let expected_host = format!("hosted_by:{caller}");
    if owner_entry.signing_authority == expected_host {
        return Ok(OwnerAuthority {
            owner_ura: owner_entry.agent_ura.clone(),
            granted_by: caller.to_string(),
        });
    }

    anyhow::bail!(
        "{TEACH} caller {caller:?} cannot teach abilities advertised by \
         {owner_agent:?}; expected advertising agent {} or its host device authority",
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

    let expected_host = format!("hosted_by:{caller}");
    if entry.signing_authority == expected_host {
        return Ok(caller.to_string());
    }

    anyhow::bail!(
        "{surface} caller {caller:?} cannot mutate abilities for agent {agent_name:?}; \
         expected the learner Agent URA {agent_ura} or its host device authority"
    );
}

/// `meta.teach { ability, learner_ura }` — the owner confers
/// learnability of one ability to ONE learner. Idempotent per
/// (ability, learner): re-teaching refreshes the grant.
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
        anyhow::bail!("learner_ura must be an Agent URA — abilities are taught to agents");
    }
    let local = crate::persistence::local_agents::load()?;
    hosted_agent_by_ura(&local, learner_ura, TEACH)?;

    let home = resolve_owner_manifest(ability)?;
    let snapshot = TaughtManifestSnapshot::from_home(&home)?;
    let authority = require_owner_authority(&env, &home.owner_agent)?;
    let ability_ura = crate::ura::owner_ability_ura(&authority.owner_ura, &home.public_name)
        .ok_or_else(|| anyhow::anyhow!("could not mint the taught ability URA"))?;
    TeachGrantStore::open_default().grant(TeachGrant::new(
        ability,
        ability_ura.clone(),
        authority.owner_ura.clone(),
        home.owner_agent.clone(),
        learner_ura,
        snapshot.manifest_hash.clone(),
        EXECUTION_MODE_DEFAULT,
        clock.now_rfc3339(),
    ))?;

    Ok(json!({
        "taught": ability,
        "ability_ura": ability_ura,
        "owner_agent": home.owner_agent,
        "owner_ura": authority.owner_ura,
        "granted_by": authority.granted_by,
        "learner_ura": learner_ura,
        "manifest_hash": snapshot.manifest_hash,
        "execution_mode": EXECUTION_MODE_DEFAULT,
    }))
}

fn stage_path_for(dest: &Path, operation: &str) -> anyhow::Result<PathBuf> {
    let file_name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("learned manifest path has no file name"))?;
    if operation == "learn" {
        let seq = LEARN_STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
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

fn stage_learned_manifest(
    snapshot: &TaughtManifestSnapshot,
    source: &Path,
    dest_dir: &Path,
    dest: &Path,
    learner: &str,
    public_name: &str,
    execution_mode: &str,
) -> anyhow::Result<StagedLearnedManifest> {
    if dest.exists() {
        anyhow::bail!(
            "agent {learner:?} already has an ability named {public_name:?}; \
             forget it first or rename — learning never overwrites"
        );
    }
    let bytes = snapshot.transferable_bytes(execution_mode, source)?;
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| anyhow::anyhow!("create {}: {e}", dest_dir.display()))?;
    let temp = stage_path_for(dest, "learn")?;
    remove_file_if_exists(&temp, "remove stale learned staging manifest")?;
    write_staged_file(&temp, bytes)?;
    Ok(StagedLearnedManifest {
        temp,
        dest: dest.to_path_buf(),
        content_hash: sha256_bytes(bytes),
    })
}

fn commit_learned_manifest(staged: &StagedLearnedManifest) -> anyhow::Result<()> {
    if staged.dest.exists() {
        anyhow::bail!(
            "commit learned manifest {}: destination already exists",
            staged.dest.display()
        );
    }
    std::fs::rename(&staged.temp, &staged.dest).map_err(|e| {
        anyhow::anyhow!(
            "commit learned manifest {} -> {}: {e}",
            staged.temp.display(),
            staged.dest.display()
        )
    })?;
    sync_parent_dir(&staged.dest)
}

fn rollback_learned_manifest(staged: &StagedLearnedManifest) -> anyhow::Result<()> {
    if staged.dest.exists() {
        let bytes = std::fs::read(&staged.dest).map_err(|e| {
            anyhow::anyhow!(
                "read committed learned manifest {} before rollback: {e}",
                staged.dest.display()
            )
        })?;
        if sha256_bytes(&bytes) != staged.content_hash {
            anyhow::bail!(
                "refusing to rollback learned manifest {} because its content no longer \
                 matches this transaction's staged bytes",
                staged.dest.display()
            );
        }
        return remove_file_if_exists(&staged.dest, "remove committed learned manifest");
    }
    remove_file_if_exists(&staged.temp, "remove staged learned manifest")
}

fn recover_acquiring_learned_manifests() -> anyhow::Result<()> {
    TeachGrantStore::open_default().recover_acquiring(|record| {
        let path = record.acquiring_manifest_path().ok_or_else(|| {
            anyhow::anyhow!("recover acquiring learned manifest: missing committed path")
        })?;
        let expected_hash = record.acquiring_manifest_hash().ok_or_else(|| {
            anyhow::anyhow!("recover acquiring learned manifest {path}: missing content hash")
        })?;
        let path = Path::new(path);
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let staging = record.acquiring_staging_manifest_path().ok_or_else(|| {
                    anyhow::anyhow!(
                        "recover acquiring learned manifest {}: committed file is absent and \
                         acquiring row has no staging path to clean",
                        path.display()
                    )
                })?;
                remove_file_if_exists(
                    Path::new(staging),
                    "remove stale acquiring learned staging manifest",
                )?;
                return Ok(AcquiringArtifactRecoveryState::NotCommitted);
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "recover acquiring learned manifest {}: read failed: {err}",
                    path.display()
                ));
            }
        };
        let actual_hash = sha256_hex(sha256_bytes(&bytes));
        if actual_hash != expected_hash {
            anyhow::bail!(
                "recover acquiring learned manifest {}: hash mismatch (ledger {}, disk {})",
                path.display(),
                expected_hash,
                actual_hash
            );
        }
        Ok(AcquiringArtifactRecoveryState::Committed)
    })?;
    Ok(())
}

fn stage_forget_manifest(
    path: Option<&Path>,
    expected_hash: Option<&str>,
) -> anyhow::Result<StagedForgottenManifest> {
    let Some(dest) = path else {
        return Ok(StagedForgottenManifest {
            temp: None,
            dest: None,
        });
    };
    let temp = stage_path_for(dest, "forget")?;
    if temp.exists() {
        if !dest.exists() {
            return Ok(StagedForgottenManifest {
                temp: Some(temp),
                dest: Some(dest.to_path_buf()),
            });
        }
        remove_file_if_exists(&temp, "remove stale forgotten staging manifest")?;
    }
    if !dest.exists() {
        return Ok(StagedForgottenManifest {
            temp: None,
            dest: Some(dest.to_path_buf()),
        });
    }
    if let Some(expected_hash) = expected_hash {
        let bytes = std::fs::read(dest).map_err(|e| {
            anyhow::anyhow!(
                "read learned manifest {} before forget hash check: {e}",
                dest.display()
            )
        })?;
        let actual_hash = sha256_hex(sha256_bytes(&bytes));
        if actual_hash != expected_hash {
            anyhow::bail!(
                "{FORGET} refuses to remove learned manifest {} because ledger hash {} does not \
                 match disk hash {}; inspect the file before retrying",
                dest.display(),
                expected_hash,
                actual_hash
            );
        }
    }
    std::fs::rename(dest, &temp).map_err(|e| {
        anyhow::anyhow!(
            "stage learned manifest removal {} -> {}: {e}",
            dest.display(),
            temp.display()
        )
    })?;
    if let Err(sync_err) = sync_parent_dir(dest) {
        let restore = std::fs::rename(&temp, dest)
            .map_err(|e| {
                anyhow::anyhow!(
                    "restore learned manifest after failed forget fsync {} -> {}: {e}",
                    temp.display(),
                    dest.display()
                )
            })
            .and_then(|()| sync_parent_dir(dest));
        return Err(append_cleanup_error(
            sync_err,
            restore,
            "restore learned manifest after failed forget staging fsync",
        ));
    }
    Ok(StagedForgottenManifest {
        temp: Some(temp),
        dest: Some(dest.to_path_buf()),
    })
}

fn commit_forget_manifest(staged: &StagedForgottenManifest) -> anyhow::Result<()> {
    let Some(temp) = staged.temp.as_ref() else {
        return Ok(());
    };
    remove_file_if_exists(temp, "remove staged forgotten manifest")
}

fn rollback_forget_manifest(staged: &StagedForgottenManifest) -> anyhow::Result<()> {
    let (Some(temp), Some(dest)) = (staged.temp.as_ref(), staged.dest.as_ref()) else {
        return Ok(());
    };
    if dest.exists() {
        return rollback_staged_file(temp);
    }
    std::fs::rename(temp, dest).map_err(|e| {
        anyhow::anyhow!(
            "restore learned manifest {} -> {}: {e}",
            temp.display(),
            dest.display()
        )
    })?;
    sync_parent_dir(dest)
}

fn rollback_staged_file(path: &Path) -> anyhow::Result<()> {
    remove_file_if_exists(path, "remove stale staged manifest")
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
                 sandbox_first is not yet an enforced runtime boundary. Acquire currently \
                 admits discovery-only manifest transfer only; remove [exec] before teaching \
                 or deploy executable code through the device ability deployment path.",
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

/// `meta.acquire { ability_ura, learner }` — the learner (a local
/// agent) acquires a taught ability and becomes the owner of its own
/// copy, under its own URA.
#[cfg(test)]
fn acquire_handler(env: EnvelopeContext, args: Value) -> anyhow::Result<Value> {
    acquire_handler_with_hot_registrar(env, args, None)
}

fn acquire_handler_with_hot_registrar(
    env: EnvelopeContext,
    args: Value,
    hot_registrar: Option<&SharedHotRegistrarCell>,
) -> anyhow::Result<Value> {
    acquire_handler_with_hot_registrar_and_clock(env, args, hot_registrar, &SystemTeachClock)
}

fn acquire_handler_with_hot_registrar_and_clock(
    env: EnvelopeContext,
    args: Value,
    hot_registrar: Option<&SharedHotRegistrarCell>,
    clock: &dyn TeachClock,
) -> anyhow::Result<Value> {
    AcquireWorkflow::new(env, args, hot_registrar, clock).run()
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
        committed.sync_runtime(hot_registrar)?.into_response()
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
                "agent {:?} already has an ability named {:?}; forget it first or rename — \
                 learning never overwrites",
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
        recover_acquiring_learned_manifests()?;
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
        let snapshot = TaughtManifestSnapshot::from_home(&plan.owner_home)?;
        snapshot.require_hash(grant.manifest_hash(), &plan.owner_home.manifest_path)?;
        let staged = stage_learned_manifest(
            &snapshot,
            &plan.owner_home.manifest_path,
            &plan.dest_dir,
            &plan.dest,
            &plan.request.learner,
            &plan.request.public_name,
            grant.execution_mode(),
        )?;
        let learned = LearnedRecord::new(
            &plan.request.public_name,
            &plan.request.learner,
            &plan.request.ability_ura,
            grant.manifest_hash(),
            clock.now_rfc3339(),
        );
        let acquired = TeachGrantStore::open_default().acquire_staged(
            &plan.request.registry_name,
            &plan.request.ability_ura,
            &plan.request.owner_ura,
            &plan.learner_ura,
            &grant,
            learned,
            staged,
        )?;
        Ok(CommittedAcquire { plan, acquired })
    }
}

#[derive(Debug)]
struct CommittedAcquire {
    plan: AuthorizedAcquire,
    acquired: crate::persistence::teach_grants::AcquiredTeachGrant,
}

impl CommittedAcquire {
    fn sync_runtime(
        self,
        hot_registrar: Option<&SharedHotRegistrarCell>,
    ) -> anyhow::Result<RuntimeSyncedAcquire> {
        let new_ura =
            crate::ura::owner_ability_ura(&self.plan.learner_ura, &self.plan.request.public_name)
                .ok_or_else(|| anyhow::anyhow!("could not mint the learner's ability URA"))?;
        let runtime_sync = match sync_learner_runtime_after_acquire(
            hot_registrar,
            &self.plan.request.learner,
            &self.plan.learner_entry_for_runtime,
        ) {
            Ok(value) => value,
            Err(sync_err) => {
                let rollback = rollback_acquire_after_runtime_sync_failure(
                    &self.acquired,
                    hot_registrar,
                    &self.plan.request.learner,
                    &self.plan.dest,
                    &self.plan.learner_entry_for_runtime,
                );
                return Err(append_cleanup_error(
                    sync_err,
                    rollback,
                    "rollback learned ability after runtime sync failure",
                ));
            }
        };
        Ok(RuntimeSyncedAcquire {
            plan: self.plan,
            acquired: self.acquired,
            new_ura,
            runtime_sync,
        })
    }
}

#[derive(Debug)]
struct RuntimeSyncedAcquire {
    plan: AuthorizedAcquire,
    acquired: crate::persistence::teach_grants::AcquiredTeachGrant,
    new_ura: String,
    runtime_sync: Value,
}

impl RuntimeSyncedAcquire {
    fn into_response(self) -> anyhow::Result<Value> {
        Ok(json!({
            "acquired": self.plan.request.public_name,
            "new_ura": self.new_ura,
            "learned_from": self.plan.request.ability_ura,
            "execution_mode": self.acquired.grant().execution_mode(),
            "manifest_hash": self.acquired.learned().manifest_hash(),
            "transfer_kind": "discovery_only_manifest",
            "invokable": false,
            "invocation_status": "not_invokable_without_exec_binding",
            "mutated_by": self.plan.mutated_by,
            "runtime_sync": self.runtime_sync,
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
        "not teachable (allow_transferred_code=false): the owner has not taught {ability_ura:?} \
         ({registry_name:?}) from owner {owner_ura:?} to {learner_ura}"
    )
}

fn rollback_acquire_after_runtime_sync_failure(
    acquired: &crate::persistence::teach_grants::AcquiredTeachGrant,
    hot_registrar: Option<&SharedHotRegistrarCell>,
    learner: &str,
    manifest_path: &Path,
    entry: &crate::registry::agents::AgentEntry,
) -> anyhow::Result<()> {
    TeachGrantStore::open_default().restore_acquired_grant_after_failure(
        acquired,
        |_| {
            stage_forget_manifest(
                Some(manifest_path),
                Some(acquired.learned().manifest_hash()),
            )
        },
        commit_forget_manifest,
        rollback_forget_manifest,
    )?;
    sync_learner_runtime_after_forget(hot_registrar, learner, entry).map(|_| ())
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

fn sync_learner_runtime_after_acquire(
    hot_registrar: Option<&SharedHotRegistrarCell>,
    learner: &str,
    entry: &crate::registry::agents::AgentEntry,
) -> anyhow::Result<Value> {
    let Some(cell) = hot_registrar else {
        return Ok(json!({
            "attempted": false,
            "runtime_not_ready": true,
            "reason": "hot_registrar_not_provided",
        }));
    };
    let Some(registrar) = cell.get().cloned() else {
        anyhow::bail!(
            "{ACQUIRE}: learned manifest landed on disk, but the hot registrar is not \
             wired; retry after daemon boot completes"
        );
    };
    let learner_name = learner.to_string();
    let entry_for_runtime = entry.clone();
    let outcome = block_on_hot_registrar(async move {
        registrar
            .register_agent(&learner_name, &entry_for_runtime)
            .await
    })
    .ok_or_else(|| {
        anyhow::anyhow!(
            "{ACQUIRE}: hot_registrar is wired but no tokio runtime is available on the \
             calling thread; learned manifest was persisted but not live-registered"
        )
    })?;
    ensure_runtime_sync_succeeded(ACQUIRE, learner, outcome)?;
    Ok(runtime_sync_value(outcome))
}

fn runtime_sync_value(outcome: HotAgentRuntimeSyncOutcome) -> Value {
    json!({
        "attempted": true,
        "registered": outcome.registered,
        "replaced": outcome.replaced,
        "failed": outcome.failed,
        "removed": outcome.removed,
        "runtime_not_ready": outcome.runtime_not_ready,
        "rejected_reserved_owner": outcome.rejected_reserved_owner,
    })
}

fn ensure_runtime_sync_succeeded(
    surface: &str,
    learner: &str,
    outcome: HotAgentRuntimeSyncOutcome,
) -> anyhow::Result<()> {
    if outcome.runtime_not_ready || outcome.rejected_reserved_owner || outcome.failed > 0 {
        anyhow::bail!(
            "{surface}: persisted the learned manifest for {learner:?}, but LocalRuntime \
             reconciliation failed (runtime_not_ready={}, rejected_reserved_owner={}, failed={})",
            outcome.runtime_not_ready,
            outcome.rejected_reserved_owner,
            outcome.failed
        );
    }
    Ok(())
}

/// `meta.forget { ability, agent }` — drop a LEARNED ability. The
/// learned ledger is the authority: a native ability never matches
/// it, so forget can never silently delete what an agent authored.
#[cfg(test)]
fn forget_handler(env: EnvelopeContext, args: Value) -> anyhow::Result<Value> {
    forget_handler_with_hot_registrar(env, args, None)
}

fn forget_handler_with_hot_registrar(
    env: EnvelopeContext,
    args: Value,
    hot_registrar: Option<&SharedHotRegistrarCell>,
) -> anyhow::Result<Value> {
    let ability = required_str(&args, "ability", FORGET)?;
    let agent = required_str(&args, "agent", FORGET)?;

    let agents = crate::registry::agents::load_agents()?;
    let agent_entry_for_runtime = agents.agents.get(agent).cloned();
    let local = crate::persistence::local_agents::load()?;
    let agent_ura = hosted_agent_by_name(&local, agent, FORGET)?
        .agent_ura
        .clone();
    let mutated_by = require_hosted_agent_authority(&env, &local, agent, &agent_ura, FORGET)?;
    let manifest = agents
        .agents
        .get(agent)
        .and_then(|e| e.root_path.as_ref())
        .map(|root| {
            root.join("abilities")
                .join(format!("{ability}.ability.toml"))
        });
    let store = TeachGrantStore::open_default();
    let staged = store.stage_forget(agent, ability, |record| {
        stage_forget_manifest(manifest.as_deref(), Some(record.manifest_hash()))
    })?;

    let runtime_pending = store.commit_forget_artifact(&staged, commit_forget_manifest)?;
    let runtime_sync = match agent_entry_for_runtime.as_ref() {
        Some(entry) => sync_learner_runtime_after_forget(hot_registrar, agent, entry)
            .with_context(|| {
                format!(
                    "{FORGET}: durable tombstone for agent {agent:?} ability {ability:?} is \
                     still present; retry forget after runtime reconciliation is available"
                )
            })?,
        None => json!({
            "attempted": false,
            "runtime_not_ready": true,
            "reason": "agent_registry_entry_missing",
        }),
    };
    let committed = store.finish_forget(&runtime_pending)?;
    let record = committed.record();

    Ok(json!({
        "forgotten": ability,
        "agent": agent,
        "had_learned_from": record.learned_from(),
        "mutated_by": mutated_by,
        "runtime_sync": runtime_sync,
        "resumed": staged.resumed(),
    }))
}

fn sync_learner_runtime_after_forget(
    hot_registrar: Option<&SharedHotRegistrarCell>,
    learner: &str,
    entry: &crate::registry::agents::AgentEntry,
) -> anyhow::Result<Value> {
    let Some(cell) = hot_registrar else {
        return Ok(json!({
            "attempted": false,
            "runtime_not_ready": true,
            "reason": "hot_registrar_not_provided",
        }));
    };
    let Some(registrar) = cell.get().cloned() else {
        anyhow::bail!(
            "{FORGET}: learned manifest was removed on disk, but the hot registrar is not \
             wired; retry after daemon boot completes"
        );
    };
    let learner_name = learner.to_string();
    let entry_for_runtime = entry.clone();
    let outcome = block_on_hot_registrar(async move {
        registrar
            .register_agent(&learner_name, &entry_for_runtime)
            .await
    })
    .ok_or_else(|| {
        anyhow::anyhow!(
            "{FORGET}: hot_registrar is wired but no tokio runtime is available on the \
             calling thread; learned manifest was removed but stale runtime row may \
             survive until daemon restart"
        )
    })?;
    ensure_runtime_sync_succeeded(FORGET, learner, outcome)?;
    Ok(runtime_sync_value(outcome))
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
            host_device_agent_ura: crate::ura::device_ura("localhost", "dev-1"),
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

        let taught_ura =
            crate::ura::owner_ability_ura(&mentor_ura, "quote").expect("mentor ability URA");
        (taught_ura, apprentice_ura, mentor_ura)
    }

    fn caller_env(caller: impl Into<String>) -> EnvelopeContext {
        EnvelopeContext::for_test(caller, "easynet:///r/test/device/local")
    }

    fn hot_registrar_cell_with_runtime(
        runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ) -> SharedHotRegistrarCell {
        let registrar =
            crate::runtime::axon_bridge::hot_agent_registrar::HotAgentRegistrar::new_pending(
                Arc::new(Vec::new()),
                Arc::new(std::sync::OnceLock::new()),
                Arc::new(
                    crate::runtime::agents::discover_ability::BridgeDiscoverFederationResolver,
                ),
            );
        registrar.set_runtime(runtime);
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

    #[test]
    fn teach_ledger_uses_the_explicit_clock() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        let ts = "2026-06-23T01:02:03Z";

        teach_handler_with_clock(
            caller_env(mentor_ura.clone()),
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
            caller_env(mentor_ura),
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
        let (taught_ura, _, _) = seed();
        let err = acquire_handler(
            caller_env(crate::ura::agent_ura("localhost", "dev", "apprentice")),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
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
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let forged_owner = crate::ura::agent_ura("localhost", "other-user", "mentor");
        let forged_ability =
            crate::ura::owner_ability_ura(&forged_owner, "quote").expect("forged ability URA");
        let err = acquire_handler(
            caller_env(apprentice_ura),
            json!({ "ability_ura": forged_ability, "learner": "apprentice" }),
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
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();

        let teach = teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("owner teaches");
        assert_eq!(teach["owner_ura"], teach["granted_by"]);
        let resp = acquire_handler(
            caller_env(apprentice_ura.clone()),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect("learner acquires");

        // The learner owns a NEW ability under its own URA…
        let new_ura = resp["new_ura"].as_str().expect("new_ura");
        let sel = AbilitySelector::parse(new_ura).expect("new URA round-trips");
        assert_eq!(sel.owner_kind(), "agent");
        assert_eq!(sel.owner_ura(), apprentice_ura, "owner is the learner now");
        assert_eq!(resp["execution_mode"], EXECUTION_MODE_DEFAULT);

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
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            caller_env(mentor_ura.clone()),
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

        let err = acquire_handler(
            caller_env(apprentice_ura.clone()),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect_err("acquire must be bound to the manifest taught by the owner");
        assert!(format!("{err}").contains("teach grant pinned"), "{err}");
        assert!(
            !std::path::Path::new(&home)
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "hash mismatch must not mint a learner copy"
        );
        assert!(
            TeachGrantStore::open_default()
                .grant_for("mentor.quote", &taught_ura, &mentor_ura, &apprentice_ura)
                .unwrap()
                .is_some(),
            "hash mismatch must not consume the grant"
        );
    }

    #[test]
    fn acquire_blocks_executable_transfer_before_staging() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            caller_env(mentor_ura.clone()),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");

        let err = acquire_handler(
            caller_env(apprentice_ura.clone()),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect_err("executable transfers are blocked until sandbox_first is enforced");

        assert!(
            format!("{err}").contains("refuses executable transferred ability"),
            "{err}"
        );
        assert!(
            format!("{err}").contains("sandbox_first is not yet an enforced runtime boundary"),
            "{err}"
        );
        let home = std::env::var("HOME").unwrap();
        assert!(
            !std::path::Path::new(&home)
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "blocked acquire must not stage or commit a learned manifest"
        );
        assert!(
            TeachGrantStore::open_default()
                .grant_for("mentor.quote", &taught_ura, &mentor_ura, &apprentice_ura)
                .unwrap()
                .is_some(),
            "blocked acquire must not consume the teach grant"
        );
    }

    #[test]
    fn acquire_with_hot_registrar_does_not_mount_declaration_only_copy() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            caller_env(mentor_ura),
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
                    caller_env(apprentice_ura),
                    json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("learner acquires metadata-only ability");

        assert_eq!(resp["runtime_sync"]["attempted"], true);
        assert_eq!(resp["runtime_sync"]["failed"], 0);
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
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            caller_env(mentor_ura),
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
                    caller_env(apprentice_ura.clone()),
                    json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("learner acquires");
        assert!(
            std::path::Path::new(&std::env::var("HOME").unwrap())
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "precondition: acquire persisted the learned discovery row"
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
                    caller_env(apprentice_ura),
                    json!({ "ability": "quote", "agent": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("forget");
        assert_eq!(resp["runtime_sync"]["attempted"], true);
        assert!(
            !crate::support::async_bridge::run_blocking(
                runtime.has_ability(&runtime_key),
                crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
            ),
            "forget must reconcile stale live LocalRuntime rows"
        );
    }

    #[test]
    fn forget_runtime_sync_failure_keeps_retryable_tombstone() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("owner teaches");
        acquire_handler(
            caller_env(apprentice_ura.clone()),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect("learner acquires discovery-only manifest");

        let unwired_cell = SharedHotRegistrarCell::new();
        let err = forget_handler_with_hot_registrar(
            caller_env(apprentice_ura.clone()),
            json!({ "ability": "quote", "agent": "apprentice" }),
            Some(&unwired_cell),
        )
        .expect_err("unwired runtime reconciliation must not report forget success");
        let err = err.to_string();
        assert!(err.contains("durable tombstone"), "{err}");

        let grants: Value = serde_json::from_slice(
            &std::fs::read(crate::persistence::teach_grants::path()).unwrap(),
        )
        .unwrap();
        let learned = grants["learned"].as_array().expect("learned ledger rows");
        assert_eq!(learned.len(), 1);
        assert_eq!(learned[0]["state"], "forgetting");
        assert!(
            !std::path::Path::new(&std::env::var("HOME").unwrap())
                .join("agents/apprentice/abilities/quote.ability.toml")
                .exists(),
            "artifact cleanup may have committed, but ledger must keep the retryable tombstone"
        );

        let runtime = easynet_axon::invocation::LocalRuntime::new();
        let hot_cell = hot_registrar_cell_with_runtime(Arc::clone(&runtime));
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let resp = tokio_runtime
            .block_on(async {
                forget_handler_with_hot_registrar(
                    caller_env(apprentice_ura),
                    json!({ "ability": "quote", "agent": "apprentice" }),
                    Some(&hot_cell),
                )
            })
            .expect("retry finishes runtime cleanup and tombstone finalization");
        assert_eq!(resp["resumed"], true);
        let grants: Value = serde_json::from_slice(
            &std::fs::read(crate::persistence::teach_grants::path()).unwrap(),
        )
        .unwrap();
        assert!(grants["learned"].as_array().unwrap().is_empty());
    }

    #[test]
    fn learning_never_overwrites_an_existing_ability() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            caller_env(mentor_ura.clone()),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("teach");
        let home = std::env::var("HOME").unwrap();
        std::fs::write(
            std::path::Path::new(&home).join("agents/apprentice/abilities/quote.ability.toml"),
            "name = \"quote\"\ndescription = \"the apprentice's own quote\"\n\n[input_schema]\ntype = \"object\"\n",
        )
        .expect("native manifest");
        let err = acquire_handler(
            caller_env(apprentice_ura.clone()),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect_err("must refuse to clobber");
        assert!(format!("{err}").contains("never overwrites"), "{err}");
    }

    #[test]
    fn forget_removes_only_what_was_learned() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            caller_env(mentor_ura.clone()),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("teach");
        acquire_handler(
            caller_env(apprentice_ura.clone()),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect("acquire");

        // A native ability is not forgettable…
        let err = forget_handler(
            caller_env(mentor_ura),
            json!({ "ability": "quote", "agent": "mentor" }),
        )
        .expect_err("mentor never LEARNED quote");
        assert!(format!("{err}").contains("never learned"), "{err}");

        // …the learned copy is.
        forget_handler(
            caller_env(apprentice_ura),
            json!({ "ability": "quote", "agent": "apprentice" }),
        )
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
    fn forget_rejects_learned_manifest_changed_after_acquire() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");
        acquire_handler(
            caller_env(apprentice_ura.clone()),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect("acquire");

        let home = std::env::var("HOME").unwrap();
        let learned_manifest =
            std::path::Path::new(&home).join("agents/apprentice/abilities/quote.ability.toml");
        std::fs::write(
            &learned_manifest,
            "name = \"quote\"\n\
             description = \"mutated learner copy\"\n\n\
             [input_schema]\n\
             type = \"object\"\n",
        )
        .expect("mutate learned manifest");

        let err = forget_handler(
            caller_env(apprentice_ura),
            json!({ "ability": "quote", "agent": "apprentice" }),
        )
        .expect_err("forget must not delete a drifted learned manifest");
        assert!(
            format!("{err}").contains("refuses to remove learned manifest"),
            "{err}"
        );
        assert!(
            learned_manifest.exists(),
            "refused forget must leave the drifted file for operator inspection"
        );
    }

    #[test]
    fn acquire_refuses_callers_that_do_not_authorize_the_learner() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("teach");

        let stranger = crate::ura::agent_ura("localhost", "dev", "stranger");
        let err = acquire_handler(
            caller_env(stranger),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
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
        let (taught_ura, apprentice_ura, mentor_ura) = seed_declaration_only();
        teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura.clone() }),
        )
        .expect("teach");
        acquire_handler(
            caller_env(apprentice_ura),
            json!({ "ability_ura": taught_ura, "learner": "apprentice" }),
        )
        .expect("acquire");

        let stranger = crate::ura::agent_ura("localhost", "dev", "stranger");
        let err = forget_handler(
            caller_env(stranger),
            json!({ "ability": "quote", "agent": "apprentice" }),
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
            caller_env(mentor_ura),
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
            caller_env(mentor_ura),
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
        let (_, apprentice_ura, _) = seed();
        let stranger = crate::ura::agent_ura("localhost", "dev", "stranger");

        let err = teach_handler(
            caller_env(stranger),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect_err("unauthorized caller must not write a grant");
        assert!(
            format!("{err}").contains("cannot teach abilities advertised by"),
            "{err}"
        );
    }

    #[test]
    fn teach_allows_the_host_device_authority_for_a_local_owner() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, _) = seed();
        let host = crate::ura::device_ura("localhost", "dev-1");

        let resp = teach_handler(
            caller_env(host.clone()),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("host device signs for the local owner");
        assert_eq!(resp["granted_by"], host);
    }
}
