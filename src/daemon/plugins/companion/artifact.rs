// EasyNet CLI — desktop companion artifact filesystem helpers
// ============================================================
//
// File: src/daemon/plugins/companion/artifact.rs
// Description: Shared filesystem operations for companion platform artifacts.

use std::path::{Path, PathBuf};

use crate::daemon::plugins::errors::{PluginHostError, Result};
use sha2::{Digest, Sha256};

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

pub fn artifact_fingerprint(path: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(path, path, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for rel in files {
        let file = if rel.as_os_str().is_empty() {
            path.to_path_buf()
        } else {
            path.join(&rel)
        };
        let meta = std::fs::metadata(&file).map_err(|source| PluginHostError::ReadFailed {
            path: file.clone(),
            source,
        })?;
        let body = std::fs::read(&file).map_err(|source| PluginHostError::ReadFailed {
            path: file.clone(),
            source,
        })?;
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(file_fingerprint_metadata(&meta).as_bytes());
        hasher.update([0]);
        hasher.update(body);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let meta = std::fs::metadata(current).map_err(|source| PluginHostError::ReadFailed {
        path: current.to_path_buf(),
        source,
    })?;
    if meta.is_file() {
        let rel = current.strip_prefix(root).unwrap_or(current).to_path_buf();
        files.push(rel);
        return Ok(());
    }
    if !meta.is_dir() {
        return Ok(());
    }

    let mut entries = std::fs::read_dir(current)
        .map_err(|source| PluginHostError::ReadFailed {
            path: current.to_path_buf(),
            source,
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| PluginHostError::ReadFailed {
            path: current.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        collect_files(root, &entry.path(), files)?;
    }
    Ok(())
}

fn file_fingerprint_metadata(meta: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        format!("file:{:o}", meta.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        "file".to_string()
    }
}
