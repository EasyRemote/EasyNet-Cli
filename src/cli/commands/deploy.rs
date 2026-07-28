// EasyNet CLI — `easynet ability deploy`
// =======================================
//
// File: src/cli/deploy.rs
// Description: Publish an ability bundle to a canonical Device URA.
//
// Per the ability-only ontology, deploying an ability is itself an
// ability invocation: the operator invokes `ability.deploy`
// on the local daemon, passing a short-lived filesystem ResourceRef
// and target Device URA. The daemon-side handler reads the bundle,
// validates the manifest, computes the integrity digest, and publishes
// through the canonical device ability registrar. Local deploy lands the
// bundle into the local registry; remote Device URAs fail closed as an
// Unsupported capability state until the capability matrix marks remote
// mutation provider-backed.
//
// What this CLI shim does
// -----------------------
//   1. Validate args locally (path exists, target resolves to a Device URA).
//   2. Mint a ResourceRef and map args → JSON request body.
//   3. Invoke `ability.deploy` through LocalDaemonSystemAbilityIssuer with the
//      minted ResourceRef URA as the daemon-system subject.
//   4. Print the daemon's response.
//
// All policy (manifest validation, signature handling, ordering of
// publish→install→activate) lives inside the ability handler.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::json;

use crate::daemon::resources::files::{resource_ref_for_local_path, FilesystemResourceCapability};
use crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer;
use crate::support::platform::output::{self, OutputFormat};

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Path to the ability directory (must contain `ability.json`).
    /// The CLI converts it to a short-lived ResourceRef before invocation.
    /// Current device deployment accepts executable manifests whose `[exec]`
    /// binding is `kind = "host_stream"`; other exec kinds are rejected by
    /// the daemon until their runtime boundaries are implemented.
    pub path: String,
    /// Target device. Defaults to 'local', which resolves to this daemon's
    /// canonical Device URA before daemon invocation. Remote targets must be
    /// canonical Device URAs; bare node ids are rejected before mutation.
    #[arg(
        long = "node",
        short = 'n',
        value_name = "DEVICE_URA",
        default_value = "local"
    )]
    pub node: String,
    /// Output format. `json` prints the daemon's deploy result to stdout
    /// so scripts can consume ability_ura, install_id, and state directly.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: DeployArgs) -> anyhow::Result<()> {
    let dir = std::path::Path::new(&args.path);
    anyhow::ensure!(dir.is_dir(), "{} is not a directory", args.path);
    anyhow::ensure!(
        !args.node.trim().is_empty(),
        "--node was given but empty; pass `local` for this device or a canonical Device URA"
    );
    let target_ura = crate::support::platform::remote_device::resolve_cli_device_target_ura(
        Some(&args.node),
        "ability deploy",
    )?;

    eprint!(
        "  deploying {} to {} ... ",
        style(&args.path).cyan(),
        style(&target_ura).cyan()
    );
    let resource_ref = resource_ref_for_local_path(dir, FilesystemResourceCapability::Read)
        .context("mint ability bundle ResourceRef")?;
    let subject_ura = resource_ref
        .get("resource_ura")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("minted ResourceRef did not include resource_ura"))?;
    let result = LocalDaemonSystemAbilityIssuer::invoke_root_for_subject(
        "ability.deploy",
        json!({
            "resource_ref": resource_ref,
            "target_ura": target_ura,
        }),
        &subject_ura,
    )
    .context("invoke ability.deploy")?;
    eprintln!("{}", style("✓").green());

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if let Some(install_id) = result.get("install_id").and_then(|v| v.as_str()) {
        output::step(&format!("install_id: {install_id}"));
    }
    match (
        result.get("state").and_then(|v| v.as_str()),
        result.get("ability_ura").and_then(|v| v.as_str()),
    ) {
        (Some("ACTIVE"), Some(ability_ura)) => {
            output::success(&format!("{ability_ura} is active on {}", args.node));
        }
        (Some("INSTALLED"), Some(ability_ura)) => {
            output::step(&format!(
                "{ability_ura} installed on {}; activation is pending route availability",
                args.node
            ));
        }
        _ => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::OutputFormat;
    use crate::cli::commands::groups::ability::AbilityAction;
    use crate::cli::{App, Command};

    #[test]
    fn deploy_json_format_is_a_stable_machine_surface() {
        let app = App::parse_from([
            "easynet",
            "ability",
            "deploy",
            "/tmp/native-ability",
            "--node",
            "local",
            "--format",
            "json",
        ]);

        match app.command {
            Command::Ability(args) => match args.action {
                AbilityAction::Deploy(deploy) => {
                    assert_eq!(deploy.path, "/tmp/native-ability");
                    assert_eq!(deploy.node, "local");
                    assert_eq!(deploy.format, OutputFormat::Json);
                }
                other => panic!("expected ability deploy, got {other:?}"),
            },
            other => panic!("expected ability command, got {other:?}"),
        }
    }

    #[test]
    fn deploy_target_ura_rejects_bare_node_id_before_daemon_payload() {
        let err = crate::support::platform::remote_device::resolve_cli_device_target_ura(
            Some("node-a"),
            "ability deploy",
        )
        .expect_err("bare node id must not be accepted");
        let message = err.to_string();
        assert!(
            message.contains("canonical URA") || message.contains("Device URA"),
            "unexpected target error: {message}"
        );
    }
}
