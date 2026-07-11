//! Fail-closed storage for the daemon key-service passphrase.
//!
//! The passphrase file is a single-assignment local secret. Once the path
//! exists, its contents are authoritative: malformed or unreadable state is
//! reported to the operator and is never replaced implicitly.

use std::fs::{self, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use crate::daemon::persistence::file_lock::ExclusiveFileLock;

const PASSPHRASE_HEX_LEN: usize = 64;

#[derive(Debug, Clone)]
pub(super) struct PassphraseStore {
    path: PathBuf,
}

impl PassphraseStore {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn load_or_create(&self) -> io::Result<(String, bool)> {
        self.load_or_create_file()
    }

    fn load_or_create_file(&self) -> io::Result<(String, bool)> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| path_error("create passphrase directory", parent, error))?;
        }

        // Serialize the read-or-create transition across every cooperating
        // CLI and daemon process. This also prevents a second reader from
        // observing the short interval between create_new and sync_all.
        let _guard = ExclusiveFileLock::acquire_for_data_path(&self.path).map_err(|error| {
            io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "acquire passphrase store lock for {}: {error}",
                    self.path.display()
                ),
            )
        })?;

        match self.read_existing() {
            Ok(passphrase) => Ok((passphrase, false)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.persist_new(&super::mint_passphrase())
            }
            Err(error) => Err(error),
        }
    }

    fn read_existing(&self) -> io::Result<String> {
        self.read_existing_with_parent_sync(crate::daemon::persistence::config::sync_parent_dir)
    }

    fn read_existing_with_parent_sync<F>(&self, sync: F) -> io::Result<String>
    where
        F: FnOnce(&Path) -> anyhow::Result<()>,
    {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|error| path_error("open passphrase", &self.path, error))?;
        let metadata = file
            .metadata()
            .map_err(|error| path_error("inspect passphrase", &self.path, error))?;
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "passphrase path {} is not a regular file",
                    self.path.display()
                ),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = metadata.permissions().mode() & 0o777;
            if mode != 0o600 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "passphrase file {} must have mode 0600, got {mode:04o}",
                        self.path.display()
                    ),
                ));
            }
        }
        let mut bytes = Vec::with_capacity(PASSPHRASE_HEX_LEN);
        file.read_to_end(&mut bytes)
            .map_err(|error| path_error("read passphrase", &self.path, error))?;
        if bytes.len() != PASSPHRASE_HEX_LEN
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "passphrase file {} must contain exactly {PASSPHRASE_HEX_LEN} lowercase hex bytes",
                    self.path.display()
                ),
            ));
        }
        let passphrase = String::from_utf8(bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "passphrase file {} is not valid UTF-8: {error}",
                    self.path.display()
                ),
            )
        })?;
        sync(&self.path).map_err(|error| {
            io::Error::other(format!(
                "sync passphrase directory for {}: {error}",
                self.path.display()
            ))
        })?;
        Ok(passphrase)
    }

    fn persist_new(&self, generated: &str) -> io::Result<(String, bool)> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }

        let mut file = match options.open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // Another creator that does not participate in our advisory
                // lock won the create_new race. Its file is authoritative;
                // strictly reread it rather than falling back to our value.
                return self.read_existing().map(|passphrase| (passphrase, false));
            }
            Err(error) => {
                return Err(path_error("create passphrase", &self.path, error));
            }
        };

        file.write_all(generated.as_bytes())
            .map_err(|error| path_error("write passphrase", &self.path, error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| path_error("secure passphrase", &self.path, error))?;
        }
        file.sync_all()
            .map_err(|error| path_error("sync passphrase", &self.path, error))?;
        crate::daemon::persistence::config::sync_parent_dir(&self.path).map_err(|error| {
            io::Error::other(format!(
                "sync passphrase directory for {}: {error}",
                self.path.display()
            ))
        })?;
        Ok((generated.to_owned(), true))
    }
}

fn path_error(action: &str, path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("{action} {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use tempfile::TempDir;

    use super::*;

    fn write_secure_fixture(path: &Path, bytes: impl AsRef<[u8]>) {
        fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn concurrent_creators_converge_on_one_persisted_passphrase() {
        const CALLERS: usize = 12;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        let barrier = Arc::new(Barrier::new(CALLERS));

        let handles = (0..CALLERS)
            .map(|_| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    PassphraseStore::new(path).load_or_create_file()
                })
            })
            .collect::<Vec<_>>();

        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        let persisted = fs::read_to_string(&path).unwrap();

        assert_eq!(
            results.iter().filter(|(_, created)| *created).count(),
            1,
            "exactly one caller must own creation"
        );
        assert!(
            results
                .iter()
                .all(|(passphrase, _)| passphrase == &persisted),
            "every caller must observe the one persisted secret"
        );
    }

    #[test]
    fn empty_existing_file_fails_closed_without_overwrite() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        write_secure_fixture(&path, []);

        let error = PassphraseStore::new(path.clone())
            .load_or_create_file()
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn non_utf8_existing_file_fails_closed_without_overwrite() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        let corrupt = vec![0xff, 0xfe, 0xfd];
        write_secure_fixture(&path, &corrupt);

        let error = PassphraseStore::new(path.clone())
            .load_or_create_file()
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn unreadable_existing_path_fails_closed_without_replacement() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        fs::create_dir(&path).unwrap();

        PassphraseStore::new(path.clone())
            .load_or_create_file()
            .unwrap_err();

        assert!(path.is_dir());
    }

    #[test]
    fn create_new_collision_strictly_rereads_the_winner() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        let winner = "a".repeat(PASSPHRASE_HEX_LEN);
        write_secure_fixture(&path, &winner);

        let result = PassphraseStore::new(path.clone())
            .persist_new(&"b".repeat(PASSPHRASE_HEX_LEN))
            .unwrap();

        assert_eq!(result, (winner.clone(), false));
        assert_eq!(fs::read_to_string(path).unwrap(), winner);
    }

    #[test]
    fn malformed_existing_hex_fails_closed_without_overwrite() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        let malformed = "g".repeat(PASSPHRASE_HEX_LEN);
        write_secure_fixture(&path, &malformed);

        let error = PassphraseStore::new(path.clone())
            .load_or_create_file()
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(path).unwrap(), malformed);
    }

    #[cfg(unix)]
    #[test]
    fn insecure_existing_mode_fails_closed_without_chmod() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        fs::write(&path, "a".repeat(PASSPHRASE_HEX_LEN)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let error = PassphraseStore::new(path.clone())
            .load_or_create_file()
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn existing_passphrase_requires_parent_directory_resync() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.pass");
        write_secure_fixture(&path, "a".repeat(PASSPHRASE_HEX_LEN));
        let store = PassphraseStore::new(path);

        let error = store
            .read_existing_with_parent_sync(|_| {
                anyhow::bail!("injected parent-directory fsync failure")
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(
            store.read_existing().unwrap(),
            "a".repeat(PASSPHRASE_HEX_LEN)
        );
    }
}
