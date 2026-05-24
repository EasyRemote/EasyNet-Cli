// EasyNet CLI — Feature-agnostic federation_invoke shim
// =====================================================
//
// File: src/support/federation_invoke_shim.rs
//
// Why this module exists
// ----------------------
// `support::federation_invoke` is gated behind `#[cfg(feature =
// "axon-pb")]` because it depends on the tonic-generated `pb::axon`
// types. Product-layer call sites (ability discovery, federation
// surfaces in `runtime::agents::*`) want to consult the federation
// directory without caring whether the daemon was compiled with the
// gRPC transport.
//
// This shim provides one symbol per public helper from
// `federation_invoke`. When the feature is on, each function delegates
// to the real implementation. When the feature is off, each function
// returns the appropriate "nothing to report" result so the caller's
// branch reads identically in both configurations.
//
// Contract for "feature off" returns
// ----------------------------------
// The off-variant must never produce a fabricated success that the
// caller could mistake for a real federation response. The current
// surfaces are read-only directory queries, so empty `Vec` is
// safe — the caller treats "no federated devices" the same regardless
// of why. If a write surface is ever added here, the off-variant must
// return `Err`, not `Ok(())`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

/// Read the federated directory through the local daemon's
/// `federation.discover` ability. See
/// [`crate::support::federation_invoke::invoke_federation_discover`]
/// for the real implementation contract.
///
/// **Feature-off behaviour:** returns `Ok(vec![])`. The daemon
/// compiled without `axon-pb` has no gRPC client to dial; from the
/// caller's perspective the federated directory is simply empty.
#[cfg(feature = "axon-pb")]
pub fn invoke_federation_discover(
    agent_ura_filter: Option<&str>,
    caller_ura: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    crate::support::federation_invoke::invoke_federation_discover(agent_ura_filter, caller_ura)
}

#[cfg(not(feature = "axon-pb"))]
pub fn invoke_federation_discover(
    _agent_ura_filter: Option<&str>,
    _caller_ura: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    Ok(Vec::new())
}
