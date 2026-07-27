// EasyNet CLI — `easynet runtime status`
// =======================================
//
// File: src/cli/status.rs
// Description: Hub connection info + device summary. Joint-plan
//              unified path: cross-device enumeration goes through
//              `federation.discover` (the same surface
//              `easynet device list` uses); ability count goes
//              through `easynet.discover`. No more
//              `node.list` — that handler is on the phase 4
//              cull list.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use serde_json::{json, Value};

use crate::cli::presentation::identity::runtime_user_binding_display;
use crate::core::ura;
use crate::daemon::boot::join_connection_state;
use crate::daemon::lifecycle::{RuntimeLifecycleService, RuntimeLifecycleStatus};
use crate::daemon::persistence::config;
use crate::support::platform::local_invoke::{
    LocalRuntimeCatalogueReadIssuer, LocalRuntimeOperationalReadIssuer, LocalRuntimeStateReadIssuer,
};
use crate::support::platform::output;

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Emit JSON instead of the human-readable report.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: StatusArgs) -> anyhow::Result<()> {
    if args.json {
        return run_json();
    }
    output::info(&format!("EasyNet CLI v{}", env!("CARGO_PKG_VERSION")));
    render_connection_state();

    let pairing_state = StatusPairingState::load();
    render_pairing_state(&pairing_state);

    let lifecycle = RuntimeLifecycleService::new();
    let report = lifecycle.status()?;
    if report.status() == RuntimeLifecycleStatus::Stopped {
        output::info("Runtime: not running");
        output::info("Run 'easynet runtime start' to start.");
        return Ok(());
    };

    // Runtime block. We avoid duplicating the URA values printed
    // above; this block is the runtime's own knobs (mode, sockets,
    // pid) — not identity. The `Hub` / `Tenant` / `Label` fields
    // on `RuntimeState` are the runtime's own copy of the pairing
    // state used during boot — they may differ from credentials
    // (e.g. before a fresh pairing reaches the daemon). When
    // they're identical to creds (the common case) reprinting them
    // would be noise; when they differ they belong in a future
    // diagnostics command, not the status table. Keep the runtime
    // block scoped to mode + transport + pid.
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(projection) = report.projection() {
        let state = projection.state();
        rows.push(("Mode", "daemon-only".to_string()));
        rows.push(("gRPC socket", state.endpoint.clone()));
        rows.push((
            "Control socket",
            crate::daemon::control::transport::try_default_socket_path()?
                .display()
                .to_string(),
        ));
    } else if let Some(discovery) = report.daemon().control_discovery() {
        rows.push((
            "Mode",
            discovery
                .daemon_identity
                .as_ref()
                .map(|identity| identity.mode.clone())
                .unwrap_or_else(|| "daemon-only".to_string()),
        ));
        if let Some(endpoint) = discovery.invocation_endpoint.as_ref() {
            rows.push(("gRPC socket", endpoint.display().to_string()));
        }
        if let Some(socket) = discovery.socket_path.as_ref() {
            rows.push(("Control socket", socket.display().to_string()));
        }
        rows.push(("PID", discovery.pid.to_string()));
        rows.push(("Projection", "missing runtime.json".to_string()));
    }
    rows.push(("Status", report.status().as_wire_str().to_string()));
    if let Some(error) = report.daemon().control_discovery_error() {
        rows.push(("Discovery error", error.to_string()));
    }
    let kv: Vec<(&str, &str)> = rows.iter().map(|(k, v)| (*k, v.as_str())).collect();
    output::kv_section(&kv);
    if matches!(
        report.status(),
        RuntimeLifecycleStatus::DaemonDiscoveryInvalid
    ) {
        output::warn("Daemon discovery is invalid; canonical runtime attach is disabled until control.json is repaired or removed.");
        return Ok(());
    }
    if matches!(
        report.status(),
        RuntimeLifecycleStatus::ProjectionMissingProcessRunning
    ) {
        output::warn("Runtime projection is missing, but daemon facts are present.");
    }
    if let Some(state) = report.projection().map(|projection| projection.state()) {
        if state.credential_verified == Some(false) {
            output::info("Credential: NOT VERIFIED (Hub was unreachable at startup)");
        }
    }

    if let Some(presence) = report.product_presence() {
        eprintln!();
        output::info("Product presence:");
        let admitted = if presence.session_admitted() {
            "true"
        } else {
            "false"
        };
        let mut rows = vec![
            (
                "Status",
                presence.directory_status().as_wire_str().to_string(),
            ),
            ("Session admitted", admitted.to_string()),
        ];
        if let Some(device_ura) = presence.device_ura() {
            rows.push(("Device URA", device_ura.to_string()));
        }
        let kv: Vec<(&str, &str)> = rows.iter().map(|(k, v)| (*k, v.as_str())).collect();
        output::kv_section(&kv);
    }

    if !report.daemon().invocation_accepting() {
        output::warn("Local daemon invocation endpoint is not accepting connections.");
        return Ok(());
    }

    match StatusRuntimeReadPolicy::for_pairing_state(&pairing_state) {
        StatusRuntimeReadPolicy::UserRuntimeState => {
            let health_probe = LocalRuntimeStateReadIssuer::invoke(
                "observe.health",
                json!({"source": "runtime.status"}),
            );
            match health_probe {
                Ok(_) => {}
                Err(e) => {
                    // The transport layer already converts the common case
                    // (daemon.sock missing/refused because the daemon process
                    // is gone) into an actionable daemon-offline error with a
                    // recovery hint. Surface that one directly; wrapping it in
                    // "despite runtime metadata: …" duplicated the diagnosis
                    // and made the actionable line harder to read. For
                    // genuinely-unexpected failures (permission, protocol
                    // mismatch, etc.) keep the wrapping so the diagnosis
                    // context is preserved.
                    let inner = format!("{e}");
                    if matches!(
                        crate::support::platform::local_invoke::classify_invoke_failure(&e),
                        crate::support::platform::local_invoke::LocalInvokeFailureClass::DaemonOffline
                    ) {
                        output::warn(&inner);
                    } else {
                        output::warn(&format!(
                            "Local daemon is not responding to observe.health despite runtime metadata: {inner}"
                        ));
                    }
                    return Ok(());
                }
            }

            // Fleet view — go through `federation.discover` (the joint-plan
            // unified path the rest of the CLI uses). DirectoryEntries land
            // with a `status` field (`active` / `stale` / `draining`); we
            // count `active` as online so the summary line matches what
            // `easynet device list` shows.
            let entries = fetch_directory_entries()?;
            let total = entries.len();
            let online = entries
                .iter()
                .filter(|e| e.get("status").and_then(Value::as_str) == Some("active"))
                .count();
            let offline = total.saturating_sub(online);
            output::info(&format!("Nodes: {online} online, {offline} offline"));
        }
        StatusRuntimeReadPolicy::DeviceOwnerOperational => {
            match LocalRuntimeOperationalReadIssuer::invoke(
                "observe.health",
                json!({"source": "runtime.status"}),
            ) {
                Ok(_) => output::info("Runtime health: daemon invocation endpoint accepting"),
                Err(e) => {
                    let inner = format!("{e}");
                    if matches!(
                        crate::support::platform::local_invoke::classify_invoke_failure(&e),
                        crate::support::platform::local_invoke::LocalInvokeFailureClass::DaemonOffline
                    ) {
                        output::warn(&inner);
                    } else {
                        output::warn(&format!(
                            "Local daemon is not responding to observe.health despite runtime metadata: {inner}"
                        ));
                    }
                    return Ok(());
                }
            }
            output::info(
                "Nodes: not queried (user-scoped federation directory requires a bound user)",
            );
        }
        StatusRuntimeReadPolicy::DaemonOperationalOnly => {
            output::info(
                "Runtime health: daemon invocation endpoint accepting (device runtime-state reads require pairing)",
            );
            output::info(
                "Nodes: not queried (user-scoped federation directory requires device pairing)",
            );
        }
    }

    // Ability count — go through easynet.discover (one call,
    // returns the full local catalogue). Cheaper than the legacy
    // O(N) per-node fan-out and matches what `easynet ability list`
    // reports.
    match LocalRuntimeCatalogueReadIssuer::invoke("meta.list_abilities", serde_json::json!({})) {
        Ok(v) => {
            let count = v
                .get("abilities")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            output::info(&format!(
                "Abilities: {count} active on this node (run 'easynet ability list' for the full catalogue)"
            ));
        }
        Err(e) => output::info(&format!("Abilities: cannot query ('{e}')")),
    }
    Ok(())
}

#[derive(Debug)]
enum StatusPairingState {
    Paired(config::Credentials),
    Unpaired,
    Invalid { reason: String },
}

impl StatusPairingState {
    fn load() -> Self {
        Self::from_credentials_result(config::load_credentials_optional())
    }

    fn from_credentials_result(result: anyhow::Result<Option<config::Credentials>>) -> Self {
        match result {
            Ok(Some(credentials)) => Self::Paired(credentials),
            Ok(None) => Self::Unpaired,
            Err(error) => Self::Invalid {
                reason: format!("{error:#}"),
            },
        }
    }

    #[cfg(test)]
    fn label(&self) -> &'static str {
        match self {
            Self::Paired(_) => "paired",
            Self::Unpaired => "unpaired",
            Self::Invalid { .. } => "invalid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusRuntimeReadPolicy {
    UserRuntimeState,
    DeviceOwnerOperational,
    DaemonOperationalOnly,
}

impl StatusRuntimeReadPolicy {
    fn for_pairing_state(state: &StatusPairingState) -> Self {
        match state {
            StatusPairingState::Paired(credentials) => match credentials.runtime_user_binding() {
                Ok(config::RuntimeUserBinding::Bound { .. }) => Self::UserRuntimeState,
                Ok(config::RuntimeUserBinding::Unbound { .. }) => Self::DeviceOwnerOperational,
                Err(_) => Self::DaemonOperationalOnly,
            },
            StatusPairingState::Unpaired | StatusPairingState::Invalid { .. } => {
                Self::DaemonOperationalOnly
            }
        }
    }
}

fn render_pairing_state(state: &StatusPairingState) {
    match state {
        StatusPairingState::Paired(creds) => render_paired_credentials(creds),
        StatusPairingState::Unpaired => {
            output::info("Device: not paired (run 'easynet device join <token>')");
            eprintln!();
        }
        StatusPairingState::Invalid { reason } => {
            output::info(
                "Device: credentials invalid (run 'easynet device join <token>' to re-pair)",
            );
            output::kv_section(&[("Reason", reason.as_str())]);
            eprintln!();
        }
    }
}

// Pairing block — addressed by URA (the ontology-canonical identity per
// RFC-001 §3.2). The transport URL (creds.hub_endpoint) is intentionally NOT
// shown: it is an implementation detail. Same rule the `easynet --help` banner
// applies. `realm` is the v4.1.4 wire field name; the in-memory field is still
// `tenant_id` for migration reasons but the rendered label tracks the spec.
fn render_paired_credentials(creds: &config::Credentials) {
    output::info("Device pairing:");
    let realm = creds.realm_str();
    let hub_ura = ura::hub_ura(realm);
    let device_ura = ura::device_ura(realm, &creds.node_id);
    // Per RFC-001 §3.2, hub / user / device are all first-class agents; the user
    // row must use the immutable product user id, not the display username slug.
    let mut rows: Vec<(&str, &str)> = vec![("Hub", hub_ura.as_str())];
    let user_binding = runtime_user_binding_display(creds);
    rows.push(("Current user", user_binding.value()));
    rows.push(("Current device", device_ura.as_str()));
    rows.push(("Realm", realm));
    output::kv_section(&rows);
    eprintln!();
}

fn render_connection_state() {
    let snapshot = join_connection_state::latest_snapshot();
    output::info("Connection state:");
    let mut rows = vec![
        (
            "State",
            format!("{} [{}]", snapshot.state, snapshot.state_code),
        ),
        (
            "Transition",
            snapshot
                .interrupted_transition
                .clone()
                .or(snapshot.transition_id.clone())
                .unwrap_or_else(|| "-".to_string()),
        ),
    ];
    if let Some(failure) = snapshot.failure.as_ref() {
        rows.push(("Failure", failure.code.clone()));
        rows.push(("Reason", failure.message.clone()));
    }
    if !snapshot.device_ura.is_empty() {
        rows.push(("Device URA", snapshot.device_ura.clone()));
    }
    let kv: Vec<(&str, &str)> = rows.iter().map(|(k, v)| (*k, v.as_str())).collect();
    output::kv_section(&kv);
    eprintln!();
}

fn run_json() -> anyhow::Result<()> {
    let connection = join_connection_state::latest_snapshot();
    let payload = RuntimeLifecycleService::new()
        .status()?
        .to_json(json!(connection));
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

/// Pull the federated directory snapshot from the local daemon. Directory
/// failure is not an empty fleet: hidden signer, admission, or namespace
/// failures would otherwise be rendered as a valid zero-node state.
#[cfg(feature = "axon-pb")]
fn fetch_directory_entries() -> anyhow::Result<Vec<Value>> {
    require_fleet_directory_entries(
        crate::daemon::federation::directory_reader::read_federated_directory_for_current_user(
            None,
        ),
    )
}

#[cfg(not(feature = "axon-pb"))]
fn fetch_directory_entries() -> anyhow::Result<Vec<Value>> {
    anyhow::bail!("Fleet: federation.discover requires the 'axon-pb' feature")
}

fn require_fleet_directory_entries(
    result: anyhow::Result<Vec<Value>>,
) -> anyhow::Result<Vec<Value>> {
    result.map_err(|error| {
        anyhow::anyhow!(
            "Fleet: cannot query user-scoped federation.discover; status refuses to project this as an empty fleet: {error}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_credentials() -> config::Credentials {
        config::Credentials {
            node_id: "device-a".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "axon://hub.example:7700".to_string(),
            realm: "localhost".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".to_string()),
            user_id: Some("user-alice".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    fn device_only_credentials() -> config::Credentials {
        let mut credentials = complete_credentials();
        credentials.credential_token.clear();
        credentials.username = None;
        credentials.user_id = None;
        credentials.hub_pubkey_b64 = Some("hub-pubkey".to_string());
        credentials.join_receipt_hash = Some("sha256:test-join-receipt".to_string());
        credentials
    }

    #[test]
    fn status_pairing_state_reports_paired_credentials() {
        let state = StatusPairingState::from_credentials_result(Ok(Some(complete_credentials())));

        assert_eq!(state.label(), "paired");
    }

    #[test]
    fn status_pairing_state_reports_unpaired_only_for_missing_credentials() {
        let state = StatusPairingState::from_credentials_result(Ok(None));

        assert_eq!(state.label(), "unpaired");
    }

    #[test]
    fn status_pairing_state_rejects_malformed_credentials_as_invalid() {
        let state = StatusPairingState::from_credentials_result(Err(anyhow::anyhow!(
            "parse credentials from /tmp/credentials.json: expected value"
        )));

        assert_eq!(state.label(), "invalid");
        match state {
            StatusPairingState::Invalid { reason } => {
                assert!(
                    reason.contains("parse credentials"),
                    "invalid state must preserve reason: {reason}"
                );
            }
            other => panic!("expected invalid state, got {other:?}"),
        }
    }

    #[test]
    fn runtime_read_policy_uses_user_state_for_paired_device() {
        let state = StatusPairingState::from_credentials_result(Ok(Some(complete_credentials())));

        assert_eq!(
            StatusRuntimeReadPolicy::for_pairing_state(&state),
            StatusRuntimeReadPolicy::UserRuntimeState
        );
    }

    #[test]
    fn runtime_read_policy_uses_device_owner_operational_probe_for_device_only_credentials() {
        let state =
            StatusPairingState::from_credentials_result(Ok(Some(device_only_credentials())));

        assert_eq!(
            StatusRuntimeReadPolicy::for_pairing_state(&state),
            StatusRuntimeReadPolicy::DeviceOwnerOperational
        );
    }

    #[test]
    fn runtime_read_policy_uses_daemon_operational_probe_without_pairing() {
        let state = StatusPairingState::from_credentials_result(Ok(None));

        assert_eq!(
            StatusRuntimeReadPolicy::for_pairing_state(&state),
            StatusRuntimeReadPolicy::DaemonOperationalOnly
        );
    }

    #[test]
    fn runtime_read_policy_uses_daemon_operational_probe_for_invalid_pairing() {
        let state = StatusPairingState::from_credentials_result(Err(anyhow::anyhow!(
            "parse credentials from /tmp/credentials.json: expected value"
        )));

        assert_eq!(
            StatusRuntimeReadPolicy::for_pairing_state(&state),
            StatusRuntimeReadPolicy::DaemonOperationalOnly
        );
    }

    #[test]
    fn fleet_directory_failure_is_not_projected_as_empty_nodes() {
        let error =
            require_fleet_directory_entries(Err(anyhow::anyhow!("CALLER_SIGNER_UNAVAILABLE")))
                .expect_err("directory failure must fail closed");

        let message = error.to_string();
        assert!(
            message.contains("cannot query user-scoped federation.discover"),
            "wrong error: {message}"
        );
        assert!(
            message.contains("refuses to project this as an empty fleet"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn fleet_directory_success_returns_authoritative_entries() {
        let entries = vec![json!({
            "agent_ura": "easynet:///r/localhost/device/dev-1",
            "status": "active",
        })];

        assert_eq!(
            require_fleet_directory_entries(Ok(entries.clone())).expect("entries"),
            entries
        );
    }
}
