// EasyNet CLI — teach-grant directory (`teach-grants.json`)
// ===========================================================
//
// File: src/persistence/teach_grants.rs
// Description: The owner-initiative store behind GET route B
//              (seven-axes T3.3, spec §2.5): which ability descriptors their
//              owner has explicitly granted for import, and by whom.
//
// Ontology (spec §0.1-6, non-negotiable): a capability is CONFERRED
// by its owner, never pulled by a consumer. Absence of a grant IS
// the `allow_transferred_code = false` default (capability.proto
// InstallPolicy) — a descriptor with no entry here cannot be imported,
// full stop. `meta.teach` writes a grant; `meta.acquire` consumes
// one; both are ordinary ledgered invocations, so the receipt chain
// records who granted which descriptor to whom.
//
// The file also keeps the descriptor import ledger: which declaration-only
// manifests landed in which learner's workspace through `meta.acquire`.
// `meta.forget` only removes what this ledger names; it can never silently
// delete a native ability authored by the agent.
//
// Schema (operator-inspectable)
// -----------------------------
// {
//   "schema_version": "5",
//   "grants": [
//     { "ability": "<owner-local registry name, e.g. testbot.weather-probe>",
//       "owner_agent": "testbot",
//       "learner_ura": "easynet:///r/<realm>/agent/<id>",
//       "execution_mode": "sandbox_first",   // capability.proto:238 default
//       "granted_at": "<rfc3339>",
//       "admission_snapshot": "<ledger-admitted grant authority snapshot>" },
//     …
//   ],
//   "imports": [
//     { "ability_name": "weather-probe",
//       "learner_agent": "apprentice",
//       "source_descriptor_ura": "<the granted descriptor's canonical URA>",
//       "imported_at": "<rfc3339>" },
//     …
//   ]
// }
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use anyhow::Context;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};
use super::file_lock::ExclusiveFileLock;
use crate::support::errors::append_cleanup_error;
use crate::ura::AbilitySelector;

pub(crate) const FILE_NAME: &str = "teach-grants.json";
const STORE_SCHEMA_VERSION: &str = "5";
const TEACH_GRANT_SNAPSHOT_KIND: &str = "teach_grant_admission_snapshot_v1";
const TEACH_GRANT_ENVELOPE_ABILITY: &str = "meta.teach";
static STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Default execution posture for future executable descriptor transfer
/// (capability.proto:238). A string by protocol design — the proto
/// field is a string with documented values, not an enum.
pub const EXECUTION_MODE_DEFAULT: &str = "sandbox_first";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeachGrantsFile {
    schema_version: String,
    grants: Vec<TeachGrant>,
    imports: Vec<DescriptorImportRecord>,
}

impl Default for TeachGrantsFile {
    fn default() -> Self {
        Self {
            schema_version: STORE_SCHEMA_VERSION.to_string(),
            grants: Vec::new(),
            imports: Vec::new(),
        }
    }
}

impl TeachGrantsFile {
    fn validate_schema(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if self.schema_version != STORE_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported teach grants schema_version {:?} in {}; expected {:?}",
                self.schema_version,
                path.display(),
                STORE_SCHEMA_VERSION
            );
        }
        for grant in &self.grants {
            grant.validate_stored()?;
        }
        for import in &self.imports {
            if let Some(grant) = &import.pending_grant {
                grant.validate_stored()?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeachGrant {
    ability: String,
    ability_ura: String,
    owner_ura: String,
    granted_by_ura: String,
    owner_agent: String,
    learner_ura: String,
    manifest_hash: String,
    execution_mode: String,
    granted_at: String,
    admission_snapshot: TeachGrantAdmissionSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeachGrantDraft {
    pub ability: String,
    pub ability_ura: String,
    pub owner_ura: String,
    pub granted_by_ura: String,
    pub owner_agent: String,
    pub learner_ura: String,
    pub manifest_hash: String,
    pub execution_mode: String,
    pub granted_at: String,
    pub admission_snapshot: TeachGrantAdmissionSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeachGrantAdmissionSnapshot {
    kind: String,
    invocation_id: String,
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    envelope_ability: String,
    invocation_nonce_hex: String,
    causal_context: Value,
    authority: TeachGrantAuthoritySnapshot,
    granted_ability: String,
    granted_ability_ura: String,
    owner_ura: String,
    granted_by_ura: String,
    learner_ura: String,
    manifest_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeachGrantAdmissionSnapshotDraft {
    pub invocation_id: String,
    pub caller_ura: String,
    pub callee_ura: String,
    pub subject_ura: String,
    pub envelope_ability: String,
    pub invocation_nonce_hex: String,
    pub causal_context: Value,
    pub authority: TeachGrantAuthoritySnapshot,
    pub granted_ability: String,
    pub granted_ability_ura: String,
    pub owner_ura: String,
    pub granted_by_ura: String,
    pub learner_ura: String,
    pub manifest_hash: String,
}

/// Authority facts admitted with a teach grant.
///
/// Invariant 1: this is not raw CLI metadata. Each variant is the durable
/// projection of an authority path that `meta.teach` already admitted.
///
/// Invariant 2: validation is variant-specific because direct agent calls,
/// direct owner calls and signed loopback hosted-agent delegation have distinct
/// accountability roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "authority_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TeachGrantAuthoritySnapshot {
    DirectOwnerCaller {
        owner_ura: String,
    },
    HostedAgentDelegation {
        agent_ura: String,
        host_device_ura: String,
        ability: String,
    },
}

impl TeachGrantAuthoritySnapshot {
    #[must_use]
    pub fn direct_owner(owner_ura: impl Into<String>) -> Self {
        Self::DirectOwnerCaller {
            owner_ura: owner_ura.into(),
        }
    }

    #[must_use]
    pub fn hosted_agent_delegation(
        agent_ura: impl Into<String>,
        host_device_ura: impl Into<String>,
        ability: impl Into<String>,
    ) -> Self {
        Self::HostedAgentDelegation {
            agent_ura: agent_ura.into(),
            host_device_ura: host_device_ura.into(),
            ability: ability.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescriptorImportRecord {
    ability_name: String,
    learner_agent: String,
    source_descriptor_ura: String,
    manifest_hash: String,
    imported_at: String,
    state: DescriptorImportState,
    #[serde(default)]
    acquiring_manifest_path: Option<String>,
    #[serde(default)]
    acquiring_staging_manifest_path: Option<String>,
    #[serde(default)]
    acquiring_manifest_hash: Option<String>,
    #[serde(default)]
    pending_grant: Option<TeachGrant>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptorImportState {
    Acquiring,
    #[default]
    Active,
    Forgetting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquiringArtifactRecoveryState {
    Committed,
    NotCommitted,
}

struct AcquiringRecoveryDecision {
    record: DescriptorImportRecord,
    artifact: AcquiringArtifactRecoveryState,
}

pub trait AcquiringArtifactTxn {
    fn committed_artifact_path(&self) -> String;

    fn staging_artifact_path(&self) -> Option<String>;

    fn content_hash(&self) -> String;

    fn commit(&self) -> anyhow::Result<()>;

    fn rollback(&self) -> anyhow::Result<()>;
}

impl TeachGrant {
    #[must_use]
    pub fn from_draft(draft: TeachGrantDraft) -> Self {
        Self {
            ability: draft.ability,
            ability_ura: draft.ability_ura,
            owner_ura: draft.owner_ura,
            granted_by_ura: draft.granted_by_ura,
            owner_agent: draft.owner_agent,
            learner_ura: draft.learner_ura,
            manifest_hash: draft.manifest_hash,
            execution_mode: draft.execution_mode,
            granted_at: draft.granted_at,
            admission_snapshot: draft.admission_snapshot,
        }
    }

    #[must_use]
    pub fn execution_mode(&self) -> &str {
        &self.execution_mode
    }

    #[must_use]
    pub fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }

    fn validate_stored(&self) -> anyhow::Result<()> {
        require_non_empty("teach grant ability", &self.ability)?;
        require_non_empty("teach grant ability_ura", &self.ability_ura)?;
        require_non_empty("teach grant owner_ura", &self.owner_ura)?;
        require_non_empty("teach grant granted_by_ura", &self.granted_by_ura)?;
        require_non_empty("teach grant owner_agent", &self.owner_agent)?;
        require_non_empty("teach grant learner_ura", &self.learner_ura)?;
        require_non_empty("teach grant manifest_hash", &self.manifest_hash)?;
        require_non_empty("teach grant execution_mode", &self.execution_mode)?;
        require_non_empty("teach grant granted_at", &self.granted_at)?;
        self.admission_snapshot.validate_for_grant(self)
    }

    fn validate_acquire_request(
        &self,
        registry_name: &str,
        ability_ura: &str,
        owner_ura: &str,
        learner_ura: &str,
    ) -> anyhow::Result<()> {
        if self.ability != registry_name
            || self.ability_ura != ability_ura
            || self.owner_ura != owner_ura
            || self.learner_ura != learner_ura
        {
            anyhow::bail!(
                "teach grant identity mismatch during acquire admission; grant=({:?}, {:?}, {:?}, {:?}), request=({registry_name:?}, {ability_ura:?}, {owner_ura:?}, {learner_ura:?})",
                self.ability,
                self.ability_ura,
                self.owner_ura,
                self.learner_ura
            );
        }
        self.admission_snapshot.validate_for_grant(self)
    }
}

impl From<TeachGrantDraft> for TeachGrant {
    fn from(draft: TeachGrantDraft) -> Self {
        Self::from_draft(draft)
    }
}

impl TeachGrantAdmissionSnapshot {
    pub fn from_draft(draft: TeachGrantAdmissionSnapshotDraft) -> anyhow::Result<Self> {
        let snapshot = Self {
            kind: TEACH_GRANT_SNAPSHOT_KIND.to_string(),
            invocation_id: draft.invocation_id,
            caller_ura: draft.caller_ura,
            callee_ura: draft.callee_ura,
            subject_ura: draft.subject_ura,
            envelope_ability: draft.envelope_ability,
            invocation_nonce_hex: draft.invocation_nonce_hex,
            causal_context: draft.causal_context,
            authority: draft.authority,
            granted_ability: draft.granted_ability,
            granted_ability_ura: draft.granted_ability_ura,
            owner_ura: draft.owner_ura,
            granted_by_ura: draft.granted_by_ura,
            learner_ura: draft.learner_ura,
            manifest_hash: draft.manifest_hash,
        };
        snapshot.validate_shape()?;
        Ok(snapshot)
    }

    fn validate_shape(&self) -> anyhow::Result<()> {
        if self.kind != TEACH_GRANT_SNAPSHOT_KIND {
            anyhow::bail!(
                "unsupported teach grant snapshot kind {:?}; expected {:?}",
                self.kind,
                TEACH_GRANT_SNAPSHOT_KIND
            );
        }
        require_non_empty("teach grant snapshot invocation_id", &self.invocation_id)?;
        require_non_empty("teach grant snapshot caller_ura", &self.caller_ura)?;
        require_non_empty("teach grant snapshot callee_ura", &self.callee_ura)?;
        require_non_empty("teach grant snapshot subject_ura", &self.subject_ura)?;
        require_non_empty(
            "teach grant snapshot envelope_ability",
            &self.envelope_ability,
        )?;
        let selected_envelope_ability = canonical_envelope_ability(&self.envelope_ability)
            .with_context(|| {
                format!(
                    "invalid teach grant snapshot envelope ability {:?}",
                    self.envelope_ability
                )
            })?;
        if selected_envelope_ability != TEACH_GRANT_ENVELOPE_ABILITY {
            anyhow::bail!(
                "teach grant snapshot envelope ability {:?} resolved to {:?}; expected {:?}",
                self.envelope_ability,
                selected_envelope_ability,
                TEACH_GRANT_ENVELOPE_ABILITY
            );
        }
        let nonce = hex::decode(&self.invocation_nonce_hex).map_err(|err| {
            anyhow::anyhow!(
                "teach grant snapshot invocation nonce must be hex encoded 16 bytes: {err}"
            )
        })?;
        if nonce.len() != 16 {
            anyhow::bail!(
                "teach grant snapshot invocation nonce must be 16 bytes, got {}",
                nonce.len()
            );
        }
        self.authority.validate_shape()?;
        require_non_empty(
            "teach grant snapshot granted_ability",
            &self.granted_ability,
        )?;
        require_non_empty(
            "teach grant snapshot granted_ability_ura",
            &self.granted_ability_ura,
        )?;
        require_non_empty("teach grant snapshot owner_ura", &self.owner_ura)?;
        require_non_empty("teach grant snapshot granted_by_ura", &self.granted_by_ura)?;
        require_non_empty("teach grant snapshot learner_ura", &self.learner_ura)?;
        require_non_empty("teach grant snapshot manifest_hash", &self.manifest_hash)?;
        Ok(())
    }

    fn validate_for_grant(&self, grant: &TeachGrant) -> anyhow::Result<()> {
        self.validate_shape()?;
        let snapshot_matches_grant = self.granted_ability == grant.ability
            && self.granted_ability_ura == grant.ability_ura
            && self.owner_ura == grant.owner_ura
            && self.granted_by_ura == grant.granted_by_ura
            && self.learner_ura == grant.learner_ura
            && self.manifest_hash == grant.manifest_hash;
        if !snapshot_matches_grant {
            anyhow::bail!(
                "teach grant snapshot does not match grant facts for descriptor {:?} to {:?}",
                grant.ability_ura,
                grant.learner_ura
            );
        }
        if self.subject_ura != grant.ability_ura {
            anyhow::bail!(
                "teach grant snapshot subject {:?} does not bind granted descriptor {:?}",
                self.subject_ura,
                grant.ability_ura
            );
        }
        self.authority.validate_for_grant(self, grant)?;
        Ok(())
    }
}

impl TeachGrantAuthoritySnapshot {
    fn validate_shape(&self) -> anyhow::Result<()> {
        match self {
            Self::DirectOwnerCaller { owner_ura } => {
                require_non_empty("teach grant authority owner_ura", owner_ura)
            }
            Self::HostedAgentDelegation {
                agent_ura,
                host_device_ura,
                ability,
            } => {
                require_non_empty("teach grant authority agent_ura", agent_ura)?;
                require_non_empty("teach grant authority host_device_ura", host_device_ura)?;
                require_non_empty("teach grant authority ability", ability)
            }
        }
    }

    fn validate_for_grant(
        &self,
        snapshot: &TeachGrantAdmissionSnapshot,
        grant: &TeachGrant,
    ) -> anyhow::Result<()> {
        match self {
            Self::DirectOwnerCaller { owner_ura } => {
                if owner_ura != &grant.owner_ura
                    || snapshot.caller_ura != *owner_ura
                    || grant.granted_by_ura != *owner_ura
                {
                    anyhow::bail!(
                        "teach grant direct-owner authority does not match caller/owner/granted_by"
                    );
                }
            }
            Self::HostedAgentDelegation {
                agent_ura,
                host_device_ura,
                ability,
            } => {
                let delegated_ability = canonical_envelope_ability(ability).with_context(|| {
                    format!("invalid teach grant hosted-agent authority ability {ability:?}")
                })?;
                if agent_ura != &grant.owner_ura
                    || host_device_ura != &grant.granted_by_ura
                    || snapshot.caller_ura != crate::ura::LOCAL_SYSTEM_AGENT_URA
                    || snapshot.callee_ura != *host_device_ura
                    || delegated_ability != TEACH_GRANT_ENVELOPE_ABILITY
                {
                    anyhow::bail!(
                        "teach grant hosted-agent authority does not match owner/host/caller/callee/ability"
                    );
                }
            }
        }
        Ok(())
    }
}

fn require_non_empty(field: &'static str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    Ok(())
}

fn canonical_envelope_ability(raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim();
    require_non_empty("teach grant snapshot envelope ability", trimmed)?;
    if trimmed == TEACH_GRANT_ENVELOPE_ABILITY {
        return Ok(trimmed.to_string());
    }
    let selector = AbilitySelector::parse(trimmed)?;
    Ok(strip_descriptor_version(selector.local_registry_ability()).to_string())
}

fn strip_descriptor_version(local_registry_ability: &str) -> &str {
    local_registry_ability
        .split_once('@')
        .map_or(local_registry_ability, |(ability, _version)| ability)
}

pub struct AcquireStagedGrant<T> {
    registry_name: String,
    ability_ura: String,
    owner_ura: String,
    learner_ura: String,
    expected_grant: TeachGrant,
    import_record: DescriptorImportRecord,
    staged: T,
}

impl<T> AcquireStagedGrant<T> {
    /// Build a staged acquire request whose grant, import row, and staged
    /// artifact identity describe the same descriptor transfer.
    ///
    /// What this is NOT: a DTO for callers to fill piecemeal. The store owns
    /// the acquire state machine, so external callers can only submit a request
    /// that passes the same identity checks the store later enforces under its
    /// file lock.
    pub fn new(
        registry_name: impl Into<String>,
        ability_ura: impl Into<String>,
        owner_ura: impl Into<String>,
        learner_ura: impl Into<String>,
        expected_grant: TeachGrant,
        import_record: DescriptorImportRecord,
        staged: T,
    ) -> anyhow::Result<Self> {
        let registry_name = registry_name.into();
        let ability_ura = ability_ura.into();
        let owner_ura = owner_ura.into();
        let learner_ura = learner_ura.into();
        require_non_empty("acquire staged registry_name", &registry_name)?;
        require_non_empty("acquire staged ability_ura", &ability_ura)?;
        require_non_empty("acquire staged owner_ura", &owner_ura)?;
        require_non_empty("acquire staged learner_ura", &learner_ura)?;
        expected_grant.validate_acquire_request(
            &registry_name,
            &ability_ura,
            &owner_ura,
            &learner_ura,
        )?;
        if import_record.state() != DescriptorImportState::Active {
            anyhow::bail!(
                "acquire staged import record must start active before the store marks it acquiring"
            );
        }
        if import_record.source_descriptor_ura != ability_ura {
            anyhow::bail!(
                "acquire staged import source {:?} does not match granted descriptor {:?}",
                import_record.source_descriptor_ura,
                ability_ura
            );
        }
        if import_record.manifest_hash != expected_grant.manifest_hash {
            anyhow::bail!(
                "acquire staged import manifest hash {:?} does not match grant hash {:?}",
                import_record.manifest_hash,
                expected_grant.manifest_hash
            );
        }
        Ok(Self {
            registry_name,
            ability_ura,
            owner_ura,
            learner_ura,
            expected_grant,
            import_record,
            staged,
        })
    }
}

impl DescriptorImportRecord {
    #[must_use]
    pub fn new(
        ability_name: impl Into<String>,
        learner_agent: impl Into<String>,
        source_descriptor_ura: impl Into<String>,
        manifest_hash: impl Into<String>,
        imported_at: impl Into<String>,
    ) -> Self {
        Self {
            ability_name: ability_name.into(),
            learner_agent: learner_agent.into(),
            source_descriptor_ura: source_descriptor_ura.into(),
            manifest_hash: manifest_hash.into(),
            imported_at: imported_at.into(),
            state: DescriptorImportState::Active,
            acquiring_manifest_path: None,
            acquiring_staging_manifest_path: None,
            acquiring_manifest_hash: None,
            pending_grant: None,
        }
    }

    #[must_use]
    pub fn source_descriptor_ura(&self) -> &str {
        &self.source_descriptor_ura
    }

    #[must_use]
    pub fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }

    #[must_use]
    pub fn state(&self) -> DescriptorImportState {
        self.state
    }

    #[must_use]
    pub fn acquiring_manifest_path(&self) -> Option<&str> {
        self.acquiring_manifest_path.as_deref()
    }

    #[must_use]
    pub fn acquiring_staging_manifest_path(&self) -> Option<&str> {
        self.acquiring_staging_manifest_path.as_deref()
    }

    #[must_use]
    pub fn acquiring_manifest_hash(&self) -> Option<&str> {
        self.acquiring_manifest_hash.as_deref()
    }

    fn mark_active(&mut self) {
        self.state = DescriptorImportState::Active;
        self.acquiring_manifest_path = None;
        self.acquiring_staging_manifest_path = None;
        self.acquiring_manifest_hash = None;
        self.pending_grant = None;
    }

    fn mark_acquiring(
        &mut self,
        committed_manifest_path: impl Into<String>,
        staging_manifest_path: Option<String>,
        manifest_hash: impl Into<String>,
        pending_grant: TeachGrant,
    ) {
        self.state = DescriptorImportState::Acquiring;
        self.acquiring_manifest_path = Some(committed_manifest_path.into());
        self.acquiring_staging_manifest_path = staging_manifest_path;
        let manifest_hash = manifest_hash.into();
        self.manifest_hash = manifest_hash.clone();
        self.acquiring_manifest_hash = Some(manifest_hash);
        self.pending_grant = Some(pending_grant);
    }

    fn mark_forgetting(&mut self) {
        self.state = DescriptorImportState::Forgetting;
    }

    fn same_identity(&self, other: &DescriptorImportRecord) -> bool {
        self.ability_name == other.ability_name
            && self.learner_agent == other.learner_agent
            && self.source_descriptor_ura == other.source_descriptor_ura
            && self.manifest_hash == other.manifest_hash
            && self.imported_at == other.imported_at
    }
}

pub fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

pub struct TeachGrantStore {
    path: PathBuf,
}

struct TeachGrantStoreLock {
    _process: MutexGuard<'static, ()>,
    _file: ExclusiveFileLock,
}

/// Committed `meta.acquire` transaction state.
///
/// The object exists so post-ledger side effects, such as hot runtime
/// registration, can roll the acquire back without guessing which grant was
/// consumed or which descriptor-import ledger row was written.
#[derive(Debug)]
pub struct AcquiredTeachGrant {
    grant: TeachGrant,
    import_record: DescriptorImportRecord,
}

impl AcquiredTeachGrant {
    pub fn grant(&self) -> &TeachGrant {
        &self.grant
    }

    pub fn import_record(&self) -> &DescriptorImportRecord {
        &self.import_record
    }
}

/// Durable `meta.forget` transaction.
///
/// The descriptor-import ledger row is marked `forgetting` before runtime
/// cleanup starts. That tombstone blocks duplicate acquire/forget races and
/// lets a retry finish the delete instead of replaying an already-staged
/// manifest as active state.
#[derive(Debug)]
pub struct StagedDescriptorImportRemoval<T> {
    record: DescriptorImportRecord,
    staged_artifact: T,
    resumed: bool,
}

impl<T> StagedDescriptorImportRemoval<T> {
    #[must_use]
    pub fn record(&self) -> &DescriptorImportRecord {
        &self.record
    }

    #[must_use]
    pub fn staged_artifact(&self) -> &T {
        &self.staged_artifact
    }

    #[must_use]
    pub fn resumed(&self) -> bool {
        self.resumed
    }
}

#[derive(Debug)]
pub struct RuntimePendingDescriptorImportRemoval {
    record: DescriptorImportRecord,
}

impl RuntimePendingDescriptorImportRemoval {
    #[must_use]
    pub fn record(&self) -> &DescriptorImportRecord {
        &self.record
    }
}

#[derive(Debug)]
pub struct CommittedDescriptorImportRemoval {
    record: DescriptorImportRecord,
}

impl CommittedDescriptorImportRemoval {
    #[must_use]
    pub fn record(&self) -> &DescriptorImportRecord {
        &self.record
    }
}

impl TeachGrantStore {
    #[must_use]
    pub fn open_default() -> Self {
        Self { path: path() }
    }

    pub fn grant(&self, grant: TeachGrant) -> anyhow::Result<()> {
        self.update(|directory| {
            directory.grants.retain(|g| {
                let same_bound_identity = g.ability_ura == grant.ability_ura
                    && g.owner_ura == grant.owner_ura
                    && g.learner_ura == grant.learner_ura;
                !same_bound_identity
            });
            directory.grants.push(grant);
            Ok(())
        })
    }

    pub fn grant_for(
        &self,
        registry_name: &str,
        ability_ura: &str,
        owner_ura: &str,
        learner_ura: &str,
    ) -> anyhow::Result<Option<TeachGrant>> {
        let _guard = self.lock()?;
        let directory = self.load_unlocked()?;
        Ok(directory
            .grant_index_for(registry_name, ability_ura, owner_ura, learner_ura)
            .map(|idx| directory.grants[idx].clone()))
    }

    pub fn recover_acquiring(
        &self,
        mut recover_artifact: impl FnMut(
            &DescriptorImportRecord,
        ) -> anyhow::Result<AcquiringArtifactRecoveryState>,
    ) -> anyhow::Result<usize> {
        let records = self.snapshot_acquiring_records()?;
        let mut decisions = Vec::with_capacity(records.len());
        for record in records {
            let artifact = recover_artifact(&record)?;
            decisions.push(AcquiringRecoveryDecision { record, artifact });
        }
        self.apply_acquiring_recovery(decisions)
    }

    fn snapshot_acquiring_records(&self) -> anyhow::Result<Vec<DescriptorImportRecord>> {
        let _guard = self.lock()?;
        let directory = self.load_unlocked()?;
        Ok(directory
            .imports
            .iter()
            .filter(|record| record.state() == DescriptorImportState::Acquiring)
            .cloned()
            .collect())
    }

    fn apply_acquiring_recovery(
        &self,
        decisions: Vec<AcquiringRecoveryDecision>,
    ) -> anyhow::Result<usize> {
        if decisions.is_empty() {
            return Ok(0);
        }
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let mut recovered = 0usize;
        for decision in decisions {
            let Some(idx) = directory.import_by_record(&decision.record) else {
                continue;
            };
            if directory.imports[idx].state() != DescriptorImportState::Acquiring {
                continue;
            }
            match decision.artifact {
                AcquiringArtifactRecoveryState::Committed => {
                    directory.imports[idx].mark_active();
                    recovered += 1;
                }
                AcquiringArtifactRecoveryState::NotCommitted => {
                    if let Some(grant) = decision.record.pending_grant.clone() {
                        if directory.grant_index_for_record(&grant).is_none() {
                            directory.grants.push(grant);
                        }
                    }
                    directory.imports.remove(idx);
                    recovered += 1;
                }
            }
        }
        if recovered > 0 {
            self.write_unlocked(&directory)?;
        }
        Ok(recovered)
    }

    pub fn acquire_staged<T>(
        &self,
        request: AcquireStagedGrant<T>,
    ) -> anyhow::Result<AcquiredTeachGrant>
    where
        T: AcquiringArtifactTxn,
    {
        let AcquireStagedGrant {
            registry_name,
            ability_ura,
            owner_ura,
            learner_ura,
            expected_grant,
            import_record,
            staged,
        } = request;
        let acquired = match (|| {
            let _guard = self.lock()?;
            let mut directory = self.load_unlocked()?;
            let grant_idx = directory
                .grant_index_for(&registry_name, &ability_ura, &owner_ura, &learner_ura)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "descriptor grant missing (allow_transferred_code=false): owner {owner_ura:?} \
                         has not granted descriptor {ability_ura:?} ({registry_name:?}) to learner {learner_ura}"
                    )
                })?;
            let grant = directory.grants[grant_idx].clone();
            if grant != expected_grant {
                anyhow::bail!(
                    "teach grant changed during acquire admission; retry acquire so execution \
                     posture is evaluated against the committed grant"
                );
            }
            grant.validate_acquire_request(
                &registry_name,
                &ability_ura,
                &owner_ura,
                &learner_ura,
            )?;
            if directory
                .import_by_active_or_forgetting(
                    &import_record.learner_agent,
                    &import_record.ability_name,
                )
                .is_some()
            {
                anyhow::bail!(
                    "agent {:?} already imported descriptor {:?} or has a forget transaction in progress; \
                     finish forgetting it first or rename; descriptor import never overwrites",
                    import_record.learner_agent,
                    import_record.ability_name
                );
            }

            let mut acquiring = import_record;
            acquiring.mark_acquiring(
                staged.committed_artifact_path(),
                staged.staging_artifact_path(),
                staged.content_hash(),
                grant.clone(),
            );
            directory.grants.remove(grant_idx);
            directory.imports.push(acquiring.clone());
            self.write_unlocked(&directory)?;
            Ok(AcquiredTeachGrant {
                grant,
                import_record: acquiring,
            })
        })() {
            Ok(acquired) => acquired,
            Err(ledger_err) => {
                return Err(append_cleanup_error(
                    ledger_err,
                    staged.rollback(),
                    "rollback staged descriptor-import manifest",
                ));
            }
        };

        if let Err(commit_err) = staged.commit() {
            let rollback_err = staged.rollback();
            let restore_err = self.restore_acquired_ledger_after_artifact_commit_failure(&acquired);
            return Err(append_cleanup_error(
                append_cleanup_error(
                    commit_err,
                    rollback_err,
                    "rollback staged descriptor-import manifest",
                ),
                restore_err,
                "restore teach grant ledger after failed descriptor-import manifest commit",
            ));
        }
        let active_import = self.commit_acquired_ledger_after_artifact_commit(&acquired)?;
        Ok(AcquiredTeachGrant {
            grant: acquired.grant,
            import_record: active_import,
        })
    }

    fn commit_acquired_ledger_after_artifact_commit(
        &self,
        acquired: &AcquiredTeachGrant,
    ) -> anyhow::Result<DescriptorImportRecord> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(import_idx) = directory.import_by_record(&acquired.import_record) else {
            anyhow::bail!(
                "commit acquired ledger: acquiring row for agent {:?} ability {:?} is absent",
                acquired.import_record.learner_agent,
                acquired.import_record.ability_name
            );
        };
        if directory.imports[import_idx].state() != DescriptorImportState::Acquiring {
            anyhow::bail!(
                "commit acquired ledger: descriptor-import row for agent {:?} ability {:?} is not acquiring",
                acquired.import_record.learner_agent,
                acquired.import_record.ability_name
            );
        }
        directory.imports[import_idx].mark_active();
        let active = directory.imports[import_idx].clone();
        self.write_unlocked(&directory)?;
        Ok(active)
    }

    pub fn stage_forget<T>(
        &self,
        learner_agent: &str,
        ability_name: &str,
        stage_remove: impl FnOnce(&DescriptorImportRecord) -> anyhow::Result<T>,
    ) -> anyhow::Result<StagedDescriptorImportRemoval<T>> {
        let (record, resumed) = self.begin_forget_ledger(learner_agent, ability_name)?;
        let staged = match stage_remove(&record) {
            Ok(staged) => staged,
            Err(stage_err) if !resumed => {
                return Err(append_cleanup_error(
                    stage_err,
                    self.rollback_forget_ledger(&record),
                    "restore forget ledger tombstone",
                ));
            }
            Err(stage_err) => return Err(stage_err),
        };
        Ok(StagedDescriptorImportRemoval {
            record,
            staged_artifact: staged,
            resumed,
        })
    }

    pub fn commit_forget_artifact<T>(
        &self,
        staged: &StagedDescriptorImportRemoval<T>,
        commit_remove: impl FnOnce(&T) -> anyhow::Result<()>,
    ) -> anyhow::Result<RuntimePendingDescriptorImportRemoval> {
        self.require_forgetting_ledger_row(staged.record())?;
        commit_remove(staged.staged_artifact())?;
        self.require_forgetting_ledger_row(staged.record())?;
        Ok(RuntimePendingDescriptorImportRemoval {
            record: staged.record().clone(),
        })
    }

    pub fn finish_forget(
        &self,
        pending: &RuntimePendingDescriptorImportRemoval,
    ) -> anyhow::Result<CommittedDescriptorImportRemoval> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(idx) = directory.import_by_record(pending.record()) else {
            anyhow::bail!(
                "finish forget: descriptor-import ledger row for agent {:?} ability {:?} is already absent",
                pending.record.learner_agent,
                pending.record.ability_name
            );
        };
        if directory.imports[idx].state() != DescriptorImportState::Forgetting {
            anyhow::bail!(
                "finish forget: descriptor-import ledger row for agent {:?} ability {:?} is not in \
                 forgetting state",
                pending.record.learner_agent,
                pending.record.ability_name
            );
        }
        let record = directory.imports.remove(idx);
        self.write_unlocked(&directory)?;
        Ok(CommittedDescriptorImportRemoval { record })
    }

    fn begin_forget_ledger(
        &self,
        learner_agent: &str,
        ability_name: &str,
    ) -> anyhow::Result<(DescriptorImportRecord, bool)> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(idx) = directory.import_by_active_or_forgetting(learner_agent, ability_name)
        else {
            anyhow::bail!(
                "agent {learner_agent:?} has no imported descriptor {ability_name:?}; \
                 forget only removes descriptor imports, while native abilities are removed by their author"
            );
        };
        let record = directory.imports[idx].clone();
        match record.state() {
            DescriptorImportState::Acquiring => {
                anyhow::bail!(
                    "agent {learner_agent:?} has an acquire transaction in progress for {ability_name:?}; \
                     finish or recover acquire before forgetting it"
                );
            }
            DescriptorImportState::Forgetting => Ok((record, true)),
            DescriptorImportState::Active => {
                directory.imports[idx].mark_forgetting();
                self.write_unlocked(&directory)?;
                Ok((record, false))
            }
        }
    }

    fn require_forgetting_ledger_row(&self, record: &DescriptorImportRecord) -> anyhow::Result<()> {
        let _guard = self.lock()?;
        let directory = self.load_unlocked()?;
        let Some(idx) = directory.import_by_record(record) else {
            anyhow::bail!(
                "commit forget: descriptor-import ledger row for agent {:?} ability {:?} is already absent",
                record.learner_agent,
                record.ability_name
            );
        };
        if directory.imports[idx].state() != DescriptorImportState::Forgetting {
            anyhow::bail!(
                "commit forget: descriptor-import ledger row for agent {:?} ability {:?} is not in \
                 forgetting state",
                record.learner_agent,
                record.ability_name
            );
        }
        Ok(())
    }

    fn rollback_forget_ledger(&self, record: &DescriptorImportRecord) -> anyhow::Result<()> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(idx) = directory.import_by_record(record) else {
            anyhow::bail!(
                "rollback forget: descriptor-import ledger row for agent {:?} ability {:?} is already absent",
                record.learner_agent,
                record.ability_name
            );
        };
        if directory.imports[idx].state() == DescriptorImportState::Forgetting {
            directory.imports[idx].mark_active();
            self.write_unlocked(&directory)?;
        }
        Ok(())
    }

    fn update<T>(
        &self,
        f: impl FnOnce(&mut TeachGrantsFile) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let out = f(&mut directory)?;
        self.write_unlocked(&directory)?;
        Ok(out)
    }

    fn load_unlocked(&self) -> anyhow::Result<TeachGrantsFile> {
        if !self.path.exists() {
            return Ok(TeachGrantsFile::default());
        }
        let bytes = fs::read(&self.path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", self.path.display()))?;
        let file: TeachGrantsFile = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", self.path.display()))?;
        file.validate_schema(&self.path)?;
        Ok(file)
    }

    fn write_unlocked(&self, file: &TeachGrantsFile) -> anyhow::Result<()> {
        file.validate_schema(&self.path)?;
        let dir = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("teach grant store path has no parent"))?;
        fs::create_dir_all(dir)
            .map_err(|e| anyhow::anyhow!("create_dir_all {}: {e}", dir.display()))?;
        let json = serde_json::to_string_pretty(file)?;
        atomic_write_with_permissions(
            &self.path,
            json.as_bytes(),
            WritePermissions::OwnerReadWrite,
        )
    }

    fn lock(&self) -> anyhow::Result<TeachGrantStoreLock> {
        let process = STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let file = ExclusiveFileLock::acquire_for_data_path(&self.path)?;
        Ok(TeachGrantStoreLock {
            _process: process,
            _file: file,
        })
    }

    fn restore_acquired_ledger_after_artifact_commit_failure(
        &self,
        acquired: &AcquiredTeachGrant,
    ) -> anyhow::Result<()> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(import_idx) = directory.import_by_record(&acquired.import_record) else {
            anyhow::bail!(
                "restore acquired ledger: descriptor-import ledger row for agent {:?} ability {:?} \
                 is already absent",
                acquired.import_record.learner_agent,
                acquired.import_record.ability_name
            );
        };
        let import_record = directory.imports[import_idx].clone();
        if import_record != acquired.import_record {
            anyhow::bail!(
                "restore acquired ledger: descriptor-import ledger row changed under rollback; \
                 refusing to restore grant for {:?}",
                acquired.import_record.ability_name
            );
        }
        if directory.grant_index_for_record(&acquired.grant).is_some() {
            anyhow::bail!(
                "restore acquired ledger: grant for {:?} to {:?} is already present",
                acquired.grant.ability_ura,
                acquired.grant.learner_ura
            );
        }
        directory.imports.remove(import_idx);
        directory.grants.push(acquired.grant.clone());
        self.write_unlocked(&directory)
    }
}

impl TeachGrantsFile {
    fn grant_index_for_record(&self, record: &TeachGrant) -> Option<usize> {
        self.grants.iter().position(|grant| grant == record)
    }

    fn grant_index_for(
        &self,
        ability: &str,
        ability_ura: &str,
        owner_ura: &str,
        learner_ura: &str,
    ) -> Option<usize> {
        self.grants.iter().position(|g| {
            g.ability == ability
                && g.ability_ura == ability_ura
                && g.owner_ura == owner_ura
                && g.learner_ura == learner_ura
        })
    }

    /// Active or tombstoned import row for `(learner, ability)`. Used by
    /// acquire/forget admission so an in-progress forget cannot be treated as
    /// free space for a second descriptor import.
    fn import_by_active_or_forgetting(
        &self,
        learner_agent: &str,
        ability_name: &str,
    ) -> Option<usize> {
        self.imports
            .iter()
            .position(|l| l.learner_agent == learner_agent && l.ability_name == ability_name)
    }

    fn import_by_record(&self, record: &DescriptorImportRecord) -> Option<usize> {
        self.imports
            .iter()
            .position(|candidate| candidate.same_identity(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MANIFEST_HASH: &str = "sha256:test";

    #[derive(Debug)]
    struct TestAcquiringArtifact {
        committed_path: String,
        staging_path: Option<String>,
        content_hash: String,
        commit_error: Option<&'static str>,
        rollback_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }

    impl TestAcquiringArtifact {
        fn committed() -> Self {
            Self {
                committed_path: "/tmp/committed-copy".to_string(),
                staging_path: Some("/tmp/staged-copy".to_string()),
                content_hash: "sha256:test".to_string(),
                commit_error: None,
                rollback_flag: None,
            }
        }

        fn with_commit_error(
            message: &'static str,
            rollback_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        ) -> Self {
            Self {
                commit_error: Some(message),
                rollback_flag: Some(rollback_flag),
                ..Self::committed()
            }
        }
    }

    impl AcquiringArtifactTxn for TestAcquiringArtifact {
        fn committed_artifact_path(&self) -> String {
            self.committed_path.clone()
        }

        fn staging_artifact_path(&self) -> Option<String> {
            self.staging_path.clone()
        }

        fn content_hash(&self) -> String {
            self.content_hash.clone()
        }

        fn commit(&self) -> anyhow::Result<()> {
            if let Some(message) = self.commit_error {
                anyhow::bail!(message);
            }
            Ok(())
        }

        fn rollback(&self) -> anyhow::Result<()> {
            if let Some(flag) = &self.rollback_flag {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(())
        }
    }

    fn mentor_quote_grant(learner_ura: &str) -> TeachGrant {
        TeachGrant::from_draft(TeachGrantDraft {
            ability: "mentor.quote".to_string(),
            ability_ura: "easynet:///r/acme/ability/mentor.quote".to_string(),
            owner_ura: "easynet:///r/acme/agent/mentor".to_string(),
            granted_by_ura: "easynet:///r/acme/agent/mentor".to_string(),
            owner_agent: "mentor".to_string(),
            learner_ura: learner_ura.to_string(),
            manifest_hash: TEST_MANIFEST_HASH.to_string(),
            execution_mode: EXECUTION_MODE_DEFAULT.to_string(),
            granted_at: "t0".to_string(),
            admission_snapshot: mentor_quote_snapshot(learner_ura),
        })
    }

    fn mentor_quote_snapshot(learner_ura: &str) -> TeachGrantAdmissionSnapshot {
        TeachGrantAdmissionSnapshot::from_draft(TeachGrantAdmissionSnapshotDraft {
            invocation_id: "test-invocation".to_string(),
            caller_ura: "easynet:///r/acme/agent/mentor".to_string(),
            callee_ura: "easynet:///r/acme/agent/mentor".to_string(),
            subject_ura: "easynet:///r/acme/ability/mentor.quote".to_string(),
            envelope_ability: "meta.teach".to_string(),
            invocation_nonce_hex: hex::encode([0xA5; 16]),
            causal_context: serde_json::json!({"kind": "none"}),
            authority: TeachGrantAuthoritySnapshot::direct_owner("easynet:///r/acme/agent/mentor"),
            granted_ability: "mentor.quote".to_string(),
            granted_ability_ura: "easynet:///r/acme/ability/mentor.quote".to_string(),
            owner_ura: "easynet:///r/acme/agent/mentor".to_string(),
            granted_by_ura: "easynet:///r/acme/agent/mentor".to_string(),
            learner_ura: learner_ura.to_string(),
            manifest_hash: TEST_MANIFEST_HASH.to_string(),
        })
        .expect("test snapshot is valid")
    }

    fn acquire_mentor_quote_request<T>(
        learner_ura: &str,
        expected_grant: TeachGrant,
        staged: T,
    ) -> AcquireStagedGrant<T> {
        AcquireStagedGrant::new(
            "mentor.quote",
            "easynet:///r/acme/ability/mentor.quote",
            "easynet:///r/acme/agent/mentor",
            learner_ura,
            expected_grant,
            DescriptorImportRecord::new(
                "quote",
                "apprentice",
                "easynet:///r/acme/ability/mentor.quote",
                TEST_MANIFEST_HASH,
                "t1",
            ),
            staged,
        )
        .expect("test staged acquire request is valid")
    }

    #[test]
    fn absent_grant_is_the_default_refusal() {
        let file = TeachGrantsFile::default();
        assert!(
            file.grant_index_for(
                "testbot.weather-probe",
                "easynet:///r/acme/ability/testbot.weather-probe",
                "easynet:///r/acme/agent/testbot",
                "ura"
            )
            .is_none(),
            "no entry means allow_transferred_code=false"
        );
    }

    #[test]
    fn load_rejects_unversioned_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, br#"{"grants":[],"imports":[]}"#).unwrap();
        let store = TeachGrantStore { path };

        let err = store.load_unlocked().expect_err("missing schema version");
        assert!(err.to_string().contains("parse"), "{err}");
    }

    #[test]
    fn load_rejects_grant_with_mismatched_invocation_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut file = TeachGrantsFile::default();
        let mut grant = mentor_quote_grant("easynet:///r/acme/agent/apprentice");
        grant.admission_snapshot.manifest_hash = "sha256:tampered".to_string();
        file.grants.push(grant);
        std::fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
        let store = TeachGrantStore { path };

        let err = store
            .load_unlocked()
            .expect_err("grant snapshot must match grant facts");
        assert!(
            err.to_string()
                .contains("teach grant snapshot does not match grant facts"),
            "{err}"
        );
    }

    #[test]
    fn admission_snapshot_accepts_versioned_teach_ability_ura() {
        let mut grant = mentor_quote_grant("easynet:///r/acme/agent/apprentice");
        grant.admission_snapshot.envelope_ability =
            "easynet:///r/cli/ability/device.local.meta.teach@1.0.0".to_string();

        grant
            .validate_stored()
            .expect("versioned selected route URA still resolves to meta.teach");
    }

    #[test]
    fn admission_snapshot_rejects_versioned_non_teach_ability_ura() {
        let mut grant = mentor_quote_grant("easynet:///r/acme/agent/apprentice");
        grant.admission_snapshot.envelope_ability =
            "easynet:///r/cli/ability/device.local.meta.acquire@1.0.0".to_string();

        let err = grant
            .validate_stored()
            .expect_err("non-teach selected route URA must not admit a teach grant");
        assert!(err.to_string().contains("expected \"meta.teach\""), "{err}");
    }

    #[test]
    fn hosted_delegation_snapshot_binds_caller_to_local_system_and_callee_to_host() {
        let learner = "easynet:///r/acme/agent/apprentice";
        let host = "easynet:///r/acme/device/host-1";
        let owner = "easynet:///r/acme/agent/mentor";
        // The daemon presents the signed delegation under its local-system
        // identity (caller), while the host device it delegates for is the
        // envelope callee. Both bindings are load-bearing.
        let mut grant = TeachGrant::from_draft(TeachGrantDraft {
            ability: "mentor.quote".to_string(),
            ability_ura: "easynet:///r/acme/ability/mentor.quote".to_string(),
            owner_ura: owner.to_string(),
            granted_by_ura: host.to_string(),
            owner_agent: "mentor".to_string(),
            learner_ura: learner.to_string(),
            manifest_hash: TEST_MANIFEST_HASH.to_string(),
            execution_mode: EXECUTION_MODE_DEFAULT.to_string(),
            granted_at: "t0".to_string(),
            admission_snapshot: TeachGrantAdmissionSnapshot::from_draft(
                TeachGrantAdmissionSnapshotDraft {
                    invocation_id: "hosted-teach".to_string(),
                    caller_ura: crate::ura::LOCAL_SYSTEM_AGENT_URA.to_string(),
                    callee_ura: host.to_string(),
                    subject_ura: "easynet:///r/acme/ability/mentor.quote".to_string(),
                    envelope_ability: "meta.teach".to_string(),
                    invocation_nonce_hex: hex::encode([0xB6; 16]),
                    causal_context: serde_json::json!({"kind": "none"}),
                    authority: TeachGrantAuthoritySnapshot::hosted_agent_delegation(
                        owner,
                        host,
                        "meta.teach",
                    ),
                    granted_ability: "mentor.quote".to_string(),
                    granted_ability_ura: "easynet:///r/acme/ability/mentor.quote".to_string(),
                    owner_ura: owner.to_string(),
                    granted_by_ura: host.to_string(),
                    learner_ura: learner.to_string(),
                    manifest_hash: TEST_MANIFEST_HASH.to_string(),
                },
            )
            .expect("hosted snapshot is valid"),
        });
        grant
            .validate_stored()
            .expect("hosted snapshot accepts the local-system caller delegating for the host");

        grant.admission_snapshot.caller_ura = host.to_string();
        let err = grant.validate_stored().expect_err(
            "hosted delegation must reject a caller that is not the local-system identity",
        );
        assert!(err.to_string().contains("caller"), "{err}");
    }

    #[test]
    fn grant_is_scoped_to_one_learner() {
        let mut file = TeachGrantsFile::default();
        file.grants.push(TeachGrant::from_draft(TeachGrantDraft {
            ability: "testbot.weather-probe".to_string(),
            ability_ura: "easynet:///r/acme/ability/testbot.weather-probe".to_string(),
            owner_ura: "easynet:///r/acme/agent/testbot".to_string(),
            granted_by_ura: "easynet:///r/acme/agent/testbot".to_string(),
            owner_agent: "testbot".to_string(),
            learner_ura: "ura-b".to_string(),
            manifest_hash: TEST_MANIFEST_HASH.to_string(),
            execution_mode: EXECUTION_MODE_DEFAULT.to_string(),
            granted_at: "t0".to_string(),
            admission_snapshot: TeachGrantAdmissionSnapshot::from_draft(
                TeachGrantAdmissionSnapshotDraft {
                    invocation_id: "test-invocation".to_string(),
                    caller_ura: "easynet:///r/acme/agent/testbot".to_string(),
                    callee_ura: "easynet:///r/acme/agent/testbot".to_string(),
                    subject_ura: "easynet:///r/acme/ability/testbot.weather-probe".to_string(),
                    envelope_ability: "meta.teach".to_string(),
                    invocation_nonce_hex: hex::encode([0xA5; 16]),
                    causal_context: serde_json::json!({"kind": "none"}),
                    authority: TeachGrantAuthoritySnapshot::direct_owner(
                        "easynet:///r/acme/agent/testbot",
                    ),
                    granted_ability: "testbot.weather-probe".to_string(),
                    granted_ability_ura: "easynet:///r/acme/ability/testbot.weather-probe"
                        .to_string(),
                    owner_ura: "easynet:///r/acme/agent/testbot".to_string(),
                    granted_by_ura: "easynet:///r/acme/agent/testbot".to_string(),
                    learner_ura: "ura-b".to_string(),
                    manifest_hash: TEST_MANIFEST_HASH.to_string(),
                },
            )
            .expect("test snapshot is valid"),
        }));
        assert!(file
            .grant_index_for(
                "testbot.weather-probe",
                "easynet:///r/acme/ability/testbot.weather-probe",
                "easynet:///r/acme/agent/testbot",
                "ura-b"
            )
            .is_some());
        assert!(
            file.grant_index_for(
                "testbot.weather-probe",
                "easynet:///r/acme/ability/testbot.weather-probe",
                "easynet:///r/acme/agent/testbot",
                "ura-c"
            )
            .is_none(),
            "a grant confers to ONE learner, not to the world"
        );
        assert!(
            file.grant_index_for(
                "testbot.weather-probe",
                "easynet:///r/acme/ability/testbot.weather-probe",
                "easynet:///r/acme/agent/other-testbot",
                "ura-b"
            )
            .is_none(),
            "a grant is bound to the owner URA, not just the local registry name"
        );
    }

    #[test]
    fn acquire_staged_commit_failure_rolls_back_ledger_and_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let learner = "easynet:///r/acme/agent/apprentice";
        let grant = mentor_quote_grant(learner);
        store.grant(grant.clone()).unwrap();
        let rolled_back = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rolled_back_for_closure = std::sync::Arc::clone(&rolled_back);

        let err = store
            .acquire_staged(acquire_mentor_quote_request(
                learner,
                grant.clone(),
                TestAcquiringArtifact::with_commit_error(
                    "commit copy failed",
                    rolled_back_for_closure,
                ),
            ))
            .unwrap_err();

        assert!(err.to_string().contains("commit copy failed"));
        assert!(rolled_back.load(std::sync::atomic::Ordering::SeqCst));
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(
            file.imports.is_empty(),
            "failed acquire must not leave a descriptor-import ledger row"
        );
        assert!(
            file.grant_index_for(
                "mentor.quote",
                "easynet:///r/acme/ability/mentor.quote",
                "easynet:///r/acme/agent/mentor",
                "easynet:///r/acme/agent/apprentice"
            )
            .is_some(),
            "failed acquire must not consume the teach grant"
        );
    }

    #[test]
    fn acquire_staged_success_consumes_grant_and_records_import() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let learner = "easynet:///r/acme/agent/apprentice";
        let grant = mentor_quote_grant(learner);
        store.grant(grant.clone()).unwrap();

        store
            .acquire_staged(acquire_mentor_quote_request(
                learner,
                grant.clone(),
                TestAcquiringArtifact::committed(),
            ))
            .unwrap();

        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(
            file.grant_index_for(
                "mentor.quote",
                "easynet:///r/acme/ability/mentor.quote",
                "easynet:///r/acme/agent/mentor",
                learner
            )
            .is_none(),
            "successful acquire consumes exactly that teach grant"
        );
        assert_eq!(file.imports.len(), 1);
    }

    #[test]
    fn recover_acquiring_committed_artifact_marks_import_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let grant = mentor_quote_grant("easynet:///r/acme/agent/apprentice");
        let mut import_record = DescriptorImportRecord::new(
            "quote",
            "apprentice",
            "easynet:///r/acme/ability/mentor.quote",
            TEST_MANIFEST_HASH,
            "t1",
        );
        import_record.mark_acquiring(
            "/tmp/committed-copy",
            Some("/tmp/staged-copy".to_string()),
            "sha256:test",
            grant,
        );
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                imports: vec![import_record],
                ..TeachGrantsFile::default()
            })
            .unwrap();

        let recovered = store
            .recover_acquiring(|_| Ok(AcquiringArtifactRecoveryState::Committed))
            .unwrap();
        assert_eq!(recovered, 1);
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(file.grants.is_empty());
        assert_eq!(file.imports.len(), 1);
        assert_eq!(file.imports[0].state(), DescriptorImportState::Active);
        assert!(
            file.imports[0].acquiring_manifest_path().is_none(),
            "active descriptor-import row must not retain acquiring metadata"
        );
    }

    #[test]
    fn recover_acquiring_missing_artifact_restores_pending_grant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let grant = mentor_quote_grant("easynet:///r/acme/agent/apprentice");
        let mut import_record = DescriptorImportRecord::new(
            "quote",
            "apprentice",
            "easynet:///r/acme/ability/mentor.quote",
            TEST_MANIFEST_HASH,
            "t1",
        );
        import_record.mark_acquiring(
            "/tmp/committed-copy",
            Some("/tmp/staged-copy".to_string()),
            "sha256:test",
            grant.clone(),
        );
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                imports: vec![import_record],
                ..TeachGrantsFile::default()
            })
            .unwrap();

        let recovered = store
            .recover_acquiring(|_| Ok(AcquiringArtifactRecoveryState::NotCommitted))
            .unwrap();
        assert_eq!(recovered, 1);
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(file.imports.is_empty());
        assert!(file.grant_index_for_record(&grant).is_some());
    }

    #[test]
    fn stage_forget_keeps_tombstone_until_runtime_finalization() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                imports: vec![DescriptorImportRecord::new(
                    "quote",
                    "apprentice",
                    "easynet:///r/acme/ability/mentor.quote",
                    TEST_MANIFEST_HASH,
                    "t1",
                )],
                ..TeachGrantsFile::default()
            })
            .unwrap();

        let staged = store
            .stage_forget("apprentice", "quote", |_| Ok("staged-remove".to_string()))
            .unwrap();
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert_eq!(file.imports.len(), 1);
        assert_eq!(file.imports[0].state(), DescriptorImportState::Forgetting);

        let pending = store.commit_forget_artifact(&staged, |_| Ok(())).unwrap();
        assert_eq!(
            pending.record().source_descriptor_ura(),
            "easynet:///r/acme/ability/mentor.quote"
        );
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert_eq!(file.imports.len(), 1);
        assert_eq!(
            file.imports[0].state(),
            DescriptorImportState::Forgetting,
            "artifact commit must keep the tombstone until runtime cleanup succeeds"
        );

        let committed = store.finish_forget(&pending).unwrap();
        assert_eq!(
            committed.record().source_descriptor_ura(),
            "easynet:///r/acme/ability/mentor.quote"
        );
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(
            file.imports.is_empty(),
            "finalization removes the tombstone row"
        );
    }

    #[test]
    fn stage_forget_marks_ledger_before_artifact_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                imports: vec![DescriptorImportRecord::new(
                    "quote",
                    "apprentice",
                    "easynet:///r/acme/ability/mentor.quote",
                    TEST_MANIFEST_HASH,
                    "t1",
                )],
                ..TeachGrantsFile::default()
            })
            .unwrap();

        let staged = store
            .stage_forget("apprentice", "quote", |_| {
                let body = std::fs::read(&path)?;
                let file: TeachGrantsFile = serde_json::from_slice(&body)?;
                assert_eq!(
                    file.imports[0].state(),
                    DescriptorImportState::Forgetting,
                    "forget intent must be durable before artifact staging runs"
                );
                Ok("staged-remove".to_string())
            })
            .unwrap();
        assert!(!staged.resumed());
    }

    #[test]
    fn stage_forget_resumes_existing_forgetting_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let mut import_record = DescriptorImportRecord::new(
            "quote",
            "apprentice",
            "easynet:///r/acme/ability/mentor.quote",
            TEST_MANIFEST_HASH,
            "t1",
        );
        import_record.mark_forgetting();
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                imports: vec![import_record],
                ..TeachGrantsFile::default()
            })
            .unwrap();

        let staged = store
            .stage_forget("apprentice", "quote", |_| Ok("staged-remove".to_string()))
            .unwrap();
        assert!(staged.resumed());
        let pending = store.commit_forget_artifact(&staged, |_| Ok(())).unwrap();
        let committed = store.finish_forget(&pending).unwrap();

        assert_eq!(
            committed.record().source_descriptor_ura(),
            "easynet:///r/acme/ability/mentor.quote"
        );
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(file.imports.is_empty(), "retry must finish the tombstone");
    }

    #[test]
    fn commit_forget_cleanup_failure_keeps_tombstone_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                imports: vec![DescriptorImportRecord::new(
                    "quote",
                    "apprentice",
                    "easynet:///r/acme/ability/mentor.quote",
                    TEST_MANIFEST_HASH,
                    "t1",
                )],
                ..TeachGrantsFile::default()
            })
            .unwrap();

        let staged = store
            .stage_forget("apprentice", "quote", |_| Ok("staged-remove".to_string()))
            .unwrap();
        let err = store
            .commit_forget_artifact(&staged, |_| anyhow::bail!("simulated cleanup failure"))
            .unwrap_err();
        assert!(err.to_string().contains("simulated cleanup failure"));

        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert_eq!(file.imports.len(), 1);
        assert_eq!(
            file.imports[0].state(),
            DescriptorImportState::Forgetting,
            "cleanup failure must preserve the durable tombstone for retry"
        );
    }
}
