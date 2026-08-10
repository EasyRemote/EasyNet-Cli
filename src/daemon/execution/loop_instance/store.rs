// EasyNet CLI — Loop disk-backed JSON store
// ==========================================
//
// File: src/daemon/execution/loop_instance/store.rs
// Description: One JSON file per loop instance under
//              `~/.easynet/tenants/<tenant>/loops/<id>.json`.
//              Same atomic-overwrite semantics as the schedule
//              store. Corrupt or obsolete records fail boot explicitly;
//              durable automation is never silently discarded.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::core::domain::LoopId;
use crate::core::domain::{LoopInstance, TenantId};
use crate::daemon::persistence::tenant_paths::{ensure, TenantKind};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct OnDisk {
    schema_version: u32,
    #[serde(flatten)]
    instance: LoopInstance,
}

pub struct LoopStore {
    dir: PathBuf,
}

impl LoopStore {
    pub fn open(tenant: &TenantId) -> anyhow::Result<Self> {
        let dir = ensure(tenant, TenantKind::Loops)?;
        Ok(Self { dir })
    }

    #[cfg(test)]
    fn open_at(dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir)?;
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
            let ent = ent.context("read loop directory entry")?;
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let body = std::fs::read_to_string(&path)
                .with_context(|| format!("read loop record {}", path.display()))?;
            let record: OnDisk = serde_json::from_str(&body)
                .with_context(|| format!("parse loop record {}", path.display()))?;
            if record.schema_version != SCHEMA_VERSION {
                anyhow::bail!(
                    "loop record {} schema_version {} is not supported by this runtime",
                    path.display(),
                    record.schema_version
                );
            }
            out.push(record.instance);
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
    use crate::core::domain::{AgentId, DeferredInvocationAuthority, LoopState};

    fn instance(id: &str) -> LoopInstance {
        LoopInstance {
            id: LoopId::new(id),
            tenant: TenantId::default_v1(),
            worker_agent: AgentId::new("alice"),
            authority: DeferredInvocationAuthority {
                accountable_user_ura: crate::core::ura::user_ura("default", "user-1"),
                creator_invocation_id: "test-loop-create".to_string(),
                controller_callee_ura: crate::core::ura::device_agent_ura(
                    "default",
                    "self",
                    crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
                ),
                target_callee_ura: crate::core::ura::agent_ura("default", "user-1", "alice"),
                execution_host_ura: crate::core::ura::device_ura("default", "self"),
            },
            verify_expr: "true".into(),
            body_prompt: "do work".into(),
            max_iters: 5,
            current_iter: 0,
            state: LoopState::Pending,
            invocation_ledger: Vec::new(),
            last_body_output: None,
            last_verify_output: None,
        }
    }

    #[test]
    fn save_then_load_round_trips_instance() {
        let home = tempfile::tempdir().expect("loop store test directory");
        let store = LoopStore::open_at(home.path().join("loops")).unwrap();
        store.save(&instance("rt-1")).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, LoopId::new("rt-1"));
        assert_eq!(loaded[0].state, LoopState::Pending);
        store.delete(&loaded[0].id).unwrap();
    }

    #[test]
    fn load_all_rejects_corrupt_or_obsolete_records() {
        let home = tempfile::tempdir().expect("loop store test directory");
        let dir = home.path().join("loops");
        let store = LoopStore::open_at(dir.clone()).unwrap();
        std::fs::write(dir.join("corrupt.json"), "{ not json").unwrap();
        let error = store
            .load_all()
            .expect_err("corrupt durable loop state must fail closed");
        assert!(error.to_string().contains("corrupt.json"), "{error:#}");

        std::fs::remove_file(dir.join("corrupt.json")).unwrap();
        let mut obsolete = serde_json::to_value(OnDisk {
            schema_version: SCHEMA_VERSION,
            instance: instance("obsolete"),
        })
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
        obsolete.remove("schema_version");
        std::fs::write(
            dir.join("obsolete.json"),
            serde_json::to_string_pretty(&obsolete).unwrap(),
        )
        .unwrap();
        let error = store
            .load_all()
            .expect_err("missing loop schema version must fail closed");
        assert!(error.to_string().contains("obsolete.json"), "{error:#}");
    }
}
