// EasyNet CLI — Execution / Session sub-service
// ==============================================
//
// File: src/runtime/execution/session/mod.rs
// Description: Session sub-service skeleton. PR-ATTACH fills in the
//              live-session tracker + timeline broadcast; this file
//              holds the empty handle so the Kernel can be
//              instantiated before the feature PR lands.
//
// Isolation rule: must NOT import from `execution::permission`,
// `execution::discuss`, `execution::schedule`, `execution::loop_instance`.
// Cross-service talk goes through the Kernel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::domain::{Session, SessionId};

/// Session sub-service handle. v1 is a zero-state stub; PR-ATTACH
/// will wire it to the existing `runtime::session::Session` store
/// plus a live-list index.
#[derive(Debug, Default)]
pub struct SessionService {
    // PR-ATTACH: live-session index + broadcast channel registry
}

impl SessionService {
    pub fn new() -> Self {
        Self::default()
    }

    /// List currently-active sessions. v1 returns empty;
    /// PR-ATTACH implements.
    pub fn list_active(&self) -> Vec<Session> {
        Vec::new()
    }

    pub fn get(&self, _id: &SessionId) -> Option<Session> {
        None
    }
}
