//! File: `src/daemon/persistence/agent_lifecycle.rs`
//! Description: Durable coordination for hosted-Agent lifecycle mutations.
//!
//! Protocol responsibility: persist local purge transaction progress, the
//! identity-fencing publication outbox, finite retry state, and delivery
//! claims. This module performs no network I/O.
//!
//! Implementation approach: explicit journal and publication state machines
//! are validated on every locked read-modify-write cycle. Contradictory state
//! fails closed.
//!
//! Usage contract: callers hold [`AgentLifecycleMutationGuard`] for local
//! registry/journal mutation, but claim outbox work before releasing all store
//! locks and performing bounded provider calls.
//!
//! Architectural position: daemon persistence/domain layer below Agent
//! lifecycle handlers and above atomic JSON/file-lock infrastructure.
//!
//! Every `agent.start`, `agent.stop`, and `agent.purge` read-modify-write cycle
//! holds [`AgentLifecycleMutationGuard`]. The guard combines a process-local
//! mutex with the repository's OS advisory file lock so threads and separate
//! daemon/CLI processes share one serialization boundary.
//!
//! Destructive purge additionally persists [`AgentPurgeJournal`] before the
//! root is renamed. The application layer advances the journal through a
//! reversible local phase, persists `Committed`, finishes identity-bound local
//! deletion, then hands publication to a separate durable outbox. A committed
//! journal is never compensated, and an unavailable publisher never retains a
//! quarantine.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};

use super::agent_registry::{AgentEntry, AgentRegistry};
use super::config::{self, WritePermissions};
use super::file_lock::ExclusiveFileLock;
use super::local_agents::LocalAgentsFile;

const JOURNAL_FILE_NAME: &str = "agent-lifecycle-purge.json";
const JOURNAL_SCHEMA_VERSION: u32 = 4;
const PUBLICATION_OUTBOX_FILE_NAME: &str = "agent-purge-publication-outbox.json";
const PUBLICATION_OUTBOX_SCHEMA_VERSION: u32 = 4;
pub(crate) const PUBLICATION_MAX_ATTEMPTS_PER_STAGE: u32 = 5;
pub(crate) const PUBLICATION_DRAIN_BATCH_SIZE: usize = 16;

static PROCESS_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
static OUTBOX_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
static DRAIN_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) struct AgentLifecycleMutationGuard {
    _thread_guard: MutexGuard<'static, ()>,
    _process_guard: ExclusiveFileLock,
}

/// Dedicated bounded-drain serialization. It is intentionally distinct from
/// the lifecycle mutation guard: local Agent mutations continue while one
/// bounded network batch is in flight, but two processes cannot publish the
/// same claimed entry concurrently.
pub(crate) struct AgentPurgePublicationDrainGuard {
    _thread_guard: MutexGuard<'static, ()>,
    _process_guard: ExclusiveFileLock,
}

impl AgentPurgePublicationDrainGuard {
    pub(crate) fn try_acquire() -> anyhow::Result<Option<Self>> {
        let thread_guard = match DRAIN_MUTEX.get_or_init(|| Mutex::new(())).try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let Some(process_guard) = ExclusiveFileLock::try_acquire_for_data_path(
            &config::state_dir().join("agent-purge-publication-drain"),
        )?
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            _thread_guard: thread_guard,
            _process_guard: process_guard,
        }))
    }
}

impl AgentLifecycleMutationGuard {
    pub(crate) fn acquire() -> anyhow::Result<Self> {
        let thread_guard = PROCESS_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let process_guard = ExclusiveFileLock::acquire_for_data_path(&journal_path())?;
        Ok(Self {
            _thread_guard: thread_guard,
            _process_guard: process_guard,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentPurgeStage {
    Prepared,
    Quarantined,
    RuntimeSynchronized,
    RegistryPersisted,
    IdentityPersisted,
    AuthorityCommitted,
    Committed,
    Finalized,
    TombstonePrepared,
    OutboxEnqueued,
}

impl AgentPurgeStage {
    pub(crate) fn is_committed(self) -> bool {
        matches!(
            self,
            Self::Committed | Self::Finalized | Self::TombstonePrepared | Self::OutboxEnqueued
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentRootIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
}

impl AgentRootIdentity {
    pub(crate) fn from_path(path: &std::path::Path) -> anyhow::Result<Self> {
        #[cfg(unix)]
        {
            Self::from_metadata(&fs::symlink_metadata(path)?)
        }
        #[cfg(windows)]
        {
            windows_file_identity(path)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            anyhow::bail!("filesystem identity is unsupported on this target")
        }
    }

    #[cfg(unix)]
    pub(crate) fn from_metadata(metadata: &fs::Metadata) -> anyhow::Result<Self> {
        use std::os::unix::fs::MetadataExt as _;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    #[cfg(unix)]
    pub(crate) fn matches_metadata(&self, metadata: &fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt as _;
        self.device == metadata.dev() && self.inode == metadata.ino()
    }

    pub(crate) fn matches_path(&self, path: &std::path::Path) -> anyhow::Result<bool> {
        Ok(self == &Self::from_path(path)?)
    }
}

#[cfg(windows)]
fn windows_file_identity(path: &std::path::Path) -> anyhow::Result<AgentRootIdentity> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(anyhow::anyhow!(
            "open Windows filesystem identity handle {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    let result = unsafe { GetFileInformationByHandle(handle, information.as_mut_ptr()) };
    let close_result = unsafe { CloseHandle(handle) };
    if result == 0 {
        return Err(anyhow::anyhow!(
            "query Windows filesystem identity {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    if close_result == 0 {
        return Err(anyhow::anyhow!(
            "close Windows filesystem identity handle {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let information = unsafe { information.assume_init() };
    Ok(AgentRootIdentity {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPurgePublication {
    pub host_device_ura: String,
    pub generation: u64,
    pub projection_revision: u64,
    pub projection_digest: String,
    pub tombstone_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AgentPurgePublicationPlan {
    Undetermined,
    NotRequired {
        reason: AgentPurgeNoPublicationReason,
    },
    Required {
        publication: AgentPurgePublication,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentPurgeNoPublicationReason {
    NoActiveOwnerProjection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgentPurgeJournal {
    pub schema_version: u32,
    pub transaction_id: String,
    pub stage: AgentPurgeStage,
    pub name: String,
    pub agent_ura: String,
    pub root_path: PathBuf,
    pub quarantine_path: PathBuf,
    pub root_identity: Option<AgentRootIdentity>,
    pub publication_plan: AgentPurgePublicationPlan,
    pub removed_entry: AgentEntry,
    pub original_registry: AgentRegistry,
    pub original_local_agents: LocalAgentsFile,
}

impl AgentPurgeJournal {
    #[expect(
        clippy::too_many_arguments,
        reason = "a purge journal atomically snapshots every recovery-critical pre-mutation fact"
    )]
    pub(crate) fn new(
        transaction_id: String,
        name: String,
        agent_ura: String,
        root_path: PathBuf,
        quarantine_path: PathBuf,
        removed_entry: AgentEntry,
        original_registry: AgentRegistry,
        original_local_agents: LocalAgentsFile,
    ) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            transaction_id,
            stage: AgentPurgeStage::Prepared,
            name,
            agent_ura,
            root_path,
            quarantine_path,
            root_identity: None,
            publication_plan: AgentPurgePublicationPlan::Undetermined,
            removed_entry,
            original_registry,
            original_local_agents,
        }
    }

    pub(crate) fn advance(&mut self, stage: AgentPurgeStage) -> anyhow::Result<()> {
        let valid = matches!(
            (self.stage, stage),
            (AgentPurgeStage::Prepared, AgentPurgeStage::Quarantined)
                | (
                    AgentPurgeStage::Quarantined,
                    AgentPurgeStage::RuntimeSynchronized
                )
                | (
                    AgentPurgeStage::RuntimeSynchronized,
                    AgentPurgeStage::RegistryPersisted
                )
                | (
                    AgentPurgeStage::RegistryPersisted,
                    AgentPurgeStage::IdentityPersisted
                )
                | (
                    AgentPurgeStage::IdentityPersisted,
                    AgentPurgeStage::AuthorityCommitted
                )
                | (
                    AgentPurgeStage::AuthorityCommitted,
                    AgentPurgeStage::Committed
                )
                | (AgentPurgeStage::Committed, AgentPurgeStage::Finalized)
                | (
                    AgentPurgeStage::Finalized,
                    AgentPurgeStage::TombstonePrepared
                )
                | (
                    AgentPurgeStage::TombstonePrepared,
                    AgentPurgeStage::OutboxEnqueued
                )
        );
        if !valid {
            anyhow::bail!(
                "invalid Agent purge journal transition {:?} -> {:?}",
                self.stage,
                stage
            );
        }
        self.stage = stage;
        save_purge_journal(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentPurgePublicationStage {
    TombstonePending,
    RevokePending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum AgentPurgePublicationRetryState {
    Ready,
    BackingOff {
        eligible_drain_epoch: u64,
    },
    Claimed {
        claim_id: String,
        drain_epoch: u64,
        delivery_fence: u64,
    },
    ReconciliationRequired {
        dead_lettered_at_unix_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPurgePublicationFailureEvidence {
    pub stage: AgentPurgePublicationStage,
    pub attempt: u32,
    pub observed_at_unix_ms: u64,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPurgePublicationRetry {
    pub state: AgentPurgePublicationRetryState,
    pub attempts: u32,
    pub last_failure: Option<AgentPurgePublicationFailureEvidence>,
    pub last_reconciliation: Option<AgentPurgePublicationFailureEvidence>,
}

impl Default for AgentPurgePublicationRetry {
    fn default() -> Self {
        Self {
            state: AgentPurgePublicationRetryState::Ready,
            attempts: 0,
            last_failure: None,
            last_reconciliation: None,
        }
    }
}

impl AgentPurgePublicationRetry {
    fn checked_delay_epochs(attempt: u32) -> anyhow::Result<u64> {
        if attempt == 0 || attempt > PUBLICATION_MAX_ATTEMPTS_PER_STAGE {
            anyhow::bail!("publication retry attempt {attempt} is outside the finite budget");
        }
        let exponent = attempt - 1;
        1_u64
            .checked_shl(exponent)
            .ok_or_else(|| anyhow::anyhow!("publication retry delay overflow at attempt {attempt}"))
    }

    fn is_claimable(&self, drain_epoch: u64, force_backoff: bool) -> bool {
        match self.state {
            AgentPurgePublicationRetryState::Ready => true,
            AgentPurgePublicationRetryState::BackingOff {
                eligible_drain_epoch,
            } => force_backoff || drain_epoch >= eligible_drain_epoch,
            AgentPurgePublicationRetryState::Claimed { .. } => false,
            AgentPurgePublicationRetryState::ReconciliationRequired { .. } => false,
        }
    }

    fn claim(
        &mut self,
        drain_epoch: u64,
        force_backoff: bool,
        claim_id: String,
        delivery_fence: u64,
    ) -> anyhow::Result<bool> {
        if !self.is_claimable(drain_epoch, force_backoff) {
            return Ok(false);
        }
        if claim_id.trim().is_empty() {
            anyhow::bail!("publication claim ID must not be empty");
        }
        if drain_epoch == 0 || delivery_fence == 0 {
            anyhow::bail!("publication claim requires monotonic epoch and delivery fence");
        }
        self.state = AgentPurgePublicationRetryState::Claimed {
            claim_id,
            drain_epoch,
            delivery_fence,
        };
        Ok(true)
    }

    fn claimed_by(&self, expected_claim_id: &str) -> bool {
        matches!(
            &self.state,
            AgentPurgePublicationRetryState::Claimed { claim_id, .. }
                if claim_id == expected_claim_id
        )
    }

    fn record_failure(
        &mut self,
        stage: AgentPurgePublicationStage,
        claim_id: &str,
        now_unix_ms: u64,
        error: String,
    ) -> anyhow::Result<()> {
        let drain_epoch = match &self.state {
            AgentPurgePublicationRetryState::Claimed {
                claim_id: current_claim_id,
                drain_epoch,
                ..
            } if current_claim_id == claim_id => *drain_epoch,
            _ => anyhow::bail!("publication failure does not own the durable claim"),
        };
        if error.trim().is_empty() {
            anyhow::bail!("publication failure evidence must not be empty");
        }
        self.attempts = self
            .attempts
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("publication retry attempt overflow"))?;
        if self.attempts > PUBLICATION_MAX_ATTEMPTS_PER_STAGE {
            anyhow::bail!("publication retry budget was already exhausted");
        }
        let evidence = AgentPurgePublicationFailureEvidence {
            stage,
            attempt: self.attempts,
            observed_at_unix_ms: now_unix_ms,
            error,
        };
        self.state = if self.attempts == PUBLICATION_MAX_ATTEMPTS_PER_STAGE {
            AgentPurgePublicationRetryState::ReconciliationRequired {
                dead_lettered_at_unix_ms: now_unix_ms,
            }
        } else {
            let delay_epochs = Self::checked_delay_epochs(self.attempts)?;
            // `delay_epochs` counts complete future drains to defer. The
            // following epoch is therefore the first epoch eligible to claim.
            let eligible_drain_epoch = drain_epoch
                .checked_add(delay_epochs)
                .and_then(|last_deferred_epoch| last_deferred_epoch.checked_add(1))
                .ok_or_else(|| anyhow::anyhow!("publication retry epoch overflow"))?;
            AgentPurgePublicationRetryState::BackingOff {
                eligible_drain_epoch,
            }
        };
        self.last_failure = Some(evidence);
        Ok(())
    }

    fn reset_after_stage_progress_under_claim(&mut self) -> anyhow::Result<()> {
        let AgentPurgePublicationRetryState::Claimed {
            claim_id,
            drain_epoch,
            delivery_fence,
        } = &self.state
        else {
            anyhow::bail!("publication stage progress requires a durable claim");
        };
        self.state = AgentPurgePublicationRetryState::Claimed {
            claim_id: claim_id.clone(),
            drain_epoch: *drain_epoch,
            delivery_fence: *delivery_fence,
        };
        self.attempts = 0;
        self.last_failure = None;
        Ok(())
    }

    fn manual_retry(&mut self) -> anyhow::Result<()> {
        if !matches!(
            self.state,
            AgentPurgePublicationRetryState::ReconciliationRequired { .. }
        ) {
            anyhow::bail!("manual publication retry requires reconciliation_required state");
        }
        self.last_reconciliation = self.last_failure.clone();
        self.state = AgentPurgePublicationRetryState::Ready;
        self.attempts = 0;
        self.last_failure = None;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPurgePublicationEntry {
    pub transaction_id: String,
    pub name: String,
    pub agent_ura: String,
    pub publication: AgentPurgePublication,
    pub stage: AgentPurgePublicationStage,
    pub retry: AgentPurgePublicationRetry,
    pub next_delivery_fence: u64,
}

impl AgentPurgePublicationEntry {
    pub(crate) fn claim(
        &mut self,
        drain_epoch: u64,
        force_backoff: bool,
        claim_id: String,
    ) -> anyhow::Result<bool> {
        let delivery_fence = self.next_delivery_fence;
        let claimed = self
            .retry
            .claim(drain_epoch, force_backoff, claim_id, delivery_fence)?;
        if claimed {
            self.next_delivery_fence = self
                .next_delivery_fence
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("publication delivery fence exhausted"))?;
        }
        Ok(claimed)
    }

    pub(crate) fn delivery_fence(&self) -> Option<u64> {
        match self.retry.state {
            AgentPurgePublicationRetryState::Claimed { delivery_fence, .. } => Some(delivery_fence),
            _ => None,
        }
    }

    pub(crate) fn claim_id(&self) -> Option<&str> {
        match &self.retry.state {
            AgentPurgePublicationRetryState::Claimed { claim_id, .. } => Some(claim_id),
            _ => None,
        }
    }

    pub(crate) fn record_claim_failure(
        &mut self,
        claim_id: &str,
        now_unix_ms: u64,
        error: String,
    ) -> anyhow::Result<()> {
        self.retry
            .record_failure(self.stage, claim_id, now_unix_ms, error)
    }

    pub(crate) fn advance_claim_to_revoke(&mut self, claim_id: &str) -> anyhow::Result<()> {
        if self.stage != AgentPurgePublicationStage::TombstonePending {
            anyhow::bail!("only a tombstone claim can advance to revoke");
        }
        if !self.retry.claimed_by(claim_id) {
            anyhow::bail!("publication stage progress does not own the durable claim");
        }
        self.stage = AgentPurgePublicationStage::RevokePending;
        self.retry.reset_after_stage_progress_under_claim()
    }

    pub(crate) fn manual_retry(&mut self) -> anyhow::Result<()> {
        self.retry.manual_retry()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentPurgePublicationReconciliation {
    Retry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPurgeReconciliationCommand {
    pub command_id: String,
    pub transaction_id: String,
    pub actor_ura: String,
    pub action: AgentPurgePublicationReconciliation,
}

impl AgentPurgeReconciliationCommand {
    fn digest(&self) -> anyhow::Result<String> {
        use sha2::Digest as _;
        if !valid_hex_id(&self.command_id)
            || !valid_hex_id(&self.transaction_id)
            || self.actor_ura.trim().is_empty()
        {
            anyhow::bail!("purge reconciliation command has an invalid identity");
        }
        let bytes = serde_json::to_vec(self)?;
        Ok(format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(bytes))
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorizedPurgeReconciliation {
    actor_ura: String,
    authority_reference: String,
}

impl AuthorizedPurgeReconciliation {
    /// Constructed by the application admission boundary after Manage-level
    /// authorization. The persistence FSM never accepts an unauthenticated
    /// actor string in place of this proof.
    pub(crate) fn from_admission(
        actor_ura: impl Into<String>,
        authority_reference: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let actor_ura = actor_ura.into();
        let authority_reference = authority_reference.into();
        if actor_ura.trim().is_empty() || authority_reference.trim().is_empty() {
            anyhow::bail!("purge reconciliation authorization is incomplete");
        }
        Ok(Self {
            actor_ura,
            authority_reference,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPurgeReconciliationAudit {
    pub command: AgentPurgeReconciliationCommand,
    pub command_digest: String,
    pub authority_reference: String,
    pub failure_evidence: AgentPurgePublicationFailureEvidence,
    pub retained_delivery_fence: u64,
    pub applied_at_unix_ms: u64,
    pub outcome: AgentPurgePublicationEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentPurgeReconciliationOutcome {
    pub entry: AgentPurgePublicationEntry,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPurgePublicationOutbox {
    pub schema_version: u32,
    pub next_drain_epoch: u64,
    pub entries: Vec<AgentPurgePublicationEntry>,
    pub reconciliation_audit: Vec<AgentPurgeReconciliationAudit>,
}

impl Default for AgentPurgePublicationOutbox {
    fn default() -> Self {
        Self {
            schema_version: PUBLICATION_OUTBOX_SCHEMA_VERSION,
            next_drain_epoch: 1,
            entries: Vec::new(),
            reconciliation_audit: Vec::new(),
        }
    }
}

impl AgentPurgePublicationOutbox {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != PUBLICATION_OUTBOX_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported Agent purge publication outbox schema {}; expected {}",
                self.schema_version,
                PUBLICATION_OUTBOX_SCHEMA_VERSION
            );
        }
        if self.next_drain_epoch == 0 || self.next_drain_epoch == u64::MAX {
            anyhow::bail!("Agent purge publication outbox has exhausted its drain epoch");
        }
        let mut transaction_ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        let mut agent_uras = std::collections::BTreeSet::new();
        for entry in &self.entries {
            if entry.transaction_id.trim().is_empty()
                || entry.name.trim().is_empty()
                || entry.agent_ura.trim().is_empty()
            {
                anyhow::bail!("Agent purge publication outbox contains an empty identity");
            }
            if entry.transaction_id.len() != 32
                || !entry
                    .transaction_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                anyhow::bail!(
                    "Agent purge publication outbox contains invalid transaction `{}`",
                    entry.transaction_id
                );
            }
            if !transaction_ids.insert(entry.transaction_id.as_str()) {
                anyhow::bail!(
                    "Agent purge publication outbox contains duplicate transaction `{}`",
                    entry.transaction_id
                );
            }
            if !names.insert(entry.name.as_str()) || !agent_uras.insert(entry.agent_ura.as_str()) {
                anyhow::bail!(
                    "Agent purge publication outbox contains duplicate pending identity `{}` / `{}`",
                    entry.name,
                    entry.agent_ura
                );
            }
            validate_publication_entry(entry)?;
        }
        let mut reconciliation_ids = std::collections::BTreeSet::new();
        for audit in &self.reconciliation_audit {
            if audit.command_digest != audit.command.digest()?
                || audit.command.actor_ura.trim().is_empty()
                || audit.authority_reference.trim().is_empty()
                || audit.retained_delivery_fence == 0
                || audit.applied_at_unix_ms == u64::MAX
                || audit.outcome.transaction_id != audit.command.transaction_id
                || !reconciliation_ids.insert(audit.command.command_id.as_str())
            {
                anyhow::bail!("Agent purge publication outbox has corrupt reconciliation audit");
            }
            validate_publication_entry(&audit.outcome)?;
        }
        Ok(())
    }
}

impl AgentPurgePublicationOutbox {
    pub(crate) fn begin_drain_epoch(&mut self) -> anyhow::Result<u64> {
        let epoch = self.next_drain_epoch;
        self.next_drain_epoch = epoch
            .checked_add(1)
            .filter(|next| *next < u64::MAX)
            .ok_or_else(|| anyhow::anyhow!("publication drain epoch exhausted"))?;
        for entry in &mut self.entries {
            if matches!(
                entry.retry.state,
                AgentPurgePublicationRetryState::Claimed { .. }
            ) {
                entry.retry.state = if entry.retry.attempts == 0 {
                    AgentPurgePublicationRetryState::Ready
                } else {
                    AgentPurgePublicationRetryState::BackingOff {
                        eligible_drain_epoch: epoch,
                    }
                };
            }
        }
        Ok(epoch)
    }
}

fn validate_publication_entry(entry: &AgentPurgePublicationEntry) -> anyhow::Result<()> {
    if entry.publication.host_device_ura.trim().is_empty()
        || entry.publication.generation == 0
        || entry.publication.projection_revision == 0
        || entry.publication.projection_digest.trim().is_empty()
        || entry.publication.tombstone_payload.is_empty()
        || entry.next_delivery_fence == 0
        || entry.next_delivery_fence == u64::MAX
    {
        anyhow::bail!(
            "Agent purge publication transaction `{}` has incomplete publication facts",
            entry.transaction_id
        );
    }
    if entry.retry.attempts > PUBLICATION_MAX_ATTEMPTS_PER_STAGE {
        anyhow::bail!(
            "Agent purge publication transaction `{}` exceeds the finite retry budget",
            entry.transaction_id
        );
    }
    let validate_evidence = |evidence: &AgentPurgePublicationFailureEvidence,
                             require_current_attempt: bool|
     -> anyhow::Result<()> {
        if evidence.stage != entry.stage
            || evidence.attempt == 0
            || evidence.attempt > PUBLICATION_MAX_ATTEMPTS_PER_STAGE
            || evidence.error.trim().is_empty()
            || evidence.observed_at_unix_ms == u64::MAX
            || (require_current_attempt && evidence.attempt != entry.retry.attempts)
        {
            anyhow::bail!(
                "Agent purge publication transaction `{}` has contradictory failure evidence",
                entry.transaction_id
            );
        }
        Ok(())
    };
    if let Some(evidence) = &entry.retry.last_reconciliation {
        if evidence.error.trim().is_empty()
            || evidence.attempt != PUBLICATION_MAX_ATTEMPTS_PER_STAGE
            || evidence.observed_at_unix_ms == u64::MAX
        {
            anyhow::bail!(
                "Agent purge publication transaction `{}` has invalid reconciliation evidence",
                entry.transaction_id
            );
        }
    }
    match &entry.retry.state {
        AgentPurgePublicationRetryState::Ready => {
            if entry.retry.attempts != 0 || entry.retry.last_failure.is_some() {
                anyhow::bail!(
                    "Agent purge publication transaction `{}` has contradictory ready state",
                    entry.transaction_id
                );
            }
        }
        AgentPurgePublicationRetryState::BackingOff {
            eligible_drain_epoch,
        } => {
            let evidence = entry.retry.last_failure.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Agent purge publication transaction `{}` backs off without failure evidence",
                    entry.transaction_id
                )
            })?;
            if entry.retry.attempts == 0
                || entry.retry.attempts >= PUBLICATION_MAX_ATTEMPTS_PER_STAGE
                || *eligible_drain_epoch == 0
                || *eligible_drain_epoch == u64::MAX
            {
                anyhow::bail!(
                    "Agent purge publication transaction `{}` has invalid backing-off budget",
                    entry.transaction_id
                );
            }
            validate_evidence(evidence, true)?;
        }
        AgentPurgePublicationRetryState::Claimed {
            claim_id,
            drain_epoch,
            delivery_fence,
        } => {
            if claim_id.trim().is_empty()
                || *drain_epoch == 0
                || *delivery_fence == 0
                || *delivery_fence >= entry.next_delivery_fence
            {
                anyhow::bail!(
                    "Agent purge publication transaction `{}` has invalid claim lease",
                    entry.transaction_id
                );
            }
            match (&entry.retry.last_failure, entry.retry.attempts) {
                (None, 0) => {}
                (Some(evidence), attempts)
                    if attempts > 0 && attempts < PUBLICATION_MAX_ATTEMPTS_PER_STAGE =>
                {
                    validate_evidence(evidence, true)?;
                }
                _ => anyhow::bail!(
                    "Agent purge publication transaction `{}` has contradictory claimed state",
                    entry.transaction_id
                ),
            }
        }
        AgentPurgePublicationRetryState::ReconciliationRequired {
            dead_lettered_at_unix_ms,
        } => {
            let evidence = entry.retry.last_failure.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Agent purge publication transaction `{}` requires reconciliation without evidence",
                    entry.transaction_id
                )
            })?;
            if entry.retry.attempts != PUBLICATION_MAX_ATTEMPTS_PER_STAGE
                || *dead_lettered_at_unix_ms == u64::MAX
                || *dead_lettered_at_unix_ms != evidence.observed_at_unix_ms
            {
                anyhow::bail!(
                    "Agent purge publication transaction `{}` has contradictory reconciliation state",
                    entry.transaction_id
                );
            }
            validate_evidence(evidence, true)?;
        }
    }
    Ok(())
}

pub(crate) fn journal_path() -> PathBuf {
    config::state_dir().join(JOURNAL_FILE_NAME)
}

pub(crate) fn load_purge_journal() -> anyhow::Result<Option<AgentPurgeJournal>> {
    let path = journal_path();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(anyhow::anyhow!(
                "read Agent purge journal {}: {error}",
                path.display()
            ));
        }
    };
    let journal: AgentPurgeJournal = serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::anyhow!("parse Agent purge journal {}: {error}", path.display())
    })?;
    if journal.schema_version != JOURNAL_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported Agent purge journal schema {} at {}; expected {}",
            journal.schema_version,
            path.display(),
            JOURNAL_SCHEMA_VERSION
        );
    }
    Ok(Some(journal))
}

pub(crate) fn save_purge_journal(journal: &AgentPurgeJournal) -> anyhow::Result<()> {
    let path = journal_path();
    let bytes = serde_json::to_vec_pretty(journal)?;
    config::atomic_write_with_permissions(&path, &bytes, WritePermissions::OwnerReadWrite)
        .map_err(|error| anyhow::anyhow!("persist Agent purge journal {}: {error}", path.display()))
}

pub(crate) fn clear_purge_journal() -> anyhow::Result<()> {
    let path = journal_path();
    match fs::remove_file(&path) {
        Ok(()) => config::sync_parent_dir(&path)
            .map_err(|error| anyhow::anyhow!("sync Agent purge journal removal: {error:#}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow::anyhow!(
            "remove Agent purge journal {}: {error}",
            path.display()
        )),
    }
}

pub(crate) fn publication_outbox_path() -> PathBuf {
    config::state_dir().join(PUBLICATION_OUTBOX_FILE_NAME)
}

pub(crate) fn load_publication_outbox() -> anyhow::Result<AgentPurgePublicationOutbox> {
    let _thread_guard = OUTBOX_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = publication_outbox_path();
    let _process_guard = ExclusiveFileLock::acquire_for_data_path(&path)?;
    load_publication_outbox_unlocked(&path)
}

pub(crate) fn update_publication_outbox<T>(
    mutate: impl FnOnce(&mut AgentPurgePublicationOutbox) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let _thread_guard = OUTBOX_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = publication_outbox_path();
    let _process_guard = ExclusiveFileLock::acquire_for_data_path(&path)?;
    let mut outbox = load_publication_outbox_unlocked(&path)?;
    let output = mutate(&mut outbox)?;
    outbox.validate()?;
    let bytes = serde_json::to_vec_pretty(&outbox)?;
    config::atomic_write_with_permissions(&path, &bytes, WritePermissions::OwnerReadWrite)
        .map_err(|error| {
            anyhow::anyhow!(
                "persist Agent purge publication outbox {}: {error}",
                path.display()
            )
        })?;
    Ok(output)
}

pub(crate) fn reconcile_publication(
    command: &AgentPurgeReconciliationCommand,
    authorization: &AuthorizedPurgeReconciliation,
    now_unix_ms: u64,
) -> anyhow::Result<AgentPurgeReconciliationOutcome> {
    let digest = command.digest()?;
    if command.actor_ura != authorization.actor_ura || now_unix_ms == u64::MAX {
        anyhow::bail!("purge reconciliation authorization does not bind the command actor");
    }
    update_publication_outbox(|outbox| {
        if let Some(audit) = outbox
            .reconciliation_audit
            .iter()
            .find(|audit| audit.command.command_id == command.command_id)
        {
            if audit.command_digest != digest || audit.command != *command {
                anyhow::bail!("purge reconciliation command ID is bound to a different command");
            }
            return Ok(AgentPurgeReconciliationOutcome {
                entry: audit.outcome.clone(),
                replayed: true,
            });
        }
        let entry = outbox
            .entries
            .iter_mut()
            .find(|entry| entry.transaction_id == command.transaction_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Agent purge publication transaction `{}` does not exist",
                    command.transaction_id
                )
            })?;
        let evidence = entry.retry.last_failure.clone().ok_or_else(|| {
            anyhow::anyhow!("purge reconciliation requires retained failure evidence")
        })?;
        let retained_delivery_fence = entry.next_delivery_fence;
        match command.action {
            AgentPurgePublicationReconciliation::Retry => entry.manual_retry()?,
        }
        let result = entry.clone();
        outbox
            .reconciliation_audit
            .push(AgentPurgeReconciliationAudit {
                command: command.clone(),
                command_digest: digest,
                authority_reference: authorization.authority_reference.clone(),
                failure_evidence: evidence,
                retained_delivery_fence,
                applied_at_unix_ms: now_unix_ms,
                outcome: result.clone(),
            });
        outbox
            .reconciliation_audit
            .sort_by(|a, b| a.command.command_id.cmp(&b.command.command_id));
        Ok(AgentPurgeReconciliationOutcome {
            entry: result,
            replayed: false,
        })
    })
}

fn valid_hex_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn load_publication_outbox_unlocked(
    path: &std::path::Path,
) -> anyhow::Result<AgentPurgePublicationOutbox> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AgentPurgePublicationOutbox::default());
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "read Agent purge publication outbox {}: {error}",
                path.display()
            ));
        }
    };
    let outbox: AgentPurgePublicationOutbox = serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::anyhow!(
            "parse Agent purge publication outbox {}: {error}",
            path.display()
        )
    })?;
    outbox.validate()?;
    Ok(outbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD_ENV: &str = "EASYNET_AGENT_LIFECYCLE_LOCK_CHILD";
    const READY_ENV: &str = "EASYNET_AGENT_LIFECYCLE_LOCK_READY";
    const DRAIN_CHILD_ENV: &str = "EASYNET_AGENT_PURGE_DRAIN_LOCK_CHILD";

    #[test]
    fn publication_retry_budget_resets_at_stage_progress() {
        let mut entry = publication_entry("00000000000000000000000000000001", "agent-reset");
        entry.claim(900, false, "claim-1".to_string()).unwrap();
        entry
            .record_claim_failure("claim-1", 1_000, "tombstone unavailable".to_string())
            .unwrap();
        assert_eq!(entry.retry.attempts, 1);
        assert!(entry.retry.last_failure.is_some());

        entry.claim(1_001, true, "claim-2".to_string()).unwrap();
        entry.advance_claim_to_revoke("claim-2").unwrap();

        assert_eq!(entry.stage, AgentPurgePublicationStage::RevokePending);
        assert_eq!(entry.retry.attempts, 0);
        assert!(entry.retry.last_failure.is_none());
        assert!(matches!(
            entry.retry.state,
            AgentPurgePublicationRetryState::Claimed { .. }
        ));
    }

    #[test]
    fn first_backoff_defers_one_complete_scheduled_drain() {
        let mut entry = publication_entry("00000000000000000000000000000009", "agent-backoff");
        entry.claim(10, false, "claim-1".to_string()).unwrap();
        entry
            .record_claim_failure("claim-1", 1_000, "revoke unavailable".to_string())
            .unwrap();

        assert!(matches!(
            entry.retry.state,
            AgentPurgePublicationRetryState::BackingOff {
                eligible_drain_epoch: 12
            }
        ));
        assert!(!entry.claim(11, false, "claim-2".to_string()).unwrap());
        assert!(entry.claim(12, false, "claim-3".to_string()).unwrap());
    }

    fn publication_entry(transaction_id: &str, name: &str) -> AgentPurgePublicationEntry {
        AgentPurgePublicationEntry {
            transaction_id: transaction_id.to_string(),
            name: name.to_string(),
            agent_ura: crate::core::ura::agent_ura("test", "alice", name),
            publication: AgentPurgePublication {
                host_device_ura: "easynet:///r/test/device/host".to_string(),
                generation: 1,
                projection_revision: 1,
                projection_digest: format!("sha256:{transaction_id}"),
                tombstone_payload: br#"{"owner_ura":"test"}"#.to_vec(),
            },
            stage: AgentPurgePublicationStage::TombstonePending,
            retry: AgentPurgePublicationRetry::default(),
            next_delivery_fence: 1,
        }
    }

    #[test]
    fn zero_drain_epoch_claim_is_rejected_without_state_mutation() {
        let mut entry = publication_entry("00000000000000000000000000000002", "agent-overflow");
        entry
            .claim(0, false, "overflow".to_string())
            .expect_err("claim requires a monotonic nonzero drain epoch");
        assert_eq!(entry.retry, AgentPurgePublicationRetry::default());
    }

    #[test]
    fn finite_budget_enters_reconciliation_and_manual_retry_retains_evidence() {
        let mut entry = publication_entry("00000000000000000000000000000003", "agent-dead");
        for attempt in 1..=PUBLICATION_MAX_ATTEMPTS_PER_STAGE {
            let now = u64::from(attempt) * 10_000;
            let claim_id = format!("claim-{attempt}");
            entry.claim(now, true, claim_id.clone()).unwrap();
            entry
                .record_claim_failure(&claim_id, now + 1, format!("failure-{attempt}"))
                .unwrap();
        }
        assert!(matches!(
            entry.retry.state,
            AgentPurgePublicationRetryState::ReconciliationRequired { .. }
        ));
        assert!(!entry.claim(100_000, true, "automatic".to_string()).unwrap());
        let terminal = entry.retry.last_failure.clone().unwrap();

        entry.manual_retry().unwrap();

        assert_eq!(entry.retry.state, AgentPurgePublicationRetryState::Ready);
        assert_eq!(entry.retry.attempts, 0);
        assert_eq!(entry.retry.last_reconciliation, Some(terminal));
    }

    #[test]
    fn authorized_reconciliation_is_idempotent_audited_and_conflict_closed() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut entry = publication_entry("00000000000000000000000000000006", "audit");
        for attempt in 1..=PUBLICATION_MAX_ATTEMPTS_PER_STAGE {
            let claim_id = format!("claim-{attempt}");
            entry
                .claim(u64::from(attempt), true, claim_id.clone())
                .unwrap();
            entry
                .record_claim_failure(&claim_id, u64::from(attempt), "poison".into())
                .unwrap();
        }
        update_publication_outbox(|outbox| {
            outbox.entries.push(entry);
            Ok(())
        })
        .unwrap();
        let command = AgentPurgeReconciliationCommand {
            command_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            transaction_id: "00000000000000000000000000000006".into(),
            actor_ura: "easynet:///r/test/user/operator".into(),
            action: AgentPurgePublicationReconciliation::Retry,
        };
        let denied = AuthorizedPurgeReconciliation::from_admission(
            "easynet:///r/test/user/other",
            "admission:manage:1",
        )
        .unwrap();
        assert!(reconcile_publication(&command, &denied, 100).is_err());

        let authorization = AuthorizedPurgeReconciliation::from_admission(
            command.actor_ura.clone(),
            "admission:manage:1",
        )
        .unwrap();
        let first = reconcile_publication(&command, &authorization, 100).unwrap();
        assert!(!first.replayed);
        let replay = reconcile_publication(&command, &authorization, 101).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.entry, first.entry);
        let persisted = load_publication_outbox().unwrap();
        assert_eq!(persisted.reconciliation_audit.len(), 1);
        assert_eq!(persisted.reconciliation_audit[0].retained_delivery_fence, 6);
        assert_eq!(
            persisted.reconciliation_audit[0].failure_evidence.error,
            "poison"
        );
        drop(persisted);
        update_publication_outbox(|outbox| {
            outbox
                .entries
                .retain(|entry| entry.transaction_id != command.transaction_id);
            Ok(())
        })
        .unwrap();
        let completed_replay = reconcile_publication(&command, &authorization, 102).unwrap();
        assert!(completed_replay.replayed);
        assert_eq!(completed_replay.entry, first.entry);

        let mut conflict = command;
        conflict.actor_ura = "easynet:///r/test/user/conflict".into();
        let conflict_authorization = AuthorizedPurgeReconciliation::from_admission(
            conflict.actor_ura.clone(),
            "admission:manage:2",
        )
        .unwrap();
        assert!(reconcile_publication(&conflict, &conflict_authorization, 103).is_err());
    }

    #[test]
    fn contradictory_backoff_state_is_rejected_fail_closed() {
        let mut entry = publication_entry("00000000000000000000000000000004", "agent-corrupt");
        entry.retry.state = AgentPurgePublicationRetryState::BackingOff {
            eligible_drain_epoch: u64::MAX,
        };
        entry.retry.attempts = 1;
        entry.retry.last_failure = Some(AgentPurgePublicationFailureEvidence {
            stage: AgentPurgePublicationStage::RevokePending,
            attempt: 1,
            observed_at_unix_ms: 1_000,
            error: "contradictory stage".to_string(),
        });
        let outbox = AgentPurgePublicationOutbox {
            schema_version: PUBLICATION_OUTBOX_SCHEMA_VERSION,
            next_drain_epoch: 1,
            entries: vec![entry],
            reconciliation_audit: Vec::new(),
        };
        assert!(outbox.validate().is_err());
    }

    #[test]
    fn lifecycle_lock_child_process() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        let _guard = AgentLifecycleMutationGuard::acquire().expect("child acquires lifecycle lock");
        let ready = PathBuf::from(std::env::var_os(READY_ENV).expect("ready marker path"));
        fs::write(&ready, b"locked").expect("publish child lock acquisition");
        std::thread::sleep(std::time::Duration::from_millis(1_200));
    }

    #[test]
    fn lifecycle_lock_serializes_independent_processes() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let ready = config::state_dir().join("lifecycle-lock-child-ready");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("daemon::persistence::agent_lifecycle::tests::lifecycle_lock_child_process")
            .arg("--nocapture")
            .env(CHILD_ENV, "1")
            .env(READY_ENV, &ready)
            .env("HOME", config::home_dir())
            .spawn()
            .expect("spawn independent lifecycle-lock holder");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready.exists(), "child did not publish lock acquisition");
        assert!(
            ExclusiveFileLock::try_acquire_for_data_path(&journal_path())
                .unwrap()
                .is_none(),
            "second process must not enter lifecycle mutation while child holds lock"
        );

        assert!(child.wait().expect("wait for lock child").success());
        assert!(
            ExclusiveFileLock::try_acquire_for_data_path(&journal_path())
                .unwrap()
                .is_some(),
            "lock must be released when the owning process exits"
        );
    }

    #[test]
    fn publication_drain_guard_child_process() {
        if std::env::var_os(DRAIN_CHILD_ENV).is_none() {
            return;
        }
        let _guard = AgentPurgePublicationDrainGuard::try_acquire()
            .expect("child checks drain lock")
            .expect("child acquires drain lock");
        let ready = PathBuf::from(std::env::var_os(READY_ENV).expect("ready marker path"));
        fs::write(&ready, b"locked").expect("publish child drain lock acquisition");
        std::thread::sleep(std::time::Duration::from_millis(1_200));
    }

    #[test]
    fn publication_drain_guard_excludes_independent_process() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let ready = config::state_dir().join("publication-drain-child-ready");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("daemon::persistence::agent_lifecycle::tests::publication_drain_guard_child_process")
            .arg("--nocapture")
            .env(DRAIN_CHILD_ENV, "1")
            .env(READY_ENV, &ready)
            .env("HOME", config::home_dir())
            .spawn()
            .expect("spawn independent drain-lock holder");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready.exists(), "child did not acquire drain lock");
        assert!(AgentPurgePublicationDrainGuard::try_acquire()
            .unwrap()
            .is_none());
        assert!(child.wait().unwrap().success());
        assert!(AgentPurgePublicationDrainGuard::try_acquire()
            .unwrap()
            .is_some());
    }
}
