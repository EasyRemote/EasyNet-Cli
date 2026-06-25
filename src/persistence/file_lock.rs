// EasyNet CLI - persistence file locks
// ====================================
//
// File: src/persistence/file_lock.rs
// Description: Small OS-level exclusive lock used by JSON stores that perform
//              read-modify-write updates under ~/.easynet.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

/// Exclusive advisory lock for one persisted data file.
///
/// What it is: a process-spanning guard around `<data-file>.lock` held with
/// the operating system's advisory lock primitive.
///
/// What it is not: a distributed lock or stale-process detector. EasyNet-Cli
/// only needs to protect two local daemon/CLI processes sharing one HOME from
/// interleaving JSON read-modify-write cycles.
pub(crate) struct ExclusiveFileLock {
    file: File,
    path: PathBuf,
}

pub(crate) struct SharedFileLock {
    file: File,
    path: PathBuf,
}

impl ExclusiveFileLock {
    /// Acquire the lock associated with `data_path`, creating the parent
    /// directory and lock file as needed.
    pub(crate) fn acquire_for_data_path(data_path: &Path) -> anyhow::Result<Self> {
        let lock_path = lock_path_for(data_path);
        let parent = lock_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("lock path has no parent: {}", lock_path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("create lock dir {}: {e}", parent.display()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| anyhow::anyhow!("open lock {}: {e}", lock_path.display()))?;
        lock_exclusive(&file, &lock_path)?;
        Ok(Self {
            file,
            path: lock_path,
        })
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        let _ = unlock(&self.file, &self.path);
    }
}

impl SharedFileLock {
    /// Acquire a shared advisory lock associated with `data_path`, creating the
    /// parent directory and lock file as needed.
    pub(crate) fn acquire_for_data_path(data_path: &Path) -> anyhow::Result<Self> {
        let lock_path = lock_path_for(data_path);
        let parent = lock_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("lock path has no parent: {}", lock_path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("create lock dir {}: {e}", parent.display()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| anyhow::anyhow!("open lock {}: {e}", lock_path.display()))?;
        lock_shared(&file, &lock_path)?;
        Ok(Self {
            file,
            path: lock_path,
        })
    }
}

impl Drop for SharedFileLock {
    fn drop(&mut self) {
        let _ = unlock(&self.file, &self.path);
    }
}

fn lock_path_for(data_path: &Path) -> PathBuf {
    let file_name = data_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("store");
    data_path.with_file_name(format!("{file_name}.lock"))
}

fn lock_exclusive(file: &File, path: &Path) -> anyhow::Result<()> {
    file.lock_exclusive()
        .map_err(|e| anyhow::anyhow!("lock {}: {e}", path.display()))
}

fn lock_shared(file: &File, path: &Path) -> anyhow::Result<()> {
    file.lock_shared()
        .map_err(|e| anyhow::anyhow!("shared lock {}: {e}", path.display()))
}

fn unlock(file: &File, path: &Path) -> anyhow::Result<()> {
    file.unlock()
        .map_err(|e| anyhow::anyhow!("unlock {}: {e}", path.display()))
}
