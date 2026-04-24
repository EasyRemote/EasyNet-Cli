// EasyNet CLI — Execution / Loop sub-service
// ===========================================
//
// File: src/runtime/execution/loop_instance/mod.rs
// Description: Loop sub-service skeleton. PR-LOOP fills in the
//              loop-instance registry + Status/Terminal observer;
//              the EAL Stage 3 executor (src/eal/interpreter.rs
//              fn execute_loop) stays the underlying engine.
//
// Loop boundary (per docs/rfc/loop-primitive-v1.md)
// -------------------------------------------------
// Loop is a local control primitive — a "worker + verify + retry"
// closure bounded by `max_iters`. It is NOT a planner, an agent
// team, a cost-aware router, or cross-loop coordination. A future
// planner will consume `LoopInstance` as one primitive among
// several; it will not live inside this module.
//
// Plan v10.3 C* unity: when PR-LOOP lands, each iteration's body /
// verify step becomes its own Invocation routed through
// `Kernel::invoke`. The loop controller here does not drive steps
// directly.
//
// Isolation rule: must NOT import from sibling execution sub-
// services. Cross-service talk goes through the Kernel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::domain::{LoopId, LoopInstance};

#[derive(Debug, Default)]
pub struct LoopService {
    // PR-LOOP: loop-instance registry + terminal observer
}

impl LoopService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self, _id: &LoopId) -> Option<LoopInstance> {
        None
    }

    pub fn cancel(&self, _id: &LoopId) -> anyhow::Result<()> {
        anyhow::bail!("loop.cancel not yet implemented (pending PR-LOOP)")
    }
}
