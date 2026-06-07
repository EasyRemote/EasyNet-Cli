// EasyNet CLI — Owner Projection Cursor Store (AXON-RFC-005 Phase C)
// ==================================================================
//
// Owns ~/.easynet/owner-projections.json. The file is intentionally
// only a publication cursor: protocol semantics such as ability
// summaries, descriptor hashes, and projection digests live in
// runtime::owner_projection. Keeping this layer dumb prevents the
// on-disk schema from becoming a second resolver implementation.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};

const FILE_NAME: &str = "owner-projections.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OwnerProjectionCursorFile {
    #[serde(default)]
    pub projections: Vec<OwnerProjectionCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OwnerProjectionCursor {
    pub owner_ura: String,
    pub host_device_ura: String,
    pub projection_revision: u64,
    pub projection_digest: String,
    pub content_fingerprint: String,
    pub lease_expires_unix_ms: i64,
    pub updated_at: String,
}

pub(crate) fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

pub(crate) fn load() -> anyhow::Result<OwnerProjectionCursorFile> {
    let path = path();
    if !path.exists() {
        return Ok(OwnerProjectionCursorFile::default());
    }
    let data = fs::read(&path)?;
    Ok(serde_json::from_slice(&data)?)
}

pub(crate) fn save(file: &OwnerProjectionCursorFile) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(file)?;
    atomic_write_with_permissions(&path(), &json, WritePermissions::OwnerReadWrite)
}

impl OwnerProjectionCursorFile {
    pub(crate) fn cursor_for(&self, owner_ura: &str) -> Option<&OwnerProjectionCursor> {
        self.projections.iter().find(|p| p.owner_ura == owner_ura)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_round_trip_preserves_owner_cursor() {
        let _home = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(OwnerProjectionCursor {
            owner_ura: "easynet:///r/acme/device/01DEV".into(),
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            projection_revision: 7,
            projection_digest: "abc".into(),
            content_fingerprint: "def".into(),
            lease_expires_unix_ms: 0,
            updated_at: "2026-06-07T00:00:00Z".into(),
        });

        save(&file).expect("save cursor");
        assert_eq!(load().expect("load cursor"), file);
    }

    #[test]
    fn upsert_replaces_by_owner_and_keeps_stable_order() {
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(OwnerProjectionCursor {
            owner_ura: "z".into(),
            host_device_ura: "host".into(),
            projection_revision: 1,
            projection_digest: "old".into(),
            content_fingerprint: "old".into(),
            lease_expires_unix_ms: 0,
            updated_at: "t1".into(),
        });
        file.upsert(OwnerProjectionCursor {
            owner_ura: "a".into(),
            host_device_ura: "host".into(),
            projection_revision: 1,
            projection_digest: "digest".into(),
            content_fingerprint: "fingerprint".into(),
            lease_expires_unix_ms: 0,
            updated_at: "t2".into(),
        });
        file.upsert(OwnerProjectionCursor {
            owner_ura: "z".into(),
            host_device_ura: "host".into(),
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
}
