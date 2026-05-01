// EasyNet CLI — Federation Group
// ===============================
//
// File: src/cli/groups/federation.rs
// Description: `easynet federation …` — inspect cross-hub
//              federation state.
//
// Verbs:
//   peers              List federation peers and trusted hubs    (-> cli::federation_peers)
//   discover           Read the cross-realm directory cell        (-> cli::federation_discover)
//
// Verbs DELIBERATELY ABSENT:
//
//   add / remove — operators edit `~/.easynet/daemon-config.toml`
//                  and `[/etc]/easynet/realm-trust.toml` directly,
//                  then `kill -HUP <daemon-pid>` to reload.
//                  Adding the verbs would mean a CLI surface that
//                  bypasses the SIGHUP-aware reload mechanism
//                  PR-N1 commits 9/N + 10/N already ship; the
//                  edit-and-SIGHUP workflow is the one source of
//                  truth.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};

use crate::facade::cli::federation_peers;
#[cfg(feature = "axon-pb")]
use crate::facade::cli::federation_discover;

#[derive(Debug, Args)]
pub struct FederationArgs {
    #[command(subcommand)]
    pub action: FederationAction,
}

#[derive(Debug, Subcommand)]
pub enum FederationAction {
    /// List federation peers and trusted hubs the local daemon
    /// can reach. Reads `~/.easynet/daemon-config.toml`'s
    /// `[daemon.federated_peers]` table and the realm-trust
    /// anchor's `[[trusted_agent]] role = "hub"` blocks.
    Peers(federation_peers::PeersArgs),

    /// Read the cross-realm directory federation view from the
    /// running daemon. Calls `federation.discover` over the
    /// daemon's gRPC UDS and renders the returned
    /// `DirectoryEntry` list. Optional `--user-id` filter
    /// applies the PR-N4 INV-5 privacy default; optional
    /// `--agent-uri` filter narrows to a single URI.
    #[cfg(feature = "axon-pb")]
    Discover(federation_discover::DiscoverArgs),
}

pub fn run(args: FederationArgs) -> anyhow::Result<()> {
    match args.action {
        FederationAction::Peers(a) => federation_peers::run(a),
        #[cfg(feature = "axon-pb")]
        FederationAction::Discover(a) => federation_discover::run(a),
    }
}
