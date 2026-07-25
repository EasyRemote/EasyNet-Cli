// EasyNet CLI — plugin install state files
// ========================================
//
// File: src/daemon/plugins/install/state.rs
// Description: Atomic state/lock persistence for installed plugin packages.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::daemon::persistence::config;
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::package::PluginPackage;

const STATE_DIR: &str = "state";
const PLUGINS_STATE_FILE: &str = "plugins.toml";
const PLUGIN_LOCK_FILE: &str = "plugin-lock.toml";

/// State/lock store rooted at `~/.easynet/plugins/state`.
///
/// What this is NOT: package-directory transaction logic. This type owns only
/// the two TOML projections that decide which package versions are active.
pub(super) struct PluginStateStore {
    root: PathBuf,
}

impl PluginStateStore {
    /// Construct a state store for one plugin root.
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Read active installed package records. Missing state is an empty index.
    pub(super) fn read(&self) -> Result<PluginStateToml> {
        let path = self.state_path(PLUGINS_STATE_FILE);
        if !path.exists() {
            return Ok(PluginStateToml::default());
        }
        let body = fs::read_to_string(&path).map_err(|source| PluginHostError::ReadFailed {
            path: path.clone(),
            source,
        })?;
        PluginStateToml::parse_active_projection(&body, &path)
    }

    /// Atomically write state and lock, restoring the prior projection if lock
    /// commit fails after state has already been replaced.
    pub(super) fn write(&self, state: &PluginStateToml) -> Result<()> {
        fs::create_dir_all(self.root.join(STATE_DIR)).map_err(|source| {
            PluginHostError::WriteFailed {
                path: self.root.join(STATE_DIR),
                source,
            }
        })?;
        let plugins_path = self.state_path(PLUGINS_STATE_FILE);
        let lock_path = self.state_path(PLUGIN_LOCK_FILE);
        let previous_plugins = read_optional_file(&plugins_path)?;
        let previous_lock = read_optional_file(&lock_path)?;
        let body =
            toml::to_string_pretty(state).map_err(|source| PluginHostError::WriteFailed {
                path: plugins_path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            })?;
        if let Err(source) = config::atomic_write(&plugins_path, body.as_bytes()) {
            return Err(PluginHostError::WriteFailed {
                path: plugins_path,
                source: std::io::Error::other(source),
            });
        }
        if let Err(source) = config::atomic_write(&lock_path, body.as_bytes()) {
            let _ = restore_optional_file(&plugins_path, previous_plugins);
            let _ = restore_optional_file(&lock_path, previous_lock);
            return Err(PluginHostError::WriteFailed {
                path: lock_path,
                source: std::io::Error::other(source),
            });
        }
        Ok(())
    }

    fn state_path(&self, name: &str) -> PathBuf {
        self.root.join(STATE_DIR).join(name)
    }
}

/// Active plugin package projection persisted in `plugins.toml` and lockfile.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginStateToml {
    pub plugins: Vec<InstalledPluginRecord>,
}

/// One active installed plugin version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstalledPluginRecord {
    pub id: String,
    pub version: String,
    pub hash: String,
}

impl PluginStateToml {
    /// Parse an existing active-state projection.
    ///
    /// Missing files are handled by [`PluginStateStore::read`]. Once a state
    /// file exists, it is package authority and must be structurally complete;
    /// the parser must not repair malformed state into an empty active set.
    pub(crate) fn parse_active_projection(body: &str, path: &Path) -> Result<Self> {
        let state: Self =
            toml::from_str(body).map_err(|source| PluginHostError::ManifestParseFailed {
                path: path.to_path_buf(),
                source,
            })?;
        state.validate(path)?;
        Ok(state)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        let mut active_ids = BTreeMap::<&str, &str>::new();
        let mut active_versions = BTreeMap::<(&str, &str), ()>::new();
        for record in &self.plugins {
            record.validate(path)?;
            if active_versions
                .insert((record.id.as_str(), record.version.as_str()), ())
                .is_some()
            {
                return Err(PluginHostError::DuplicatePackageVersion {
                    id: record.id.clone(),
                    version: record.version.clone(),
                });
            }
            if let Some(first_version) = active_ids.insert(&record.id, &record.version) {
                return Err(PluginHostError::DuplicatePackageId {
                    id: record.id.clone(),
                    first_version: first_version.to_string(),
                    second_version: record.version.clone(),
                });
            }
        }
        Ok(())
    }
}

impl InstalledPluginRecord {
    fn validate(&self, path: &Path) -> Result<()> {
        let fields = [
            ("id", self.id.as_str()),
            ("version", self.version.as_str()),
            ("hash", self.hash.as_str()),
        ];
        for (field, value) in fields {
            if value.trim().is_empty() {
                return Err(PluginHostError::InvalidPluginState {
                    path: path.to_path_buf(),
                    reason: format!("installed plugin record has empty {field}"),
                });
            }
        }
        Ok(())
    }

    pub(super) fn from_package(package: &PluginPackage) -> Self {
        Self {
            id: package.manifest().id().to_string(),
            version: package.manifest().version().to_string(),
            hash: package.hash().as_str().to_string(),
        }
    }
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>> {
    if !path.exists() {
        return Ok(None);
    }
    fs::read(path)
        .map(Some)
        .map_err(|source| PluginHostError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })
}

fn restore_optional_file(path: &Path, previous: Option<Vec<u8>>) -> Result<()> {
    match previous {
        Some(bytes) => {
            config::atomic_write(path, &bytes).map_err(|source| PluginHostError::WriteFailed {
                path: path.to_path_buf(),
                source: std::io::Error::other(source),
            })
        }
        None => {
            if path.exists() {
                fs::remove_file(path).map_err(|source| PluginHostError::WriteFailed {
                    path: path.to_path_buf(),
                    source,
                })?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_state_file(root: &Path, body: &str) -> PathBuf {
        let state_dir = root.join(STATE_DIR);
        fs::create_dir_all(&state_dir).expect("state dir");
        let path = state_dir.join(PLUGINS_STATE_FILE);
        fs::write(&path, body).expect("state body");
        path
    }

    #[test]
    fn plugin_state_store_treats_missing_file_as_empty_fresh_install() {
        let root = tempfile::tempdir().expect("root");
        let state = PluginStateStore::new(root.path())
            .read()
            .expect("missing state is a fresh install");

        assert!(state.plugins.is_empty());
    }

    #[test]
    fn plugin_state_store_rejects_existing_state_without_plugins_projection() {
        let root = tempfile::tempdir().expect("root");
        write_state_file(root.path(), "");

        let err = PluginStateStore::new(root.path())
            .read()
            .expect_err("existing state must declare plugins explicitly");

        assert!(matches!(err, PluginHostError::ManifestParseFailed { .. }));
    }

    #[test]
    fn plugin_state_store_rejects_unknown_state_fields() {
        let root = tempfile::tempdir().expect("root");
        write_state_file(
            root.path(),
            r#"
plugins = []
retired_repaired = true
"#,
        );

        let err = PluginStateStore::new(root.path())
            .read()
            .expect_err("unknown state fields must not be preserved as compat data");

        assert!(matches!(err, PluginHostError::ManifestParseFailed { .. }));
        assert!(
            format!("{err}").contains("unknown field `retired_repaired`"),
            "parse error should name rejected state field: {err}"
        );
    }

    #[test]
    fn plugin_state_store_rejects_blank_active_record_identity() {
        let root = tempfile::tempdir().expect("root");
        write_state_file(
            root.path(),
            r#"
[[plugins]]
id = ""
version = "0.1.0"
hash = "abc123"
"#,
        );

        let err = PluginStateStore::new(root.path())
            .read()
            .expect_err("blank plugin identity must not become active state");

        assert!(matches!(err, PluginHostError::InvalidPluginState { .. }));
        assert!(err.to_string().contains("empty id"));
    }

    #[test]
    fn plugin_state_store_rejects_duplicate_active_package_row() {
        let root = tempfile::tempdir().expect("root");
        write_state_file(
            root.path(),
            r#"
[[plugins]]
id = "test.plugin"
version = "0.1.0"
hash = "abc123"

[[plugins]]
id = "test.plugin"
version = "0.1.0"
hash = "abc123"
"#,
        );

        let err = PluginStateStore::new(root.path())
            .read()
            .expect_err("duplicate active package row must be rejected");

        assert!(matches!(
            err,
            PluginHostError::DuplicatePackageVersion { .. }
        ));
    }

    #[test]
    fn plugin_state_store_rejects_multiple_active_versions_for_one_package() {
        let root = tempfile::tempdir().expect("root");
        write_state_file(
            root.path(),
            r#"
[[plugins]]
id = "test.plugin"
version = "0.1.0"
hash = "abc123"

[[plugins]]
id = "test.plugin"
version = "0.2.0"
hash = "def456"
"#,
        );

        let err = PluginStateStore::new(root.path())
            .read()
            .expect_err("one package id cannot have two active versions");

        assert!(matches!(err, PluginHostError::DuplicatePackageId { .. }));
    }
}
