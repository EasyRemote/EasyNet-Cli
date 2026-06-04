// EasyNet CLI — media handlers (PR3a onwards)
// =============================================
//
// File: src/runtime/agents/media/mod.rs
//
// Real-handler home for the eight RFC-005 v3.2 media abilities.
// PR2 stubbed every media handler in `media_abilities.rs`; PR3a
// onwards swaps individual stubs out for real implementations one
// at a time as consumers materialise.
//
// Module layout
// -------------
// One file per ability that gets a real handler. Each module
// exports a single `register(reg: &mut AxonAbilityCatalog)`
// fn that routes the corresponding `media_abilities::ABILITY_*`
// name through `register_*_with_envelope` (per
// **INV-SUBJECT-ENVELOPE**: media handlers MUST take the AXIOM
// 7-tuple `subject` from the envelope, not from args).
//
// What stays in `media_abilities.rs`
// ----------------------------------
// The single source of truth for ability *names*, descriptions,
// input schemas, RFC-006 class assignments, and dispatch shapes
// (the `ABILITIES` table). Real handlers in this directory pull
// the ability name from those constants so a rename in the table
// trips a compile error here, not a silent registration miss.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

/// `camera.snapshot` (RFC-005 v3.2 A3) — real handler. PR3a's
/// vertical slice from ability invocation through resource
/// resolution to base64-encoded JPEG receipt body. Synthetic
/// frame backend (no hardware); the real cpal/nokhwa swap is a
/// later PR.
pub mod camera_snapshot;

#[cfg(target_os = "macos")]
mod avfoundation_camera;

/// `screen.snapshot` (RFC-005 v3.2 A8) — real handler. PR3
/// vertical slice mirroring `camera_snapshot`'s shape. Real
/// backend (`XcapBackend`) captures the primary monitor; tests
/// use `SyntheticScreenBackend` so the suite runs hardware-free.
pub mod screen_snapshot;

/// `mic.subscribe` (RFC-005 v3.2 A1) — real handler. cpal-backed
/// `CpalMicBackend` opens the default input device on a
/// dedicated thread and broadcasts S16LE PCM frames through a
/// `tokio::sync::broadcast`. Tests use `SyntheticMicBackend`
/// which emits a single zero-filled frame.
pub mod mic_subscribe;
pub mod resource_bootstrap;
pub mod resource_subject;
