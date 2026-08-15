// EasyNet CLI — remote desktop transport blocker taxonomy
// =======================================================
//
// File: plugins/remote-desktop/src/transport_blocker.rs
// Description: Canonical failure taxonomy for non-route WebRTC transport blockers.

use crate::daemon::plugins::remote_desktop::target::{FrontendAction, TargetResolutionError};

/// Canonical projection for deterministic WebRTC blockers that happen before
/// ICE/relay route selection.
///
/// This type separates backend/capability blockers from route blockers:
/// `transport_route_unavailable` remains reserved for host/STUN/TURN/EasyNet
/// candidate failure, while missing or failed native capture/encode/runtime
/// backends use the SPEC target/runtime failure taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTransportBlocker {
    reason_code: TargetResolutionError,
    failure_domain: &'static str,
    recoverability: &'static str,
}

impl RemoteDesktopTransportBlocker {
    pub(in crate::daemon::plugins::remote_desktop) fn from_webrtc_error(
        reason: &str,
    ) -> Option<Self> {
        match reason {
            "native_media_plugin_required" | "webrtc_transport_backend_unavailable" => Some(Self {
                reason_code: TargetResolutionError::CaptureBackendUnavailable,
                failure_domain: "runtime",
                recoverability: "unsupported",
            }),
            "native_media_pipeline_failed" => Some(Self {
                reason_code: TargetResolutionError::ScreenCaptureKitStreamStartFailed,
                failure_domain: "media_source",
                recoverability: "unsupported",
            }),
            _ => None,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn reason_code_str(self) -> &'static str {
        self.reason_code.as_str()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn failure_domain(self) -> &'static str {
        self.failure_domain
    }

    pub(in crate::daemon::plugins::remote_desktop) fn recoverability(self) -> &'static str {
        self.recoverability
    }

    pub(in crate::daemon::plugins::remote_desktop) fn frontend_action(self) -> FrontendAction {
        self.reason_code.frontend_action()
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteDesktopTransportBlocker;

    #[test]
    fn backend_unavailable_maps_to_capture_backend_unavailable() {
        let blocker = RemoteDesktopTransportBlocker::from_webrtc_error(
            "webrtc_transport_backend_unavailable",
        )
        .expect("backend blocker");

        assert_eq!(blocker.reason_code_str(), "capture_backend_unavailable");
        assert_eq!(blocker.failure_domain(), "runtime");
        assert_eq!(blocker.recoverability(), "unsupported");
        assert_eq!(blocker.frontend_action().as_str(), "show_unsupported");
    }

    #[test]
    fn native_pipeline_failure_maps_to_screencapturekit_stream_start_failed() {
        let blocker =
            RemoteDesktopTransportBlocker::from_webrtc_error("native_media_pipeline_failed")
                .expect("pipeline blocker");

        assert_eq!(
            blocker.reason_code_str(),
            "screencapturekit_stream_start_failed"
        );
        assert_eq!(blocker.failure_domain(), "media_source");
        assert_eq!(blocker.frontend_action().as_str(), "show_unsupported");
    }
}
