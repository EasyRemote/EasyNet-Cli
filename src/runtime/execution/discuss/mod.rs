// EasyNet CLI — Execution / Discuss sub-service
// ==============================================
//
// File: src/runtime/execution/discuss/mod.rs
// Description: Discuss sub-service skeleton. PR-DISCUSS fills in
//              the room store + turn broadcast; v1 holds the
//              empty handle.
//
// Isolation rule: must NOT import from sibling execution sub-
// services. Cross-service talk goes through the Kernel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::domain::{DiscussRoom, RoomId};

#[derive(Debug, Default)]
pub struct DiscussService {
    // PR-DISCUSS: room registry + per-room broadcast channel
}

impl DiscussService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> Vec<DiscussRoom> {
        Vec::new()
    }

    pub fn create(
        &self,
        _participants: Vec<String>,
        _topic: Option<String>,
    ) -> anyhow::Result<RoomId> {
        anyhow::bail!("discuss.create not yet implemented (pending PR-DISCUSS)")
    }
}
