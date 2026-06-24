// EasyNet CLI — teach-grant directory (`teach-grants.json`)
// ===========================================================
//
// File: src/persistence/teach_grants.rs
// Description: The owner-initiative store behind GET route B
//              (seven-axes T3.3, spec §2.5): which abilities their
//              owner has explicitly made learnable, and by whom.
//
// Ontology (spec §0.1-6, non-negotiable): a capability is CONFERRED
// by its owner, never pulled by a consumer. Absence of a grant IS
// the `allow_transferred_code = false` default (capability.proto
// InstallPolicy) — an ability with no entry here is not learnable,
// full stop. `meta.teach` writes a grant; `meta.acquire` consumes
// one; both are ordinary ledgered invocations, so the receipt chain
// records who conferred what to whom.
//
// The file also keeps the LEARNED ledger: which manifests landed in
// which learner's workspace through `meta.acquire`. `meta.forget`
// only removes what this ledger names — a learner can unlearn a
// taught copy, never silently delete a native ability.
//
// Schema (operator-inspectable)
// -----------------------------
// {
//   "schema_version": "1",
//   "grants": [
//     { "ability": "<owner-local registry name, e.g. testbot.weather-probe>",
//       "owner_agent": "testbot",
//       "learner_ura": "easynet:///r/<realm>/agent/<id>",
//       "execution_mode": "sandbox_first",   // capability.proto:238 default
//       "granted_at": "<rfc3339>" },
//     …
//   ],
//   "learned": [
//     { "ability_name": "weather-probe",
//       "learner_agent": "apprentice",
//       "learned_from": "<the taught ability's canonical URA>",
//       "learned_at": "<rfc3339>" },
//     …
//   ]
// }
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};
use super::file_lock::ExclusiveFileLock;

pub(crate) const FILE_NAME: &str = "teach-grants.json";
const STORE_SCHEMA_VERSION: &str = "1";
static STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Default execution posture for transferred code
/// (capability.proto:238). A string by protocol design — the proto
/// field is a string with documented values, not an enum.
pub const EXECUTION_MODE_DEFAULT: &str = "sandbox_first";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeachGrantsFile {
    schema_version: String,
    #[serde(default)]
    grants: Vec<TeachGrant>,
    #[serde(default)]
    learned: Vec<LearnedRecord>,
}

impl Default for TeachGrantsFile {
    fn default() -> Self {
        Self {
            schema_version: STORE_SCHEMA_VERSION.to_string(),
            grants: Vec::new(),
            learned: Vec::new(),
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
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeachGrant {
    ability: String,
    #[serde(default)]
    ability_ura: String,
    #[serde(default)]
    owner_ura: String,
    owner_agent: String,
    learner_ura: String,
    manifest_hash: String,
    execution_mode: String,
    granted_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedRecord {
    ability_name: String,
    learner_agent: String,
    learned_from: String,
    manifest_hash: String,
    learned_at: String,
    #[serde(default)]
    state: LearnedRecordState,
    #[serde(default)]
    acquiring_manifest_path: Option<String>,
    #[serde(default)]
    acquiring_staging_manifest_path: Option<String>,
    #[serde(default)]
    acquiring_manifest_hash: Option<String>,
    #[serde(default)]
    pending_grant: Option<TeachGrant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearnedRecordState {
    Acquiring,
    Active,
    Forgetting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquiringArtifactRecoveryState {
    Committed,
    NotCommitted,
}

pub trait AcquiringArtifactTxn {
    fn committed_artifact_path(&self) -> String;

    fn staging_artifact_path(&self) -> Option<String>;

    fn content_hash(&self) -> String;

    fn commit(&self) -> anyhow::Result<()>;

    fn rollback(&self) -> anyhow::Result<()>;
}

impl Default for LearnedRecordState {
    fn default() -> Self {
        Self::Active
    }
}

impl TeachGrant {
    #[must_use]
    pub fn new(
        ability: impl Into<String>,
        ability_ura: impl Into<String>,
        owner_ura: impl Into<String>,
        owner_agent: impl Into<String>,
        learner_ura: impl Into<String>,
        manifest_hash: impl Into<String>,
        execution_mode: impl Into<String>,
        granted_at: impl Into<String>,
    ) -> Self {
        Self {
            ability: ability.into(),
            ability_ura: ability_ura.into(),
            owner_ura: owner_ura.into(),
            owner_agent: owner_agent.into(),
            learner_ura: learner_ura.into(),
            manifest_hash: manifest_hash.into(),
            execution_mode: execution_mode.into(),
            granted_at: granted_at.into(),
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
}

impl LearnedRecord {
    #[must_use]
    pub fn new(
        ability_name: impl Into<String>,
        learner_agent: impl Into<String>,
        learned_from: impl Into<String>,
        manifest_hash: impl Into<String>,
        learned_at: impl Into<String>,
    ) -> Self {
        Self {
            ability_name: ability_name.into(),
            learner_agent: learner_agent.into(),
            learned_from: learned_from.into(),
            manifest_hash: manifest_hash.into(),
            learned_at: learned_at.into(),
            state: LearnedRecordState::Active,
            acquiring_manifest_path: None,
            acquiring_staging_manifest_path: None,
            acquiring_manifest_hash: None,
            pending_grant: None,
        }
    }

    #[must_use]
    pub fn learned_from(&self) -> &str {
        &self.learned_from
    }

    #[must_use]
    pub fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }

    #[must_use]
    pub fn state(&self) -> LearnedRecordState {
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
        self.state = LearnedRecordState::Active;
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
        self.state = LearnedRecordState::Acquiring;
        self.acquiring_manifest_path = Some(committed_manifest_path.into());
        self.acquiring_staging_manifest_path = staging_manifest_path;
        let manifest_hash = manifest_hash.into();
        self.manifest_hash = manifest_hash.clone();
        self.acquiring_manifest_hash = Some(manifest_hash);
        self.pending_grant = Some(pending_grant);
    }

    fn mark_forgetting(&mut self) {
        self.state = LearnedRecordState::Forgetting;
    }

    fn same_identity(&self, other: &LearnedRecord) -> bool {
        self.ability_name == other.ability_name
            && self.learner_agent == other.learner_agent
            && self.learned_from == other.learned_from
            && self.manifest_hash == other.manifest_hash
            && self.learned_at == other.learned_at
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
/// consumed or which learned ledger row was written.
#[derive(Debug)]
pub struct AcquiredTeachGrant {
    grant: TeachGrant,
    learned: LearnedRecord,
}

impl AcquiredTeachGrant {
    pub fn grant(&self) -> &TeachGrant {
        &self.grant
    }

    pub fn learned(&self) -> &LearnedRecord {
        &self.learned
    }
}

/// Durable `meta.forget` transaction.
///
/// The learned ledger row is marked `forgetting` before runtime cleanup starts.
/// That tombstone blocks duplicate acquire/forget races and lets a retry finish
/// the delete instead of replaying an already-staged manifest as active state.
#[derive(Debug)]
pub struct StagedForgottenTeachGrant<T> {
    record: LearnedRecord,
    staged_artifact: T,
    resumed: bool,
}

impl<T> StagedForgottenTeachGrant<T> {
    #[must_use]
    pub fn record(&self) -> &LearnedRecord {
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
pub struct RuntimePendingForgottenTeachGrant {
    record: LearnedRecord,
}

impl RuntimePendingForgottenTeachGrant {
    #[must_use]
    pub fn record(&self) -> &LearnedRecord {
        &self.record
    }
}

#[derive(Debug)]
pub struct CommittedForgottenTeachGrant {
    record: LearnedRecord,
}

impl CommittedForgottenTeachGrant {
    #[must_use]
    pub fn record(&self) -> &LearnedRecord {
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
            &LearnedRecord,
        ) -> anyhow::Result<AcquiringArtifactRecoveryState>,
    ) -> anyhow::Result<usize> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let mut recovered = 0usize;
        let mut idx = 0usize;
        while idx < directory.learned.len() {
            if directory.learned[idx].state() != LearnedRecordState::Acquiring {
                idx += 1;
                continue;
            }
            let record = directory.learned[idx].clone();
            match recover_artifact(&record)? {
                AcquiringArtifactRecoveryState::Committed => {
                    directory.learned[idx].mark_active();
                    recovered += 1;
                    idx += 1;
                    continue;
                }
                AcquiringArtifactRecoveryState::NotCommitted => {}
            }
            if let Some(grant) = record.pending_grant.clone() {
                if directory.grant_index_for_record(&grant).is_none() {
                    directory.grants.push(grant);
                }
            }
            directory.learned.remove(idx);
            recovered += 1;
        }
        if recovered > 0 {
            self.write_unlocked(&directory)?;
        }
        Ok(recovered)
    }

    pub fn acquire_staged<T>(
        &self,
        registry_name: &str,
        ability_ura: &str,
        owner_ura: &str,
        learner_ura: &str,
        expected_grant: &TeachGrant,
        learned: LearnedRecord,
        staged: T,
    ) -> anyhow::Result<AcquiredTeachGrant>
    where
        T: AcquiringArtifactTxn,
    {
        let acquired = match (|| {
            let _guard = self.lock()?;
            let mut directory = self.load_unlocked()?;
            let grant_idx = directory
                .grant_index_for(registry_name, ability_ura, owner_ura, learner_ura)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "not teachable (allow_transferred_code=false): the owner has not \
                     taught {ability_ura:?} ({registry_name:?}) from owner {owner_ura:?} to {learner_ura}"
                    )
                })?;
            let grant = directory.grants[grant_idx].clone();
            if &grant != expected_grant {
                anyhow::bail!(
                    "teach grant changed during acquire admission; retry acquire so execution \
                     posture is evaluated against the committed grant"
                );
            }
            if directory
                .learned_by_active_or_forgetting(&learned.learner_agent, &learned.ability_name)
                .is_some()
            {
                anyhow::bail!(
                    "agent {:?} already learned {:?} or has a forget transaction in progress; \
                     finish forgetting it first or rename — learning never overwrites",
                    learned.learner_agent,
                    learned.ability_name
                );
            }

            let mut acquiring = learned;
            acquiring.mark_acquiring(
                staged.committed_artifact_path(),
                staged.staging_artifact_path(),
                staged.content_hash(),
                grant.clone(),
            );
            directory.grants.remove(grant_idx);
            directory.learned.push(acquiring.clone());
            self.write_unlocked(&directory)?;
            Ok(AcquiredTeachGrant {
                grant,
                learned: acquiring,
            })
        })() {
            Ok(acquired) => acquired,
            Err(ledger_err) => {
                return Err(append_cleanup_error(
                    ledger_err,
                    staged.rollback(),
                    "rollback staged learned manifest",
                ));
            }
        };

        if let Err(commit_err) = staged.commit() {
            let rollback_err = staged.rollback();
            let restore_err = self.restore_acquired_ledger_after_artifact_commit_failure(&acquired);
            return Err(append_cleanup_error(
                append_cleanup_error(commit_err, rollback_err, "rollback staged learned manifest"),
                restore_err,
                "restore teach grant ledger after failed learned-manifest commit",
            ));
        }
        let active_learned = self.commit_acquired_ledger_after_artifact_commit(&acquired)?;
        Ok(AcquiredTeachGrant {
            grant: acquired.grant,
            learned: active_learned,
        })
    }

    fn commit_acquired_ledger_after_artifact_commit(
        &self,
        acquired: &AcquiredTeachGrant,
    ) -> anyhow::Result<LearnedRecord> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(learned_idx) = directory.learned_by_record(&acquired.learned) else {
            anyhow::bail!(
                "commit acquired ledger: acquiring row for agent {:?} ability {:?} is absent",
                acquired.learned.learner_agent,
                acquired.learned.ability_name
            );
        };
        if directory.learned[learned_idx].state() != LearnedRecordState::Acquiring {
            anyhow::bail!(
                "commit acquired ledger: learned row for agent {:?} ability {:?} is not acquiring",
                acquired.learned.learner_agent,
                acquired.learned.ability_name
            );
        }
        directory.learned[learned_idx].mark_active();
        let active = directory.learned[learned_idx].clone();
        self.write_unlocked(&directory)?;
        Ok(active)
    }

    pub fn restore_acquired_grant_after_failure<T>(
        &self,
        acquired: &AcquiredTeachGrant,
        stage_remove: impl FnOnce(&LearnedRecord) -> anyhow::Result<T>,
        commit_remove: impl FnOnce(&T) -> anyhow::Result<()>,
        rollback_remove: impl FnOnce(&T) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let learned = {
            let _guard = self.lock()?;
            let directory = self.load_unlocked()?;
            let Some(learned_idx) = directory.learned_by(
                &acquired.learned.learner_agent,
                &acquired.learned.ability_name,
            ) else {
                anyhow::bail!(
                    "restore acquired teach grant: learned ledger row for agent {:?} ability {:?} \
                     is already absent",
                    acquired.learned.learner_agent,
                    acquired.learned.ability_name
                );
            };
            let learned = directory.learned[learned_idx].clone();
            if learned != acquired.learned {
                anyhow::bail!(
                    "restore acquired teach grant: learned ledger row changed under rollback; \
                     refusing to restore grant for {:?}",
                    acquired.learned.ability_name
                );
            }
            if directory.grant_index_for_record(&acquired.grant).is_some() {
                anyhow::bail!(
                    "restore acquired teach grant: grant for {:?} to {:?} is already present",
                    acquired.grant.ability_ura,
                    acquired.grant.learner_ura
                );
            }
            learned
        };

        let staged = stage_remove(&learned)?;
        let restore_result = (|| {
            let _guard = self.lock()?;
            let mut directory = self.load_unlocked()?;
            let Some(learned_idx) = directory.learned_by_record(&learned) else {
                anyhow::bail!(
                    "restore acquired teach grant: learned ledger row changed before restore \
                     commit for agent {:?} ability {:?}",
                    learned.learner_agent,
                    learned.ability_name
                );
            };
            if directory.grant_index_for_record(&acquired.grant).is_some() {
                anyhow::bail!(
                    "restore acquired teach grant: grant for {:?} to {:?} is already present",
                    acquired.grant.ability_ura,
                    acquired.grant.learner_ura
                );
            }
            directory.learned.remove(learned_idx);
            directory.grants.push(acquired.grant.clone());
            self.write_unlocked(&directory)?;
            Ok(directory)
        })();
        let directory = match restore_result {
            Ok(directory) => directory,
            Err(write_err) => {
                return Err(append_cleanup_error(
                    write_err,
                    rollback_remove(&staged),
                    "restore staged learned manifest",
                ));
            }
        };
        if let Err(commit_err) = commit_remove(&staged) {
            let artifact_rollback = rollback_remove(&staged);
            let restore_err =
                self.restore_acquired_ledger_unlocked(directory, acquired.grant.clone(), learned);
            return Err(append_cleanup_error(
                append_cleanup_error(
                    commit_err,
                    artifact_rollback,
                    "restore staged learned manifest",
                ),
                restore_err,
                "restore acquired teach ledger after failed rollback cleanup",
            ));
        }
        Ok(())
    }

    pub fn stage_forget<T>(
        &self,
        learner_agent: &str,
        ability_name: &str,
        stage_remove: impl FnOnce(&LearnedRecord) -> anyhow::Result<T>,
    ) -> anyhow::Result<StagedForgottenTeachGrant<T>> {
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
        Ok(StagedForgottenTeachGrant {
            record,
            staged_artifact: staged,
            resumed,
        })
    }

    pub fn commit_forget_artifact<T>(
        &self,
        staged: &StagedForgottenTeachGrant<T>,
        commit_remove: impl FnOnce(&T) -> anyhow::Result<()>,
    ) -> anyhow::Result<RuntimePendingForgottenTeachGrant> {
        self.require_forgetting_ledger_row(staged.record())?;
        commit_remove(staged.staged_artifact())?;
        self.require_forgetting_ledger_row(staged.record())?;
        Ok(RuntimePendingForgottenTeachGrant {
            record: staged.record().clone(),
        })
    }

    pub fn finish_forget(
        &self,
        pending: &RuntimePendingForgottenTeachGrant,
    ) -> anyhow::Result<CommittedForgottenTeachGrant> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(idx) = directory.learned_by_record(pending.record()) else {
            anyhow::bail!(
                "finish forget: learned ledger row for agent {:?} ability {:?} is already absent",
                pending.record.learner_agent,
                pending.record.ability_name
            );
        };
        if directory.learned[idx].state() != LearnedRecordState::Forgetting {
            anyhow::bail!(
                "finish forget: learned ledger row for agent {:?} ability {:?} is not in \
                 forgetting state",
                pending.record.learner_agent,
                pending.record.ability_name
            );
        }
        let record = directory.learned.remove(idx);
        self.write_unlocked(&directory)?;
        Ok(CommittedForgottenTeachGrant { record })
    }

    fn begin_forget_ledger(
        &self,
        learner_agent: &str,
        ability_name: &str,
    ) -> anyhow::Result<(LearnedRecord, bool)> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(idx) = directory.learned_by_active_or_forgetting(learner_agent, ability_name)
        else {
            anyhow::bail!(
                "agent {learner_agent:?} never learned {ability_name:?} — only learned \
                 abilities can be forgotten (native abilities are removed by their author, \
                 not by forget)"
            );
        };
        let record = directory.learned[idx].clone();
        match record.state() {
            LearnedRecordState::Acquiring => {
                anyhow::bail!(
                    "agent {learner_agent:?} has an acquire transaction in progress for {ability_name:?}; \
                     finish or recover acquire before forgetting it"
                );
            }
            LearnedRecordState::Forgetting => Ok((record, true)),
            LearnedRecordState::Active => {
                directory.learned[idx].mark_forgetting();
                self.write_unlocked(&directory)?;
                Ok((record, false))
            }
        }
    }

    fn require_forgetting_ledger_row(&self, record: &LearnedRecord) -> anyhow::Result<()> {
        let _guard = self.lock()?;
        let directory = self.load_unlocked()?;
        let Some(idx) = directory.learned_by_record(record) else {
            anyhow::bail!(
                "commit forget: learned ledger row for agent {:?} ability {:?} is already absent",
                record.learner_agent,
                record.ability_name
            );
        };
        if directory.learned[idx].state() != LearnedRecordState::Forgetting {
            anyhow::bail!(
                "commit forget: learned ledger row for agent {:?} ability {:?} is not in \
                 forgetting state",
                record.learner_agent,
                record.ability_name
            );
        }
        Ok(())
    }

    fn rollback_forget_ledger(&self, record: &LearnedRecord) -> anyhow::Result<()> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(idx) = directory.learned_by_record(record) else {
            anyhow::bail!(
                "rollback forget: learned ledger row for agent {:?} ability {:?} is already absent",
                record.learner_agent,
                record.ability_name
            );
        };
        if directory.learned[idx].state() == LearnedRecordState::Forgetting {
            directory.learned[idx].mark_active();
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

    fn restore_acquired_ledger_unlocked(
        &self,
        mut directory: TeachGrantsFile,
        grant: TeachGrant,
        mut learned: LearnedRecord,
    ) -> anyhow::Result<()> {
        if let Some(grant_idx) = directory.grant_index_for_record(&grant) {
            directory.grants.remove(grant_idx);
        }
        learned.mark_active();
        directory.learned.push(learned);
        self.write_unlocked(&directory)
    }

    fn restore_acquired_ledger_after_artifact_commit_failure(
        &self,
        acquired: &AcquiredTeachGrant,
    ) -> anyhow::Result<()> {
        let _guard = self.lock()?;
        let mut directory = self.load_unlocked()?;
        let Some(learned_idx) = directory.learned_by_record(&acquired.learned) else {
            anyhow::bail!(
                "restore acquired ledger: learned ledger row for agent {:?} ability {:?} \
                 is already absent",
                acquired.learned.learner_agent,
                acquired.learned.ability_name
            );
        };
        let learned = directory.learned[learned_idx].clone();
        if learned != acquired.learned {
            anyhow::bail!(
                "restore acquired ledger: learned ledger row changed under rollback; \
                 refusing to restore grant for {:?}",
                acquired.learned.ability_name
            );
        }
        if directory.grant_index_for_record(&acquired.grant).is_some() {
            anyhow::bail!(
                "restore acquired ledger: grant for {:?} to {:?} is already present",
                acquired.grant.ability_ura,
                acquired.grant.learner_ura
            );
        }
        directory.learned.remove(learned_idx);
        directory.grants.push(acquired.grant.clone());
        self.write_unlocked(&directory)
    }
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

    /// Active learned-ledger row for `(learner, ability)`.
    fn learned_by(&self, learner_agent: &str, ability_name: &str) -> Option<usize> {
        self.learned.iter().position(|l| {
            l.learner_agent == learner_agent
                && l.ability_name == ability_name
                && l.state == LearnedRecordState::Active
        })
    }

    /// Active or tombstoned row for `(learner, ability)`. Used by
    /// acquire/forget admission so an in-progress forget cannot be treated as
    /// free space for a second learned copy.
    fn learned_by_active_or_forgetting(
        &self,
        learner_agent: &str,
        ability_name: &str,
    ) -> Option<usize> {
        self.learned
            .iter()
            .position(|l| l.learner_agent == learner_agent && l.ability_name == ability_name)
    }

    fn learned_by_record(&self, record: &LearnedRecord) -> Option<usize> {
        self.learned
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
        std::fs::write(&path, br#"{"grants":[],"learned":[]}"#).unwrap();
        let store = TeachGrantStore { path };

        let err = store.load_unlocked().expect_err("missing schema version");
        assert!(err.to_string().contains("parse"), "{err}");
    }

    #[test]
    fn grant_is_scoped_to_one_learner() {
        let mut file = TeachGrantsFile::default();
        file.grants.push(TeachGrant::new(
            "testbot.weather-probe",
            "easynet:///r/acme/ability/testbot.weather-probe",
            "easynet:///r/acme/agent/testbot",
            "testbot",
            "ura-b",
            TEST_MANIFEST_HASH,
            EXECUTION_MODE_DEFAULT,
            "t0",
        ));
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
        let grant = TeachGrant::new(
            "mentor.quote",
            "easynet:///r/acme/ability/mentor.quote",
            "easynet:///r/acme/agent/mentor",
            "mentor",
            "easynet:///r/acme/agent/apprentice",
            TEST_MANIFEST_HASH,
            EXECUTION_MODE_DEFAULT,
            "t0",
        );
        store.grant(grant.clone()).unwrap();
        let rolled_back = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rolled_back_for_closure = std::sync::Arc::clone(&rolled_back);

        let err = store
            .acquire_staged(
                "mentor.quote",
                "easynet:///r/acme/ability/mentor.quote",
                "easynet:///r/acme/agent/mentor",
                "easynet:///r/acme/agent/apprentice",
                &grant,
                LearnedRecord::new(
                    "quote",
                    "apprentice",
                    "easynet:///r/acme/ability/mentor.quote",
                    TEST_MANIFEST_HASH,
                    "t1",
                ),
                TestAcquiringArtifact::with_commit_error(
                    "commit copy failed",
                    rolled_back_for_closure,
                ),
            )
            .unwrap_err();

        assert!(err.to_string().contains("commit copy failed"));
        assert!(rolled_back.load(std::sync::atomic::Ordering::SeqCst));
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(
            file.learned.is_empty(),
            "failed acquire must not leave a learned ledger row"
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
    fn acquire_staged_success_consumes_grant_and_records_learned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let learner = "easynet:///r/acme/agent/apprentice";
        let grant = TeachGrant::new(
            "mentor.quote",
            "easynet:///r/acme/ability/mentor.quote",
            "easynet:///r/acme/agent/mentor",
            "mentor",
            learner,
            TEST_MANIFEST_HASH,
            EXECUTION_MODE_DEFAULT,
            "t0",
        );
        store.grant(grant.clone()).unwrap();

        store
            .acquire_staged(
                "mentor.quote",
                "easynet:///r/acme/ability/mentor.quote",
                "easynet:///r/acme/agent/mentor",
                learner,
                &grant,
                LearnedRecord::new(
                    "quote",
                    "apprentice",
                    "easynet:///r/acme/ability/mentor.quote",
                    TEST_MANIFEST_HASH,
                    "t1",
                ),
                TestAcquiringArtifact::committed(),
            )
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
        assert_eq!(file.learned.len(), 1);
    }

    #[test]
    fn recover_acquiring_committed_artifact_marks_learned_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let grant = TeachGrant::new(
            "mentor.quote",
            "easynet:///r/acme/ability/mentor.quote",
            "easynet:///r/acme/agent/mentor",
            "mentor",
            "easynet:///r/acme/agent/apprentice",
            TEST_MANIFEST_HASH,
            EXECUTION_MODE_DEFAULT,
            "t0",
        );
        let mut learned = LearnedRecord::new(
            "quote",
            "apprentice",
            "easynet:///r/acme/ability/mentor.quote",
            TEST_MANIFEST_HASH,
            "t1",
        );
        learned.mark_acquiring(
            "/tmp/committed-copy",
            Some("/tmp/staged-copy".to_string()),
            "sha256:test",
            grant,
        );
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                learned: vec![learned],
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
        assert_eq!(file.learned.len(), 1);
        assert_eq!(file.learned[0].state(), LearnedRecordState::Active);
        assert!(
            file.learned[0].acquiring_manifest_path().is_none(),
            "active learned row must not retain acquiring metadata"
        );
    }

    #[test]
    fn recover_acquiring_missing_artifact_restores_pending_grant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let grant = TeachGrant::new(
            "mentor.quote",
            "easynet:///r/acme/ability/mentor.quote",
            "easynet:///r/acme/agent/mentor",
            "mentor",
            "easynet:///r/acme/agent/apprentice",
            TEST_MANIFEST_HASH,
            EXECUTION_MODE_DEFAULT,
            "t0",
        );
        let mut learned = LearnedRecord::new(
            "quote",
            "apprentice",
            "easynet:///r/acme/ability/mentor.quote",
            TEST_MANIFEST_HASH,
            "t1",
        );
        learned.mark_acquiring(
            "/tmp/committed-copy",
            Some("/tmp/staged-copy".to_string()),
            "sha256:test",
            grant.clone(),
        );
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                learned: vec![learned],
                ..TeachGrantsFile::default()
            })
            .unwrap();

        let recovered = store
            .recover_acquiring(|_| Ok(AcquiringArtifactRecoveryState::NotCommitted))
            .unwrap();
        assert_eq!(recovered, 1);
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(file.learned.is_empty());
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
                learned: vec![LearnedRecord::new(
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
        assert_eq!(file.learned.len(), 1);
        assert_eq!(file.learned[0].state(), LearnedRecordState::Forgetting);

        let pending = store.commit_forget_artifact(&staged, |_| Ok(())).unwrap();
        assert_eq!(
            pending.record().learned_from(),
            "easynet:///r/acme/ability/mentor.quote"
        );
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert_eq!(file.learned.len(), 1);
        assert_eq!(
            file.learned[0].state(),
            LearnedRecordState::Forgetting,
            "artifact commit must keep the tombstone until runtime cleanup succeeds"
        );

        let committed = store.finish_forget(&pending).unwrap();
        assert_eq!(
            committed.record().learned_from(),
            "easynet:///r/acme/ability/mentor.quote"
        );
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(
            file.learned.is_empty(),
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
                learned: vec![LearnedRecord::new(
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
                    file.learned[0].state(),
                    LearnedRecordState::Forgetting,
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
        let mut learned = LearnedRecord::new(
            "quote",
            "apprentice",
            "easynet:///r/acme/ability/mentor.quote",
            TEST_MANIFEST_HASH,
            "t1",
        );
        learned.mark_forgetting();
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                learned: vec![learned],
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
            committed.record().learned_from(),
            "easynet:///r/acme/ability/mentor.quote"
        );
        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(file.learned.is_empty(), "retry must finish the tombstone");
    }

    #[test]
    fn commit_forget_cleanup_failure_keeps_tombstone_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        store
            .write_unlocked(&TeachGrantsFile {
                grants: Vec::new(),
                learned: vec![LearnedRecord::new(
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
        assert_eq!(file.learned.len(), 1);
        assert_eq!(
            file.learned[0].state(),
            LearnedRecordState::Forgetting,
            "cleanup failure must preserve the durable tombstone for retry"
        );
    }

    #[test]
    fn restore_acquired_grant_after_failure_restores_authorization_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let store = TeachGrantStore { path: path.clone() };
        let learner = "easynet:///r/acme/agent/apprentice";
        let grant = TeachGrant::new(
            "mentor.quote",
            "easynet:///r/acme/ability/mentor.quote",
            "easynet:///r/acme/agent/mentor",
            "mentor",
            learner,
            TEST_MANIFEST_HASH,
            EXECUTION_MODE_DEFAULT,
            "t0",
        );
        store.grant(grant.clone()).unwrap();

        let acquired = store
            .acquire_staged(
                "mentor.quote",
                "easynet:///r/acme/ability/mentor.quote",
                "easynet:///r/acme/agent/mentor",
                learner,
                &grant,
                LearnedRecord::new(
                    "quote",
                    "apprentice",
                    "easynet:///r/acme/ability/mentor.quote",
                    TEST_MANIFEST_HASH,
                    "t1",
                ),
                TestAcquiringArtifact::committed(),
            )
            .unwrap();

        store
            .restore_acquired_grant_after_failure(
                &acquired,
                |_| Ok("staged-remove".to_string()),
                |_| Ok(()),
                |_| Ok(()),
            )
            .unwrap();

        let body = std::fs::read(&path).unwrap();
        let file: TeachGrantsFile = serde_json::from_slice(&body).unwrap();
        assert!(
            file.learned.is_empty(),
            "rollback must remove the learned ledger row"
        );
        assert!(
            file.grant_index_for(
                "mentor.quote",
                "easynet:///r/acme/ability/mentor.quote",
                "easynet:///r/acme/agent/mentor",
                learner
            )
            .is_some(),
            "rollback must restore the consumed teach grant"
        );
    }
}
