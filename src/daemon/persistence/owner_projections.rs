// EasyNet CLI — Owner Projection Cursor Store (AXON-RFC-005 Phase C)
// ==================================================================
//
// Owns ~/.easynet/owner-projections.json. The file is intentionally
// only a publication cursor: protocol semantics such as ability
// summaries, descriptor hashes, and projection digests live in
// daemon::federation::read_model::owner_projection. Keeping this layer dumb
// prevents the on-disk schema from becoming a second resolver implementation.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};
use super::file_lock::ExclusiveFileLock;

const FILE_NAME: &str = "owner-projections.json";
const SCHEMA_VERSION: u32 = 2;
static STORE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OwnerProjectionCursorFile {
    pub schema_version: u32,
    pub projections: Vec<OwnerProjectionCursor>,
}

impl Default for OwnerProjectionCursorFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            projections: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OwnerProjectionCursor {
    pub owner_ura: String,
    pub host_device_ura: String,
    pub generation: u64,
    pub lifecycle: OwnerProjectionCursorLifecycle,
    pub projection_revision: u64,
    pub projection_digest: String,
    pub content_fingerprint: String,
    pub lease_expires_unix_ms: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerProjectionCursorLifecycle {
    Active,
    Retired,
}

pub(crate) fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

pub(crate) fn load() -> anyhow::Result<OwnerProjectionCursorFile> {
    let _thread_guard = lock_store();
    let data_path = path();
    let _process_guard = ExclusiveFileLock::acquire_for_data_path(&data_path)?;
    load_and_migrate_unlocked(&data_path)
}

pub(crate) fn update<T>(
    mutate: impl FnOnce(&mut OwnerProjectionCursorFile) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let _thread_guard = lock_store();
    let data_path = path();
    let _process_guard = ExclusiveFileLock::acquire_for_data_path(&data_path)?;
    let mut file = load_and_migrate_unlocked(&data_path)?;
    let output = mutate(&mut file)?;
    save_unlocked(&data_path, &file)?;
    Ok(output)
}

#[cfg(test)]
pub(crate) fn replace(file: &OwnerProjectionCursorFile) -> anyhow::Result<()> {
    let _thread_guard = lock_store();
    let data_path = path();
    let _process_guard = ExclusiveFileLock::acquire_for_data_path(&data_path)?;
    save_unlocked(&data_path, file)
}

fn lock_store() -> MutexGuard<'static, ()> {
    STORE_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn load_and_migrate_unlocked(path: &std::path::Path) -> anyhow::Result<OwnerProjectionCursorFile> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OwnerProjectionCursorFile::default());
        }
        Err(error) => return Err(error.into()),
    };
    let value: serde_json::Value = serde_json::from_slice(&data).map_err(|error| {
        anyhow::anyhow!("parse owner projection store {}: {error}", path.display())
    })?;
    match value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version) if version == u64::from(SCHEMA_VERSION) => {
            let file: OwnerProjectionCursorFile =
                serde_json::from_value(value).map_err(|error| {
                    anyhow::anyhow!(
                        "parse owner projection schema {SCHEMA_VERSION} at {}: {error}",
                        path.display()
                    )
                })?;
            file.validate()?;
            Ok(file)
        }
        Some(version) => anyhow::bail!(
            "unsupported owner projection schema {version} at {}; expected {SCHEMA_VERSION}",
            path.display()
        ),
        None => migrate_legacy_schema_unlocked(path, value),
    }
}

fn save_unlocked(path: &std::path::Path, file: &OwnerProjectionCursorFile) -> anyhow::Result<()> {
    file.validate()?;
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(file)?;
    atomic_write_with_permissions(path, &json, WritePermissions::OwnerReadWrite).map_err(Into::into)
}

#[derive(Deserialize)]
struct LegacyOwnerProjectionCursorFile {
    #[serde(default)]
    projections: Vec<LegacyOwnerProjectionCursor>,
}

#[derive(Deserialize)]
struct LegacyOwnerProjectionCursor {
    owner_ura: String,
    host_device_ura: String,
    projection_revision: u64,
    projection_digest: String,
    content_fingerprint: String,
    lease_expires_unix_ms: i64,
    updated_at: String,
}

fn migrate_legacy_schema_unlocked(
    path: &std::path::Path,
    value: serde_json::Value,
) -> anyhow::Result<OwnerProjectionCursorFile> {
    let legacy: LegacyOwnerProjectionCursorFile =
        serde_json::from_value(value).map_err(|error| {
            anyhow::anyhow!(
                "migrate legacy owner projection store {}: {error}",
                path.display()
            )
        })?;
    let file = OwnerProjectionCursorFile {
        schema_version: SCHEMA_VERSION,
        projections: legacy
            .projections
            .into_iter()
            .map(|cursor| OwnerProjectionCursor {
                owner_ura: cursor.owner_ura,
                host_device_ura: cursor.host_device_ura,
                generation: 1,
                lifecycle: OwnerProjectionCursorLifecycle::Active,
                projection_revision: cursor.projection_revision,
                projection_digest: cursor.projection_digest,
                content_fingerprint: cursor.content_fingerprint,
                lease_expires_unix_ms: cursor.lease_expires_unix_ms,
                updated_at: cursor.updated_at,
            })
            .collect(),
    };
    file.validate()?;
    save_unlocked(path, &file)?;
    Ok(file)
}

impl OwnerProjectionCursorFile {
    fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            anyhow::bail!(
                "owner projection file schema {} does not match writer schema {SCHEMA_VERSION}",
                self.schema_version
            );
        }
        let mut owners = std::collections::BTreeSet::new();
        for cursor in &self.projections {
            if cursor.owner_ura.trim().is_empty() || cursor.host_device_ura.trim().is_empty() {
                anyhow::bail!("owner projection cursor contains an empty owner or host URA");
            }
            if cursor.generation == 0 || cursor.projection_revision == 0 {
                anyhow::bail!(
                    "owner projection cursor for `{}` has a zero generation or revision",
                    cursor.owner_ura
                );
            }
            if !owners.insert(cursor.owner_ura.as_str()) {
                anyhow::bail!(
                    "owner projection store contains duplicate cursor `{}`",
                    cursor.owner_ura
                );
            }
        }
        Ok(())
    }

    pub(crate) fn cursor_for(&self, owner_ura: &str) -> Option<&OwnerProjectionCursor> {
        self.projections.iter().find(|p| p.owner_ura == owner_ura)
    }

    pub(crate) fn active_cursor_for(&self, owner_ura: &str) -> Option<&OwnerProjectionCursor> {
        self.cursor_for(owner_ura)
            .filter(|cursor| cursor.lifecycle == OwnerProjectionCursorLifecycle::Active)
    }

    pub(crate) fn upsert(&mut self, cursor: OwnerProjectionCursor) {
        if let Some(existing) = self
            .projections
            .iter_mut()
            .find(|p| p.owner_ura == cursor.owner_ura)
        {
            *existing = cursor;
        } else {
            self.projections.push(cursor);
        }
        self.projections
            .sort_by(|a, b| a.owner_ura.cmp(&b.owner_ura));
    }

    /// Retire an active cursor while retaining its generation and revision
    /// high-water marks. A future publication for the same URA must advance
    /// beyond both, preventing an old Hub row from becoming current again.
    pub(crate) fn retire(&mut self, owner_ura: &str) -> bool {
        let Some(cursor) = self
            .projections
            .iter_mut()
            .find(|cursor| cursor.owner_ura == owner_ura)
        else {
            return false;
        };
        let was_active = cursor.lifecycle == OwnerProjectionCursorLifecycle::Active;
        cursor.lifecycle = OwnerProjectionCursorLifecycle::Retired;
        was_active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_store_is_migrated_once_without_losing_revision_high_water() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let data_path = path();
        fs::create_dir_all(data_path.parent().unwrap()).unwrap();
        fs::write(
            &data_path,
            br#"{
  "projections": [{
    "owner_ura": "easynet:///r/acme/agent/alice.worker",
    "host_device_ura": "easynet:///r/acme/device/01DEV",
    "projection_revision": 19,
    "projection_digest": "digest-19",
    "content_fingerprint": "fingerprint-19",
    "lease_expires_unix_ms": 0,
    "updated_at": "2026-07-01T00:00:00Z"
  }]
}"#,
        )
        .unwrap();

        let migrated = load().expect("legacy store migrates under the writer lock");
        let cursor = migrated
            .cursor_for("easynet:///r/acme/agent/alice.worker")
            .unwrap();
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert_eq!(cursor.generation, 1);
        assert_eq!(cursor.lifecycle, OwnerProjectionCursorLifecycle::Active);
        assert_eq!(cursor.projection_revision, 19);

        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&data_path).unwrap()).unwrap();
        assert_eq!(persisted["schema_version"], SCHEMA_VERSION);
        assert_eq!(persisted["projections"][0]["generation"], 1);
        assert_eq!(persisted["projections"][0]["lifecycle"], "active");
        assert_eq!(load().unwrap(), migrated);
    }

    #[test]
    fn save_load_round_trip_preserves_owner_cursor() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(OwnerProjectionCursor {
            owner_ura: "easynet:///r/acme/device/01DEV".into(),
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            generation: 1,
            lifecycle: OwnerProjectionCursorLifecycle::Active,
            projection_revision: 7,
            projection_digest: "abc".into(),
            content_fingerprint: "def".into(),
            lease_expires_unix_ms: 0,
            updated_at: "2026-06-07T00:00:00Z".into(),
        });

        replace(&file).expect("save cursor");
        assert_eq!(load().expect("load cursor"), file);
    }

    #[test]
    fn upsert_replaces_by_owner_and_keeps_stable_order() {
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(OwnerProjectionCursor {
            owner_ura: "z".into(),
            host_device_ura: "host".into(),
            generation: 1,
            lifecycle: OwnerProjectionCursorLifecycle::Active,
            projection_revision: 1,
            projection_digest: "old".into(),
            content_fingerprint: "old".into(),
            lease_expires_unix_ms: 0,
            updated_at: "t1".into(),
        });
        file.upsert(OwnerProjectionCursor {
            owner_ura: "a".into(),
            host_device_ura: "host".into(),
            generation: 1,
            lifecycle: OwnerProjectionCursorLifecycle::Active,
            projection_revision: 1,
            projection_digest: "digest".into(),
            content_fingerprint: "fingerprint".into(),
            lease_expires_unix_ms: 0,
            updated_at: "t2".into(),
        });
        file.upsert(OwnerProjectionCursor {
            owner_ura: "z".into(),
            host_device_ura: "host".into(),
            generation: 1,
            lifecycle: OwnerProjectionCursorLifecycle::Active,
            projection_revision: 2,
            projection_digest: "new".into(),
            content_fingerprint: "new".into(),
            lease_expires_unix_ms: 0,
            updated_at: "t3".into(),
        });

        assert_eq!(file.projections.len(), 2);
        assert_eq!(file.projections[0].owner_ura, "a");
        assert_eq!(file.projections[1].projection_revision, 2);
        assert_eq!(file.cursor_for("z").unwrap().projection_digest, "new");
    }

    #[test]
    fn retire_preserves_owner_high_water_and_reports_active_transition() {
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(OwnerProjectionCursor {
            owner_ura: "easynet:///r/acme/agent/alice.claude".into(),
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            generation: 4,
            lifecycle: OwnerProjectionCursorLifecycle::Active,
            projection_revision: 3,
            projection_digest: "d".into(),
            content_fingerprint: "f".into(),
            lease_expires_unix_ms: 0,
            updated_at: "t".into(),
        });

        assert!(file.retire("easynet:///r/acme/agent/alice.claude"));
        let retired = file
            .cursor_for("easynet:///r/acme/agent/alice.claude")
            .expect("retirement retains high-water cursor");
        assert_eq!(retired.generation, 4);
        assert_eq!(retired.projection_revision, 3);
        assert_eq!(retired.lifecycle, OwnerProjectionCursorLifecycle::Retired);
        assert!(!file.retire("easynet:///r/acme/agent/alice.claude"));
    }
}
