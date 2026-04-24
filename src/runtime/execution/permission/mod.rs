// EasyNet CLI — Execution / Permission sub-service
// =================================================
//
// File: src/runtime/execution/permission/mod.rs
// Description: Permission sub-service skeleton. PR-PERM fills in
//              the PermissionBroker trait + AllowAllBroker +
//              SubscriberBroker. v1 exposes the handle shape and
//              the v2-reserved `AskContext.capability_claim` field.
//
// v1 contract (per docs/rfc/permission-broker-v1.md)
// --------------------------------------------------
// v1 permission is an *approval broker* — interactive human
// judgement before a sensitive action. It is NOT capability
// security. PR-PERM pins the four "v1 does not guarantee" clauses
// in the RFC; this module's doc mirrors them at the code level so
// a future reader reaching for `capability_claim` is reminded why
// it is always `None` in v1.
//
// Isolation rule: must NOT import from sibling execution sub-
// services. Cross-service talk goes through the Kernel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::domain::{PermissionDecision, PermissionId, PermissionRequest};

/// Context a handler supplies to the broker at admission time.
/// PR-PERM extends this with the current dispatch's run_id and
/// sensitivity classification.
///
/// `capability_claim` is reserved for v2 signed invocation. In v1
/// it is always `None`; setting it to `Some(_)` in v1 has no
/// effect but future callers should not pre-populate it either.
pub struct AskContext {
    pub prompt: String,
    /// v2 capability-claim payload (AXIOM §6.3). v1 always None.
    #[allow(dead_code)]
    pub capability_claim: Option<CapabilityClaim>,
}

/// Opaque v2 capability-claim placeholder. v1 defines the type so
/// `AskContext` compiles; v2 will expand it to a signed-envelope
/// struct under AXIOM §6.3.
pub struct CapabilityClaim {
    #[allow(dead_code)]
    pub(crate) _v2_signed_bytes: Vec<u8>,
}

/// Permission sub-service handle. v1 is a zero-state stub;
/// PR-PERM implements the pending queue + broker trait.
#[derive(Debug, Default)]
pub struct PermissionService {
    // PR-PERM: pending queue + broker trait object
}

impl PermissionService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending(&self) -> Vec<PermissionRequest> {
        Vec::new()
    }

    pub fn decide(&self, _id: &PermissionId, _decision: PermissionDecision) -> anyhow::Result<()> {
        Ok(())
    }
}
