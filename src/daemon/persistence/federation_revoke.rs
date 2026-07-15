//! Durable Hub inventory and generation-bound federation revoke FSM.
//!
//! Hosted Agent registration and purge revoke share one locked source of
//! truth. A revoke is first persisted as `Prepared`, then conditionally
//! retires the exact inventory generation, and only then becomes `Applied`.
//! Recovery resumes `Prepared`; an `Applied` outcome is immutable.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::config::{self, WritePermissions};
use super::file_lock::ExclusiveFileLock;

const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "hub-hosted-agent-inventory.json";
pub(crate) const REVOKE_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum DurableSigningAuthority {
    SelfSigned,
    HostedBy { host_ura: String },
}

impl DurableSigningAuthority {
    pub(crate) fn authority_ura<'a>(&'a self, agent_ura: &'a str) -> &'a str {
        match self {
            Self::SelfSigned => agent_ura,
            Self::HostedBy { host_ura } => host_ura,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InventoryLifecycle {
    Active,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HostedAgentInventoryRecord {
    pub agent_ura: String,
    pub generation: u64,
    pub public_key_hex: String,
    pub host_node_id: Option<String>,
    pub signing_authority: DurableSigningAuthority,
    pub lifecycle: InventoryLifecycle,
}

impl HostedAgentInventoryRecord {
    fn validate(&self) -> anyhow::Result<()> {
        if self.agent_ura.trim().is_empty() || self.generation == 0 {
            anyhow::bail!("hosted Agent inventory record has an invalid identity");
        }
        if let DurableSigningAuthority::HostedBy { host_ura } = &self.signing_authority {
            if host_ura.trim().is_empty() {
                anyhow::bail!("hosted Agent inventory record has an empty authority");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FederationRevokeCommand {
    pub protocol_version: u32,
    pub transaction_id: String,
    pub agent_ura: String,
    pub generation: u64,
    pub reason: String,
    pub authority_ura: String,
    pub target_ura: String,
}

impl FederationRevokeCommand {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        if self.protocol_version != REVOKE_PROTOCOL_VERSION {
            anyhow::bail!("unsupported federation revoke protocol version");
        }
        if !valid_transaction_id(&self.transaction_id)
            || self.agent_ura.trim().is_empty()
            || self.generation == 0
            || self.reason.trim().is_empty()
            || self.authority_ura.trim().is_empty()
            || self.target_ura.trim().is_empty()
            || self.target_ura != self.agent_ura
        {
            anyhow::bail!("federation revoke command is incomplete or contradictory");
        }
        Ok(())
    }

    pub(crate) fn canonical_digest(&self) -> anyhow::Result<String> {
        self.validate()?;
        let mut hash = Sha256::new();
        hash.update(b"easynet.federation.revoke.command\0");
        hash_field(&mut hash, &self.protocol_version.to_string());
        hash_field(&mut hash, &self.transaction_id);
        hash_field(&mut hash, &self.agent_ura);
        hash_field(&mut hash, &self.generation.to_string());
        hash_field(&mut hash, &self.reason);
        hash_field(&mut hash, &self.authority_ura);
        hash_field(&mut hash, &self.target_ura);
        Ok(format!("sha256:{}", hex::encode(hash.finalize())))
    }
}

fn hash_field(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value.as_bytes());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederationRevokeDisposition {
    Retired,
    AlreadyRetired,
    SupersededByNewIncarnation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FederationRevokeOutcome {
    pub disposition: FederationRevokeDisposition,
    pub was_active: bool,
    pub presence_session_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum FederationRevokeState {
    Prepared {
        observed_was_active: bool,
        presence_session_id: Option<u64>,
        prepared_at_unix_ms: u64,
    },
    Applied {
        outcome: FederationRevokeOutcome,
        applied_at_unix_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FederationRevokeTransaction {
    command: FederationRevokeCommand,
    command_digest: String,
    max_delivery_fence: u64,
    state: FederationRevokeState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PurgeProjectionDeliveryCommand {
    pub protocol_version: u32,
    pub transaction_id: String,
    pub owner_ura: String,
    pub generation: u64,
    pub projection_revision: u64,
    pub projection_digest: String,
    pub authority_ura: String,
}

impl PurgeProjectionDeliveryCommand {
    fn canonical_digest(&self) -> anyhow::Result<String> {
        if self.protocol_version != REVOKE_PROTOCOL_VERSION
            || !valid_transaction_id(&self.transaction_id)
            || self.owner_ura.trim().is_empty()
            || self.generation == 0
            || self.projection_revision == 0
            || self.projection_digest.trim().is_empty()
            || self.authority_ura.trim().is_empty()
        {
            anyhow::bail!("purge projection delivery command is incomplete");
        }
        let mut hash = Sha256::new();
        hash.update(b"easynet.purge.projection.delivery\0");
        hash_field(&mut hash, &self.protocol_version.to_string());
        hash_field(&mut hash, &self.transaction_id);
        hash_field(&mut hash, &self.owner_ura);
        hash_field(&mut hash, &self.generation.to_string());
        hash_field(&mut hash, &self.projection_revision.to_string());
        hash_field(&mut hash, &self.projection_digest);
        hash_field(&mut hash, &self.authority_ura);
        Ok(format!("sha256:{}", hex::encode(hash.finalize())))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PurgeProjectionDeliveryRecord {
    command: PurgeProjectionDeliveryCommand,
    command_digest: String,
    max_delivery_fence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HubHostedAgentInventoryFile {
    schema_version: u32,
    records: Vec<HostedAgentInventoryRecord>,
    revoke_transactions: Vec<FederationRevokeTransaction>,
    #[serde(default)]
    projection_deliveries: Vec<PurgeProjectionDeliveryRecord>,
}

impl Default for HubHostedAgentInventoryFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            records: Vec::new(),
            revoke_transactions: Vec::new(),
            projection_deliveries: Vec::new(),
        }
    }
}

impl HubHostedAgentInventoryFile {
    fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            anyhow::bail!("unsupported Hub hosted-Agent inventory schema");
        }
        let mut agents = std::collections::BTreeSet::new();
        for record in &self.records {
            record.validate()?;
            if !agents.insert(record.agent_ura.as_str()) {
                anyhow::bail!("duplicate hosted Agent inventory identity");
            }
        }
        let mut transactions = std::collections::BTreeSet::new();
        for transaction in &self.revoke_transactions {
            transaction.command.validate()?;
            if transaction.command_digest != transaction.command.canonical_digest()?
                || transaction.max_delivery_fence == 0
                || !transactions.insert(transaction.command.transaction_id.as_str())
            {
                anyhow::bail!("corrupt federation revoke transaction");
            }
            match &transaction.state {
                FederationRevokeState::Prepared {
                    prepared_at_unix_ms,
                    ..
                } if *prepared_at_unix_ms == u64::MAX => {
                    anyhow::bail!("federation revoke has an invalid prepared timestamp")
                }
                FederationRevokeState::Applied {
                    applied_at_unix_ms, ..
                } if *applied_at_unix_ms == u64::MAX => {
                    anyhow::bail!("federation revoke has an invalid applied timestamp")
                }
                _ => {}
            }
        }
        let mut projection_transactions = std::collections::BTreeSet::new();
        for delivery in &self.projection_deliveries {
            if delivery.command_digest != delivery.command.canonical_digest()?
                || delivery.max_delivery_fence == 0
                || !projection_transactions.insert(delivery.command.transaction_id.as_str())
            {
                anyhow::bail!("corrupt purge projection delivery fence");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegistrationOutcome {
    Inserted,
    AdvancedGeneration,
    Idempotent,
}

pub(crate) fn register_agent(
    mut record: HostedAgentInventoryRecord,
) -> anyhow::Result<RegistrationOutcome> {
    record.validate()?;
    record.lifecycle = InventoryLifecycle::Active;
    update(|file| {
        let Some(current) = file
            .records
            .iter_mut()
            .find(|current| current.agent_ura == record.agent_ura)
        else {
            file.records.push(record);
            file.records.sort_by(|a, b| a.agent_ura.cmp(&b.agent_ura));
            return Ok(RegistrationOutcome::Inserted);
        };
        if record.generation < current.generation {
            anyhow::bail!("stale hosted Agent generation cannot replace durable inventory");
        }
        if record.generation == current.generation {
            if current == &record {
                return Ok(RegistrationOutcome::Idempotent);
            }
            anyhow::bail!(
                "hosted Agent generation is already bound to different registration facts"
            );
        }
        *current = record;
        Ok(RegistrationOutcome::AdvancedGeneration)
    })
}

pub(crate) fn active_inventory() -> anyhow::Result<Vec<HostedAgentInventoryRecord>> {
    read().map(|file| {
        file.records
            .into_iter()
            .filter(|record| record.lifecycle == InventoryLifecycle::Active)
            .collect()
    })
}

/// Persist the purge projection delivery fence before mutating the Hub read
/// model. Same-command higher-fence takeover is legal; stale workers and
/// command rebinding fail closed.
pub(crate) fn record_projection_delivery(
    command: &PurgeProjectionDeliveryCommand,
    delivery_fence: u64,
) -> anyhow::Result<bool> {
    let digest = command.canonical_digest()?;
    if delivery_fence == 0 {
        anyhow::bail!("purge projection delivery fence must be nonzero");
    }
    update(|file| {
        if let Some(current) = file
            .projection_deliveries
            .iter_mut()
            .find(|current| current.command.transaction_id == command.transaction_id)
        {
            if current.command != *command || current.command_digest != digest {
                anyhow::bail!("purge projection transaction command digest conflict");
            }
            if delivery_fence < current.max_delivery_fence {
                anyhow::bail!("stale purge projection delivery fence");
            }
            let replayed = delivery_fence == current.max_delivery_fence;
            current.max_delivery_fence = delivery_fence;
            return Ok(replayed);
        }
        file.projection_deliveries
            .push(PurgeProjectionDeliveryRecord {
                command: command.clone(),
                command_digest: digest,
                max_delivery_fence: delivery_fence,
            });
        file.projection_deliveries
            .sort_by(|a, b| a.command.transaction_id.cmp(&b.command.transaction_id));
        Ok(false)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrepareRevokeOutcome {
    Prepared,
    Applied(FederationRevokeOutcome),
}

pub(crate) fn prepare_revoke(
    command: &FederationRevokeCommand,
    delivery_fence: u64,
    observed_was_active: bool,
    presence_session_id: Option<u64>,
    now_unix_ms: u64,
) -> anyhow::Result<PrepareRevokeOutcome> {
    let digest = command.canonical_digest()?;
    if delivery_fence == 0 || now_unix_ms == u64::MAX {
        anyhow::bail!("federation revoke requires a finite delivery fence and timestamp");
    }
    update(|file| {
        if let Some(transaction) = file
            .revoke_transactions
            .iter_mut()
            .find(|transaction| transaction.command.transaction_id == command.transaction_id)
        {
            if transaction.command_digest != digest || transaction.command != *command {
                anyhow::bail!("federation revoke transaction command digest conflict");
            }
            if delivery_fence < transaction.max_delivery_fence {
                anyhow::bail!("stale federation revoke delivery fence");
            }
            transaction.max_delivery_fence = delivery_fence;
            return match &transaction.state {
                FederationRevokeState::Prepared { .. } => Ok(PrepareRevokeOutcome::Prepared),
                FederationRevokeState::Applied { outcome, .. } => {
                    Ok(PrepareRevokeOutcome::Applied(outcome.clone()))
                }
            };
        }
        file.revoke_transactions.push(FederationRevokeTransaction {
            command: command.clone(),
            command_digest: digest,
            max_delivery_fence: delivery_fence,
            state: FederationRevokeState::Prepared {
                observed_was_active,
                presence_session_id,
                prepared_at_unix_ms: now_unix_ms,
            },
        });
        file.revoke_transactions
            .sort_by(|a, b| a.command.transaction_id.cmp(&b.command.transaction_id));
        Ok(PrepareRevokeOutcome::Prepared)
    })
}

pub(crate) fn apply_prepared_revoke(
    transaction_id: &str,
    delivery_fence: u64,
    now_unix_ms: u64,
) -> anyhow::Result<(FederationRevokeOutcome, bool)> {
    if delivery_fence == 0 || now_unix_ms == u64::MAX {
        anyhow::bail!("federation revoke apply requires finite facts");
    }
    update(|file| {
        let index = file
            .revoke_transactions
            .iter()
            .position(|transaction| transaction.command.transaction_id == transaction_id)
            .ok_or_else(|| anyhow::anyhow!("federation revoke transaction is not prepared"))?;
        let transaction = &file.revoke_transactions[index];
        if delivery_fence != transaction.max_delivery_fence {
            anyhow::bail!("stale federation revoke delivery fence cannot apply");
        }
        if let FederationRevokeState::Applied { outcome, .. } = &transaction.state {
            return Ok((outcome.clone(), true));
        }
        let (observed_was_active, presence_session_id) = match transaction.state {
            FederationRevokeState::Prepared {
                observed_was_active,
                presence_session_id,
                ..
            } => (observed_was_active, presence_session_id),
            FederationRevokeState::Applied { .. } => unreachable!(),
        };
        let command = transaction.command.clone();
        let record = file
            .records
            .iter_mut()
            .find(|record| record.agent_ura == command.agent_ura)
            .ok_or_else(|| {
                anyhow::anyhow!("revoke target is absent from durable hosted Agent inventory")
            })?;
        if record.signing_authority.authority_ura(&record.agent_ura) != command.authority_ura {
            anyhow::bail!("revoke authority does not match durable registration");
        }
        let disposition = if record.generation > command.generation {
            FederationRevokeDisposition::SupersededByNewIncarnation
        } else if record.generation < command.generation {
            anyhow::bail!("revoke generation is ahead of durable registration");
        } else if record.lifecycle == InventoryLifecycle::Retired {
            FederationRevokeDisposition::AlreadyRetired
        } else {
            record.lifecycle = InventoryLifecycle::Retired;
            FederationRevokeDisposition::Retired
        };
        let outcome = FederationRevokeOutcome {
            disposition,
            was_active: observed_was_active,
            presence_session_id,
        };
        file.revoke_transactions[index].state = FederationRevokeState::Applied {
            outcome: outcome.clone(),
            applied_at_unix_ms: now_unix_ms,
        };
        Ok((outcome, false))
    })
}

/// Complete every durable `Prepared` transaction during Hub boot before the
/// active inventory is hydrated into process-local read models.
pub(crate) fn recover_prepared_revokes() -> anyhow::Result<Vec<FederationRevokeOutcome>> {
    let prepared = read()?
        .revoke_transactions
        .into_iter()
        .filter_map(|transaction| {
            matches!(transaction.state, FederationRevokeState::Prepared { .. }).then_some((
                transaction.command.transaction_id,
                transaction.max_delivery_fence,
            ))
        })
        .collect::<Vec<_>>();
    let now_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow::anyhow!("Hub revoke recovery clock precedes epoch: {error}"))?
        .as_millis()
        .try_into()
        .map_err(|_| anyhow::anyhow!("Hub revoke recovery clock exceeds u64"))?;
    prepared
        .into_iter()
        .map(|(transaction_id, delivery_fence)| {
            apply_prepared_revoke(&transaction_id, delivery_fence, now_unix_ms)
                .map(|(outcome, _)| outcome)
        })
        .collect()
}

fn read() -> anyhow::Result<HubHostedAgentInventoryFile> {
    let path = path();
    let _guard = ExclusiveFileLock::acquire_for_data_path(&path)?;
    load_unlocked(&path)
}

fn update<T>(
    mutate: impl FnOnce(&mut HubHostedAgentInventoryFile) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let path = path();
    let _guard = ExclusiveFileLock::acquire_for_data_path(&path)?;
    let mut file = load_unlocked(&path)?;
    let output = mutate(&mut file)?;
    save_unlocked(&path, &file)?;
    Ok(output)
}

fn load_unlocked(path: &Path) -> anyhow::Result<HubHostedAgentInventoryFile> {
    match fs::read(path) {
        Ok(bytes) => {
            let file: HubHostedAgentInventoryFile = serde_json::from_slice(&bytes)
                .map_err(|error| anyhow::anyhow!("parse {}: {error}", path.display()))?;
            file.validate()?;
            Ok(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(HubHostedAgentInventoryFile::default())
        }
        Err(error) => Err(error.into()),
    }
}

fn save_unlocked(path: &Path, file: &HubHostedAgentInventoryFile) -> anyhow::Result<()> {
    file.validate()?;
    fs::create_dir_all(config::state_dir())?;
    config::atomic_write_with_permissions(
        path,
        &serde_json::to_vec_pretty(file)?,
        WritePermissions::OwnerReadWrite,
    )
    .map_err(Into::into)
}

fn path() -> PathBuf {
    config::state_dir().join(FILE_NAME)
}

fn valid_transaction_id(transaction_id: &str) -> bool {
    transaction_id.len() == 32
        && transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(generation: u64) -> HostedAgentInventoryRecord {
        HostedAgentInventoryRecord {
            agent_ura: "easynet:///r/test/agent/alice.worker".into(),
            generation,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: DurableSigningAuthority::HostedBy {
                host_ura: "easynet:///r/test/device/dev-1".into(),
            },
            lifecycle: InventoryLifecycle::Active,
        }
    }

    fn command(generation: u64) -> FederationRevokeCommand {
        FederationRevokeCommand {
            protocol_version: REVOKE_PROTOCOL_VERSION,
            transaction_id: "0123456789abcdef0123456789abcdef".into(),
            agent_ura: "easynet:///r/test/agent/alice.worker".into(),
            generation,
            reason: "agent.purge".into(),
            authority_ura: "easynet:///r/test/device/dev-1".into(),
            target_ura: "easynet:///r/test/agent/alice.worker".into(),
        }
    }

    #[test]
    fn prepared_recovery_applies_once_and_persists_exact_outcome() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register_agent(record(1)).unwrap();
        let command = command(1);
        assert_eq!(
            prepare_revoke(&command, 7, true, Some(19), 1_000).unwrap(),
            PrepareRevokeOutcome::Prepared
        );
        let (first, replayed) = apply_prepared_revoke(&command.transaction_id, 7, 1_001).unwrap();
        assert!(!replayed);
        assert_eq!(first.disposition, FederationRevokeDisposition::Retired);
        let recovered = prepare_revoke(&command, 8, false, None, 2_000).unwrap();
        assert_eq!(recovered, PrepareRevokeOutcome::Applied(first));
    }

    #[test]
    fn changed_command_field_conflicts_with_persisted_digest() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register_agent(record(1)).unwrap();
        let command = command(1);
        prepare_revoke(&command, 1, false, None, 1).unwrap();
        let mut changed = command;
        changed.reason = "agent.stop".into();
        assert!(prepare_revoke(&changed, 2, false, None, 2).is_err());
    }

    #[test]
    fn delayed_old_transaction_cannot_retire_new_incarnation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register_agent(record(1)).unwrap();
        let old = command(1);
        prepare_revoke(&old, 1, true, None, 1).unwrap();
        register_agent(record(2)).unwrap();
        let (outcome, _) = apply_prepared_revoke(&old.transaction_id, 1, 2).unwrap();
        assert_eq!(
            outcome.disposition,
            FederationRevokeDisposition::SupersededByNewIncarnation
        );
        assert_eq!(active_inventory().unwrap(), vec![record(2)]);
    }

    #[test]
    fn takeover_fence_rejects_delayed_old_worker() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register_agent(record(1)).unwrap();
        let command = command(1);
        prepare_revoke(&command, 3, true, None, 1).unwrap();
        prepare_revoke(&command, 4, true, None, 2).unwrap();
        assert!(apply_prepared_revoke(&command.transaction_id, 3, 3).is_err());
        assert!(apply_prepared_revoke(&command.transaction_id, 4, 4).is_ok());
    }

    #[test]
    fn projection_takeover_rejects_delayed_slow_worker_fence() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let command = PurgeProjectionDeliveryCommand {
            protocol_version: REVOKE_PROTOCOL_VERSION,
            transaction_id: "fedcba9876543210fedcba9876543210".into(),
            owner_ura: "easynet:///r/test/agent/alice.worker".into(),
            generation: 3,
            projection_revision: 9,
            projection_digest: "sha256:tombstone-9".into(),
            authority_ura: "easynet:///r/test/device/dev-1".into(),
        };
        assert!(!record_projection_delivery(&command, 10).unwrap());
        assert!(!record_projection_delivery(&command, 11).unwrap());
        assert!(record_projection_delivery(&command, 10).is_err());
        assert!(record_projection_delivery(&command, 11).unwrap());
    }

    #[test]
    fn hub_boot_recovery_completes_prepared_before_inventory_hydration() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register_agent(record(1)).unwrap();
        let command = command(1);
        prepare_revoke(&command, 4, true, Some(9), 1).unwrap();

        let recovered = recover_prepared_revokes().unwrap();

        assert_eq!(recovered.len(), 1);
        assert_eq!(
            recovered[0].disposition,
            FederationRevokeDisposition::Retired
        );
        assert!(active_inventory().unwrap().is_empty());
    }
}
