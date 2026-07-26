// EasyNet CLI
// ===========
//
// File: src/cli/reset.rs
// Description: `easynet device reset` — sever the trust relationship between this
//              device and the Hub by deleting local runtime state at the
//              configured lifecycle boundary.
//
// Protocol Responsibility:
// - Default scope removes ~/.easynet/credentials.json (node_id,
//   credential_token, deploy_signature).
// - Explicit purge scope removes the whole ~/.easynet local state root, so
//   stale keyring, descriptor, registry, and discovery state cannot pressure
//   canonical invocation paths into compatibility fallback.
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
// - Irreversible locally: re-pairing requires a new token from the Hub
//   dashboard.
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
use std::path::Path;

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
    /// Remove the entire local EasyNet state root (`~/.easynet`) instead of
    /// only the pairing credentials. This is the clean cutover path for
    /// incompatible local keyring, descriptor/read-model, registry, and daemon
    /// discovery state; invocation/resolver code must remain fail-closed rather
    /// than repairing those files through compatibility fallback.
    #[arg(long)]
    pub purge_local_state: bool,
}

pub fn run(args: ResetArgs) -> anyhow::Result<()> {
    let reset_scope = LocalResetScope::from_args(&args);
    // Guard 1: refuse if runtime is active (heartbeat would break).
    // Also capture the lifecycle report for best-effort deregister before cleanup.
    let lifecycle_report = RuntimeLifecycleService::new().status()?;
    if !args.force && reset_runtime_is_active(lifecycle_report.status()) {
        anyhow::bail!(
            "runtime is currently running — run 'easynet runtime stop' first, or use 'easynet device reset --force'"
        );
    }
    let credential_state = ResetCredentialState::load();

    // Guard 2: interactive confirmation before destroying credentials.
    //
    // `easynet device reset` deletes ~/.easynet/credentials.json, after which
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
        let prompt = reset_scope.confirmation_prompt(&credential_state);
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
        match credential_state.device_ura_for_revoke() {
            ResetRevokeCredential::Ready(device_ura) => {
                match invoke_federation_revoke_for_reset(&device_ura) {
                    Ok(_) => output::info("Device deregistered with hub (federation.revoke)"),
                    Err(e) => output::warn(&format!(
                        "federation.revoke failed (continuing local reset): {e}"
                    )),
                }
            }
            ResetRevokeCredential::Unavailable(reason) => {
                output::warn(&format!(
                    "federation.revoke skipped (continuing local reset): {reason}"
                ));
            }
        }
    }

    // Clean up stale runtime.json (process dead) after deregister attempt.
    if matches!(
        lifecycle_report.status(),
        RuntimeLifecycleStatus::ProjectionPresentProcessMissing
    ) && matches!(reset_scope, LocalResetScope::CredentialsOnly)
    {
        config::remove()?;
    }

    reset_scope.execute()?;
    output::success(reset_scope.success_message());
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalResetScope {
    CredentialsOnly,
    LocalStateRoot,
}

impl LocalResetScope {
    fn from_args(args: &ResetArgs) -> Self {
        if args.purge_local_state {
            Self::LocalStateRoot
        } else {
            Self::CredentialsOnly
        }
    }

    fn confirmation_prompt(self, credential_state: &ResetCredentialState) -> String {
        let node_id = credential_state.prompt_subject_label();
        match self {
            Self::CredentialsOnly => format!(
                "This will delete local credentials for node '{node_id}'. \
                 Re-pairing requires a fresh token from the Hub. Continue?"
            ),
            Self::LocalStateRoot => format!(
                "This will permanently delete the local EasyNet state root '{}' \
                 for node '{node_id}', including credentials, keyring, descriptors, \
                 registry, discovery, logs, and local daemon state. Re-pairing \
                 requires a fresh token from the Hub. Continue?",
                config::state_dir().display()
            ),
        }
    }

    fn execute(self) -> anyhow::Result<()> {
        match self {
            Self::CredentialsOnly => config::delete_credentials(),
            Self::LocalStateRoot => purge_local_state_root(),
        }
    }

    fn success_message(self) -> &'static str {
        match self {
            Self::CredentialsOnly => "Device credentials removed",
            Self::LocalStateRoot => "Local EasyNet state root removed",
        }
    }
}

fn purge_local_state_root() -> anyhow::Result<()> {
    let root = config::state_dir();
    validate_local_state_purge_root(&root)?;
    let metadata = match std::fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(anyhow::anyhow!(
                "inspect local EasyNet state root {}: {error}",
                root.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        anyhow::bail!(
            "refusing to purge symlinked EasyNet state root {}",
            root.display()
        );
    }
    if metadata.is_dir() {
        std::fs::remove_dir_all(&root)
            .map_err(|error| anyhow::anyhow!("remove {}: {error}", root.display()))?;
    } else {
        std::fs::remove_file(&root)
            .map_err(|error| anyhow::anyhow!("remove {}: {error}", root.display()))?;
    }
    config::sync_parent_dir(&root)?;
    Ok(())
}

fn validate_local_state_purge_root(root: &Path) -> anyhow::Result<()> {
    if !root.is_absolute() {
        anyhow::bail!(
            "refusing to purge non-absolute EasyNet state root {}",
            root.display()
        );
    }
    if root.file_name().and_then(|name| name.to_str()) != Some(".easynet") {
        anyhow::bail!(
            "refusing to purge unexpected EasyNet state root {}",
            root.display()
        );
    }
    if root.parent().is_none() {
        anyhow::bail!(
            "refusing to purge parentless EasyNet state root {}",
            root.display()
        );
    }
    Ok(())
}

#[derive(Debug)]
enum ResetCredentialState {
    Paired(config::Credentials),
    Missing,
    Invalid { reason: String },
}

#[derive(Debug, PartialEq, Eq)]
enum ResetRevokeCredential {
    Ready(String),
    Unavailable(String),
}

impl ResetCredentialState {
    fn load() -> Self {
        Self::from_credentials_result(config::load_credentials_optional())
    }

    fn from_credentials_result(result: anyhow::Result<Option<config::Credentials>>) -> Self {
        match result {
            Ok(Some(credentials)) => Self::Paired(credentials),
            Ok(None) => Self::Missing,
            Err(error) => Self::Invalid {
                reason: format!("{error:#}"),
            },
        }
    }

    fn prompt_subject_label(&self) -> String {
        match self {
            Self::Paired(credentials) => credentials.node_id.clone(),
            Self::Missing => "<no credentials on disk>".to_string(),
            Self::Invalid { reason } => format!("<invalid credentials: {reason}>"),
        }
    }

    fn device_ura_for_revoke(&self) -> ResetRevokeCredential {
        match self {
            Self::Paired(credentials) => ResetRevokeCredential::Ready(
                crate::core::ura::device_ura(&credentials.realm, &credentials.node_id),
            ),
            Self::Missing => ResetRevokeCredential::Unavailable("no credentials".to_string()),
            Self::Invalid { reason } => ResetRevokeCredential::Unavailable(format!(
                "invalid credentials; cannot derive device URA: {reason}"
            )),
        }
    }

    #[cfg(test)]
    fn label(&self) -> &'static str {
        match self {
            Self::Paired(_) => "paired",
            Self::Missing => "missing",
            Self::Invalid { .. } => "invalid",
        }
    }
}

fn reset_runtime_is_active(status: RuntimeLifecycleStatus) -> bool {
    matches!(
        status,
        RuntimeLifecycleStatus::Running
            | RuntimeLifecycleStatus::ProjectionMissingProcessRunning
            | RuntimeLifecycleStatus::ControlOnlyInvocationDown
            | RuntimeLifecycleStatus::DaemonDiscoveryInvalid
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_federation_revoke_for_reset(device_ura: &str) -> anyhow::Result<()> {
    crate::daemon::invocation::routing::remote_invoke::invoke_federation_revoke(
        device_ura,
        "device-reset",
        device_ura,
    )
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_federation_revoke_for_reset(_device_ura: &str) -> anyhow::Result<()> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(
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
    fn reset_credential_state_reports_paired_credentials() {
        let state = ResetCredentialState::from_credentials_result(Ok(Some(paired_credentials())));

        assert_eq!(state.label(), "paired");
        assert_eq!(
            state.prompt_subject_label(),
            "node-reset-test",
            "paired reset prompt should name the node id"
        );
        assert_eq!(
            state.device_ura_for_revoke(),
            ResetRevokeCredential::Ready(
                "easynet:///r/tenant-reset-test/device/node-reset-test".to_string()
            )
        );
    }

    #[test]
    fn reset_credential_state_reports_missing_only_for_absent_credentials() {
        let state = ResetCredentialState::from_credentials_result(Ok(None));

        assert_eq!(state.label(), "missing");
        assert_eq!(state.prompt_subject_label(), "<no credentials on disk>");
        assert_eq!(
            state.device_ura_for_revoke(),
            ResetRevokeCredential::Unavailable("no credentials".to_string())
        );
    }

    #[test]
    fn reset_credential_state_reports_invalid_existing_credentials() {
        let state = ResetCredentialState::from_credentials_result(Err(anyhow::anyhow!(
            "parse credentials from /tmp/credentials.json: expected value"
        )));

        assert_eq!(state.label(), "invalid");
        let prompt = state.prompt_subject_label();
        assert!(
            prompt.contains("<invalid credentials: parse credentials"),
            "invalid prompt must not look like missing credentials: {prompt}"
        );
        match state.device_ura_for_revoke() {
            ResetRevokeCredential::Unavailable(reason) => assert!(
                reason.contains("invalid credentials; cannot derive device URA"),
                "invalid revoke state must preserve reason: {reason}"
            ),
            other => panic!("invalid credentials must not produce revoke URA: {other:?}"),
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
            purge_local_state: false,
        })
        .expect_err("malformed runtime projection must block reset");

        assert!(
            error.to_string().contains("load runtime projection failed"),
            "wrong error: {error:#}"
        );
        config::load_credentials().expect("credentials must remain after failed reset");
    }

    #[test]
    fn reset_deletes_malformed_credentials_without_classifying_as_missing() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        let credentials_path = config::state_dir().join("credentials.json");
        std::fs::write(&credentials_path, "{ not json").expect("malformed credentials");

        let state = ResetCredentialState::load();
        assert_eq!(state.label(), "invalid");
        assert!(
            state
                .prompt_subject_label()
                .contains("<invalid credentials:"),
            "malformed credentials should be visible before reset cleanup"
        );

        run(ResetArgs {
            force: false,
            yes: true,
            purge_local_state: false,
        })
        .expect("reset should remove malformed local credentials");

        assert!(
            !credentials_path.exists(),
            "reset must delete malformed credentials after explicit --yes"
        );
    }

    #[test]
    fn reset_removes_stale_runtime_projection_through_lifecycle_report() {
        let _home = HomeGuard::new();
        config::save_credentials(&paired_credentials()).expect("credentials");
        config::save(&stale_runtime_state()).expect("stale projection");

        run(ResetArgs {
            force: false,
            yes: true,
            purge_local_state: false,
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

    #[test]
    fn reset_purge_local_state_removes_keyring_descriptor_and_registry_root() {
        let _home = HomeGuard::new();
        config::save_credentials(&paired_credentials()).expect("credentials");
        let state_dir = config::state_dir();
        std::fs::write(state_dir.join("keyring.enc"), "stale-keyring").expect("stale keyring");
        std::fs::create_dir_all(state_dir.join("agents/agent-a/descriptors"))
            .expect("descriptor dir");
        std::fs::write(
            state_dir.join("agents/agent-a/descriptors/meta.list_abilities.json"),
            "{}",
        )
        .expect("stale descriptor");
        std::fs::write(state_dir.join("control.json"), "{}").expect("stale discovery");

        run(ResetArgs {
            force: true,
            yes: true,
            purge_local_state: true,
        })
        .expect("purge local state reset");

        assert!(
            !state_dir.exists(),
            "purge reset must remove the local state root instead of preserving stale subtrees"
        );
    }

    #[test]
    fn local_state_purge_root_rejects_relative_and_non_easynet_paths() {
        let relative = Path::new(".easynet");
        let error = validate_local_state_purge_root(relative)
            .expect_err("relative purge root must fail closed");
        assert!(
            error.to_string().contains("non-absolute"),
            "wrong relative-root error: {error:#}"
        );

        let unexpected = std::env::temp_dir().join("not-easynet-state");
        let error = validate_local_state_purge_root(&unexpected)
            .expect_err("unexpected purge root must fail closed");
        assert!(
            error.to_string().contains("unexpected EasyNet state root"),
            "wrong unexpected-root error: {error:#}"
        );
    }
}
