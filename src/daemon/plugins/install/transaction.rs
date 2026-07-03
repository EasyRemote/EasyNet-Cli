// EasyNet CLI — plugin install directory transactions
// ===================================================
//
// File: src/daemon/plugins/install/transaction.rs
// Description: Staging, rollback, and package tree movement helpers.

use std::fs;
use std::path::{Path, PathBuf};

use crate::daemon::plugins::errors::{PluginHostError, Result};

const STAGING_DIR: &str = ".staging";
const ROLLBACK_DIR: &str = ".rollback";

/// Copy an unpacked package tree into a transaction staging directory.
///
/// What this is NOT: manifest validation. The caller must validate the staged
/// package after copy, before it is renamed into `installed/`.
pub(super) fn copy_tree(source_path: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target).map_err(|source| PluginHostError::WriteFailed {
        path: target.to_path_buf(),
        source,
    })?;
    for entry in fs::read_dir(source_path).map_err(|source| PluginHostError::ReadFailed {
        path: source_path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| PluginHostError::ReadFailed {
            path: source_path.to_path_buf(),
            source,
        })?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)
                .map_err(|source| PluginHostError::WriteFailed { path: to, source })?;
        }
    }
    Ok(())
}

/// Create a unique staging directory for one install/update transaction.
pub(super) fn txn_dir(root: &Path, prefix: &str) -> Result<PathBuf> {
    let dir = root.join(STAGING_DIR).join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&dir).map_err(|source| PluginHostError::WriteFailed {
        path: dir.clone(),
        source,
    })?;
    Ok(dir)
}

/// Allocate a rollback directory for a package version being displaced.
pub(super) fn rollback_dir(root: &Path, id: &str, version: &str) -> Result<PathBuf> {
    let dir = root.join(ROLLBACK_DIR).join(format!(
        "{id}-{version}-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent).map_err(|source| PluginHostError::WriteFailed {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(dir)
}
