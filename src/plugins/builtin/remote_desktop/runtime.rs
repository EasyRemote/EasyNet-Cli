// EasyNet CLI — remote desktop plugin runtime
// ==============================================
//
// File: src/plugins/builtin/remote_desktop/runtime.rs
// Description: Device-side remote desktop control-plane handlers.
//
// Protocol Responsibility:
// - Implements the EasyNet-Cli side of the Axon-owned remote desktop
//   session contract draft.
// - Requires the acted-on display/window/application resource to be
//   supplied as Envelope.subject, never as JSON arguments.
// - Keeps production media transport explicit: this module creates and
//   tracks sessions, but does not pretend the preview JPEG stream is a
//   WebRTC remote desktop media plane.
//
// Implementation Approach:
// - In-process session store mirrors the existing voice-call v1 model:
//   deterministic state transitions, bounded event snapshots, and
//   idempotent terminal close.
// - `preview_stream` and InvokeBidi are surfaced as honest diagnostic
//   fallbacks. They never mark the production WebRTC media plane ready.
//
// Usage Contract:
// - `remote_desktop.create_session` MUST be called with
//   `subject = resource_ura` for a display/window/application.
// - WebRTC SDP/ICE calls are accepted, audited, and routed to a device-side
//   WebRTC endpoint when the local media SDK exposes a transport-ready backend.
//
// Architectural Position:
// - CLI device adapter layer. Axon owns canonical protocol semantics;
//   this file implements the current device runtime behavior.

use std::sync::Arc;

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenSnapshotBackend;
use crate::plugins::remote_desktop::config::RemoteDesktopRuntimeConfig;
use crate::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::plugins::remote_desktop::transport::{
    DirectWebRtcEndpoint, RemoteDesktopTransportManager,
};

/// Runtime-owned state for the remote desktop plugin.
///
/// Invariant 1: every mutable session row is reachable only through this
/// plugin instance, never through process-global storage.
/// Invariant 2: transport handles are torn down through the same plugin
/// instance that created them, so lease expiry, explicit close, and test reset
/// share one lifecycle path.
#[derive(Clone)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopPlugin {
    sessions: Arc<RemoteDesktopSessionStore>,
    transports: Arc<RemoteDesktopTransportManager>,
    screen_backend: Arc<dyn ScreenSnapshotBackend>,
    config: RemoteDesktopRuntimeConfig,
}

impl RemoteDesktopPlugin {
    pub(in crate::plugins::builtin::remote_desktop) fn new(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        config: RemoteDesktopRuntimeConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Arc::new(RemoteDesktopSessionStore::new()),
            transports: Arc::new(RemoteDesktopTransportManager::new()),
            screen_backend,
            config,
        })
    }

    pub(in crate::plugins::builtin::remote_desktop) const fn config(
        &self,
    ) -> RemoteDesktopRuntimeConfig {
        self.config
    }

    pub(in crate::plugins::builtin::remote_desktop) fn session_store(
        &self,
    ) -> Arc<RemoteDesktopSessionStore> {
        Arc::clone(&self.sessions)
    }

    pub(in crate::plugins::builtin::remote_desktop) fn endpoint(
        &self,
        session_id: &str,
    ) -> Option<DirectWebRtcEndpoint> {
        self.transports.endpoint(session_id)
    }

    pub(in crate::plugins::builtin::remote_desktop) fn transport_manager(
        &self,
    ) -> Arc<RemoteDesktopTransportManager> {
        Arc::clone(&self.transports)
    }

    pub(in crate::plugins::builtin::remote_desktop) fn screen_backend(
        &self,
    ) -> Arc<dyn ScreenSnapshotBackend> {
        Arc::clone(&self.screen_backend)
    }
}
