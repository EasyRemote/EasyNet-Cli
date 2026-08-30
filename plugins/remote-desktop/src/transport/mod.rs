// EasyNet CLI — remote desktop transport boundary
// =================================================
//
// File: plugins/remote-desktop/src/transport/mod.rs
// Description: Transport-plane ownership boundary for the builtin remote
// desktop plugin.
//
// This module owns transport handles and terminal-call guards. It does not
// parse ability requests, serialize view DTOs, or maintain remote desktop
// session state. Those responsibilities belong to handlers, view, and session
// modules respectively.

mod manager;
mod media_source;
mod terminal;
mod webrtc;
#[cfg(any(
    test,
    not(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))
))]
mod webrtc_audio;
#[cfg(any(
    test,
    not(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))
))]
mod webrtc_baseline_media;
#[cfg(any(
    test,
    all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )
))]
mod webrtc_encoded_audio;
mod webrtc_endpoint;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
mod webrtc_hosted_media;
mod webrtc_media;
mod webrtc_negotiation;
mod webrtc_sender_feedback;

pub(in crate::daemon::plugins::remote_desktop) use manager::{
    DirectWebRtcEndpoint, PreviewTaskGroupCompletion, RemoteDesktopTransportManager,
    RetiredDiagnosticPreview, RetiredDirectWebRtcEndpoint, TransportSettlementAdmissionPermit,
    TransportSettlementFailureKind, TransportSettlementJob, TransportSettlementJobContext,
    TransportSettlementQueue, TransportSettlementStatus, TRANSPORT_SETTLEMENT_DEADLINE,
};
pub(in crate::daemon::plugins::remote_desktop) use terminal::BidiTerminalGuard;
pub(in crate::daemon::plugins::remote_desktop) use webrtc::{
    apply_pending_remote_ice_candidates, apply_remote_ice_candidate_values, DirectWebRtcHandler,
    DirectWebRtcHandlerConfig,
};
pub(in crate::daemon::plugins::remote_desktop) use webrtc_endpoint::{
    start_direct_webrtc_endpoint, StartDirectWebRtcEndpointRequest,
};
pub(in crate::daemon::plugins::remote_desktop) use webrtc_media::{
    close_peer_connection_bounded, run_direct_webrtc_media_loop, DirectWebRtcSession,
};
pub(in crate::daemon::plugins::remote_desktop) use webrtc_negotiation::{
    negotiate_remote_offer, RemoteOfferNegotiation,
};
