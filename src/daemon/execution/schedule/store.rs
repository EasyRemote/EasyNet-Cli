// EasyNet CLI — Schedule disk-backed JSON store
// ==============================================
//
// File: src/daemon/execution/schedule/store.rs
// Description: One JSON file per schedule under
//              `~/.easynet/tenants/<tenant>/schedules/<id>.json`.
//              The file format is `serde_json::to_string_pretty`
//              of a `ScheduleEntry` plus a leading `schema_version`.
//
// Why one file per schedule
// -------------------------
// `git diff` friendliness: enabling/disabling one schedule
// rewrites one file, not the whole directory. A future
// `easynet schedule export` / `import` workflow can move
// individual files. Atomic overwrite (`tmp + rename`) keeps a
// concurrent reader from seeing a partial write.
//
// schema_version handling
// -----------------------
// v1 writes `1`; readers require it. Missing schema facts are
// obsolete state, not permission to repair legacy files at every
// read site.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::core::domain::{ScheduleEntry, ScheduleId, TenantId};
use crate::daemon::persistence::tenant_paths::{ensure, TenantKind};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct OnDisk {
    schema_version: u32,
    #[serde(flatten)]
    entry: ScheduleEntry,
}

/// JSON-file store for one tenant's schedules.
pub struct ScheduleStore {
    dir: PathBuf,
}

impl ScheduleStore {
    /// Create or resolve the store directory for `tenant`. v1 only
    /// creates the directory; entries are read/written lazily.
    pub fn open(tenant: &TenantId) -> anyhow::Result<Self> {
        let dir = ensure(tenant, TenantKind::Schedules)?;
        Ok(Self { dir })
    }

    /// Read every `*.json` file under the store's directory and
    /// parse into `ScheduleEntry`. Files that fail to parse are
    /// logged to stderr and skipped — one corrupt schedule must
    /// not poison the rest of the daemon.
    pub fn load_all(&self) -> anyhow::Result<Vec<ScheduleEntry>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for ent in entries {
            let ent = match ent {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[schedule] read_dir entry error: {e}");
                    continue;
                }
            };
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match parse_on_disk_schedule(&s) {
                    Ok(d) => out.push(d.entry),
                    Err(e) => eprintln!(
                        "[schedule] failed to parse {}: {e} (skipping)",
                        path.display()
                    ),
                },
                Err(e) => eprintln!(
                    "[schedule] failed to read {}: {e} (skipping)",
                    path.display()
                ),
            }
        }
        Ok(out)
    }

    /// Atomic overwrite: write to `<id>.json.tmp`, then rename.
    /// A reader observing the directory mid-write either sees the
    /// prior file (atomic move) or the new one — never a partial.
    pub fn save(&self, entry: &ScheduleEntry) -> anyhow::Result<()> {
        let on_disk = OnDisk {
            schema_version: SCHEMA_VERSION,
            entry: entry.clone(),
        };
        let body = serialize_on_disk_schedule(&on_disk)?;
        let final_path = self.dir.join(format!("{}.json", entry.id.as_str()));
        let tmp_path = self.dir.join(format!("{}.json.tmp", entry.id.as_str()));
        std::fs::write(&tmp_path, body)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Remove the on-disk file for `id`. Returns Ok even when the
    /// file is already absent — the in-memory cache is the
    /// authoritative "does this exist" check; the disk is just
    /// the durable shadow.
    pub fn delete(&self, id: &ScheduleId) -> anyhow::Result<()> {
        let p = self.dir.join(format!("{}.json", id.as_str()));
        if p.exists() {
            std::fs::remove_file(p)?;
        }
        Ok(())
    }

    /// Borrow the directory. Only consumed by this module's tests.
    #[cfg(test)]
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }
}

fn parse_on_disk_schedule(input: &str) -> anyhow::Result<OnDisk> {
    let value: serde_json::Value = serde_json::from_str(input)?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("schedule record must be a JSON object"))?;
    let schema_version = object
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("schedule record missing explicit schema_version"))?;
    if schema_version != u64::from(SCHEMA_VERSION) {
        anyhow::bail!(
            "schedule record schema_version {schema_version} is not supported by this runtime"
        );
    }
    if !object.contains_key("prompt") {
        anyhow::bail!("schedule record missing explicit prompt field");
    }
    Ok(serde_json::from_value(value)?)
}

fn serialize_on_disk_schedule(on_disk: &OnDisk) -> anyhow::Result<String> {
    let mut value = serde_json::to_value(on_disk)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("schedule record serialization must produce an object"))?;
    object
        .entry("prompt".to_string())
        .or_insert(serde_json::Value::Null);
    Ok(serde_json::to_string_pretty(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::domain::{
        AgentId, MisfirePolicy, NodeId, ScheduleEntry, ScheduleId, TenantId,
    };

    fn temp_tenant() -> TenantId {
        // Use a random tenant id per test so concurrent tests
        // don't share the on-disk directory. The path_for_tenant
        // helper roots under `~/.easynet/tenants/<id>/...`.
        TenantId::new(format!("test-sched-{}", uuid::Uuid::new_v4()))
    }

    fn entry(id: &str) -> ScheduleEntry {
        ScheduleEntry {
            id: ScheduleId::new(id),
            tenant: TenantId::default_v1(),
            target_node: NodeId::new("self"),
            target_agent: AgentId::new("alice"),
            cron_expr: "0 9 * * *".into(),
            misfire_policy: MisfirePolicy::Skip,
            catch_up_window_secs: None,
            enabled: true,
            prompt: None,
        }
    }

    #[test]
    fn save_then_load_round_trips_entry() {
        // The point of the on-disk format is round-trip stability;
        // a regression that broke serde rename of `MisfirePolicy`
        // (which uses snake_case) would fail this test.
        let tenant = temp_tenant();
        let store = ScheduleStore::open(&tenant).unwrap();
        let e = entry("rt-1");
        store.save(&e).unwrap();
        let raw = std::fs::read_to_string(store.dir().join("rt-1.json")).unwrap();
        assert!(
            raw.contains("\"prompt\": null"),
            "current schedule schema must write explicit prompt null: {raw}"
        );
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, ScheduleId::new("rt-1"));
        assert_eq!(loaded[0].cron_expr, "0 9 * * *");
        assert_eq!(loaded[0].misfire_policy, MisfirePolicy::Skip);
        // Cleanup.
        store.delete(&loaded[0].id).unwrap();
    }

    #[test]
    fn load_all_skips_unparseable_files_without_failing() {
        // Spirit: a hand-edited corrupt schedule must not stop the
        // daemon from booting. Plant a bogus file alongside a
        // valid one and assert load_all returns the valid one.
        let tenant = temp_tenant();
        let store = ScheduleStore::open(&tenant).unwrap();
        store.save(&entry("good")).unwrap();
        std::fs::write(store.dir().join("bad.json"), "{ not valid json").unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, ScheduleId::new("good"));
        // Cleanup.
        std::fs::remove_dir_all(store.dir()).ok();
    }

    #[test]
    fn load_all_skips_records_missing_current_schema_facts() {
        let tenant = temp_tenant();
        let store = ScheduleStore::open(&tenant).unwrap();
        store.save(&entry("good")).unwrap();

        let missing_prompt = serde_json::to_value(OnDisk {
            schema_version: SCHEMA_VERSION,
            entry: entry("missing-prompt"),
        })
        .unwrap();
        let mut missing_prompt = missing_prompt.as_object().unwrap().clone();
        missing_prompt.remove("prompt");
        std::fs::write(
            store.dir().join("missing-prompt.json"),
            serde_json::to_string_pretty(&missing_prompt).unwrap(),
        )
        .unwrap();

        let mut missing_schema = serde_json::to_value(OnDisk {
            schema_version: SCHEMA_VERSION,
            entry: entry("missing-schema"),
        })
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
        missing_schema.remove("schema_version");
        std::fs::write(
            store.dir().join("missing-schema.json"),
            serde_json::to_string_pretty(&missing_schema).unwrap(),
        )
        .unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, ScheduleId::new("good"));
        std::fs::remove_dir_all(store.dir()).ok();
    }

    #[test]
    fn parse_on_disk_schedule_rejects_unsupported_schema_version() {
        let body = serde_json::to_string(&OnDisk {
            schema_version: SCHEMA_VERSION + 1,
            entry: entry("future"),
        })
        .unwrap();

        let error = parse_on_disk_schedule(&body).expect_err("future schema must fail closed");
        assert!(
            error.to_string().contains("schema_version"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn delete_is_idempotent_when_file_already_absent() {
        let tenant = temp_tenant();
        let store = ScheduleStore::open(&tenant).unwrap();
        store.delete(&ScheduleId::new("nope")).unwrap();
    }
}
