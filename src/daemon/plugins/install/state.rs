// EasyNet CLI — plugin install state files
// ========================================
//
// File: src/daemon/plugins/install/state.rs
// Description: Atomic state/lock persistence for installed plugin packages.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::package::PluginPackage;
use crate::persistence::config;

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
        toml::from_str(&body)
            .map_err(|source| PluginHostError::ManifestParseFailed { path, source })
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
pub struct PluginStateToml {
    #[serde(default)]
    pub plugins: Vec<InstalledPluginRecord>,
}

/// One active installed plugin version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledPluginRecord {
    pub id: String,
    pub version: String,
    pub hash: String,
}

impl InstalledPluginRecord {
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
