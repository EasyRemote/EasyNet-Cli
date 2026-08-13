// EasyNet CLI — media resource abilities
// ======================================
//
// File: src/daemon/ability/builtins/resources/media/mod.rs
//
// Resource-plane home for the RFC-005 v3.2 media abilities. The
// `abilities` table owns public names, metadata, and still-unwired
// stubs; real backend modules own registration for their names.
//
// Module layout
// -------------
// One file per ability that gets a real handler. Each module
// exports a single `register(reg: &mut AxonAbilityCatalog)`
// fn that routes the corresponding `media::ABILITY_*`
// name through `register_*_with_envelope` (per
// **INV-SUBJECT-ENVELOPE**: media handlers MUST take the AXIOM
// 7-tuple `subject` from the envelope, not from args).
//
// What stays in `abilities.rs`
// ----------------------------
// The single source of truth for ability *names*, descriptions,
// input schemas, RFC-006 class assignments, and dispatch shapes.
// Real handlers in this directory pull the ability name from
// those constants so a rename in the table trips a compile error
// here, not a silent registration miss.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

mod abilities;
pub use abilities::*;

/// `camera.snapshot`, `camera.subscribe`, `camera.record_start`, and
/// `camera.record_stop` — real camera handlers. Production uses
/// native AVFoundation still capture on macOS and nokhwa-backed
/// camera IO on other platforms; tests use `SyntheticBackend` so the
/// suite remains hardware-free.
#[cfg(any(feature = "native-media", feature = "headless-media"))]
pub mod camera_snapshot;

#[cfg(all(target_os = "macos", feature = "native-media"))]
mod avfoundation_camera;

/// `screen.snapshot` (RFC-005 v3.2 A8) — real handler. PR3
/// vertical slice mirroring `camera_snapshot`'s shape. Real
/// backend (`XcapBackend`) captures the primary monitor; tests
/// use `SyntheticScreenBackend` so the suite runs hardware-free.
#[cfg(any(feature = "native-media", feature = "headless-media"))]
pub mod screen_snapshot;

/// `mic.subscribe` (RFC-005 v3.2 A1) — real handler. cpal-backed
/// `CpalMicBackend` opens the default input device on a
/// dedicated thread and broadcasts S16LE PCM frames through a
/// `tokio::sync::broadcast`. Tests use `SyntheticMicBackend`
/// which emits a single zero-filled frame.
#[cfg(any(feature = "native-media", feature = "headless-media"))]
pub mod mic_subscribe;
#[cfg(feature = "native-media")]
pub mod resource_bootstrap;
#[cfg(not(feature = "native-media"))]
pub mod resource_bootstrap {
    use crate::daemon::persistence::resources::ResourceEntry;

    pub const REMOTE_TARGET_FRESHNESS_TTL_MS: u64 = 5_000;

    #[derive(Debug, Clone, PartialEq)]
    pub struct RemoteTargetInventoryRefresh {
        pub observed_at_ms: u64,
        pub freshness_ttl_ms: u64,
        pub resources: Vec<ResourceEntry>,
        pub retired_count: usize,
        pub screen_target_discovery_available: bool,
    }

    /// Headless runtime builds do not probe host media devices. Public media
    /// descriptors remain registered by `abilities.rs` as unavailable stubs,
    /// so callers receive canonical invocation receipts instead of build-time
    /// GUI/Wayland dependencies leaking into non-media products.
    pub fn seed_default_device_resources(
        _realm: &str,
        _owner_agent: &str,
    ) -> anyhow::Result<usize> {
        Ok(0)
    }

    pub fn refresh_remote_targets(
        _realm: &str,
        _owner_agent: &str,
    ) -> anyhow::Result<RemoteTargetInventoryRefresh> {
        Ok(RemoteTargetInventoryRefresh {
            observed_at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
            freshness_ttl_ms: REMOTE_TARGET_FRESHNESS_TTL_MS,
            resources: Vec::new(),
            retired_count: 0,
            screen_target_discovery_available: false,
        })
    }

    pub fn watch_remote_target_inventory(
        realm: &str,
        owner_agent: &str,
    ) -> anyhow::Result<RemoteTargetInventoryRefresh> {
        refresh_remote_targets(realm, owner_agent)
    }
}
pub mod resource_subject;
