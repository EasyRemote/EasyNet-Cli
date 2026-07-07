// EasyNet CLI — desktop companion artifact filesystem helpers
// ============================================================
//
// File: src/daemon/plugins/companion/artifact.rs
// Description: Shared filesystem operations for companion platform artifacts.

use std::path::Path;

use crate::daemon::plugins::errors::{PluginHostError, Result};

pub fn copy_dir_replacing(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PluginHostError::WriteFailed {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    if target.exists() {
        std::fs::remove_dir_all(target).map_err(|source| PluginHostError::WriteFailed {
            path: target.to_path_buf(),
            source,
        })?;
    }
    copy_dir_recursive(source, target)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target).map_err(|err| PluginHostError::WriteFailed {
        path: target.to_path_buf(),
        source: err,
    })?;
    for entry in std::fs::read_dir(source).map_err(|err| PluginHostError::ReadFailed {
        path: source.to_path_buf(),
        source: err,
    })? {
        let entry = entry.map_err(|err| PluginHostError::ReadFailed {
            path: source.to_path_buf(),
            source: err,
        })?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let meta = entry
            .metadata()
            .map_err(|err| PluginHostError::ReadFailed {
                path: from.clone(),
                source: err,
            })?;
        if meta.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|err| PluginHostError::WriteFailed {
                path: to,
                source: err,
            })?;
        }
    }
    Ok(())
}
