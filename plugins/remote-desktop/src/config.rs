// EasyNet CLI — remote desktop runtime configuration
// ===================================================
//
// File: plugins/remote-desktop/src/config.rs
// Description: Host manifest limits projected into plugin-local runtime knobs.

use crate::daemon::plugins::PluginRuntimeLimits;

const MIN_REMOTE_DESKTOP_SESSIONS: usize = 1;
pub(in crate::daemon::plugins::remote_desktop) const MIN_FRAME_QUEUE_DEPTH: usize = 1;

/// Host-enforced runtime limits for the remote desktop plugin.
///
/// Invariant 1: these values come from `plugins/remote-desktop/plugin.toml`
/// through the plugin host, not from a second Rust mirror of the package
/// manifest.
///
/// Invariant 2: zero is never admitted. A malformed manifest must degrade to
/// the smallest bounded value rather than creating an unbounded store or a
/// zero-capacity media channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopRuntimeConfig {
    max_sessions: usize,
    max_frame_queue: usize,
}

impl RemoteDesktopRuntimeConfig {
    /// Maximum concurrent remote desktop sessions retained in memory.
    pub(in crate::daemon::plugins::remote_desktop) const fn max_sessions(self) -> usize {
        self.max_sessions
    }

    /// Maximum downstream media/control frames buffered per attached session.
    pub(in crate::daemon::plugins::remote_desktop) const fn max_frame_queue(self) -> usize {
        self.max_frame_queue
    }
}

impl From<PluginRuntimeLimits> for RemoteDesktopRuntimeConfig {
    fn from(limits: PluginRuntimeLimits) -> Self {
        Self {
            max_sessions: limits.max_sessions().max(MIN_REMOTE_DESKTOP_SESSIONS),
            max_frame_queue: limits.max_frame_queue().max(MIN_FRAME_QUEUE_DEPTH),
        }
    }
}
