//! Durable Voice aggregates for a realm Hub deployment.
//!
//! Construction requires an operator-supplied shared storage root. Every Hub
//! replica for a realm must mount the same POSIX-locking, rename-atomic
//! filesystem there. Daemon-local state directories are never consulted.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::daemon::ability::builtins::resources::voice_contract::{
    VoiceCallAggregate, VoiceCallCasOutcome, VoiceCallRepository, VoiceCallRepositoryEntry,
    VoiceCallRepositoryQualification,
};
use crate::daemon::persistence::file_lock::{ExclusiveFileLock, SharedFileLock};

const SCHEMA_VERSION: u32 = 1;
pub const VOICE_SHARED_ROOT_ENV: &str = "EASYNET_HUB_VOICE_SHARED_ROOT";

#[derive(Debug, Serialize, Deserialize)]
struct RealmVoiceFile {
    schema_version: u32,
    realm: String,
    calls: Vec<VoiceCallAggregate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurableWriteOutcome {
    Committed,
    Ambiguous,
}

impl RealmVoiceFile {
    fn empty(realm: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            realm,
            calls: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct HubRealmVoiceCallRepository {
    realm: String,
    path: PathBuf,
    provider_id: String,
}

impl HubRealmVoiceCallRepository {
    pub fn from_env(realm: &str) -> anyhow::Result<Option<std::sync::Arc<Self>>> {
        let Some(root) = std::env::var_os(VOICE_SHARED_ROOT_ENV) else {
            return Ok(None);
        };
        Self::open_qualified(root, realm).map(|repository| Some(std::sync::Arc::new(repository)))
    }

    #[cfg(test)]
    pub(crate) fn open(root: impl Into<PathBuf>, realm: &str) -> anyhow::Result<Self> {
        Self::open_qualified(root, realm)
    }

    fn open_qualified(root: impl Into<PathBuf>, realm: &str) -> anyhow::Result<Self> {
        let root = root.into();
        if !root.is_absolute() {
            anyhow::bail!("{VOICE_SHARED_ROOT_ENV} must be an absolute shared filesystem path");
        }
        if realm.trim().is_empty()
            || !realm
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        {
            anyhow::bail!("invalid Voice repository realm {realm:?}");
        }
        fs::create_dir_all(&root)?;
        let canonical_root = fs::canonicalize(&root)?;
        let path = canonical_root.join(format!("voice-calls-{realm}.json"));
        let repository = Self {
            realm: realm.to_string(),
            provider_id: format!("shared-posix:{}:{realm}", canonical_root.display()),
            path,
        };
        repository.initialize_and_probe()?;
        Ok(repository)
    }

    fn initialize_and_probe(&self) -> anyhow::Result<()> {
        let _lock = ExclusiveFileLock::acquire_for_data_path(&self.path)?;
        if !self.path.exists() {
            self.write_unlocked(&RealmVoiceFile::empty(self.realm.clone()))?;
        }
        self.read_unlocked().map(|_| ())
    }

    fn read_unlocked(&self) -> anyhow::Result<RealmVoiceFile> {
        let bytes = fs::read(&self.path)?;
        let file: RealmVoiceFile = serde_json::from_slice(&bytes)?;
        if file.schema_version != SCHEMA_VERSION || file.realm != self.realm {
            anyhow::bail!(
                "Voice shared store identity mismatch at {}: expected schema {} realm {:?}",
                self.path.display(),
                SCHEMA_VERSION,
                self.realm
            );
        }
        let mut keys = std::collections::BTreeSet::new();
        for aggregate in &file.calls {
            aggregate.validate_recovered()?;
            let authority = crate::core::ura::parse_ura(aggregate.authority_ura())?;
            if authority.realm != self.realm {
                anyhow::bail!("Voice shared store contains a foreign realm authority");
            }
            if !keys.insert((aggregate.authority_ura(), aggregate.call_id())) {
                anyhow::bail!("Voice shared store contains a duplicate aggregate key");
            }
        }
        Ok(file)
    }

    fn write_unlocked(&self, file: &RealmVoiceFile) -> anyhow::Result<()> {
        match self.write_with_outcome_unlocked(file)? {
            DurableWriteOutcome::Committed => Ok(()),
            DurableWriteOutcome::Ambiguous => anyhow::bail!(
                "Voice shared store commit completed but directory durability is ambiguous"
            ),
        }
    }

    fn write_with_outcome_unlocked(
        &self,
        file: &RealmVoiceFile,
    ) -> anyhow::Result<DurableWriteOutcome> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Voice store has no parent"))?;
        let temp = parent.join(format!(
            ".voice-calls.{}.{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        output.write_all(&serde_json::to_vec_pretty(file)?)?;
        output.sync_all()?;
        fs::rename(&temp, &self.path)?;
        if File::open(parent)
            .and_then(|directory| directory.sync_all())
            .is_err()
        {
            return Ok(DurableWriteOutcome::Ambiguous);
        }
        Ok(DurableWriteOutcome::Committed)
    }

    fn validate_authority(&self, authority_ura: &str) -> anyhow::Result<()> {
        let authority = crate::core::ura::parse_ura(authority_ura)?;
        if authority.kind != crate::core::ura::URAKind::Authority || authority.realm != self.realm {
            anyhow::bail!(
                "Voice repository for realm {:?} rejects authority {authority_ura:?}",
                self.realm
            );
        }
        Ok(())
    }
}

impl VoiceCallRepository for HubRealmVoiceCallRepository {
    fn qualification(&self) -> VoiceCallRepositoryQualification {
        VoiceCallRepositoryQualification::production(self.provider_id.clone())
    }

    fn insert_if_absent(&self, aggregate: VoiceCallAggregate) -> anyhow::Result<bool> {
        self.validate_authority(aggregate.authority_ura())?;
        aggregate.validate_recovered()?;
        let _lock = ExclusiveFileLock::acquire_for_data_path(&self.path)?;
        let mut file = self.read_unlocked()?;
        if file.calls.iter().any(|current| {
            current.authority_ura() == aggregate.authority_ura()
                && current.call_id() == aggregate.call_id()
        }) {
            return Ok(false);
        }
        file.calls.push(aggregate);
        file.calls.sort_by(|left, right| {
            (left.authority_ura(), left.call_id()).cmp(&(right.authority_ura(), right.call_id()))
        });
        self.write_unlocked(&file)?;
        Ok(true)
    }

    fn load(
        &self,
        authority_ura: &str,
        call_id: &str,
    ) -> anyhow::Result<Option<VoiceCallAggregate>> {
        self.validate_authority(authority_ura)?;
        let _lock = SharedFileLock::acquire_for_data_path(&self.path)?;
        Ok(self
            .read_unlocked()?
            .calls
            .into_iter()
            .find(|call| call.authority_ura() == authority_ura && call.call_id() == call_id))
    }

    fn list(&self, authority_ura: &str) -> anyhow::Result<Vec<VoiceCallRepositoryEntry>> {
        self.validate_authority(authority_ura)?;
        let _lock = SharedFileLock::acquire_for_data_path(&self.path)?;
        Ok(self
            .read_unlocked()?
            .calls
            .into_iter()
            .filter(|call| call.authority_ura() == authority_ura)
            .map(|call| {
                VoiceCallRepositoryEntry::new(
                    authority_ura.to_string(),
                    call.call_id().to_string(),
                    call,
                )
            })
            .collect())
    }

    fn compare_and_swap(
        &self,
        authority_ura: &str,
        call_id: &str,
        expected_revision: u64,
        replacement: VoiceCallAggregate,
    ) -> anyhow::Result<VoiceCallCasOutcome> {
        self.validate_authority(authority_ura)?;
        replacement.validate_repository_key(authority_ura, call_id)?;
        replacement.validate_cas_replacement(expected_revision)?;
        let _lock = ExclusiveFileLock::acquire_for_data_path(&self.path)?;
        let mut file = self.read_unlocked()?;
        let current = file
            .calls
            .iter_mut()
            .find(|call| call.authority_ura() == authority_ura && call.call_id() == call_id)
            .ok_or_else(|| anyhow::anyhow!("Voice CAS target {call_id:?} does not exist"))?;
        if current.revision() != expected_revision {
            return Ok(VoiceCallCasOutcome::Current(current.clone()));
        }
        *current = replacement.clone();
        match self.write_with_outcome_unlocked(&file)? {
            DurableWriteOutcome::Committed => Ok(VoiceCallCasOutcome::Committed(replacement)),
            DurableWriteOutcome::Ambiguous => Ok(VoiceCallCasOutcome::Ambiguous),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_instances_and_restart_observe_one_revision() {
        let root = tempfile::tempdir().unwrap();
        let hub = crate::core::ura::hub_ura("voice-shared");
        let first = HubRealmVoiceCallRepository::open(root.path(), "voice-shared").unwrap();
        let second = HubRealmVoiceCallRepository::open(root.path(), "voice-shared").unwrap();
        let aggregate = VoiceCallAggregate::new(hub.clone(), "call-1".into(), None, 10);
        assert!(first.insert_if_absent(aggregate.clone()).unwrap());
        assert_eq!(second.load(&hub, "call-1").unwrap().unwrap().revision(), 1);
        let mut replacement = second.load(&hub, "call-1").unwrap().unwrap();
        replacement
            .join("shared-command", "alice".into(), None, 20)
            .unwrap();
        replacement.bump_revision().unwrap();
        assert!(matches!(
            second
                .compare_and_swap(&hub, "call-1", 1, replacement.clone())
                .unwrap(),
            VoiceCallCasOutcome::Committed(_)
        ));
        assert_eq!(first.load(&hub, "call-1").unwrap().unwrap().revision(), 2);
        drop(first);
        drop(second);
        let restarted = HubRealmVoiceCallRepository::open(root.path(), "voice-shared").unwrap();
        assert_eq!(
            restarted.load(&hub, "call-1").unwrap().unwrap(),
            replacement
        );
        let qualification = restarted.qualification();
        qualification.validate_production().unwrap();
        assert!(qualification.provider_id().starts_with("shared-posix:"));
        assert!(qualification.is_durable());
        assert!(qualification.is_realm_scoped());
        assert!(qualification.has_linearizable_cas());
        assert!(qualification.has_idempotent_commands());
    }
}
