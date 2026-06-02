// EasyNet CLI — Loop disk-backed JSON store
// ==========================================
//
// File: src/runtime/execution/loop_instance/store.rs
// Description: One JSON file per loop instance under
//              `~/.easynet/tenants/<tenant>/loops/<id>.json`.
//              Same atomic-overwrite semantics as the schedule
//              store; same one-corrupt-file-must-not-poison-boot
//              tolerance.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::persistence::tenant_paths::{ensure, TenantKind};
#[cfg(test)]
use crate::runtime::domain::LoopId;
use crate::runtime::domain::{LoopInstance, TenantId};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(flatten)]
    instance: LoopInstance,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

pub struct LoopStore {
    dir: PathBuf,
}

impl LoopStore {
    pub fn open(tenant: &TenantId) -> anyhow::Result<Self> {
        let dir = ensure(tenant, TenantKind::Loops)?;
        Ok(Self { dir })
    }

    pub fn load_all(&self) -> anyhow::Result<Vec<LoopInstance>> {
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
                    eprintln!("[loop] read_dir entry error: {e}");
                    continue;
                }
            };
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<OnDisk>(&s) {
                    Ok(d) => out.push(d.instance),
                    Err(e) => {
                        eprintln!("[loop] failed to parse {}: {e} (skipping)", path.display())
                    }
                },
                Err(e) => eprintln!("[loop] failed to read {}: {e} (skipping)", path.display()),
            }
        }
        Ok(out)
    }

    pub fn save(&self, instance: &LoopInstance) -> anyhow::Result<()> {
        let on_disk = OnDisk {
            schema_version: SCHEMA_VERSION,
            instance: instance.clone(),
        };
        let body = serde_json::to_string_pretty(&on_disk)?;
        let final_path = self.dir.join(format!("{}.json", instance.id.as_str()));
        let tmp_path = self.dir.join(format!("{}.json.tmp", instance.id.as_str()));
        std::fs::write(&tmp_path, body)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn delete(&self, id: &LoopId) -> anyhow::Result<()> {
        let p = self.dir.join(format!("{}.json", id.as_str()));
        if p.exists() {
            std::fs::remove_file(p)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::domain::{AgentId, LoopState};

    fn temp_tenant() -> TenantId {
        TenantId::new(format!("test-loop-{}", uuid::Uuid::new_v4()))
    }

    fn instance(id: &str) -> LoopInstance {
        LoopInstance {
            id: LoopId::new(id),
            tenant: TenantId::default_v1(),
            worker_agent: AgentId::new("alice"),
            verify_expr: "true".into(),
            body_prompt: "do work".into(),
            max_iters: 5,
            current_iter: 0,
            state: LoopState::Pending,
            last_body_output: None,
            last_verify_output: None,
        }
    }

    #[test]
    fn save_then_load_round_trips_instance() {
        let tenant = temp_tenant();
        let store = LoopStore::open(&tenant).unwrap();
        store.save(&instance("rt-1")).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, LoopId::new("rt-1"));
        assert_eq!(loaded[0].state, LoopState::Pending);
        store.delete(&loaded[0].id).unwrap();
    }
}
