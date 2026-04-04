// EasyNet CLI
// ===========
//
// File: src/cli/reset.rs
// Description: `easynet reset` — sever the trust relationship between this device and the Hub
//              by deleting locally stored credentials.
//
// Protocol Responsibility:
// - Removes ~/.easynet/credentials.json (node_id, credential_token, deploy_signature).
// - Does NOT notify the Hub — the device will appear as "offline" until the Hub's
//   heartbeat timeout expires, then transition to REMOVED.
// - Does NOT remove runtime.json or device_settings.json; those are orthogonal.
//
// Safety:
// - Refuses to run while a runtime is active (runtime.json exists AND process alive)
//   unless --force is given, because a running heartbeat daemon would fail repeatedly
//   with missing credentials. Stale runtime.json (dead process) is cleaned up silently.
//
// Usage Contract:
// - Irreversible locally: re-pairing requires a new token from the Hub dashboard.
// - Safe to run while disconnected. Should NOT be run while `easynet start` is active
//   (the running heartbeat will fail on next cycle since credentials are gone).
//
// Architectural Position:
// - Terminal state of the device lifecycle: join → start → stop → reset.
// - Inverse of join.rs. Together they form the pairing/unpairing boundary.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::shared::{config, net, output};

#[derive(Debug, Args)]
pub struct ResetArgs {
    /// Force reset even if a runtime is currently running
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: ResetArgs) -> anyhow::Result<()> {
    // Guard: refuse if runtime is active (heartbeat would break).
    if !args.force {
        if let Ok(state) = config::load() {
            let alive = state.pid.is_some_and(net::is_pid_alive);
            if alive {
                anyhow::bail!(
                    "runtime is currently running — run `easynet stop` first, or use `easynet reset --force`"
                );
            }
            // Stale runtime.json (process dead) — clean it up silently.
            config::remove().ok();
        }
    }

    config::delete_credentials()?;
    output::success("Device credentials removed");
    Ok(())
}
