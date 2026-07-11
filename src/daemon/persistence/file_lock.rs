// EasyNet CLI - persistence file locks
// ====================================
//
// File: src/daemon/persistence/file_lock.rs
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

/// Which advisory lock to acquire. The only behavioural difference between
/// the two lock newtypes is the fs2 primitive selected here; everything
/// else (lock-path derivation, parent-dir creation, open flags, unlock on
/// Drop) is identical and lives in [`open_and_lock`].
#[derive(Clone, Copy)]
enum LockMode {
    Exclusive,
    Shared,
}

impl LockMode {
    fn apply(self, file: &File, path: &Path) -> anyhow::Result<()> {
        match self {
            LockMode::Exclusive => file
                .lock_exclusive()
                .map_err(|e| anyhow::anyhow!("lock {}: {e}", path.display())),
            LockMode::Shared => file
                .lock_shared()
                .map_err(|e| anyhow::anyhow!("shared lock {}: {e}", path.display())),
        }
    }
}

impl ExclusiveFileLock {
    /// Acquire the lock associated with `data_path`, creating the parent
    /// directory and lock file as needed.
    pub(crate) fn acquire_for_data_path(data_path: &Path) -> anyhow::Result<Self> {
        let (file, path) = open_and_lock(data_path, LockMode::Exclusive)?;
        Ok(Self { file, path })
    }

    /// Try to acquire the process-spanning lease without waiting. Long-lived
    /// service owners use this at boot so a second process fails explicitly
    /// instead of hanging behind the active owner.
    pub(crate) fn try_acquire_for_data_path(data_path: &Path) -> anyhow::Result<Option<Self>> {
        let (file, path) = open_lock_file(data_path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file, path })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(anyhow::anyhow!("try lock {}: {error}", path.display())),
        }
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
        let (file, path) = open_and_lock(data_path, LockMode::Shared)?;
        Ok(Self { file, path })
    }
}

impl Drop for SharedFileLock {
    fn drop(&mut self) {
        let _ = unlock(&self.file, &self.path);
    }
}

/// Derive the lock path, ensure its directory exists, open the lock file,
/// and take `mode`. The sole differing primitive between the exclusive and
/// shared guards is `mode`; sharing this body means a future change to the
/// open or error-handling path is made once.
fn open_and_lock(data_path: &Path, mode: LockMode) -> anyhow::Result<(File, PathBuf)> {
    let (file, lock_path) = open_lock_file(data_path)?;
    mode.apply(&file, &lock_path)?;
    Ok((file, lock_path))
}

fn open_lock_file(data_path: &Path) -> anyhow::Result<(File, PathBuf)> {
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
    Ok((file, lock_path))
}

fn lock_path_for(data_path: &Path) -> PathBuf {
    let file_name = data_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("store");
    data_path.with_file_name(format!("{file_name}.lock"))
}

fn unlock(file: &File, path: &Path) -> anyhow::Result<()> {
    file.unlock()
        .map_err(|e| anyhow::anyhow!("unlock {}: {e}", path.display()))
}
