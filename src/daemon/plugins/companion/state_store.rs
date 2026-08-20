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
#[serde(deny_unknown_fields)]
pub struct CompanionStateRecord {
    pub id: String,
    pub version: String,
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
#[serde(deny_unknown_fields)]
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
    pub fn default_path() -> Result<PathBuf> {
        Self::default_path_for_home(dirs::home_dir())
    }

    fn default_path_for_home(home: Option<PathBuf>) -> Result<PathBuf> {
        let home = home.ok_or_else(|| PluginHostError::InvalidCompanionManifest {
            id: "state_store".to_string(),
            reason: "desktop companion state store requires an OS home directory".to_string(),
        })?;
        if home.as_os_str().is_empty() {
            return Err(PluginHostError::InvalidCompanionManifest {
                id: "state_store".to_string(),
                reason: "desktop companion state store home directory must not be empty"
                    .to_string(),
            });
        }
        Ok(home.join(".easynet/companions/state.toml"))
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
        let state: CompanionStateToml =
            toml::from_str(&body).map_err(|source| PluginHostError::InvalidCompanionManifest {
                id: "state_store".to_string(),
                reason: format!("parse {}: {source}", self.path.display()),
            })?;
        validate_state(&state).map_err(|reason| PluginHostError::InvalidCompanionManifest {
            id: "state_store".to_string(),
            reason: format!("validate {}: {reason}", self.path.display()),
        })?;
        Ok(state)
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
            .record(id, version)?
            .map(|record| record.desired_state)
            .unwrap_or_default())
    }

    pub fn record(&self, id: &str, version: &str) -> Result<Option<CompanionStateRecord>> {
        Ok(self
            .read()?
            .companion
            .into_iter()
            .find(|record| record.id == id && record.version == version))
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

fn validate_state(state: &CompanionStateToml) -> std::result::Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for (index, record) in state.companion.iter().enumerate() {
        if record.id.trim().is_empty() {
            return Err(format!("companion[{index}].id must be non-empty"));
        }
        if record.version.trim().is_empty() {
            return Err(format!("companion[{index}].version must be non-empty"));
        }
        let key = (record.id.as_str(), record.version.as_str());
        if !seen.insert(key) {
            return Err(format!(
                "duplicate companion state row for {}@{}",
                record.id, record.version
            ));
        }
    }
    Ok(())
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

    #[test]
    fn state_store_rejects_schema_incomplete_rows() {
        let root = tempfile::tempdir().expect("root");
        let path = root.path().join("state.toml");
        let store = DesktopCompanionStateStore::new(&path);

        for (body, expected) in [
            (
                r#"[[companion]]
id = "easynet.desktop.menubar"
version = "0.1.0"
"#,
                "missing field `desired_state`",
            ),
            (
                r#"[[companion]]
id = ""
version = "0.1.0"
desired_state = "enabled"
"#,
                "companion[0].id must be non-empty",
            ),
            (
                r#"[[companion]]
id = "easynet.desktop.menubar"
version = "0.1.0"
desired_state = "enabled"
legacy_state = "running"
"#,
                "unknown field `legacy_state`",
            ),
        ] {
            std::fs::write(&path, body).expect("write malformed state");
            let err = store
                .read()
                .expect_err("schema-incomplete companion state must fail closed")
                .to_string();
            assert!(err.contains(expected), "expected {expected:?}; got {err}");
        }
    }

    #[test]
    fn state_store_rejects_duplicate_rows() {
        let root = tempfile::tempdir().expect("root");
        let path = root.path().join("state.toml");
        std::fs::write(
            &path,
            r#"[[companion]]
id = "easynet.desktop.menubar"
version = "0.1.0"
desired_state = "enabled"

[[companion]]
id = "easynet.desktop.menubar"
version = "0.1.0"
desired_state = "disabled"
"#,
        )
        .expect("write duplicate state");
        let store = DesktopCompanionStateStore::new(path);
        let err = store
            .desired_state("easynet.desktop.menubar", "0.1.0")
            .expect_err("duplicate desired-state rows must fail closed")
            .to_string();
        assert!(
            err.contains("duplicate companion state row"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn state_store_preserves_missing_file_and_absent_record_defaults() {
        let root = tempfile::tempdir().expect("root");
        let store = DesktopCompanionStateStore::new(root.path().join("state.toml"));

        assert_eq!(
            store
                .desired_state("easynet.desktop.menubar", "0.1.0")
                .expect("missing state file is fresh-install empty"),
            CompanionDesiredState::Disabled
        );
    }

    #[test]
    fn default_path_rejects_missing_home_before_cwd_fallback() {
        let error = DesktopCompanionStateStore::default_path_for_home(None)
            .expect_err("missing home must fail before cwd fallback")
            .to_string();
        assert!(
            error.contains("requires an OS home directory"),
            "wrong error: {error}"
        );

        let error = DesktopCompanionStateStore::default_path_for_home(Some(PathBuf::new()))
            .expect_err("empty home must fail before cwd fallback")
            .to_string();
        assert!(
            error.contains("home directory must not be empty"),
            "wrong error: {error}"
        );
    }
}
