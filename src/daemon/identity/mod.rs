// EasyNet CLI - daemon identity domain
// ====================================
//
// File: src/daemon/identity/mod.rs
// Description: Daemon-owned identity handles and signing clients.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod local_invocation;
pub(crate) mod receipt_signing;
pub mod self_identity;
mod signer_policy;

pub(crate) use signer_policy::signer_policy_ref;
