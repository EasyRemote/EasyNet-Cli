// EasyNet CLI — desktop companion state store
// ===========================================
//
// File: src/daemon/plugins/companion/state_store.rs
// Description: Desired-state memory for desktop companion packages.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::daemon::plugins::errors::{PluginHostError, Result};

use super::status::CompanionDesiredState;

/// One desired-state row in `~/.easynet/companions/state.toml`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CompanionStateRecord {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub desired_state: CompanionDesiredState,
    #[serde(default)]
    pub last_action: Option<String>,
    #[serde(default)]
    pub last_action_unix_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// File shape for companion desired-state memory.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CompanionStateToml {
    #[serde(default)]
    pub companion: Vec<CompanionStateRecord>,
}

/// Durable desired-state storage. Observed process state is never read from
/// this file.
#[derive(Clone, Debug)]
pub struct DesktopCompanionStateStore {
    path: PathBuf,
}

impl DesktopCompanionStateStore {
    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".easynet/companions/state.toml")
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> Result<CompanionStateToml> {
        if !self.path.exists() {
            return Ok(CompanionStateToml::default());
        }
        let body =
            std::fs::read_to_string(&self.path).map_err(|source| PluginHostError::ReadFailed {
                path: self.path.clone(),
                source,
            })?;
        toml::from_str(&body).map_err(|source| PluginHostError::InvalidCompanionManifest {
            id: "state_store".to_string(),
            reason: format!("parse {}: {source}", self.path.display()),
        })
    }

    pub fn write(&self, state: &CompanionStateToml) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| PluginHostError::WriteFailed {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let body = toml::to_string_pretty(state).map_err(|source| {
            PluginHostError::InvalidCompanionManifest {
                id: "state_store".to_string(),
                reason: format!("serialize companion state: {source}"),
            }
        })?;
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, body).map_err(|source| PluginHostError::WriteFailed {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, &self.path).map_err(|source| PluginHostError::WriteFailed {
            path: self.path.clone(),
            source,
        })
    }

    pub fn desired_state(&self, id: &str, version: &str) -> Result<CompanionDesiredState> {
        Ok(self
            .read()?
            .companion
            .into_iter()
            .find(|record| record.id == id && record.version == version)
            .map(|record| record.desired_state)
            .unwrap_or_default())
    }

    pub fn set_desired_state(
        &self,
        id: &str,
        version: &str,
        desired_state: CompanionDesiredState,
        action: &str,
        error: Option<String>,
    ) -> Result<()> {
        let mut state = self.read()?;
        if let Some(record) = state
            .companion
            .iter_mut()
            .find(|record| record.id == id && record.version == version)
        {
            record.desired_state = desired_state;
            record.last_action = Some(action.to_string());
            record.last_action_unix_ms = Some(current_unix_ms());
            record.last_error = error;
        } else {
            state.companion.push(CompanionStateRecord {
                id: id.to_string(),
                version: version.to_string(),
                desired_state,
                last_action: Some(action.to_string()),
                last_action_unix_ms: Some(current_unix_ms()),
                last_error: error,
            });
        }
        state
            .companion
            .sort_by(|a, b| a.id.cmp(&b.id).then(a.version.cmp(&b.version)));
        self.write(&state)
    }

    pub fn remove(&self, id: &str, version: &str) -> Result<()> {
        let mut state = self.read()?;
        state
            .companion
            .retain(|record| !(record.id == id && record.version == version));
        self.write(&state)
    }
}

fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_store_round_trips_desired_state() {
        let root = tempfile::tempdir().expect("root");
        let store = DesktopCompanionStateStore::new(root.path().join("state.toml"));

        store
            .set_desired_state(
                "easynet.desktop.menubar",
                "0.1.0",
                CompanionDesiredState::Enabled,
                "enable",
                None,
            )
            .expect("write");

        assert_eq!(
            store
                .desired_state("easynet.desktop.menubar", "0.1.0")
                .expect("read"),
            CompanionDesiredState::Enabled
        );
    }
}
