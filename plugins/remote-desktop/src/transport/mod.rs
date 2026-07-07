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
mod terminal;
mod webrtc;
mod webrtc_baseline_media;
mod webrtc_endpoint;
mod webrtc_media;
#[cfg(target_os = "macos")]
mod webrtc_native_media;
mod webrtc_negotiation;

pub(in crate::daemon::plugins::remote_desktop) use manager::{
    DirectWebRtcEndpoint, RemoteDesktopTransportManager,
};
pub(in crate::daemon::plugins::remote_desktop) use terminal::BidiTerminalGuard;
pub(in crate::daemon::plugins::remote_desktop) use webrtc::{
    apply_pending_remote_ice_candidates, apply_remote_ice_candidate_values, DirectWebRtcHandler,
};
pub(in crate::daemon::plugins::remote_desktop) use webrtc_endpoint::{
    start_direct_webrtc_endpoint, StartDirectWebRtcEndpointRequest,
};
pub(in crate::daemon::plugins::remote_desktop) use webrtc_media::{
    run_direct_webrtc_media_loop, DirectWebRtcSession,
};
pub(in crate::daemon::plugins::remote_desktop) use webrtc_negotiation::{
    negotiate_remote_offer, RemoteOfferNegotiation,
};
