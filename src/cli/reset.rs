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

use crate::persistence::config;
use crate::shared::{self, net, output};

#[derive(Debug, Args)]
pub struct ResetArgs {
    /// Proceed even when a runtime is still active (`easynet start` would
    /// otherwise need to stop first). This does NOT skip the confirmation
    /// prompt — see `--yes`.
    #[arg(long)]
    pub force: bool,
    /// Skip the interactive confirmation. Required for non-interactive /
    /// CI use, where the terminal-detection guard in `output::confirm`
    /// would otherwise refuse. Orthogonal to `--force`: `--yes` skips
    /// the confirmation, `--force` skips the "runtime-still-running"
    /// guard.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run(args: ResetArgs) -> anyhow::Result<()> {
    // Guard 1: refuse if runtime is active (heartbeat would break).
    // Also capture the runtime state for best-effort deregister before cleanup.
    let runtime_state = config::load().ok();
    if !args.force {
        if let Some(ref state) = runtime_state {
            if state.pid.is_some_and(net::is_pid_alive) {
                anyhow::bail!(
                    "runtime is currently running — run `easynet stop` first, or use `easynet reset --force`"
                );
            }
        }
    }

    // Guard 2: interactive confirmation before destroying credentials.
    //
    // `easynet reset` deletes ~/.easynet/credentials.json, after which
    // the ONLY way to re-pair this device is to request a fresh pairing
    // token from the Hub dashboard — there is no local undo. A single
    // mistyped command or shell-history replay would silently sever the
    // trust relationship. The prompt is the stop-the-world checkpoint.
    //
    // `--yes` is the documented bypass for scripts and CI; it's kept as
    // a separate flag from `--force` so the two intents (skip
    // confirmation vs. ignore the running-runtime guard) cannot be
    // conflated by either readers or accidental invocations.
    if !args.yes {
        let node_id = config::load_credentials()
            .ok()
            .map(|c| c.node_id)
            .unwrap_or_else(|| "<no credentials on disk>".to_string());
        let prompt = format!(
            "This will delete local credentials for node '{node_id}'. \
             Re-pairing requires a fresh token from the Hub. Continue?"
        );
        if !output::confirm(&prompt)? {
            output::info("Cancelled.");
            return Ok(());
        }
    }

    // Best-effort: notify Hub before deleting local credentials. We log
    // the outcome accurately — the previous unconditional "deregistered"
    // message lied to the operator on transient Hub failures, which then
    // looked indistinguishable from a successful clean-up in the audit
    // trail.
    if let Ok(creds) = config::load_credentials() {
        if let Some(ref state) = runtime_state {
            if let Ok(bridge) = shared::connect_bridge_to(&state.endpoint) {
                match bridge.deregister_node(&creds.tenant_id, &creds.node_id, "device reset") {
                    Ok(_) => output::info("Node deregistered from Hub"),
                    Err(e) => output::warn(&format!(
                        "Hub deregister failed (continuing local reset): {e}"
                    )),
                }
            }
        }
    }

    // Clean up stale runtime.json (process dead) after deregister attempt.
    if let Some(ref state) = runtime_state {
        if !state.pid.is_some_and(net::is_pid_alive) {
            config::remove().ok();
        }
    }

    config::delete_credentials()?;
    output::success("Device credentials removed");
    Ok(())
}
