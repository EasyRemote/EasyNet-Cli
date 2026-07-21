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
// - Removes stale runtime.json only through the lifecycle status report.
//
// Safety:
// - Refuses to run while a runtime is active (daemon lifecycle facts visible)
//   unless --force is given, because a running heartbeat daemon would fail repeatedly
//   with missing credentials. Corrupt runtime projection aborts before credentials
//   are deleted.
//
// Usage Contract:
// - Irreversible locally: re-pairing requires a new token from the Hub dashboard.
// - Safe to run while disconnected. Should NOT be run while
//   `easynet runtime start` is active.
//   (the running heartbeat will fail on next cycle since credentials are gone).
//
// Architectural Position:
// - Terminal state of the device lifecycle: join → start → stop → reset.
// - Inverse of join.rs. Together they form the pairing/unpairing boundary.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::daemon::lifecycle::{RuntimeLifecycleService, RuntimeLifecycleStatus};
use crate::daemon::persistence::config;
use crate::support::platform::output;

#[derive(Debug, Args)]
pub struct ResetArgs {
    /// Proceed even when a runtime is still active ('easynet runtime start'
    /// would
    /// otherwise need to stop first). This does NOT skip the confirmation
    /// prompt — see '--yes'.
    #[arg(long)]
    pub force: bool,
    /// Skip the interactive confirmation. Required for non-interactive /
    /// CI use, where the terminal-detection guard in 'output::confirm'
    /// would otherwise refuse. Orthogonal to '--force': '--yes' skips
    /// the confirmation, '--force' skips the "runtime-still-running"
    /// guard.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run(args: ResetArgs) -> anyhow::Result<()> {
    // Guard 1: refuse if runtime is active (heartbeat would break).
    // Also capture the lifecycle report for best-effort deregister before cleanup.
    let lifecycle_report = RuntimeLifecycleService::new().status()?;
    if !args.force && reset_runtime_is_active(lifecycle_report.status()) {
        anyhow::bail!(
            "runtime is currently running — run 'easynet runtime stop' first, or use 'easynet reset --force'"
        );
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
    //
    // Joint-plan unified path (phase 1.4): when the daemon is still
    // alive (the `--force` path bypasses guard 1, so this is the only
    // condition that lands here) call `federation.revoke` against
    // this device's URA. The former local deregistration shim
    // was ack-only — the hub never learned the device went
    // away, so directory entries lingered until the keepalive sweep.
    // The new path reaches `PresenceRegistry::force_revoke` and the
    // advertised-agent store immediately, so a downstream
    // `device list` from any peer hub shows the device gone the
    // instant `device reset --force` returns.
    if lifecycle_report.daemon().has_daemon_fact() {
        if let Ok(creds) = config::load_credentials() {
            let device_ura = crate::core::ura::device_ura(&creds.realm, &creds.node_id);
            match invoke_federation_revoke_for_reset(&device_ura) {
                Ok(_) => output::info("Device deregistered with hub (federation.revoke)"),
                Err(e) => output::warn(&format!(
                    "federation.revoke failed (continuing local reset): {e}"
                )),
            }
        }
    }

    // Clean up stale runtime.json (process dead) after deregister attempt.
    if matches!(
        lifecycle_report.status(),
        RuntimeLifecycleStatus::ProjectionPresentProcessMissing
    ) {
        config::remove()?;
    }

    config::delete_credentials()?;
    output::success("Device credentials removed");
    Ok(())
}

fn reset_runtime_is_active(status: RuntimeLifecycleStatus) -> bool {
    matches!(
        status,
        RuntimeLifecycleStatus::Running
            | RuntimeLifecycleStatus::ProjectionMissingProcessRunning
            | RuntimeLifecycleStatus::ControlOnlyInvocationDown
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_federation_revoke_for_reset(device_ura: &str) -> anyhow::Result<()> {
    crate::daemon::invocation::routing::remote_invoke::invoke_federation_revoke(
        device_ura,
        "device-reset",
    )
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_federation_revoke_for_reset(_device_ura: &str) -> anyhow::Result<()> {
    Err(
        crate::support::platform::local_invoke::federation_not_wired_error(
            "deregistering this device on reset",
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;

    fn paired_credentials() -> config::Credentials {
        config::Credentials {
            node_id: "node-reset-test".into(),
            credential_token: "token-reset-test".into(),
            hub_endpoint: "axon://hub.example:7700".into(),
            realm: "tenant-reset-test".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    fn stale_runtime_state() -> config::RuntimeState {
        config::RuntimeState {
            endpoint: "/tmp/easynet-reset-stale.sock".to_string(),
            runtime_kind: config::RuntimeKind::DaemonOnly,
            pid: Some(999_999),
            hub: None,
            tenant: Some("tenant-reset-test".to_string()),
            label: Some("node-reset-test".to_string()),
            started_at: None,
            credential_verified: None,
        }
    }

    #[test]
    fn reset_rejects_malformed_runtime_projection_before_deleting_credentials() {
        let _home = HomeGuard::new();
        config::save_credentials(&paired_credentials()).expect("credentials");
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        std::fs::write(config::runtime_state_path(), "{ not json").expect("runtime projection");

        let error = run(ResetArgs {
            force: true,
            yes: true,
        })
        .expect_err("malformed runtime projection must block reset");

        assert!(
            error.to_string().contains("load runtime projection failed"),
            "wrong error: {error:#}"
        );
        config::load_credentials().expect("credentials must remain after failed reset");
    }

    #[test]
    fn reset_removes_stale_runtime_projection_through_lifecycle_report() {
        let _home = HomeGuard::new();
        config::save_credentials(&paired_credentials()).expect("credentials");
        config::save(&stale_runtime_state()).expect("stale projection");

        run(ResetArgs {
            force: false,
            yes: true,
        })
        .expect("stale projection reset");

        assert!(
            config::load().is_err(),
            "stale runtime projection must be removed during reset"
        );
        assert!(
            config::load_credentials().is_err(),
            "credentials must be removed after successful reset"
        );
    }
}
