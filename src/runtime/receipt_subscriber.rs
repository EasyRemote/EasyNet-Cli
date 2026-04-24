// EasyNet CLI — Receipt Subscriber (v2 extension point)
// ======================================================
//
// File: src/runtime/receipt_subscriber.rs
// Description: v1-trait-only placeholder for the v2 extension point
//              that lets runtime consumers observe Receipts as they
//              are written.
//
// Why this exists in v1 if no one consumes it
// -------------------------------------------
// Plan v10.4 D1 / v10.5 R1 classify v1 as a record system (S1–S4
// semantic invariants unmet). The practical consequence: v1's
// `Receipt` is a durable record only — no runtime code consumes it
// to drive scheduling, replay, or causal enforcement.
//
// This trait is the surface those v2 consumers will hang off. Shipping
// the surface in v1 keeps the eventual v2 landing from touching every
// call site that writes Receipts: v2 just registers one or more
// implementations; v1 runs with an empty registry.
//
// No implementations live here on purpose. Adding one would violate
// the plan's explicit "v1 receipt is not a runtime driver" boundary.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::invocation::Receipt;

/// v2 extension point. A future replay engine / causal scheduler /
/// audit consumer implements this trait and registers itself with
/// the runtime; every terminal Receipt is forwarded to all
/// registered subscribers after it lands on disk.
///
/// In v1 the registry is empty (see `empty_registry` below) and
/// no runtime code invokes `on_receipt` at any call site. The
/// trait surface is frozen so v2 can land without a second-pass
/// API change.
pub trait ReceiptSubscriber: Send + Sync {
    fn on_receipt(&self, receipt: &Receipt);
}

/// The v1 empty registry. Always returns an empty Vec; v2 will
/// replace this with a real registry backed by `Arc<RwLock<...>>`.
///
/// Keeping the function here (rather than inlining `Vec::new()` at
/// call sites) pins the shape of the v1→v2 migration: the only
/// thing that changes is this function's body and the addition of
/// a registration API alongside it.
pub fn empty_registry() -> Vec<Box<dyn ReceiptSubscriber>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_has_no_subscribers_in_v1() {
        // This test pins the v1 invariant "receipts are not runtime
        // drivers" at the call-site level: no subscriber can observe
        // a Receipt because the registry is empty by construction.
        // A future v2 change that populates the registry will flip
        // this assertion and the accompanying plan doc updates.
        assert!(empty_registry().is_empty());
    }
}
