// EasyNet CLI — Execution / Schedule sub-service
// ===============================================
//
// File: src/runtime/execution/schedule/mod.rs
// Description: Schedule sub-service skeleton. PR-SCHED fills in
//              the cron store + tick runner + misfire policy
//              dispatcher; v1 holds the empty handle.
//
// Plan v10.3 C* unity constraint (PR-INVOCATION-EXEC-UNITY):
// when PR-SCHED lands, the tick loop MUST construct a full
// Invocation (caller, callee, ability, subject=schedule_id,
// causal_context=Scalar(last_receipt) or Null, args) and route
// through `Kernel::invoke`. It MUST NOT call `run_mission_inproc`
// directly. `scripts/check-invocation-unity.sh` greps for that
// violation.
//
// Isolation rule: must NOT import from sibling execution sub-
// services. Cross-service talk goes through the Kernel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::domain::{ScheduleEntry, ScheduleId};

#[derive(Debug, Default)]
pub struct ScheduleService {
    // PR-SCHED: JSON-file-backed cron store + tick state
}

impl ScheduleService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> Vec<ScheduleEntry> {
        Vec::new()
    }

    pub fn add(&self, _entry: ScheduleEntry) -> anyhow::Result<ScheduleId> {
        anyhow::bail!("schedule.add not yet implemented (pending PR-SCHED)")
    }

    pub fn remove(&self, _id: &ScheduleId) -> anyhow::Result<()> {
        anyhow::bail!("schedule.remove not yet implemented (pending PR-SCHED)")
    }

    pub fn enable(&self, _id: &ScheduleId, _enabled: bool) -> anyhow::Result<()> {
        anyhow::bail!("schedule.enable not yet implemented (pending PR-SCHED)")
    }
}
