// EasyNet CLI - daemon federation client (cross-hub transport)
// =================================================================
//
// File: src/daemon/federation/client/mod.rs
//
// Cross-hub outbound transport for canonical Invocation RPCs and directory
// subscriptions. The transport owns peer trust, TLS pinning, channel reuse,
// deadlines, and per-operation circuit breakers; it never creates a second
// ability-level invocation protocol.
//
// What lives here
// ---------------
// - `FederationClient` is the mockable peer RPC boundary.
// - `CrossHubDialer` is the tonic-backed implementation.
// - `ability_contract` contains only federation control-plane DTOs.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability_contract;
#[cfg(feature = "axon-pb")]
pub mod cross_hub_dial;
#[cfg(feature = "axon-pb")]
pub mod peer_dial;

#[cfg(feature = "axon-pb")]
pub use cross_hub_dial::{
    CrossHubDialer, DirectoryEventStream, FederationClient, FederationClientError, HubEndpoint,
};
#[cfg(feature = "axon-pb")]
pub use peer_dial::{pinned_tls_config, PinnedTlsError};
