// EasyNet CLI — DeviceAbilityStore
// =================================================================
//
// File: src/daemon/ability/builtins/device_control/ability_management/store.rs
//
// Durable catalog of device-owned abilities installed through the
// `ability.deploy` transaction. This is the "durable catalog commit"
// leg of the deploy invariant:
//
//     ability.deploy = manifest materialization
//                    + runtime binding
//                    + durable catalog commit
//
// Without it, a hot-registered device ability is "live but not
// durable" — it vanishes on the next daemon restart, leaving catalog /
// route / runtime inconsistent. At boot the registrar replays this
// store back into the live runtime.
//
// Invariants this file owns:
//   * install_id is STABLE — derived from
//     hash(ability_ura + manifest_hash). Re-deploying
//     the same manifest upserts the same row, never a duplicate
//     (plan invariant 6).
//   * writes are atomic + fsync'd where the platform supports directory
//     fsync (tmp file + rename + parent-dir sync), so a crash mid-write
//     never leaves a torn store.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use base64::Engine as _;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::daemon::persistence::config::sync_directory;
use crate::daemon::persistence::file_lock::{ExclusiveFileLock, SharedFileLock};

const STORE_SCHEMA_VERSION: &str = "1";
const STORE_FILE: &str = "device-abilities.json";
static STORE_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static STORE_LOCK: Lazy<RwLock<()>> = Lazy::new(|| RwLock::new(()));

/// One durably-recorded device ability install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceAbilityRecord {
    /// Lifecycle state for crash-recoverable install/uninstall.
    state: DeviceAbilityRecordState,
    /// Stable id: `hash(ability_ura + manifest_hash)`.
    /// Re-deploy of the same manifest yields the same id (upsert).
    install_id: String,
    /// Wire dispatch key (e.g. `er.generate`).
    public_name: String,
    /// Namespace segment (`er`); kept for re-derivation / display.
    namespace: String,
    /// Canonical device-ability URA.
    ability_ura: String,
    /// Absolute path to the deployed bundle's `ability.json`.
    /// Kept for operator display only. Replay must use the embedded
    /// `manifest_snapshot_b64`; the source bundle is not authority after
    /// deploy commit.
    manifest_path: String,
    /// SHA-256 of the manifest bytes at install time.
    manifest_hash: String,
    /// Base64 copy of the exact manifest bytes committed by deploy.
    manifest_snapshot_b64: String,
    /// Unix epoch ms when this row was written. Caller-supplied (the
    /// runtime forbids ambient clock reads); 0 if unknown.
    installed_at_unix_ms: u64,
}

/// Durable lifecycle for a device ability row.
///
/// `Installing` is an uncommitted intent. Boot recovery deletes it because
/// LocalRuntime/control-plane state is process-local and cannot survive a crash;
/// any previous `Installed` row for the same public ability remains the replay
/// authority until `commit_installed` atomically promotes the staged row.
///
/// `Removing` is a tombstone: boot replay must not re-register it, but the row
/// remains available for rollback until the uninstall state machine commits the
/// final physical delete.
///
/// `Quarantined` is a boot-recovery terminal for rows whose device authority is
/// no longer hosted by the current daemon, typically after `device join`
/// overwrote credentials with a new Hub-issued node id. Quarantined rows are
/// kept for audit but hidden from replay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAbilityRecordState {
    Installing,
    #[default]
    Installed,
    Removing,
    Quarantined,
}

impl DeviceAbilityRecord {
    /// Build a durable record with an embedded manifest snapshot.
    #[must_use]
    pub fn new_with_manifest_bytes(
        public_name: impl Into<String>,
        namespace: impl Into<String>,
        ability_ura: impl Into<String>,
        manifest_path: impl Into<String>,
        manifest_bytes: &[u8],
        installed_at_unix_ms: u64,
    ) -> Self {
        let ability_ura = ability_ura.into();
        let manifest_hash = manifest_digest(manifest_bytes);
        Self {
            state: DeviceAbilityRecordState::Installed,
            install_id: Self::derive_install_id(&ability_ura, &manifest_hash),
            public_name: public_name.into(),
            namespace: namespace.into(),
            ability_ura,
            manifest_path: manifest_path.into(),
            manifest_hash,
            manifest_snapshot_b64: base64::engine::general_purpose::STANDARD.encode(manifest_bytes),
            installed_at_unix_ms,
        }
    }

    /// Build a staged install row. Boot replay intentionally ignores this
    /// state until the deploy transaction commits it to `Installed`.
    #[must_use]
    pub fn new_installing_with_manifest_bytes(
        public_name: impl Into<String>,
        namespace: impl Into<String>,
        ability_ura: impl Into<String>,
        manifest_path: impl Into<String>,
        manifest_bytes: &[u8],
        installed_at_unix_ms: u64,
    ) -> Self {
        let mut record = Self::new_with_manifest_bytes(
            public_name,
            namespace,
            ability_ura,
            manifest_path,
            manifest_bytes,
            installed_at_unix_ms,
        );
        record.mark_installing();
        record
    }

    /// Derive the stable install_id (plan invariant 6).
    #[must_use]
    pub fn derive_install_id(ability_ura: &str, manifest_hash: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(ability_ura.as_bytes());
        hasher.update(b"\0");
        hasher.update(manifest_hash.as_bytes());
        format!("dev-{:x}", hasher.finalize())
    }

    #[must_use]
    pub fn install_id(&self) -> &str {
        &self.install_id
    }

    #[must_use]
    pub fn public_name(&self) -> &str {
        &self.public_name
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn ability_ura(&self) -> &str {
        &self.ability_ura
    }

    #[must_use]
    pub fn manifest_path(&self) -> &str {
        &self.manifest_path
    }

    #[must_use]
    pub fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }

    pub fn manifest_bytes(&self) -> anyhow::Result<Vec<u8>> {
        if self.manifest_snapshot_b64.trim().is_empty() {
            anyhow::bail!(
                "device ability record {} has no embedded manifest snapshot; redeploy required",
                self.install_id
            );
        }
        base64::engine::general_purpose::STANDARD
            .decode(self.manifest_snapshot_b64.trim())
            .map_err(|e| anyhow::anyhow!("decode embedded device ability manifest snapshot: {e}"))
    }

    #[must_use]
    pub fn installed_at_unix_ms(&self) -> u64 {
        self.installed_at_unix_ms
    }

    #[must_use]
    pub fn state(&self) -> DeviceAbilityRecordState {
        self.state
    }

    fn mark_removing(&mut self) {
        self.state = DeviceAbilityRecordState::Removing;
    }

    fn mark_quarantined(&mut self) {
        self.state = DeviceAbilityRecordState::Quarantined;
    }

    fn mark_installing(&mut self) {
        self.state = DeviceAbilityRecordState::Installing;
    }

    fn mark_installed(&mut self) {
        self.state = DeviceAbilityRecordState::Installed;
    }
}

/// Result of staging an uninstall in the durable store.
///
/// The first uninstall request moves matching rows from `Installed` to
/// `Removing`. A later retry sees the same `Removing` rows and receives the
/// same plan again, so a failed final commit is recoverable without replaying
/// stale abilities at boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAbilityRemovalPlan {
    records: Vec<DeviceAbilityRecord>,
    resumed: bool,
}

impl DeviceAbilityRemovalPlan {
    fn new(records: Vec<DeviceAbilityRecord>, resumed: bool) -> Self {
        Self { records, resumed }
    }

    #[must_use]
    pub fn records(&self) -> &[DeviceAbilityRecord] {
        &self.records
    }

    #[must_use]
    pub fn resumed(&self) -> bool {
        self.resumed
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub fn install_ids(&self) -> Vec<String> {
        self.records
            .iter()
            .map(|record| record.install_id().to_string())
            .collect()
    }

    pub fn into_records(self) -> Vec<DeviceAbilityRecord> {
        self.records
    }
}

/// SHA-256 of arbitrary bytes, rendered as `sha256:<hex>`.
#[must_use]
pub fn manifest_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreFile {
    schema_version: String,
    installed: Vec<DeviceAbilityRecord>,
}

/// On-disk store at `~/.easynet/device-abilities.json`.
pub struct DeviceAbilityStore {
    path: PathBuf,
}

struct DeviceAbilityStoreReadLock {
    _process: RwLockReadGuard<'static, ()>,
    _file: SharedFileLock,
}

struct DeviceAbilityStoreWriteLock {
    _process: RwLockWriteGuard<'static, ()>,
    _file: ExclusiveFileLock,
}

impl DeviceAbilityStore {
    /// Open the store at the canonical daemon state root.
    pub fn try_open_default() -> anyhow::Result<Self> {
        Ok(Self {
            path: crate::daemon::persistence::config::try_state_dir()?.join(STORE_FILE),
        })
    }

    /// Open the store at an explicit path (tests).
    #[must_use]
    pub fn open_at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Read every recorded install. A missing file is the valid
    /// "nothing installed yet" state and yields an empty list.
    pub fn load(&self) -> anyhow::Result<Vec<DeviceAbilityRecord>> {
        let _guard = self.lock_for_read()?;
        self.load_unlocked()
    }

    fn load_unlocked(&self) -> anyhow::Result<Vec<DeviceAbilityRecord>> {
        Ok(self
            .load_all_unlocked()?
            .into_iter()
            .filter(|record| record.state == DeviceAbilityRecordState::Installed)
            .collect())
    }

    fn load_all_unlocked(&self) -> anyhow::Result<Vec<DeviceAbilityRecord>> {
        let body = match fs::read_to_string(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let parsed: StoreFile = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!(
                "device-abilities.json at {} is corrupt: {e}",
                self.path.display()
            )
        })?;
        if parsed.schema_version != STORE_SCHEMA_VERSION {
            anyhow::bail!(
                "device-abilities.json at {} has unsupported schema_version {:?}; expected {:?}",
                self.path.display(),
                parsed.schema_version,
                STORE_SCHEMA_VERSION
            );
        }
        Ok(parsed.installed)
    }

    /// Test-only installed-row writer.
    ///
    /// Production deploys must go through `stage_install_record` plus
    /// `commit_installed`; direct Installed writes would bypass LocalRuntime and
    /// control-plane verification.
    #[cfg(test)]
    pub(crate) fn upsert(
        &self,
        record: DeviceAbilityRecord,
    ) -> anyhow::Result<Option<DeviceAbilityRecord>> {
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        let previous = match rows.iter_mut().find(|r| r.install_id == record.install_id) {
            Some(existing) => Some(std::mem::replace(existing, record)),
            None => {
                rows.push(record);
                None
            }
        };
        self.write_all(&rows)?;
        Ok(previous)
    }

    /// Stage the one durable row that may become the next live device ability.
    ///
    /// Staging never deletes the current `Installed` row. That is the crash
    /// boundary: if the daemon dies before `commit_installed`, boot replay sees
    /// the old active row and ignores the staged intent. The final commit is the
    /// only transition allowed to remove displaced active rows.
    pub fn stage_install_record(
        &self,
        mut record: DeviceAbilityRecord,
    ) -> anyhow::Result<Vec<DeviceAbilityRecord>> {
        record.mark_installing();
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        let replaced = rows
            .iter()
            .filter(|existing| {
                existing.state == DeviceAbilityRecordState::Installed
                    && (existing.install_id == record.install_id
                        || existing.ability_ura == record.ability_ura
                        || existing.public_name == record.public_name)
            })
            .cloned()
            .collect::<Vec<_>>();
        rows.retain(|existing| {
            existing.state != DeviceAbilityRecordState::Installing
                || (existing.install_id != record.install_id
                    && existing.ability_ura != record.ability_ura
                    && existing.public_name != record.public_name)
        });
        rows.push(record);
        self.write_all(&rows)?;
        Ok(replaced)
    }

    /// Commit a staged install after LocalRuntime and control-plane binding
    /// have both accepted the new ability.
    pub fn commit_installed(&self, install_id: &str) -> anyhow::Result<()> {
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        let Some(staged_idx) = rows.iter().position(|r| {
            r.install_id == install_id && r.state == DeviceAbilityRecordState::Installing
        }) else {
            anyhow::bail!("commit staged device ability install {install_id}: row missing");
        };
        let mut committed = rows[staged_idx].clone();
        committed.mark_installed();
        rows = rows
            .into_iter()
            .enumerate()
            .filter_map(|(idx, existing)| {
                if idx == staged_idx {
                    return None;
                }
                let displaced = existing.install_id == committed.install_id
                    || existing.ability_ura == committed.ability_ura
                    || existing.public_name == committed.public_name;
                (!displaced).then_some(existing)
            })
            .collect();
        rows.push(committed);
        self.write_all(&rows)
    }

    /// Roll back a failed install after `stage_install_record` succeeded but
    /// runtime binding failed. This restores every row displaced by the
    /// install in one write, avoiding a window where old rows vanish.
    pub fn rollback_install(
        &self,
        failed_install_id: &str,
        replaced: Vec<DeviceAbilityRecord>,
    ) -> anyhow::Result<()> {
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        rows.retain(|r| r.install_id != failed_install_id);
        Self::merge_records(&mut rows, replaced);
        self.write_all(&rows)
    }

    /// Recover uncommitted install intents after daemon restart.
    ///
    /// An `Installing` row means the durable active switch never happened. Any
    /// runtime/control-plane mutation from that attempt died with the process,
    /// so recovery removes the staged row and leaves any previous `Installed`
    /// row in place for replay.
    pub fn recover_installing(&self) -> anyhow::Result<usize> {
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        let before = rows.len();
        rows.retain(|record| record.state != DeviceAbilityRecordState::Installing);
        let recovered = before - rows.len();
        if recovered > 0 {
            self.write_all(&rows)?;
        }
        Ok(recovered)
    }

    /// Hide installed rows owned by a previous device authority before boot
    /// replay binds live runtime state.
    ///
    /// This is the rejoin recovery boundary: a new `credentials.json` means the
    /// daemon no longer hosts the old device URA, so replay must not try to
    /// register those abilities. The rows stay on disk as `Quarantined` for
    /// audit instead of being silently deleted.
    pub fn quarantine_unhosted_device_authority(
        &self,
        hosted_device_authority_root: &str,
    ) -> anyhow::Result<Vec<DeviceAbilityRecord>> {
        let hosted_device_authority_root = hosted_device_authority_root.trim();
        if hosted_device_authority_root.is_empty() {
            anyhow::bail!("hosted device authority root must not be empty");
        }
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        let mut quarantined = Vec::new();
        let mut changed = false;
        for row in &mut rows {
            if row.state != DeviceAbilityRecordState::Installed {
                continue;
            }
            let Ok(selector) = crate::core::ura::AbilitySelector::parse(row.ability_ura()) else {
                continue;
            };
            if selector.owner_kind() != "device" {
                continue;
            }
            if selector.owner_ura() == hosted_device_authority_root {
                continue;
            }
            quarantined.push(row.clone());
            row.mark_quarantined();
            changed = true;
        }
        if changed {
            self.write_all(&rows)?;
        }
        Ok(quarantined)
    }

    /// Restore rows previously removed by a higher-level transaction.
    /// Existing rows with the same install id are left untouched so retrying
    /// a rollback is idempotent.
    pub fn restore_records(&self, records: Vec<DeviceAbilityRecord>) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        Self::merge_records(&mut rows, records);
        self.write_all(&rows)
    }

    /// Stage an uninstall by canonical Ability URA, optionally narrowed
    /// to a specific install id. This is a durable state transition, not
    /// a physical delete: `Installed` rows become `Removing`, and existing
    /// `Removing` rows are returned again so a failed final commit can be
    /// retried cleanly.
    pub fn stage_remove_by_ability(
        &self,
        ability_ura: &str,
        install_id: Option<&str>,
    ) -> anyhow::Result<DeviceAbilityRemovalPlan> {
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        let mut records = Vec::new();
        let mut resumed = false;
        let mut changed = false;
        for record in &mut rows {
            let matches_identity = record.ability_ura == ability_ura
                && install_id.is_none_or(|id| record.install_id == id);
            if !matches_identity {
                continue;
            }
            match record.state {
                DeviceAbilityRecordState::Installed => {
                    records.push(record.clone());
                    record.mark_removing();
                    changed = true;
                }
                DeviceAbilityRecordState::Installing => {}
                DeviceAbilityRecordState::Quarantined => {}
                DeviceAbilityRecordState::Removing => {
                    records.push(record.clone());
                    resumed = true;
                }
            }
        }
        if changed {
            self.write_all(&rows)?;
        }
        Ok(DeviceAbilityRemovalPlan::new(records, resumed))
    }

    /// Commit staged removals by physically deleting their tombstone rows.
    pub fn commit_removed(&self, install_ids: &[String]) -> anyhow::Result<()> {
        if install_ids.is_empty() {
            return Ok(());
        }
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        rows.retain(|record| {
            !(record.state == DeviceAbilityRecordState::Removing
                && install_ids.iter().any(|id| id == record.install_id()))
        });
        self.write_all(&rows)
    }

    /// Remove the record with `install_id`, if present. Returns whether
    /// a row was removed. Used by the deploy transaction's rollback and
    /// by `ability.uninstall`.
    pub fn remove(&self, install_id: &str) -> anyhow::Result<bool> {
        let _guard = self.lock_for_write()?;
        let mut rows = self.load_all_unlocked()?;
        let before = rows.len();
        rows.retain(|r| r.install_id != install_id);
        let removed = rows.len() != before;
        if removed {
            self.write_all(&rows)?;
        }
        Ok(removed)
    }

    /// Atomic + durable write: serialize to a sibling tmp file, fsync
    /// it, rename over the target, then fsync the parent dir so the
    /// rename itself is durable. A crash at any point leaves either the
    /// old complete file or the new complete file, never a torn one.
    fn write_all(&self, rows: &[DeviceAbilityRecord]) -> anyhow::Result<()> {
        let file = StoreFile {
            schema_version: STORE_SCHEMA_VERSION.to_string(),
            installed: rows.to_vec(),
        };
        let body = serde_json::to_string_pretty(&file)?;

        let dir = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("device ability store path has no parent"))?;
        fs::create_dir_all(dir)?;

        let tmp = self.unique_tmp_path();
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;

        sync_directory(dir)?;
        Ok(())
    }

    fn lock_for_read(&self) -> anyhow::Result<DeviceAbilityStoreReadLock> {
        let process = STORE_LOCK
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let file = SharedFileLock::acquire_for_data_path(&self.path)?;
        Ok(DeviceAbilityStoreReadLock {
            _process: process,
            _file: file,
        })
    }

    fn lock_for_write(&self) -> anyhow::Result<DeviceAbilityStoreWriteLock> {
        let process = STORE_LOCK
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let file = ExclusiveFileLock::acquire_for_data_path(&self.path)?;
        Ok(DeviceAbilityStoreWriteLock {
            _process: process,
            _file: file,
        })
    }

    fn unique_tmp_path(&self) -> PathBuf {
        let seq = STORE_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.path
            .with_extension(format!("json.tmp.{}.{}", std::process::id(), seq))
    }

    fn merge_records(rows: &mut Vec<DeviceAbilityRecord>, records: Vec<DeviceAbilityRecord>) {
        for mut row in records {
            row.mark_installed();
            rows.retain(|existing| {
                existing.install_id != row.install_id
                    && existing.ability_ura != row.ability_ura
                    && existing.public_name != row.public_name
            });
            rows.push(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> (DeviceAbilityStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = DeviceAbilityStore::open_at(dir.path().join("device-abilities.json"));
        (store, dir)
    }

    fn record(name: &str) -> DeviceAbilityRecord {
        record_with(name, "sha256:abc", 0)
    }

    fn record_with(
        name: &str,
        manifest_marker: &str,
        installed_at_unix_ms: u64,
    ) -> DeviceAbilityRecord {
        record_with_owner(name, "x", manifest_marker, installed_at_unix_ms)
    }

    fn record_with_owner(
        name: &str,
        device_id: &str,
        manifest_marker: &str,
        installed_at_unix_ms: u64,
    ) -> DeviceAbilityRecord {
        let owner_ura = crate::core::ura::device_ura("localhost", device_id);
        let manifest_bytes = format!(r#"{{"name":"{name}","marker":"{manifest_marker}"}}"#);
        DeviceAbilityRecord::new_with_manifest_bytes(
            name.to_string(),
            "er".to_string(),
            crate::core::ura::owner_ability_ura(&owner_ura, name).expect("ability ura"),
            format!("/bundles/{name}/ability.json"),
            manifest_bytes.as_bytes(),
            installed_at_unix_ms,
        )
    }

    #[test]
    fn missing_file_loads_empty() {
        let (store, _d) = tmp_store();
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn load_rejects_missing_schema_version() {
        let (store, _d) = tmp_store();
        std::fs::write(&store.path, r#"{"installed":[]}"#).unwrap();

        let err = store
            .load()
            .expect_err("device ability store schema version is mandatory");
        assert!(format!("{err}").contains("schema_version"), "{err}");
    }

    #[test]
    fn load_rejects_record_missing_manifest_snapshot() {
        let (store, _d) = tmp_store();
        let file = StoreFile {
            schema_version: STORE_SCHEMA_VERSION.to_string(),
            installed: vec![record("generate")],
        };
        let mut value = serde_json::to_value(file).unwrap();
        value["installed"][0]
            .as_object_mut()
            .unwrap()
            .remove("manifest_snapshot_b64");
        std::fs::write(&store.path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let err = store
            .load()
            .expect_err("manifest snapshot is mandatory for replay");
        assert!(format!("{err}").contains("manifest_snapshot_b64"), "{err}");
    }

    #[test]
    fn upsert_then_load_roundtrips() {
        let (store, _d) = tmp_store();
        store.upsert(record("generate")).unwrap();
        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].public_name(), "generate");
    }

    #[test]
    fn upsert_same_id_replaces_not_duplicates() {
        // invariant 6: re-deploy same manifest -> same id -> upsert.
        let (store, _d) = tmp_store();
        store
            .upsert(record_with("generate", "sha256:abc", 0))
            .unwrap();
        let updated = record_with("generate", "sha256:abc", 1);
        let previous = store.upsert(updated).unwrap().expect("previous row");
        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 1, "same install_id must upsert, not duplicate");
        assert_eq!(previous.installed_at_unix_ms(), 0);
        assert_eq!(rows[0].installed_at_unix_ms(), 1);
    }

    #[test]
    fn staged_install_replaces_same_live_ability_and_commits_visible() {
        let (store, _d) = tmp_store();
        let original = record("generate");
        let original_id = original.install_id().to_string();
        store.stage_install_record(original.clone()).unwrap();
        assert!(
            store.load().unwrap().is_empty(),
            "a first install is hidden until commit because no active row exists yet"
        );
        store.commit_installed(&original_id).unwrap();
        let changed_manifest = record_with("generate", "sha256:def", 0);
        let changed_id = changed_manifest.install_id().to_string();
        let replaced = store.stage_install_record(changed_manifest).unwrap();

        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].install_id(), original_id);
        let live_during_stage = store.load().unwrap();
        assert_eq!(
            live_during_stage.len(),
            1,
            "replacement staging must keep the previous active row replayable"
        );
        assert_eq!(live_during_stage[0].install_id(), original_id);
        store.commit_installed(&changed_id).unwrap();
        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 1, "one live ability must have one durable row");
        assert_eq!(rows[0].install_id(), changed_id);
    }

    #[test]
    fn recover_installing_removes_uncommitted_intent_and_preserves_old_active() {
        let (store, _d) = tmp_store();
        let original = record("generate");
        let original_id = original.install_id().to_string();
        store.stage_install_record(original).unwrap();
        store.commit_installed(&original_id).unwrap();

        let changed_manifest = record_with("generate", "sha256:def", 0);
        let changed_id = changed_manifest.install_id().to_string();
        store.stage_install_record(changed_manifest).unwrap();

        let recovered = store.recover_installing().unwrap();
        assert_eq!(recovered, 1);
        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].install_id(),
            original_id,
            "crash recovery must replay the old active row, not the uncommitted replacement"
        );
        assert!(
            !store
                .load_all_unlocked()
                .unwrap()
                .iter()
                .any(|row| row.install_id() == changed_id),
            "uncommitted replacement intent must be physically removed"
        );
    }

    #[test]
    fn quarantine_unhosted_device_authority_hides_old_device_rows_from_replay() {
        let (store, _d) = tmp_store();
        let current = record_with_owner("current", "current-device", "sha256:a", 0);
        let previous = record_with_owner("previous", "old-device", "sha256:b", 0);
        let previous_id = previous.install_id().to_string();
        store.upsert(current.clone()).unwrap();
        store.upsert(previous).unwrap();

        let quarantined = store
            .quarantine_unhosted_device_authority(&crate::core::ura::device_ura(
                "localhost",
                "current-device",
            ))
            .unwrap();

        assert_eq!(quarantined.len(), 1);
        assert_eq!(quarantined[0].install_id(), previous_id);
        let replayable = store.load().unwrap();
        assert_eq!(replayable.len(), 1);
        assert_eq!(replayable[0].install_id(), current.install_id());
        let all_rows = store.load_all_unlocked().unwrap();
        assert!(all_rows.iter().any(|row| {
            row.install_id() == previous_id && row.state() == DeviceAbilityRecordState::Quarantined
        }));
    }

    #[test]
    fn distinct_ids_accumulate() {
        let (store, _d) = tmp_store();
        store.upsert(record("a")).unwrap();
        store.upsert(record("b")).unwrap();
        assert_eq!(store.load().unwrap().len(), 2);
    }

    #[test]
    fn remove_deletes_one_row() {
        let (store, _d) = tmp_store();
        let left = record("a");
        let left_id = left.install_id().to_string();
        let right = record("b");
        let right_id = right.install_id().to_string();
        store.upsert(left).unwrap();
        store.upsert(right).unwrap();
        assert!(store.remove(&left_id).unwrap());
        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].install_id(), right_id);
        assert!(!store.remove("missing").unwrap());
    }

    #[test]
    fn stage_remove_by_ability_returns_removed_rows() {
        let (store, _d) = tmp_store();
        let target = record("a");
        let target_id = target.install_id().to_string();
        let ability_ura = target.ability_ura().to_string();
        store.upsert(target).unwrap();
        let other = record("b");
        let other_id = other.install_id().to_string();
        store.upsert(other).unwrap();

        let plan = store.stage_remove_by_ability(&ability_ura, None).unwrap();
        assert!(!plan.resumed());
        assert_eq!(plan.records().len(), 1);
        assert_eq!(plan.records()[0].install_id(), target_id);
        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].install_id(), other_id);
        let all_rows = store.load_all_unlocked().unwrap();
        assert!(
            all_rows.iter().any(|row| row.install_id() == target_id
                && row.state() == DeviceAbilityRecordState::Removing),
            "stage_remove_by_ability must stage a durable tombstone"
        );
    }

    #[test]
    fn staged_remove_resumes_existing_tombstone() {
        let (store, _d) = tmp_store();
        let target = record("a");
        let target_id = target.install_id().to_string();
        let ability_ura = target.ability_ura().to_string();
        store.upsert(target).unwrap();

        let first = store.stage_remove_by_ability(&ability_ura, None).unwrap();
        assert!(!first.resumed());
        let retry = store.stage_remove_by_ability(&ability_ura, None).unwrap();

        assert!(retry.resumed());
        assert_eq!(retry.records().len(), 1);
        assert_eq!(retry.records()[0].install_id(), target_id);
        assert!(
            store.load().unwrap().is_empty(),
            "Removing tombstones must stay hidden from boot replay"
        );
    }

    #[test]
    fn commit_removed_deletes_tombstone_after_runtime_cleanup() {
        let (store, _d) = tmp_store();
        let target = record("a");
        let target_id = target.install_id().to_string();
        let ability_ura = target.ability_ura().to_string();
        store.upsert(target).unwrap();

        let plan = store.stage_remove_by_ability(&ability_ura, None).unwrap();
        assert!(store.load().unwrap().is_empty());
        store.commit_removed(&plan.install_ids()).unwrap();

        assert!(
            !store
                .load_all_unlocked()
                .unwrap()
                .iter()
                .any(|row| row.install_id() == target_id),
            "final commit must physically delete the staged tombstone"
        );
    }

    #[test]
    fn restore_records_is_idempotent() {
        let (store, _d) = tmp_store();
        let target = record("a");
        let target_id = target.install_id().to_string();
        store.restore_records(vec![target.clone()]).unwrap();
        store.restore_records(vec![target]).unwrap();

        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].install_id(), target_id);
    }

    #[test]
    fn concurrent_upserts_do_not_lose_rows() {
        let (store, _d) = tmp_store();
        let store = std::sync::Arc::new(store);
        let left = {
            let store = std::sync::Arc::clone(&store);
            std::thread::spawn(move || {
                for i in 0..40 {
                    store.upsert(record(&format!("left.{i}"))).unwrap();
                }
            })
        };
        let right = {
            let store = std::sync::Arc::clone(&store);
            std::thread::spawn(move || {
                for i in 0..40 {
                    store.upsert(record(&format!("right.{i}"))).unwrap();
                }
            })
        };
        left.join().unwrap();
        right.join().unwrap();

        let rows = store.load().unwrap();
        assert_eq!(rows.len(), 80);
    }

    #[test]
    fn concurrent_upserts_across_store_instances_do_not_lose_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device-abilities.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let left_path = path.clone();
        let right_path = path.clone();

        let left = {
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                let store = DeviceAbilityStore::open_at(left_path);
                barrier.wait();
                for i in 0..40 {
                    store.upsert(record(&format!("left.{i}"))).unwrap();
                }
            })
        };
        let right = {
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                let store = DeviceAbilityStore::open_at(right_path);
                barrier.wait();
                for i in 0..40 {
                    store.upsert(record(&format!("right.{i}"))).unwrap();
                }
            })
        };

        left.join().unwrap();
        right.join().unwrap();

        let rows = DeviceAbilityStore::open_at(path).load().unwrap();
        assert_eq!(rows.len(), 80);
    }

    #[test]
    fn install_id_is_stable_and_input_sensitive() {
        let a = DeviceAbilityRecord::derive_install_id("ura", "h");
        let b = DeviceAbilityRecord::derive_install_id("ura", "h");
        assert_eq!(a, b, "same inputs -> same id");
        let c = DeviceAbilityRecord::derive_install_id("ura", "h2");
        assert_ne!(a, c, "different manifest_hash -> different id");
    }
}
