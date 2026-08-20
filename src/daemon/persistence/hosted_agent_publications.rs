//! Device-side hosted-Agent publication state machine.
//!
//! This store is the crash boundary between local Agent lifecycle and Hub
//! directory generation assignment. A fresh incarnation id is persisted in
//! `RegistrationPending` before any network call. Ability projection is legal
//! only from `Assigned`, after the exact Hub assignment has been verified and
//! durably recorded. Frontend execution readiness begins only at `Published`,
//! after the Hub acknowledges the exact ability projection.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::daemon::federation::hosted_agent_publication::{
    HostedAgentGenerationAssignment, HostedAgentIncarnationId,
};

use super::config::{self, WritePermissions};
use super::file_lock::ExclusiveFileLock;

const SCHEMA_VERSION: u32 = 3;
const FILE_NAME: &str = "device-hosted-agent-publications.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HostedAgentDevicePublicationState {
    RegistrationPending {
        incarnation_id: HostedAgentIncarnationId,
    },
    Assigned {
        incarnation_id: HostedAgentIncarnationId,
        generation: u64,
    },
    Publishing {
        incarnation_id: HostedAgentIncarnationId,
        generation: u64,
        catalog_epoch: u64,
        projection_revision: u64,
        projection_digest: String,
    },
    Published {
        incarnation_id: HostedAgentIncarnationId,
        generation: u64,
        catalog_epoch: u64,
        projection_revision: u64,
        projection_digest: String,
    },
    Retired {
        incarnation_id: HostedAgentIncarnationId,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostedAgentDevicePublicationRecord {
    pub(crate) agent_ura: String,
    pub(crate) host_device_ura: String,
    pub(crate) desired_catalog_epoch: u64,
    pub(crate) lifecycle: HostedAgentDevicePublicationState,
    pub(crate) updated_at_unix_ms: u64,
}

impl HostedAgentDevicePublicationRecord {
    fn validate(&self) -> anyhow::Result<()> {
        super::owner_projections::validate_owner_projection_host_binding(
            &self.agent_ura,
            &self.host_device_ura,
        )
        .map_err(anyhow::Error::msg)?;
        let parsed = crate::core::ura::parse_ura(&self.agent_ura)?;
        if parsed.kind != crate::core::ura::URAKind::Agent || parsed.agent_ids().is_none() {
            anyhow::bail!("hosted publication record requires a user-owned Agent URA");
        }
        if self.desired_catalog_epoch == 0 {
            anyhow::bail!("hosted publication desired catalog epoch must be nonzero");
        }
        match &self.lifecycle {
            HostedAgentDevicePublicationState::RegistrationPending { .. } => {}
            HostedAgentDevicePublicationState::Assigned { generation, .. }
            | HostedAgentDevicePublicationState::Publishing { generation, .. }
            | HostedAgentDevicePublicationState::Published { generation, .. }
            | HostedAgentDevicePublicationState::Retired { generation, .. }
                if *generation == 0 =>
            {
                anyhow::bail!("hosted publication assignment generation must be nonzero")
            }
            HostedAgentDevicePublicationState::Publishing {
                catalog_epoch,
                projection_revision,
                projection_digest,
                ..
            }
            | HostedAgentDevicePublicationState::Published {
                catalog_epoch,
                projection_revision,
                projection_digest,
                ..
            } if *catalog_epoch == 0
                || *catalog_epoch != self.desired_catalog_epoch
                || *projection_revision == 0
                || projection_digest.trim().is_empty() =>
            {
                anyhow::bail!("published hosted Agent projection proof is incomplete")
            }
            HostedAgentDevicePublicationState::Assigned { .. }
            | HostedAgentDevicePublicationState::Publishing { .. }
            | HostedAgentDevicePublicationState::Published { .. }
            | HostedAgentDevicePublicationState::Retired { .. } => {}
        }
        Ok(())
    }

    #[must_use]
    pub(crate) fn incarnation_id(&self) -> &HostedAgentIncarnationId {
        match &self.lifecycle {
            HostedAgentDevicePublicationState::RegistrationPending { incarnation_id }
            | HostedAgentDevicePublicationState::Assigned { incarnation_id, .. }
            | HostedAgentDevicePublicationState::Publishing { incarnation_id, .. }
            | HostedAgentDevicePublicationState::Published { incarnation_id, .. }
            | HostedAgentDevicePublicationState::Retired { incarnation_id, .. } => incarnation_id,
        }
    }

    #[must_use]
    pub(crate) fn assigned_generation(&self) -> Option<u64> {
        match self.lifecycle {
            HostedAgentDevicePublicationState::Assigned { generation, .. }
            | HostedAgentDevicePublicationState::Publishing { generation, .. }
            | HostedAgentDevicePublicationState::Published { generation, .. }
            | HostedAgentDevicePublicationState::Retired { generation, .. } => Some(generation),
            HostedAgentDevicePublicationState::RegistrationPending { .. } => None,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn is_published(&self) -> bool {
        matches!(
            self.lifecycle,
            HostedAgentDevicePublicationState::Published { .. }
        )
    }

    #[must_use]
    pub(crate) fn publication_state(&self) -> &'static str {
        match self.lifecycle {
            HostedAgentDevicePublicationState::RegistrationPending { .. } => "pending",
            HostedAgentDevicePublicationState::Assigned { .. } => "assigned",
            HostedAgentDevicePublicationState::Publishing { .. } => "assigned",
            HostedAgentDevicePublicationState::Published { .. } => "published",
            HostedAgentDevicePublicationState::Retired { .. } => "retired",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostedAgentDevicePublicationFile {
    schema_version: u32,
    catalog_epoch: u64,
    records: Vec<HostedAgentDevicePublicationRecord>,
}

impl Default for HostedAgentDevicePublicationFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            catalog_epoch: 1,
            records: Vec::new(),
        }
    }
}

impl HostedAgentDevicePublicationFile {
    fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            anyhow::bail!("unsupported Device hosted-Agent publication schema")
        }
        if self.catalog_epoch == 0 {
            anyhow::bail!("hosted publication catalog epoch must be nonzero")
        }
        let mut agents = std::collections::BTreeSet::new();
        for record in &self.records {
            record.validate()?;
            if !agents.insert(record.agent_ura.as_str()) {
                anyhow::bail!("duplicate Device hosted-Agent publication record")
            }
        }
        Ok(())
    }
}

/// Begin or resume one registration attempt. `Assigned`/`Published` are returned unchanged so
/// reconnect publication reuses the same Hub incarnation. Only `Retired`
/// creates a new token.
pub(crate) fn begin_registration(
    agent_ura: &str,
    host_device_ura: &str,
    now_unix_ms: u64,
) -> anyhow::Result<HostedAgentDevicePublicationRecord> {
    update(|file| {
        if let Some(current) = file
            .records
            .iter_mut()
            .find(|record| record.agent_ura == agent_ura)
        {
            if current.host_device_ura != host_device_ura {
                anyhow::bail!(
                    "hosted Agent publication state belongs to a different Device identity"
                );
            }
            if matches!(
                current.lifecycle,
                HostedAgentDevicePublicationState::Retired { .. }
            ) {
                current.lifecycle = HostedAgentDevicePublicationState::RegistrationPending {
                    incarnation_id: HostedAgentIncarnationId::fresh(),
                };
                current.updated_at_unix_ms = now_unix_ms;
            }
            return Ok(current.clone());
        }
        let record = HostedAgentDevicePublicationRecord {
            agent_ura: agent_ura.to_string(),
            host_device_ura: host_device_ura.to_string(),
            desired_catalog_epoch: file.catalog_epoch,
            lifecycle: HostedAgentDevicePublicationState::RegistrationPending {
                incarnation_id: HostedAgentIncarnationId::fresh(),
            },
            updated_at_unix_ms: now_unix_ms,
        };
        record.validate()?;
        file.records.push(record.clone());
        file.records
            .sort_by(|left, right| left.agent_ura.cmp(&right.agent_ura));
        Ok(record)
    })
}

/// Durably fence every locally hosted Agent before an asynchronous catalog
/// republish is scheduled. The epoch is the catalog-commit linearization
/// point: any projection captured before it can no longer become Published.
/// `current_agent_uras` also materializes an intent for a newly committed
/// Agent so a stale pre-commit plan cannot create an unfenced record later.
pub(crate) fn fence_catalog_commit<'a>(
    host_device_ura: &str,
    current_agent_uras: impl IntoIterator<Item = &'a str>,
    now_unix_ms: u64,
) -> anyhow::Result<u64> {
    update(|file| {
        file.catalog_epoch = file
            .catalog_epoch
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("hosted publication catalog epoch exhausted"))?;
        let epoch = file.catalog_epoch;
        let mut current = current_agent_uras
            .into_iter()
            .map(str::to_string)
            .collect::<std::collections::BTreeSet<_>>();
        let affected = current.clone();

        for record in file.records.iter_mut().filter(|record| {
            record.host_device_ura == host_device_ura && affected.contains(&record.agent_ura)
        }) {
            current.remove(&record.agent_ura);
            advance_desired_catalog_epoch(record, epoch, now_unix_ms);
        }
        for agent_ura in current {
            let record = HostedAgentDevicePublicationRecord {
                agent_ura,
                host_device_ura: host_device_ura.to_string(),
                desired_catalog_epoch: epoch,
                lifecycle: HostedAgentDevicePublicationState::RegistrationPending {
                    incarnation_id: HostedAgentIncarnationId::fresh(),
                },
                updated_at_unix_ms: now_unix_ms,
            };
            record.validate()?;
            file.records.push(record);
        }
        file.records
            .sort_by(|left, right| left.agent_ura.cmp(&right.agent_ura));
        Ok(epoch)
    })
}

fn advance_desired_catalog_epoch(
    record: &mut HostedAgentDevicePublicationRecord,
    epoch: u64,
    now_unix_ms: u64,
) {
    if epoch <= record.desired_catalog_epoch {
        return;
    }
    record.desired_catalog_epoch = epoch;
    record.lifecycle = match &record.lifecycle {
        HostedAgentDevicePublicationState::Publishing {
            incarnation_id,
            generation,
            ..
        }
        | HostedAgentDevicePublicationState::Published {
            incarnation_id,
            generation,
            ..
        } => HostedAgentDevicePublicationState::Assigned {
            incarnation_id: incarnation_id.clone(),
            generation: *generation,
        },
        lifecycle => lifecycle.clone(),
    };
    record.updated_at_unix_ms = now_unix_ms;
}

/// Ensure a captured catalog owner has a durable intent and return the exact
/// epoch that the capture must carry through stage and acknowledgement.
pub(crate) fn catalog_epoch_for_plan(
    agent_ura: &str,
    host_device_ura: &str,
    now_unix_ms: u64,
) -> anyhow::Result<u64> {
    begin_registration(agent_ura, host_device_ura, now_unix_ms)
        .map(|record| record.desired_catalog_epoch)
}

pub(crate) fn bind_assignment(
    assignment: &HostedAgentGenerationAssignment,
    now_unix_ms: u64,
) -> anyhow::Result<HostedAgentDevicePublicationRecord> {
    assignment.validate().map_err(anyhow::Error::msg)?;
    update(|file| {
        let current = file
            .records
            .iter_mut()
            .find(|record| record.agent_ura == assignment.agent_ura)
            .ok_or_else(|| anyhow::anyhow!("hosted Agent assignment has no pending local plan"))?;
        if current.host_device_ura != assignment.host_device_ura
            || current.incarnation_id() != &assignment.incarnation_id
        {
            anyhow::bail!("Hub assignment does not match the pending hosted Agent tuple");
        }
        match current.lifecycle {
            HostedAgentDevicePublicationState::RegistrationPending { .. } => {
                current.lifecycle = HostedAgentDevicePublicationState::Assigned {
                    incarnation_id: assignment.incarnation_id.clone(),
                    generation: assignment.generation,
                };
                current.updated_at_unix_ms = now_unix_ms;
            }
            HostedAgentDevicePublicationState::Assigned { generation, .. }
            | HostedAgentDevicePublicationState::Publishing { generation, .. }
            | HostedAgentDevicePublicationState::Published { generation, .. }
                if generation == assignment.generation => {}
            HostedAgentDevicePublicationState::Assigned { .. }
            | HostedAgentDevicePublicationState::Publishing { .. }
            | HostedAgentDevicePublicationState::Published { .. } => {
                anyhow::bail!("Hub assignment conflicts with the local publication generation")
            }
            HostedAgentDevicePublicationState::Retired { .. } => {
                anyhow::bail!("retired hosted Agent incarnation cannot become active again")
            }
        }
        Ok(current.clone())
    })
}

/// Fence one outbound complete-set projection before network I/O. A newer
/// staged proof supersedes an older in-flight proof; the exact proof carried
/// here prevents the older acknowledgement from reactivating stale state.
pub(crate) fn stage_projection(
    assignment: &HostedAgentGenerationAssignment,
    catalog_epoch: u64,
    projection_revision: u64,
    projection_digest: &str,
    now_unix_ms: u64,
) -> anyhow::Result<()> {
    assignment.validate().map_err(anyhow::Error::msg)?;
    let projection_digest = projection_digest.trim();
    if catalog_epoch == 0 || projection_revision == 0 || projection_digest.is_empty() {
        anyhow::bail!("staged hosted Agent projection proof is incomplete");
    }
    update(|file| {
        let current = file
            .records
            .iter_mut()
            .find(|record| record.agent_ura == assignment.agent_ura)
            .ok_or_else(|| anyhow::anyhow!("hosted Agent projection has no assigned local plan"))?;
        if current.host_device_ura != assignment.host_device_ura
            || current.incarnation_id() != &assignment.incarnation_id
            || current.assigned_generation() != Some(assignment.generation)
        {
            anyhow::bail!("hosted Agent projection does not match the assigned tuple");
        }
        if current.desired_catalog_epoch != catalog_epoch {
            anyhow::bail!("hosted Agent projection was captured from a stale catalog epoch");
        }
        match &current.lifecycle {
            HostedAgentDevicePublicationState::Published {
                catalog_epoch: current_epoch,
                projection_revision: current_revision,
                projection_digest: current_digest,
                ..
            } if *current_epoch == catalog_epoch
                && *current_revision == projection_revision
                && current_digest == projection_digest =>
            {
                return Ok(())
            }
            HostedAgentDevicePublicationState::Publishing {
                catalog_epoch: current_epoch,
                projection_revision: current_revision,
                projection_digest: current_digest,
                ..
            }
            | HostedAgentDevicePublicationState::Published {
                catalog_epoch: current_epoch,
                projection_revision: current_revision,
                projection_digest: current_digest,
                ..
            } if *current_epoch > catalog_epoch
                || (*current_epoch == catalog_epoch && *current_revision > projection_revision)
                || (*current_revision == projection_revision
                    && current_digest != projection_digest) =>
            {
                anyhow::bail!("refuse to supersede a newer or conflicting staged projection")
            }
            HostedAgentDevicePublicationState::Retired { .. }
            | HostedAgentDevicePublicationState::RegistrationPending { .. } => {
                anyhow::bail!("hosted Agent projection requires an assigned live incarnation")
            }
            _ => {}
        }
        current.lifecycle = HostedAgentDevicePublicationState::Publishing {
            incarnation_id: assignment.incarnation_id.clone(),
            generation: assignment.generation,
            catalog_epoch,
            projection_revision,
            projection_digest: projection_digest.to_string(),
        };
        current.updated_at_unix_ms = now_unix_ms;
        Ok(())
    })
}

/// Commit executable readiness only after the Hub acknowledges the exact
/// complete-set ability projection. Replays of the same proof are idempotent;
/// a different proof must start from a newer assigned generation/revision.
pub(crate) fn mark_published(
    assignment: &HostedAgentGenerationAssignment,
    catalog_epoch: u64,
    projection_revision: u64,
    projection_digest: &str,
    now_unix_ms: u64,
) -> anyhow::Result<HostedAgentDevicePublicationRecord> {
    assignment.validate().map_err(anyhow::Error::msg)?;
    let projection_digest = projection_digest.trim();
    if catalog_epoch == 0 || projection_revision == 0 || projection_digest.is_empty() {
        anyhow::bail!("published hosted Agent projection proof is incomplete");
    }
    update(|file| {
        let current = file
            .records
            .iter_mut()
            .find(|record| record.agent_ura == assignment.agent_ura)
            .ok_or_else(|| {
                anyhow::anyhow!("hosted Agent publication has no assigned local plan")
            })?;
        if current.host_device_ura != assignment.host_device_ura
            || current.incarnation_id() != &assignment.incarnation_id
        {
            anyhow::bail!(
                "Hub projection acknowledgement does not match the assigned hosted Agent tuple"
            );
        }
        if current.desired_catalog_epoch != catalog_epoch {
            anyhow::bail!("Hub acknowledgement targets a stale catalog epoch");
        }
        match &current.lifecycle {
            HostedAgentDevicePublicationState::Publishing {
                generation,
                catalog_epoch: staged_epoch,
                projection_revision: staged_revision,
                projection_digest: staged_digest,
                ..
            } if *generation == assignment.generation
                && *staged_epoch == catalog_epoch
                && *staged_revision == projection_revision
                && staged_digest == projection_digest =>
            {
                current.lifecycle = HostedAgentDevicePublicationState::Published {
                    incarnation_id: assignment.incarnation_id.clone(),
                    generation: assignment.generation,
                    catalog_epoch,
                    projection_revision,
                    projection_digest: projection_digest.to_string(),
                };
                current.updated_at_unix_ms = now_unix_ms;
            }
            HostedAgentDevicePublicationState::Published {
                generation,
                catalog_epoch: current_epoch,
                projection_revision: current_revision,
                projection_digest: current_digest,
                ..
            } if *generation == assignment.generation
                && *current_epoch == catalog_epoch
                && *current_revision == projection_revision
                && current_digest == projection_digest => {}
            HostedAgentDevicePublicationState::Assigned { .. }
            | HostedAgentDevicePublicationState::Publishing { .. }
            | HostedAgentDevicePublicationState::Published { .. } => {
                anyhow::bail!(
                    "Hub projection acknowledgement conflicts with local publication state"
                )
            }
            HostedAgentDevicePublicationState::RegistrationPending { .. } => {
                anyhow::bail!("hosted Agent cannot become published before assignment")
            }
            HostedAgentDevicePublicationState::Retired { .. } => {
                anyhow::bail!("retired hosted Agent incarnation cannot become published")
            }
        }
        Ok(current.clone())
    })
}

pub(crate) fn retire(
    agent_ura: &str,
    incarnation_id: &HostedAgentIncarnationId,
    generation: u64,
    now_unix_ms: u64,
) -> anyhow::Result<bool> {
    update(|file| {
        let Some(current) = file
            .records
            .iter_mut()
            .find(|record| record.agent_ura == agent_ura)
        else {
            return Ok(false);
        };
        match current.lifecycle.clone() {
            HostedAgentDevicePublicationState::Assigned {
                incarnation_id: current_id,
                generation: current_generation,
            }
            | HostedAgentDevicePublicationState::Publishing {
                incarnation_id: current_id,
                generation: current_generation,
                ..
            }
            | HostedAgentDevicePublicationState::Published {
                incarnation_id: current_id,
                generation: current_generation,
                ..
            } if &current_id == incarnation_id && current_generation == generation => {
                current.lifecycle = HostedAgentDevicePublicationState::Retired {
                    incarnation_id: current_id,
                    generation,
                };
                current.updated_at_unix_ms = now_unix_ms;
                Ok(true)
            }
            HostedAgentDevicePublicationState::Retired {
                incarnation_id: current_id,
                generation: current_generation,
            } if &current_id == incarnation_id && current_generation == generation => Ok(false),
            _ => anyhow::bail!("refuse to retire a different hosted Agent incarnation"),
        }
    })
}

/// Retire the locally active incarnation selected by a generation-fenced Hub
/// revoke acknowledgement. The token is resolved from durable state and then
/// rechecked by [`retire`], so a concurrent replacement cannot be retired.
pub(crate) fn retire_generation(
    agent_ura: &str,
    generation: u64,
    now_unix_ms: u64,
) -> anyhow::Result<bool> {
    let Some(record) = record_for(agent_ura)? else {
        return Ok(false);
    };
    retire(agent_ura, record.incarnation_id(), generation, now_unix_ms)
}

pub(crate) fn record_for(
    agent_ura: &str,
) -> anyhow::Result<Option<HostedAgentDevicePublicationRecord>> {
    read().map(|file| {
        file.records
            .into_iter()
            .find(|record| record.agent_ura == agent_ura)
    })
}

/// Exact executable-readiness proof shared by route preflight and Axon's
/// authoritative runtime admission. The caller must supply the host selected
/// by its local authority boundary; a record from another Device is never a
/// readiness proof for this runtime.
pub(crate) fn require_published_for_host(
    agent_ura: &str,
    expected_host_device_ura: &str,
) -> anyhow::Result<()> {
    let record = record_for(agent_ura)?.ok_or_else(|| {
        anyhow::anyhow!(
            "local hosted Agent `{agent_ura}` has no durable publication readiness proof"
        )
    })?;
    if record.host_device_ura != expected_host_device_ura {
        anyhow::bail!(
            "local hosted Agent `{agent_ura}` publication belongs to Device `{}`, expected `{expected_host_device_ura}`",
            record.host_device_ura
        );
    }
    if record.publication_state() != "published" {
        anyhow::bail!(
            "local hosted Agent `{agent_ura}` is not execution-ready (publication_state={})",
            record.publication_state()
        );
    }
    Ok(())
}

/// Live hosted-owner intents for one Device. Publication reconciliation unions
/// this set with the current non-empty catalog owners so deleting an Agent's
/// final ability still emits an empty complete-set projection instead of
/// leaving stale Hub rows behind.
pub(crate) fn live_agent_uras_for_host(host_device_ura: &str) -> anyhow::Result<Vec<String>> {
    read().map(|file| {
        file.records
            .into_iter()
            .filter(|record| {
                record.host_device_ura == host_device_ura
                    && !matches!(
                        record.lifecycle,
                        HostedAgentDevicePublicationState::Retired { .. }
                    )
            })
            .map(|record| record.agent_ura)
            .collect()
    })
}

/// Prove that an owner projection is fenced by the exact assignment already
/// committed on this Device. This is deliberately a read-only guard: callers
/// must bind the Hub response first, then construct/persist ability state.
pub(crate) fn require_active_assignment(
    assignment: &HostedAgentGenerationAssignment,
) -> anyhow::Result<HostedAgentDevicePublicationRecord> {
    assignment.validate().map_err(anyhow::Error::msg)?;
    let record = record_for(&assignment.agent_ura)?
        .ok_or_else(|| anyhow::anyhow!("hosted Agent has no local publication lifecycle"))?;
    if record.host_device_ura != assignment.host_device_ura {
        anyhow::bail!("hosted Agent assignment belongs to a different Device identity");
    }
    match &record.lifecycle {
        HostedAgentDevicePublicationState::Assigned {
            incarnation_id,
            generation,
        }
        | HostedAgentDevicePublicationState::Publishing {
            incarnation_id,
            generation,
            ..
        }
        | HostedAgentDevicePublicationState::Published {
            incarnation_id,
            generation,
            ..
        } if incarnation_id == &assignment.incarnation_id
            && *generation == assignment.generation =>
        {
            Ok(record)
        }
        _ => anyhow::bail!("hosted Agent projection requires the exact assigned Hub generation"),
    }
}

fn read() -> anyhow::Result<HostedAgentDevicePublicationFile> {
    let data_path = path();
    let _guard = ExclusiveFileLock::acquire_for_data_path(&data_path)?;
    load_unlocked(&data_path)
}

fn update<T>(
    mutate: impl FnOnce(&mut HostedAgentDevicePublicationFile) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let data_path = path();
    let _guard = ExclusiveFileLock::acquire_for_data_path(&data_path)?;
    let mut file = load_unlocked(&data_path)?;
    let result = mutate(&mut file)?;
    save_unlocked(&data_path, &file)?;
    Ok(result)
}

fn load_unlocked(path: &Path) -> anyhow::Result<HostedAgentDevicePublicationFile> {
    match fs::read(path) {
        Ok(bytes) => {
            let file: HostedAgentDevicePublicationFile = serde_json::from_slice(&bytes)
                .map_err(|error| anyhow::anyhow!("parse {}: {error}", path.display()))?;
            file.validate()?;
            Ok(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.into()),
    }
}

fn save_unlocked(path: &Path, file: &HostedAgentDevicePublicationFile) -> anyhow::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "easynet:///r/test/agent/alice.worker";
    const HOST: &str = "easynet:///r/test/device/dev-1";

    #[test]
    fn pending_token_is_durable_and_reused_until_assignment() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let first = begin_registration(AGENT, HOST, 1).unwrap();
        let retry = begin_registration(AGENT, HOST, 2).unwrap();
        assert_eq!(first.incarnation_id(), retry.incarnation_id());
        assert_eq!(retry.assigned_generation(), None);
        assert_eq!(record_for(AGENT).unwrap(), Some(retry));
    }

    #[test]
    fn assignment_tuple_is_exact_and_persisted_before_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let pending = begin_registration(AGENT, HOST, 1).unwrap();
        let assignment = HostedAgentGenerationAssignment {
            agent_ura: AGENT.into(),
            host_device_ura: HOST.into(),
            incarnation_id: pending.incarnation_id().clone(),
            generation: 7,
        };
        let assigned = bind_assignment(&assignment, 2).unwrap();
        assert!(!assigned.is_published());
        assert_eq!(assigned.publication_state(), "assigned");
        assert_eq!(assigned.assigned_generation(), Some(7));

        stage_projection(&assignment, 1, 3, "sha256:projection", 3).unwrap();
        let published = mark_published(&assignment, 1, 3, "sha256:projection", 4).unwrap();
        assert!(published.is_published());
        assert_eq!(published.publication_state(), "published");

        let mut wrong = assignment;
        wrong.host_device_ura = "easynet:///r/test/device/dev-2".into();
        assert!(bind_assignment(&wrong, 3).is_err());
        assert_eq!(record_for(AGENT).unwrap(), Some(published));
    }

    #[test]
    fn retired_incarnation_cannot_reactivate_and_next_begin_gets_new_token() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let pending = begin_registration(AGENT, HOST, 1).unwrap();
        let assignment = HostedAgentGenerationAssignment {
            agent_ura: AGENT.into(),
            host_device_ura: HOST.into(),
            incarnation_id: pending.incarnation_id().clone(),
            generation: 1,
        };
        bind_assignment(&assignment, 2).unwrap();
        assert!(retire(AGENT, &assignment.incarnation_id, 1, 3).unwrap());
        assert!(bind_assignment(&assignment, 4).is_err());

        let next = begin_registration(AGENT, HOST, 5).unwrap();
        assert_ne!(next.incarnation_id(), &assignment.incarnation_id);
        assert_eq!(next.assigned_generation(), None);
    }

    #[test]
    fn newer_staged_projection_fences_late_acknowledgement() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let pending = begin_registration(AGENT, HOST, 1).unwrap();
        let assignment = HostedAgentGenerationAssignment {
            agent_ura: AGENT.into(),
            host_device_ura: HOST.into(),
            incarnation_id: pending.incarnation_id().clone(),
            generation: 1,
        };
        bind_assignment(&assignment, 2).unwrap();
        stage_projection(&assignment, 1, 1, "sha256:first", 3).unwrap();
        stage_projection(&assignment, 1, 2, "sha256:second", 4).unwrap();

        assert!(mark_published(&assignment, 1, 1, "sha256:first", 5).is_err());
        let published = mark_published(&assignment, 1, 2, "sha256:second", 6).unwrap();
        assert!(published.is_published());
    }

    #[test]
    fn catalog_commit_between_capture_and_ack_fences_the_old_proof() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let pending = begin_registration(AGENT, HOST, 1).unwrap();
        let assignment = HostedAgentGenerationAssignment {
            agent_ura: AGENT.into(),
            host_device_ura: HOST.into(),
            incarnation_id: pending.incarnation_id().clone(),
            generation: 1,
        };
        bind_assignment(&assignment, 2).unwrap();
        let captured_epoch = record_for(AGENT).unwrap().unwrap().desired_catalog_epoch;
        stage_projection(&assignment, captured_epoch, 1, "sha256:captured", 3).unwrap();

        let committed_epoch = fence_catalog_commit(HOST, [AGENT], 4).unwrap();
        assert!(committed_epoch > captured_epoch);
        let fenced = record_for(AGENT).unwrap().unwrap();
        assert_eq!(fenced.publication_state(), "assigned");
        assert_eq!(fenced.desired_catalog_epoch, committed_epoch);
        assert!(mark_published(&assignment, captured_epoch, 1, "sha256:captured", 5,).is_err());

        stage_projection(&assignment, committed_epoch, 2, "sha256:committed", 6).unwrap();
        assert!(
            mark_published(&assignment, committed_epoch, 2, "sha256:committed", 7,)
                .unwrap()
                .is_published()
        );
    }
}
