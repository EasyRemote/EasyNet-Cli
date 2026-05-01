// EasyNet CLI — Federation Group
// ===============================
//
// File: src/cli/groups/federation.rs
// Description: `easynet federation …` — inspect cross-hub
//              federation state.
//
// Verbs:
//   peers              List federation peers and trusted hubs    (-> cli::federation_peers)
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
//   discover     — cross-realm device enumeration. PR-N3 territory
//                  (cross-realm directory federation), not yet
//                  shipped. Until then operators construct URIs
//                  by hand from the sources `easynet federation
//                  peers` lists.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};

use crate::facade::cli::federation_peers;

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
}

pub fn run(args: FederationArgs) -> anyhow::Result<()> {
    match args.action {
        FederationAction::Peers(a) => federation_peers::run(a),
    }
}
