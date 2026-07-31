// EasyNet CLI — Device Group
// ==========================
//
// File: src/cli/groups/device.rs
// Description: `easynet device …` — manage *hosting substrates*.
//
// Ontology note (interpretation C, see ARCHITECTURE.md §6):
//
//   A device is NOT a network first-class entity. The only network
//   first-class objects are Agents (network actors) and Abilities (their
//   public methods). A device is the *physical substrate* on which agents
//   are hosted — analogous to an OS to a process. Devices are visible to
//   the CLI for diagnostic and lifecycle reasons (pairing, hardware
//   inventory, capacity), but they are NOT addressable as the "to" of an
//   ability call from outside.
//
// Verbs:
//   join <token>      Pair THIS host as a substrate                       (-> cli::join)
//   reset             Un-pair this host                                   (-> cli::reset)
//   config            Per-host runtime settings                           (-> cli::config_cmd)
//                     (semantically belongs to `runtime config`; physical
//                      command stays here for now — see ARCHITECTURE.md §8 #6)
//   list              List substrates known to the federation             (-> cli::devices)
//   show <id>         Inspect one substrate (hardware, hosted abilities)
//   abilities <id>    List a substrate's invocation-backed ability catalog
//   exec <id> -- cmd  Execute process.exec through canonical invocation
//   terminal <id>     Reserved for session-authority-backed PTY lifecycle
//   remove <id>       Drain + deregister a remote substrate
//
// Verbs DELIBERATELY ABSENT:
//
//   rename / tag — would require an `easynet_admin` ability deployed on
//                  the target substrate. That ability does not exist in
//                  this PR, so the verbs would either silently fail or
//                  succeed half-way. Skipping them is the honest choice.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{bail, Context};
use clap::{Args, Subcommand};
use console::style;
use serde_json::{json, Value};

use crate::cli::commands::ability_catalog_row::AbilityCatalogueRow;
use crate::cli::commands::{abilities, config_cmd, devices, exec, join, reset};
use crate::cli::daemon_client::remote_system_ability::{
    invoke_remote_device_system_ability, invoke_remote_device_system_ability_as_caller,
    RemoteDeviceSystemAbility,
};
use crate::support::platform::local_invoke::LocalRuntimeDeviceDirectoryReadIssuer;
use crate::support::platform::output::{self, OutputFormat};

#[derive(Debug, Args)]
pub struct DeviceArgs {
    #[command(subcommand)]
    pub action: DeviceAction,
}

#[derive(Debug, Subcommand)]
pub enum DeviceAction {
    /// Pair this host as a hosting substrate (registers credentials).
    Join(join::JoinArgs),
    /// Un-pair this host (delete local credentials and state).
    Reset(reset::ResetArgs),
    /// Show or update local runtime settings (will move to `runtime config` in a future release).
    Config(config_cmd::ConfigArgs),
    /// List hosting substrates known to the federation.
    List(devices::DevicesArgs),
    /// Show one substrate's hardware and hosted abilities.
    Show(ShowArgs),
    /// List a substrate's invocation-backed ability catalog.
    Abilities(DeviceAbilitiesArgs),
    /// Run a one-shot command through process.exec on a device.
    Exec(exec::ExecArgs),
    /// Reserved for session-authority-backed PTY lifecycle.
    Terminal(TerminalArgs),
    /// Drain in-flight work on a remote substrate, then deregister it
    /// from the federation (the device disappears from
    /// `device list`). Irreversible without a fresh pairing token —
    /// prompts for confirmation unless `--yes` is passed.
    Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Target substrate node id.
    pub node_id: String,
    /// Resolve the target through the local realm Authority instead of local
    /// device credentials.
    #[arg(long)]
    pub authority: bool,
    /// Output format. 'table' emits the human-readable view; 'json'
    /// emits the raw substrate record + abilities array. Aligned with
    /// every other list/show command — see 'support::output::OutputFormat'.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct DeviceAbilitiesArgs {
    /// Target substrate: `local`, this device's node id, this device's URA,
    /// or a canonical remote Device URA.
    pub node_id: String,
    /// Glob pattern to filter by ability name.
    #[arg(long, default_value = "")]
    pub pattern: String,
    /// Output format — table (human, default) or json.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct TerminalArgs {
    /// Target substrate: `local`, this device's node id, this device's URA,
    /// or a canonical remote Device URA.
    pub node_id: String,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Target substrate node id.
    pub node_id: String,
    /// Revoke the target through the local realm Authority instead of local
    /// device credentials.
    #[arg(long)]
    pub authority: bool,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Reason recorded on the deregister event.
    #[arg(long, default_value = "removed via easynet device remove")]
    pub reason: String,
}

pub fn run(args: DeviceArgs) -> anyhow::Result<()> {
    match args.action {
        DeviceAction::Join(a) => join::run(a),
        DeviceAction::Reset(a) => reset::run(a),
        DeviceAction::Config(a) => config_cmd::run(a),
        DeviceAction::List(a) => devices::run(a),
        DeviceAction::Show(a) => run_show(a),
        DeviceAction::Abilities(a) => run_abilities(a),
        DeviceAction::Exec(a) => exec::run(a),
        DeviceAction::Terminal(a) => run_terminal(a),
        DeviceAction::Remove(a) => run_remove(a),
    }
}

fn run_abilities(args: DeviceAbilitiesArgs) -> anyhow::Result<()> {
    abilities::run(abilities::AbilitiesArgs {
        agent: None,
        agent_ura: None,
        subject_ura: None,
        node: Some(resolve_device_catalog_target_arg(&args.node_id)?),
        pattern: args.pattern,
        format: args.format,
    })
}

fn run_terminal(args: TerminalArgs) -> anyhow::Result<()> {
    let _target = resolve_device_catalog_target_arg(&args.node_id)?;
    bail!(
        "device terminal is not cut over yet: terminal follow-up abilities require \
         x-runtime-session-authority metadata, and the CLI invocation issuer does not \
         yet mint and attach that authority. Use `easynet device abilities {}` to \
         inspect terminal.* availability; PTY lifecycle will be enabled only after \
         the session-authority-backed issuer lands.",
        args.node_id
    )
}

fn resolve_device_catalog_target_arg(raw_target: &str) -> anyhow::Result<String> {
    if is_local_device_target(raw_target) {
        Ok("local".to_string())
    } else {
        crate::support::platform::remote_device::resolve_target_device_ura(raw_target)
    }
}

fn is_local_device_target(raw_target: &str) -> bool {
    let target = raw_target.trim();
    if target.is_empty() || target.eq_ignore_ascii_case("local") {
        return true;
    }
    load_local_device_identity("device target")
        .map(|identity| target == identity.node_id || target == identity.device_ura())
        .unwrap_or(false)
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    // Cross-device dispatch flows through canonical Invocation::Invoke. The
    // CLI accepts two target states only: local/self, or an explicit canonical
    // Device URA. Bare remote ids are not enough material to construct a
    // descriptor-bound route.
    let node = describe_target(&args.node_id, args.authority)
        .with_context(|| format!("describe node {}", args.node_id))?;

    let abilities = device_show_abilities(&node)?;
    // Refer to the borrowed slot below as `&node` to keep parity
    // with the legacy variable name; the dereferences are checked.
    let node = &node;

    if args.format == OutputFormat::Json {
        let payload = json!({"node": node, "abilities": abilities});
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let display_name = node
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&args.node_id);

    let state = device_show_state(node)?;
    let paired = node
        .get("paired")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let online = state == "HEALTHY" || state == "REGISTERED" || state == "STANDALONE";

    let dot = if online {
        style("●").green()
    } else {
        style("●").red()
    };

    eprintln!();
    eprintln!("  {} {}", dot, style(display_name).bold());
    output::detail("node_id", &args.node_id);
    output::detail("state", &state);
    output::detail("paired", &format!("{paired}"));
    if let Some(os) = node.pointer("/device/os").and_then(|v| v.as_str()) {
        output::detail("os", os);
    }
    if let Some(ver) = node.pointer("/device/os_version").and_then(|v| v.as_str()) {
        output::detail("os_version", ver);
    }
    if let Some(arch) = node
        .pointer("/device/architecture")
        .and_then(|v| v.as_str())
    {
        output::detail("arch", arch);
    }
    if let Some(model) = node
        .pointer("/device/hardware_model")
        .and_then(|v| v.as_str())
    {
        output::detail("model", model);
    }
    if let Some(ms) = node
        .get("last_seen_unix_ms")
        .and_then(serde_json::Value::as_i64)
    {
        output::detail("last_seen", &output::relative_time(ms));
    }

    eprintln!();
    eprintln!(
        "  {} {} {}",
        style(format!("{}", abilities.len())).bold(),
        if abilities.len() == 1 {
            "ability"
        } else {
            "abilities"
        },
        style("hosted on this substrate").dim(),
    );
    for a in &abilities {
        let row = AbilityCatalogueRow::from_value(a)?;
        let owner = row.owner_ura().unwrap_or("-");
        let ability_ura = row.ability_ura().unwrap_or("-");
        eprintln!(
            "    {} {}  {}  {}",
            style("·").dim(),
            style(row.label()).cyan(),
            style(ability_ura).dim(),
            style(owner).dim(),
        );
    }
    eprintln!();
    eprintln!(
        "  {}",
        style(
            "Reminder: this substrate is not network-addressable on its own. \
             Network calls always target an agent's published ability; the \
             substrate is the place where that ability happens to run."
        )
        .dim()
    );
    eprintln!();
    Ok(())
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    // Joint-plan unified path: `device remove` calls
    // `federation.revoke` directly through the daemon's gRPC
    // InvocationServer (the same surface
    // `daemon/federation/advertise.rs::revoke_agent` and the heartbeat
    // sidecar's shutdown hook use). The legacy `node.remove`
    // ability was a P1.5 placeholder — local-arm refused with
    // "use device reset", remote-arm raised `federation_not_wired`
    // — so it never moved real federation state. The new path
    // reaches the hub's `PresenceRegistry::force_revoke` and the
    // advertised-agent store so downstream `device list` / `auth
    // devices` immediately stop returning the entry.

    let trimmed = args.node_id.trim();
    let scope = if args.authority {
        DeviceRemoveInvocationScope::load_local_authority("device remove")?
    } else {
        DeviceRemoveInvocationScope::LocalDevice(load_local_device_identity("device remove")?)
    };
    let (target_ura, caller_ura) = scope.target_and_caller(trimmed)?;

    if !args.yes {
        let prompt = format!(
            "Drain and deregister substrate '{}' from the federation?",
            args.node_id
        );
        if !output::confirm(&prompt)? {
            output::info("aborted");
            return Ok(());
        }
    }

    invoke_revoke(&target_ura, &args.reason, caller_ura.as_str())
        .with_context(|| format!("revoke {target_ura}"))?;

    output::success(&format!("removed {}", args.node_id));
    Ok(())
}

enum DeviceRemoveInvocationScope {
    LocalDevice(DeviceLocalIdentity),
    LocalAuthority {
        authority_ura: String,
        realm: String,
    },
}

impl DeviceRemoveInvocationScope {
    fn load_local_authority(operation: &str) -> anyhow::Result<Self> {
        let authority_ura = crate::daemon::identity::local_invocation::local_daemon_ura()
            .with_context(|| {
                format!("{operation} --authority requires a local Authority daemon")
            })?;
        let parsed = crate::core::ura::parse_ura(&authority_ura)
            .with_context(|| format!("{operation} --authority local daemon URA is invalid"))?;
        if parsed.kind != crate::core::ura::URAKind::Authority {
            bail!(
                "{operation} --authority requires a local Authority daemon, got kind={}",
                parsed.kind
            );
        }
        Ok(Self::LocalAuthority {
            authority_ura,
            realm: parsed.realm,
        })
    }

    fn target_and_caller(&self, raw_target: &str) -> anyhow::Result<(String, String)> {
        match self {
            Self::LocalDevice(local_identity) => {
                if raw_target == local_identity.node_id {
                    anyhow::bail!(
                        "refusing to revoke this device's own node id ({}); use \
                         `easynet device reset` to clear local credentials and \
                         deregister cleanly.",
                        local_identity.node_id
                    );
                }
                let target_ura = canonicalize_remove_target_ura(raw_target)?;
                let local_ura = local_identity.device_ura();
                if local_ura == target_ura {
                    anyhow::bail!(
                        "refusing to revoke this device's own URA ({local_ura}); use \
                         `easynet device reset` to clear local credentials and \
                         deregister cleanly."
                    );
                }
                Ok((target_ura, local_ura))
            }
            Self::LocalAuthority {
                authority_ura,
                realm,
            } => Ok((
                authority_realm_device_target_ura(raw_target, realm)?,
                authority_ura.clone(),
            )),
        }
    }
}

fn canonicalize_remove_target_ura(ura: &str) -> anyhow::Result<String> {
    let target = ura.trim();
    if target.is_empty() {
        bail!("device remove target must not be empty; pass a canonical Device URA");
    }
    let parsed = crate::core::ura::parse_ura(target).map_err(|err| {
        anyhow::anyhow!(
            "device remove target {target:?} is not a canonical Device URA: {err}. \
             Pass `easynet:///r/<realm>/device/<id>`."
        )
    })?;
    if parsed.kind != crate::core::ura::URAKind::Device {
        bail!(
            "device remove target {target:?} must be a canonical Device URA, got kind={}",
            parsed.kind
        );
    }
    Ok(target.to_string())
}

#[cfg(feature = "axon-pb")]
fn invoke_revoke(target_ura: &str, reason: &str, caller_ura: &str) -> anyhow::Result<()> {
    crate::daemon::invocation::routing::remote_invoke::invoke_federation_revoke(
        target_ura, reason, caller_ura,
    )
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_revoke(target_ura: &str, _reason: &str, _caller_ura: &str) -> anyhow::Result<()> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(&format!(
            "revoking {target_ura:?}"
        )),
    )
}

/// Joint-plan unified-path dispatch for `easynet device show`.
///
/// Resolves `node_id` into the right `node.describe` call:
///
///   * `local` or matches this daemon's own node id → invoke
///     `node.describe` locally over the control socket.
///   * canonical URA pointing at a remote device → canonical_invoke
///     `node.describe` against that URA.
fn describe_target(node_id: &str, authority: bool) -> anyhow::Result<Value> {
    let trimmed = node_id.trim();
    if authority {
        return describe_target_from_local_authority(trimmed);
    }
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("local") {
        return LocalRuntimeDeviceDirectoryReadIssuer::describe_node(
            serde_json::json!({"node_id": "local"}),
        )
        .context("invoke node.describe (local)");
    }

    let local_identity = load_local_device_identity("device show")?;
    match classify_device_show_target(trimmed, &local_identity)? {
        DeviceShowTarget::Local => LocalRuntimeDeviceDirectoryReadIssuer::describe_node(
            serde_json::json!({"node_id": "local"}),
        )
        .context("invoke node.describe (local)"),
        DeviceShowTarget::RemoteDevice(target_ura) => invoke_remote_describe(&target_ura),
    }
}

fn invoke_remote_describe(node: &str) -> anyhow::Result<Value> {
    invoke_remote_device_system_ability(
        node,
        RemoteDeviceSystemAbility::NodeDescribe,
        serde_json::json!({"node_id": "local"}),
        &format!("describing remote device {node:?}"),
    )
}

fn invoke_remote_describe_as_caller(target_ura: &str, caller_ura: &str) -> anyhow::Result<Value> {
    invoke_remote_device_system_ability_as_caller(
        target_ura,
        RemoteDeviceSystemAbility::NodeDescribe,
        serde_json::json!({"node_id": "local"}),
        caller_ura,
    )
}

fn describe_target_from_local_authority(raw: &str) -> anyhow::Result<Value> {
    let authority_ura = crate::daemon::identity::local_invocation::local_daemon_ura()
        .context("device show --authority requires a local Authority daemon")?;
    let authority = crate::core::ura::parse_ura(&authority_ura)
        .context("device show --authority local daemon URA is invalid")?;
    if authority.kind != crate::core::ura::URAKind::Authority {
        bail!(
            "device show --authority requires a local Authority daemon, got kind={}",
            authority.kind
        );
    }
    let target_ura = authority_realm_device_target_ura(raw, &authority.realm)?;
    invoke_remote_describe_as_caller(&target_ura, &authority_ura)
}

fn authority_realm_device_target_ura(raw: &str, realm: &str) -> anyhow::Result<String> {
    let target = raw.trim();
    if target.is_empty() || target.eq_ignore_ascii_case("local") {
        bail!("hub operator device show requires a target Device URA or node id");
    }
    if crate::core::ura::parse_ura(target).is_ok() {
        let parsed = crate::core::ura::parse_ura(target)?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            bail!(
                "device show target {target:?} must be a canonical Device URA, got kind={}",
                parsed.kind
            );
        }
        return Ok(target.to_string());
    }
    Ok(crate::core::ura::device_ura(realm, target))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceShowTarget {
    Local,
    RemoteDevice(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceLocalIdentity {
    realm: String,
    node_id: String,
}

impl DeviceLocalIdentity {
    fn from_credentials(
        creds: &crate::daemon::persistence::config::Credentials,
    ) -> anyhow::Result<Self> {
        let realm = creds.realm_str().trim();
        let node_id = creds.node_id.trim();
        if realm.is_empty() {
            bail!("local device credentials are missing realm");
        }
        if node_id.is_empty() {
            bail!("local device credentials are missing node_id");
        }
        Ok(Self {
            realm: realm.to_string(),
            node_id: node_id.to_string(),
        })
    }

    fn device_ura(&self) -> String {
        crate::core::ura::device_ura(&self.realm, &self.node_id)
    }
}

fn load_local_device_identity(operation: &str) -> anyhow::Result<DeviceLocalIdentity> {
    let creds = crate::daemon::persistence::config::load_credentials()
        .with_context(|| format!("{operation} requires complete local device credentials"))?;
    DeviceLocalIdentity::from_credentials(&creds)
}

fn classify_device_show_target(
    raw: &str,
    local_identity: &DeviceLocalIdentity,
) -> anyhow::Result<DeviceShowTarget> {
    let target = raw.trim();
    if target.is_empty()
        || target.eq_ignore_ascii_case("local")
        || target == local_identity.node_id
        || target == local_identity.device_ura()
    {
        return Ok(DeviceShowTarget::Local);
    }

    let parsed = crate::core::ura::parse_ura(target).map_err(|err| {
        anyhow::anyhow!(
            "device show remote target {target:?} is not a canonical Device URA: {err}. \
             Pass `easynet:///r/<realm>/device/<id>` or use `local` for this device."
        )
    })?;
    if parsed.kind != crate::core::ura::URAKind::Device {
        bail!(
            "device show target {target:?} must be a canonical Device URA, got kind={}",
            parsed.kind
        );
    }
    Ok(DeviceShowTarget::RemoteDevice(target.to_string()))
}

fn device_show_abilities(node: &Value) -> anyhow::Result<Vec<Value>> {
    node.get("abilities")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "node.describe response omitted `abilities`; device show requires the canonical describe abilities projection"
            )
        })
}

fn device_show_state(node: &Value) -> anyhow::Result<String> {
    node.get("state")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "node.describe response omitted canonical string `state`; device show requires the canonical describe state projection"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_identity() -> DeviceLocalIdentity {
        DeviceLocalIdentity {
            realm: "acme".to_string(),
            node_id: "dev-a".to_string(),
        }
    }

    fn complete_credentials() -> crate::daemon::persistence::config::Credentials {
        crate::daemon::persistence::config::Credentials {
            node_id: "dev-a".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "axon://hub.example:50051".to_string(),
            realm: "acme".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".to_string()),
            user_id: Some("alice-id".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    #[test]
    fn device_show_rejects_bare_remote_target() {
        let identity = local_identity();
        let error = classify_device_show_target("386b1258-3c89-494a-90a2-2321c29bf992", &identity)
            .expect_err("bare remote ids must not be accepted");

        let message = error.to_string();
        assert!(
            message.contains("not a canonical Device URA"),
            "wrong error: {message}"
        );
        assert!(
            message.contains("easynet:///r/<realm>/device/<id>"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn device_catalog_target_preserves_local_selector_and_rejects_bare_remote_id() {
        assert_eq!(
            resolve_device_catalog_target_arg("local").expect("local catalogue target"),
            "local"
        );

        let error = resolve_device_catalog_target_arg("386b1258-3c89-494a-90a2-2321c29bf992")
            .expect_err("bare remote ids must not be repaired from directory state");
        let message = error.to_string();
        assert!(
            message.contains("not a canonical URA"),
            "wrong error: {message}"
        );
        assert!(
            message.contains("easynet:///r/<realm>/device/<id>"),
            "wrong recovery guidance: {message}"
        );
    }

    #[test]
    fn device_terminal_is_explicit_session_authority_seam() {
        let error = run_terminal(TerminalArgs {
            node_id: "local".to_string(),
        })
        .expect_err("terminal must not create a PTY without session authority metadata");
        let message = error.to_string();
        assert!(
            message.contains("not cut over yet"),
            "wrong terminal seam error: {message}"
        );
        assert!(
            message.contains("x-runtime-session-authority"),
            "terminal seam must name missing authority metadata: {message}"
        );
    }

    #[test]
    fn device_show_classifies_local_and_canonical_remote_targets() {
        let identity = local_identity();
        assert_eq!(
            classify_device_show_target("local", &identity).expect("local"),
            DeviceShowTarget::Local
        );
        assert_eq!(
            classify_device_show_target("dev-a", &identity).expect("self"),
            DeviceShowTarget::Local
        );
        assert_eq!(
            classify_device_show_target("easynet:///r/acme/device/dev-a", &identity)
                .expect("self URA"),
            DeviceShowTarget::Local
        );
        assert_eq!(
            classify_device_show_target("easynet:///r/acme/device/dev-b", &identity)
                .expect("remote"),
            DeviceShowTarget::RemoteDevice("easynet:///r/acme/device/dev-b".to_string())
        );
    }

    #[test]
    fn hub_operator_device_show_projects_bare_id_into_authority_realm_device_ura() {
        assert_eq!(
            authority_realm_device_target_ura("386b1258-3c89-494a-90a2-2321c29bf992", "localhost",)
                .expect("authority-local device target"),
            "easynet:///r/localhost/device/386b1258-3c89-494a-90a2-2321c29bf992"
        );
    }

    #[test]
    fn hub_operator_device_show_rejects_non_device_ura() {
        let error =
            authority_realm_device_target_ura("easynet:///r/localhost/authority", "localhost")
                .expect_err("authority target is not a device");

        assert!(
            error.to_string().contains("must be a canonical Device URA"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn local_device_identity_requires_complete_credentials() {
        let creds = complete_credentials();
        let identity = DeviceLocalIdentity::from_credentials(&creds).expect("complete credentials");
        assert_eq!(identity.node_id, "dev-a");
        assert_eq!(identity.realm, "acme");
        assert_eq!(identity.device_ura(), "easynet:///r/acme/device/dev-a");

        let mut missing_realm = complete_credentials();
        missing_realm.realm.clear();
        let error = DeviceLocalIdentity::from_credentials(&missing_realm)
            .expect_err("blank realm must fail closed");
        assert!(
            error.to_string().contains("missing realm"),
            "wrong error: {error}"
        );

        let mut missing_node = complete_credentials();
        missing_node.node_id.clear();
        let error = DeviceLocalIdentity::from_credentials(&missing_node)
            .expect_err("blank node_id must fail closed");
        assert!(
            error.to_string().contains("missing node_id"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn device_show_requires_describe_payload_abilities() {
        let error = device_show_abilities(&json!({"node_id": "dev-a"}))
            .expect_err("missing abilities must fail closed");

        let message = error.to_string();
        assert!(
            message.contains("omitted `abilities`"),
            "wrong error: {message}"
        );
        assert!(
            message.contains("requires the canonical describe abilities projection"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn device_show_uses_describe_payload_abilities() {
        let abilities = vec![json!({"name": "meta.list_abilities"})];
        assert_eq!(
            device_show_abilities(&json!({"abilities": abilities.clone()})).expect("abilities"),
            abilities
        );
    }

    #[test]
    fn device_show_requires_string_describe_state() {
        let missing = device_show_state(&json!({"abilities": []}))
            .expect_err("missing state must fail closed");
        assert!(
            missing
                .to_string()
                .contains("omitted canonical string `state`"),
            "wrong error: {missing}"
        );

        let numeric =
            device_show_state(&json!({"state": 3})).expect_err("numeric state must fail closed");
        assert!(
            numeric
                .to_string()
                .contains("requires the canonical describe state projection"),
            "wrong error: {numeric}"
        );

        assert_eq!(
            device_show_state(&json!({"state": "HEALTHY"})).expect("string state"),
            "HEALTHY"
        );
    }

    #[test]
    fn device_remove_rejects_bare_remote_target() {
        let error = canonicalize_remove_target_ura("386b1258-3c89-494a-90a2-2321c29bf992")
            .expect_err("bare remote ids must not be accepted");

        let message = error.to_string();
        assert!(
            message.contains("not a canonical Device URA"),
            "wrong error: {message}"
        );
        assert!(
            message.contains("easynet:///r/<realm>/device/<id>"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn device_remove_accepts_only_canonical_device_ura() {
        assert_eq!(
            canonicalize_remove_target_ura("easynet:///r/acme/device/dev-b").expect("device"),
            "easynet:///r/acme/device/dev-b"
        );

        let error = canonicalize_remove_target_ura("easynet:///r/acme/authority")
            .expect_err("authority is not a removable device target");
        assert!(
            error.to_string().contains("must be a canonical Device URA"),
            "wrong error: {error}"
        );
    }
}
