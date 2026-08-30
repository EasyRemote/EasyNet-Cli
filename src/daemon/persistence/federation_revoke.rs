//! Durable Hub inventory and generation-bound federation revoke FSM.
//!
//! Hosted Agent registration and purge revoke share one locked source of
//! truth. A revoke is first persisted as `Prepared`, then conditionally
//! retires the exact inventory generation, and only then becomes `Applied`.
//! Recovery resumes `Prepared`; an `Applied` outcome is immutable.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::daemon::federation::hosted_agent_publication::{
    HostedAgentGenerationAssignment, HostedAgentIncarnationId,
};

use super::config::{self, WritePermissions};
use super::file_lock::ExclusiveFileLock;

const SCHEMA_VERSION: u32 = 2;
const PRE_INCARNATION_SCHEMA_VERSION: u32 = 1;
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
    pub incarnation_id: HostedAgentIncarnationId,
    pub generation: u64,
    pub public_key_hex: String,
    pub host_node_id: Option<String>,
    pub signing_authority: DurableSigningAuthority,
    pub lifecycle: InventoryLifecycle,
}

impl HostedAgentInventoryRecord {
    fn validate(&self) -> anyhow::Result<()> {
        if self.generation == 0 {
            anyhow::bail!("hosted Agent inventory record has an invalid identity");
        }
        HostedAgentRegistrationCommand {
            agent_ura: self.agent_ura.clone(),
            incarnation_id: self.incarnation_id.clone(),
            public_key_hex: self.public_key_hex.clone(),
            host_node_id: self.host_node_id.clone(),
            signing_authority: self.signing_authority.clone(),
        }
        .validate()
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
        incarnation_id: HostedAgentIncarnationId,
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

/// Last durable shape before hosted-Agent incarnation tokens became part of
/// the Hub allocation protocol. These types exist only at the one-way storage
/// migration boundary; runtime code never operates on them.
#[derive(Debug, Deserialize)]
struct PreIncarnationInventoryFile {
    schema_version: u32,
    records: Vec<PreIncarnationInventoryRecord>,
    revoke_transactions: Vec<PreIncarnationRevokeTransaction>,
    #[serde(default)]
    projection_deliveries: Vec<PurgeProjectionDeliveryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreIncarnationInventoryRecord {
    agent_ura: String,
    generation: u64,
    public_key_hex: String,
    host_node_id: Option<String>,
    signing_authority: DurableSigningAuthority,
    lifecycle: InventoryLifecycle,
}

#[derive(Debug, Deserialize)]
struct PreIncarnationRevokeTransaction {
    command: FederationRevokeCommand,
    command_digest: String,
    max_delivery_fence: u64,
    state: PreIncarnationRevokeState,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum PreIncarnationRevokeState {
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
            if !agents.insert(inventory_key(record)) {
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
                    incarnation_id,
                    prepared_at_unix_ms,
                    ..
                } => {
                    HostedAgentIncarnationId::parse(incarnation_id.as_str())
                        .map_err(anyhow::Error::msg)?;
                    if *prepared_at_unix_ms == u64::MAX {
                        anyhow::bail!("federation revoke has an invalid prepared timestamp");
                    }
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

/// Device-observed registration facts. Generation is intentionally absent:
/// only the Hub aggregate may allocate the realm-global incarnation counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostedAgentRegistrationCommand {
    pub agent_ura: String,
    pub incarnation_id: HostedAgentIncarnationId,
    pub public_key_hex: String,
    pub host_node_id: Option<String>,
    pub signing_authority: DurableSigningAuthority,
}

impl HostedAgentRegistrationCommand {
    fn validate(&self) -> anyhow::Result<()> {
        if self.agent_ura.trim().is_empty() || self.agent_ura.trim() != self.agent_ura {
            anyhow::bail!("hosted Agent registration has an invalid agent URA");
        }
        let agent = crate::core::ura::parse_ura(&self.agent_ura).map_err(|error| {
            anyhow::anyhow!("hosted Agent registration URA is invalid: {error}")
        })?;
        if agent.kind != crate::core::ura::URAKind::Agent || agent.agent_ids().is_none() {
            anyhow::bail!("hosted Agent registration target must be an Agent URA");
        }
        HostedAgentIncarnationId::parse(self.incarnation_id.as_str())
            .map_err(anyhow::Error::msg)?;
        if self
            .host_node_id
            .as_deref()
            .is_some_and(|host| host.trim().is_empty() || host.trim() != host)
        {
            anyhow::bail!("hosted Agent registration has an invalid host node id");
        }
        match &self.signing_authority {
            DurableSigningAuthority::HostedBy { host_ura } => {
                if host_ura.trim().is_empty() || host_ura.trim() != host_ura {
                    anyhow::bail!("hosted Agent registration has an invalid signing host");
                }
                let host = crate::core::ura::parse_ura(host_ura).map_err(|error| {
                    anyhow::anyhow!("hosted Agent registration signing host is invalid: {error}")
                })?;
                if host.kind != crate::core::ura::URAKind::Device || host.realm != agent.realm {
                    anyhow::bail!("hosted Agent registration must be bound to a same-realm Device");
                }
                if let Some(host_node_id) = &self.host_node_id {
                    if host.device_id() != Some(host_node_id.as_str()) {
                        anyhow::bail!(
                            "hosted Agent registration host node contradicts signing Device"
                        );
                    }
                }
            }
            DurableSigningAuthority::SelfSigned => {
                anyhow::bail!("Hub hosted-Agent registration requires Device signing custody")
            }
        }
        Ok(())
    }

    fn into_record(self, generation: u64) -> HostedAgentInventoryRecord {
        HostedAgentInventoryRecord {
            agent_ura: self.agent_ura,
            incarnation_id: self.incarnation_id,
            generation,
            public_key_hex: self.public_key_hex,
            host_node_id: self.host_node_id,
            signing_authority: self.signing_authority,
            lifecycle: InventoryLifecycle::Active,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostedAgentRegistrationResult {
    pub outcome: RegistrationOutcome,
    pub assignment: HostedAgentGenerationAssignment,
    pub record: HostedAgentInventoryRecord,
}

pub(crate) fn register_agent(
    command: HostedAgentRegistrationCommand,
) -> anyhow::Result<HostedAgentRegistrationResult> {
    command.validate()?;
    update(|file| {
        let Some(current) = file
            .records
            .iter_mut()
            .find(|current| current.agent_ura == command.agent_ura)
        else {
            let record = command.into_record(1);
            let result = registration_result(RegistrationOutcome::Inserted, &record);
            file.records.push(record);
            file.records
                .sort_by(|a, b| inventory_key(a).cmp(inventory_key(b)));
            return Ok(result);
        };

        match current.lifecycle {
            InventoryLifecycle::Active => {
                if current.incarnation_id != command.incarnation_id {
                    anyhow::bail!(
                        "hosted Agent already has an active incarnation; revoke it before registering another"
                    );
                }
                if !current.matches_registration_facts(&command) {
                    anyhow::bail!(
                        "hosted Agent incarnation is already bound to different registration facts"
                    );
                }
                Ok(registration_result(
                    RegistrationOutcome::Idempotent,
                    current,
                ))
            }
            InventoryLifecycle::Retired => {
                if current.incarnation_id == command.incarnation_id {
                    anyhow::bail!("retired hosted Agent incarnation cannot be reactivated");
                }
                let next_generation = current
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("hosted Agent generation exhausted"))?;
                let record = command.into_record(next_generation);
                *current = record;
                Ok(registration_result(
                    RegistrationOutcome::AdvancedGeneration,
                    current,
                ))
            }
        }
    })
}

impl HostedAgentInventoryRecord {
    fn matches_registration_facts(&self, command: &HostedAgentRegistrationCommand) -> bool {
        self.agent_ura == command.agent_ura
            && self.incarnation_id == command.incarnation_id
            && self.public_key_hex == command.public_key_hex
            && self.host_node_id == command.host_node_id
            && self.signing_authority == command.signing_authority
    }
}

fn registration_result(
    outcome: RegistrationOutcome,
    record: &HostedAgentInventoryRecord,
) -> HostedAgentRegistrationResult {
    HostedAgentRegistrationResult {
        outcome,
        assignment: HostedAgentGenerationAssignment {
            agent_ura: record.agent_ura.clone(),
            host_device_ura: record
                .signing_authority
                .authority_ura(&record.agent_ura)
                .to_string(),
            incarnation_id: record.incarnation_id.clone(),
            generation: record.generation,
        },
        record: record.clone(),
    }
}

pub(crate) fn active_inventory() -> anyhow::Result<Vec<HostedAgentInventoryRecord>> {
    read().map(|file| {
        file.records
            .into_iter()
            .filter(|record| record.lifecycle == InventoryLifecycle::Active)
            .collect()
    })
}

pub(crate) fn active_inventory_record(
    agent_ura: &str,
) -> anyhow::Result<Option<HostedAgentInventoryRecord>> {
    read().map(|file| {
        file.records.into_iter().find(|record| {
            record.agent_ura == agent_ura && record.lifecycle == InventoryLifecycle::Active
        })
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
        let current = file
            .records
            .iter()
            .find(|record| record.agent_ura == command.agent_ura)
            .ok_or_else(|| {
                anyhow::anyhow!("revoke target is absent from durable hosted Agent inventory")
            })?;
        if command.generation > current.generation {
            anyhow::bail!("revoke generation is ahead of durable registration");
        }
        if command.generation < current.generation {
            let outcome = FederationRevokeOutcome {
                disposition: FederationRevokeDisposition::SupersededByNewIncarnation,
                was_active: observed_was_active,
                presence_session_id,
            };
            file.revoke_transactions.push(FederationRevokeTransaction {
                command: command.clone(),
                command_digest: digest,
                max_delivery_fence: delivery_fence,
                state: FederationRevokeState::Applied {
                    outcome: outcome.clone(),
                    applied_at_unix_ms: now_unix_ms,
                },
            });
            file.revoke_transactions
                .sort_by(|a, b| a.command.transaction_id.cmp(&b.command.transaction_id));
            return Ok(PrepareRevokeOutcome::Applied(outcome));
        }
        if current.signing_authority.authority_ura(&current.agent_ura) != command.authority_ura {
            anyhow::bail!("revoke authority does not control the current hosted Agent incarnation");
        }
        file.revoke_transactions.push(FederationRevokeTransaction {
            command: command.clone(),
            command_digest: digest,
            max_delivery_fence: delivery_fence,
            state: FederationRevokeState::Prepared {
                incarnation_id: current.incarnation_id.clone(),
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
        let (incarnation_id, observed_was_active, presence_session_id) = match &transaction.state {
            FederationRevokeState::Prepared {
                incarnation_id,
                observed_was_active,
                presence_session_id,
                ..
            } => (
                incarnation_id.clone(),
                *observed_was_active,
                *presence_session_id,
            ),
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
        let disposition = if record.generation > command.generation {
            FederationRevokeDisposition::SupersededByNewIncarnation
        } else if record.generation < command.generation {
            anyhow::bail!("revoke generation is ahead of durable registration");
        } else if record.incarnation_id != incarnation_id {
            anyhow::bail!("revoke generation is bound to a different hosted Agent incarnation");
        } else if record.signing_authority.authority_ura(&record.agent_ura) != command.authority_ura
        {
            anyhow::bail!("revoke authority does not control the bound hosted Agent incarnation");
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
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| anyhow::anyhow!("parse {}: {error}", path.display()))?;
            let schema_version = value
                .get("schema_version")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    anyhow::anyhow!("Hub hosted-Agent inventory is missing schema_version")
                })?;
            if schema_version == u64::from(PRE_INCARNATION_SCHEMA_VERSION) {
                let legacy: PreIncarnationInventoryFile = serde_json::from_value(value)
                    .map_err(|error| anyhow::anyhow!("parse {}: {error}", path.display()))?;
                let (file, retired_active_records) = migrate_pre_incarnation_inventory(legacy)?;
                save_unlocked(path, &file).context("persist hosted-Agent inventory schema 2")?;
                let retired_active_records = retired_active_records.to_string();
                crate::op_event!(
                    component = hosted_agent_inventory,
                    kind = schema_migrated,
                    from = "1",
                    to = "2",
                    retired_active_records = retired_active_records.as_str(),
                    message = "legacy active rows became generation fences pending Device re-registration",
                );
                return Ok(file);
            }
            if schema_version != u64::from(SCHEMA_VERSION) {
                anyhow::bail!(
                    "unsupported Hub hosted-Agent inventory schema {version}; expected {SCHEMA_VERSION}",
                    version = schema_version
                );
            }
            let file: HubHostedAgentInventoryFile = serde_json::from_value(value)
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

fn migrate_pre_incarnation_inventory(
    legacy: PreIncarnationInventoryFile,
) -> anyhow::Result<(HubHostedAgentInventoryFile, usize)> {
    if legacy.schema_version != PRE_INCARNATION_SCHEMA_VERSION {
        anyhow::bail!("pre-incarnation inventory migration received the wrong schema");
    }

    // Schema 1 keyed rows by (Agent, signing authority), while schema 2 owns
    // one realm-global generation stream per Agent. Picking one of two legacy
    // authorities would silently invent custody, so ambiguous files require an
    // explicit operator decision and remain byte-for-byte untouched.
    let mut agent_uras = std::collections::BTreeSet::new();
    for record in &legacy.records {
        if !agent_uras.insert(record.agent_ura.as_str()) {
            anyhow::bail!(
                "cannot migrate Hub hosted-Agent inventory schema 1: Agent `{}` has multiple authority rows",
                record.agent_ura
            );
        }
    }

    let retired_active_records = legacy
        .records
        .iter()
        .filter(|record| record.lifecycle == InventoryLifecycle::Active)
        .count();
    let records = legacy
        .records
        .into_iter()
        .map(|record| {
            let incarnation_id = migrated_incarnation_id(&record)?;
            Ok(HostedAgentInventoryRecord {
                agent_ura: record.agent_ura,
                incarnation_id,
                generation: record.generation,
                public_key_hex: record.public_key_hex,
                host_node_id: record.host_node_id,
                signing_authority: record.signing_authority,
                // Schema 1 never persisted a Device-provided incarnation
                // token. Keeping an active row under a synthetic token would
                // make the next legitimate registration look like a takeover.
                // Retire the row as a generation fence; the owning Device must
                // re-register its durable token to advance the generation.
                lifecycle: InventoryLifecycle::Retired,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let revoke_transactions = legacy
        .revoke_transactions
        .into_iter()
        .map(|transaction| {
            let state = match transaction.state {
                PreIncarnationRevokeState::Prepared {
                    observed_was_active,
                    presence_session_id,
                    prepared_at_unix_ms,
                } => {
                    let incarnation_id = records
                        .iter()
                        .find(|record| {
                            record.agent_ura == transaction.command.agent_ura
                                && record.generation == transaction.command.generation
                                && record
                                    .signing_authority
                                    .authority_ura(&record.agent_ura)
                                    == transaction.command.authority_ura
                        })
                        .map(|record| record.incarnation_id.clone())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "cannot migrate prepared federation revoke `{}`: bound inventory generation is absent",
                                transaction.command.transaction_id
                            )
                        })?;
                    FederationRevokeState::Prepared {
                        incarnation_id,
                        observed_was_active,
                        presence_session_id,
                        prepared_at_unix_ms,
                    }
                }
                PreIncarnationRevokeState::Applied {
                    outcome,
                    applied_at_unix_ms,
                } => FederationRevokeState::Applied {
                    outcome,
                    applied_at_unix_ms,
                },
            };
            Ok(FederationRevokeTransaction {
                command: transaction.command,
                command_digest: transaction.command_digest,
                max_delivery_fence: transaction.max_delivery_fence,
                state,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let file = HubHostedAgentInventoryFile {
        schema_version: SCHEMA_VERSION,
        records,
        revoke_transactions,
        projection_deliveries: legacy.projection_deliveries,
    };
    file.validate()?;
    Ok((file, retired_active_records))
}

fn migrated_incarnation_id(
    record: &PreIncarnationInventoryRecord,
) -> anyhow::Result<HostedAgentIncarnationId> {
    let mut hash = Sha256::new();
    hash.update(b"easynet.hosted-agent.pre-incarnation-migration\0");
    hash_field(&mut hash, &record.agent_ura);
    hash_field(&mut hash, &record.generation.to_string());
    hash_field(&mut hash, &record.public_key_hex);
    hash_field(&mut hash, record.host_node_id.as_deref().unwrap_or(""));
    hash_field(
        &mut hash,
        record.signing_authority.authority_ura(&record.agent_ura),
    );
    let digest = hex::encode(hash.finalize());
    HostedAgentIncarnationId::parse(&digest[..32]).map_err(anyhow::Error::msg)
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

fn inventory_key(record: &HostedAgentInventoryRecord) -> &str {
    record.agent_ura.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incarnation(suffix: u8) -> HostedAgentIncarnationId {
        HostedAgentIncarnationId::parse(format!("{suffix:032x}")).unwrap()
    }

    fn registration(suffix: u8) -> HostedAgentRegistrationCommand {
        registration_for(suffix, "dev-1")
    }

    fn registration_for(suffix: u8, host_node_id: &str) -> HostedAgentRegistrationCommand {
        HostedAgentRegistrationCommand {
            agent_ura: "easynet:///r/test/agent/alice.worker".into(),
            incarnation_id: incarnation(suffix),
            public_key_hex: String::new(),
            host_node_id: Some(host_node_id.into()),
            signing_authority: DurableSigningAuthority::HostedBy {
                host_ura: crate::core::ura::device_ura("test", host_node_id),
            },
        }
    }

    fn assigned_record(
        generation: u64,
        suffix: u8,
        host_node_id: &str,
        lifecycle: InventoryLifecycle,
    ) -> HostedAgentInventoryRecord {
        let command = registration_for(suffix, host_node_id);
        let mut record = command.into_record(generation);
        record.lifecycle = lifecycle;
        record
    }

    fn record(generation: u64) -> HostedAgentInventoryRecord {
        assigned_record(
            generation,
            generation as u8,
            "dev-1",
            InventoryLifecycle::Active,
        )
    }

    fn register(suffix: u8) -> HostedAgentRegistrationResult {
        register_agent(registration(suffix)).unwrap()
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
    fn hub_allocates_first_generation_and_replays_same_incarnation_idempotently() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let first = register_agent(registration(1)).unwrap();
        let replay = register_agent(registration(1)).unwrap();

        assert_eq!(first.outcome, RegistrationOutcome::Inserted);
        assert_eq!(first.assignment.generation, 1);
        assert_eq!(replay.outcome, RegistrationOutcome::Idempotent);
        assert_eq!(replay.assignment, first.assignment);
        assert_eq!(active_inventory().unwrap(), vec![first.record]);
    }

    #[test]
    fn active_incarnation_rejects_new_token_and_changed_facts_without_mutation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let first = register_agent(registration(1)).unwrap().record;
        let before = std::fs::read(path()).unwrap();

        assert!(register_agent(registration(2)).is_err());
        let mut changed_facts = registration(1);
        changed_facts.public_key_hex = "01".into();
        assert!(register_agent(changed_facts).is_err());

        assert_eq!(std::fs::read(path()).unwrap(), before);
        assert_eq!(active_inventory().unwrap(), vec![first]);
    }

    #[test]
    fn retired_token_cannot_reactivate_but_new_token_receives_exact_next_generation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register(1);
        let revoke = command(1);
        prepare_revoke(&revoke, 1, false, None, 1).unwrap();
        apply_prepared_revoke(&revoke.transaction_id, 1, 2).unwrap();

        let retired_bytes = std::fs::read(path()).unwrap();
        assert!(register_agent(registration(1)).is_err());
        assert_eq!(std::fs::read(path()).unwrap(), retired_bytes);
        assert!(active_inventory().unwrap().is_empty());

        let next = register_agent(registration_for(2, "dev-2")).unwrap();
        assert_eq!(next.outcome, RegistrationOutcome::AdvancedGeneration);
        assert_eq!(next.assignment.generation, 2);
        assert_eq!(next.assignment.incarnation_id, incarnation(2));
        assert_eq!(active_inventory().unwrap(), vec![next.record]);
    }

    #[test]
    fn concurrent_same_incarnation_registration_allocates_once() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers = (0..8)
            .map(|_| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    register_agent(registration(1))
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            results
                .iter()
                .filter(|result| result.outcome == RegistrationOutcome::Inserted)
                .count(),
            1
        );
        assert!(results
            .iter()
            .all(|result| result.assignment.generation == 1));
        assert_eq!(active_inventory().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_distinct_incarnations_have_one_winner() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers = (1..=8)
            .map(|suffix| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    register_agent(registration(suffix))
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let inventory = active_inventory().unwrap();
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].generation, 1);
    }

    #[test]
    fn generation_overflow_rejects_without_mutating_retired_record() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let retired = assigned_record(u64::MAX, 1, "dev-1", InventoryLifecycle::Retired);
        update(|file| {
            file.records.push(retired.clone());
            Ok(())
        })
        .unwrap();
        let before = std::fs::read(path()).unwrap();

        let error = register_agent(registration_for(2, "dev-2")).unwrap_err();

        assert!(error.to_string().contains("generation exhausted"));
        assert_eq!(std::fs::read(path()).unwrap(), before);
        assert_eq!(read().unwrap().records, vec![retired]);
        assert!(active_inventory().unwrap().is_empty());
    }

    #[test]
    fn prepared_revoke_is_bound_to_the_observed_incarnation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register(1);
        let revoke = command(1);
        prepare_revoke(&revoke, 1, false, None, 1).unwrap();
        update(|file| {
            file.records[0].incarnation_id = incarnation(2);
            Ok(())
        })
        .unwrap();

        let error = apply_prepared_revoke(&revoke.transaction_id, 1, 2).unwrap_err();

        assert!(error
            .to_string()
            .contains("different hosted Agent incarnation"));
        assert_eq!(
            active_inventory().unwrap()[0].incarnation_id,
            incarnation(2)
        );
        assert_eq!(
            recover_prepared_revokes().unwrap_err().to_string(),
            error.to_string()
        );
    }

    #[test]
    fn pre_incarnation_schema_is_atomically_migrated_to_a_retired_generation_fence() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let data_path = path();
        std::fs::create_dir_all(data_path.parent().unwrap()).unwrap();
        std::fs::write(
            &data_path,
            br#"{
              "schema_version": 1,
              "records": [{
                "agent_ura": "easynet:///r/test/agent/alice.worker",
                "generation": 7,
                "public_key_hex": "",
                "host_node_id": "dev-1",
                "signing_authority": {
                  "kind": "hosted_by",
                  "host_ura": "easynet:///r/test/device/dev-1"
                },
                "lifecycle": "active"
              }],
              "revoke_transactions": [],
              "projection_deliveries": []
            }"#,
        )
        .unwrap();

        let migrated = read().unwrap();

        assert_eq!(migrated.schema_version, 2);
        assert_eq!(migrated.records.len(), 1);
        assert_eq!(migrated.records[0].generation, 7);
        assert_eq!(migrated.records[0].lifecycle, InventoryLifecycle::Retired);
        HostedAgentIncarnationId::parse(migrated.records[0].incarnation_id.to_string()).unwrap();
        assert!(active_inventory().unwrap().is_empty());
        let persisted = std::fs::read_to_string(data_path).unwrap();
        assert!(persisted.contains("\"schema_version\": 2"));
        assert!(persisted.contains("\"incarnation_id\""));
        assert!(persisted.contains("\"lifecycle\": \"retired\""));
    }

    #[test]
    fn ambiguous_pre_incarnation_authorities_fail_without_mutation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let data_path = path();
        std::fs::create_dir_all(data_path.parent().unwrap()).unwrap();
        let fixture = br#"{
          "schema_version": 1,
          "records": [
            {
              "agent_ura": "easynet:///r/test/agent/alice.worker",
              "generation": 1,
              "public_key_hex": "",
              "host_node_id": "dev-1",
              "signing_authority": {"kind":"hosted_by","host_ura":"easynet:///r/test/device/dev-1"},
              "lifecycle": "active"
            },
            {
              "agent_ura": "easynet:///r/test/agent/alice.worker",
              "generation": 1,
              "public_key_hex": "",
              "host_node_id": "dev-2",
              "signing_authority": {"kind":"hosted_by","host_ura":"easynet:///r/test/device/dev-2"},
              "lifecycle": "active"
            }
          ],
          "revoke_transactions": []
        }"#;
        std::fs::write(&data_path, fixture).unwrap();

        let error = read().unwrap_err();

        assert!(error.to_string().contains("multiple authority rows"));
        assert_eq!(std::fs::read(data_path).unwrap(), fixture);
    }

    #[test]
    fn pre_incarnation_prepared_revoke_is_bound_to_the_migrated_generation_fence() {
        let command = FederationRevokeCommand {
            generation: 7,
            ..command(7)
        };
        let legacy = PreIncarnationInventoryFile {
            schema_version: PRE_INCARNATION_SCHEMA_VERSION,
            records: vec![PreIncarnationInventoryRecord {
                agent_ura: command.agent_ura.clone(),
                generation: command.generation,
                public_key_hex: String::new(),
                host_node_id: Some("dev-1".into()),
                signing_authority: DurableSigningAuthority::HostedBy {
                    host_ura: command.authority_ura.clone(),
                },
                lifecycle: InventoryLifecycle::Active,
            }],
            revoke_transactions: vec![PreIncarnationRevokeTransaction {
                command: command.clone(),
                command_digest: command.canonical_digest().unwrap(),
                max_delivery_fence: 3,
                state: PreIncarnationRevokeState::Prepared {
                    observed_was_active: true,
                    presence_session_id: Some(9),
                    prepared_at_unix_ms: 11,
                },
            }],
            projection_deliveries: Vec::new(),
        };

        let (migrated, retired_active_records) = migrate_pre_incarnation_inventory(legacy).unwrap();

        assert_eq!(retired_active_records, 1);
        assert_eq!(migrated.records[0].lifecycle, InventoryLifecycle::Retired);
        match &migrated.revoke_transactions[0].state {
            FederationRevokeState::Prepared { incarnation_id, .. } => {
                assert_eq!(incarnation_id, &migrated.records[0].incarnation_id);
            }
            FederationRevokeState::Applied { .. } => panic!("prepared revoke changed state"),
        }
        migrated.validate().unwrap();
    }

    #[test]
    fn prepared_recovery_applies_once_and_persists_exact_outcome() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register(1);
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
        register(1);
        let command = command(1);
        prepare_revoke(&command, 1, false, None, 1).unwrap();
        let mut changed = command;
        changed.reason = "agent.stop".into();
        assert!(prepare_revoke(&changed, 2, false, None, 2).is_err());
    }

    #[test]
    fn delayed_old_transaction_cannot_retire_new_incarnation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register(1);
        let old = command(1);
        prepare_revoke(&old, 1, true, None, 1).unwrap();
        let mut winning_revoke = command(1);
        winning_revoke.transaction_id = "11111111111111111111111111111111".into();
        prepare_revoke(&winning_revoke, 1, true, None, 2).unwrap();
        apply_prepared_revoke(&winning_revoke.transaction_id, 1, 3).unwrap();
        register_agent(registration_for(2, "dev-2")).unwrap();
        let (outcome, _) = apply_prepared_revoke(&old.transaction_id, 1, 4).unwrap();
        assert_eq!(
            outcome.disposition,
            FederationRevokeDisposition::SupersededByNewIncarnation
        );
        assert_eq!(
            active_inventory().unwrap(),
            vec![assigned_record(2, 2, "dev-2", InventoryLifecycle::Active)]
        );
    }

    #[test]
    fn invalid_new_revoke_is_not_persisted_as_unrecoverable_prepared_work() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register(1);

        let mut ahead = command(2);
        ahead.transaction_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        assert!(prepare_revoke(&ahead, 1, false, None, 1).is_err());

        let mut wrong_authority = command(1);
        wrong_authority.transaction_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        wrong_authority.authority_ura = "easynet:///r/test/device/dev-2".into();
        assert!(prepare_revoke(&wrong_authority, 1, false, None, 1).is_err());

        let mut absent = command(1);
        absent.transaction_id = "cccccccccccccccccccccccccccccccc".into();
        absent.agent_ura = "easynet:///r/test/agent/alice.absent".into();
        absent.target_ura = absent.agent_ura.clone();
        assert!(prepare_revoke(&absent, 1, false, None, 1).is_err());

        assert!(recover_prepared_revokes().unwrap().is_empty());
        assert_eq!(active_inventory().unwrap(), vec![record(1)]);
    }

    #[test]
    fn new_old_generation_revoke_is_terminal_without_prepared_recovery() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register(1);
        let first = command(1);
        prepare_revoke(&first, 1, false, None, 1).unwrap();
        apply_prepared_revoke(&first.transaction_id, 1, 2).unwrap();
        register_agent(registration_for(2, "dev-2")).unwrap();
        let mut old = command(1);
        old.transaction_id = "22222222222222222222222222222222".into();

        let outcome = prepare_revoke(&old, 1, false, None, 1).unwrap();

        assert_eq!(
            outcome,
            PrepareRevokeOutcome::Applied(FederationRevokeOutcome {
                disposition: FederationRevokeDisposition::SupersededByNewIncarnation,
                was_active: false,
                presence_session_id: None,
            })
        );
        assert!(recover_prepared_revokes().unwrap().is_empty());
        assert_eq!(
            active_inventory().unwrap(),
            vec![assigned_record(2, 2, "dev-2", InventoryLifecycle::Active)]
        );
    }

    #[test]
    fn active_hosted_agent_requires_revoke_before_device_migration() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let first = assigned_record(1, 1, "dev-1", InventoryLifecycle::Active);

        assert_eq!(
            register_agent(registration_for(1, "dev-1"))
                .unwrap()
                .outcome,
            RegistrationOutcome::Inserted,
        );
        let error = register_agent(registration_for(2, "dev-2"))
            .expect_err("active host custody is exclusive");
        assert!(error
            .to_string()
            .contains("revoke it before registering another"));
        assert_eq!(active_inventory().unwrap(), vec![first]);
    }

    #[test]
    fn retired_hosted_agent_can_migrate_with_a_new_generation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register_agent(registration_for(1, "dev-1")).unwrap();
        let command = command(1);
        prepare_revoke(&command, 1, false, None, 1).unwrap();
        let (outcome, replayed) = apply_prepared_revoke(&command.transaction_id, 1, 2).unwrap();
        assert!(!replayed);
        assert_eq!(outcome.disposition, FederationRevokeDisposition::Retired);

        let migrated = assigned_record(2, 2, "dev-2", InventoryLifecycle::Active);
        assert_eq!(
            register_agent(registration_for(2, "dev-2"))
                .unwrap()
                .outcome,
            RegistrationOutcome::AdvancedGeneration,
        );
        assert_eq!(active_inventory().unwrap(), vec![migrated]);
    }

    #[test]
    fn takeover_fence_rejects_delayed_old_worker() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        register(1);
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
        register(1);
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
