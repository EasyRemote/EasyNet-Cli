// EasyNet CLI — `easynet ability deploy`
// =======================================
//
// File: src/cli/deploy.rs
// Description: Publish an ability bundle to a target node.
//
// Per the ability-only ontology, deploying an ability is itself an
// ability invocation: the operator invokes `ability.deploy`
// on the local daemon, passing a short-lived filesystem ResourceRef
// and target node id. The daemon-side handler reads the bundle,
// validates the manifest, computes the integrity digest, and publishes
// through the federation transport. Single-node case (the only
// deployable target is `local`) lands the bundle into the local
// registry; the multi-node fan-out lights up the day federation Invoke
// ships.
//
// What this CLI shim does
// -----------------------
//   1. Validate args locally (path exists, node id non-empty).
//   2. Mint a ResourceRef and map args → JSON request body.
//   3. invoke_local_ability("ability.deploy", body).
//   4. Print the daemon's response.
//
// All policy (manifest validation, signature handling, ordering of
// publish→install→activate) lives inside the ability handler so
// the federation Invoke replacement carries the same contract
// without rewriting the CLI.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::json;

use crate::daemon::resources::files::{resource_ref_for_local_path, FilesystemResourceCapability};
use crate::support::platform::local_invoke::invoke_local_ability_with_subject;
use crate::support::platform::output;

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Path to the ability directory (must contain `ability.json`).
    /// The CLI converts it to a short-lived ResourceRef before invocation.
    /// Current device deployment accepts executable manifests whose `[exec]`
    /// binding is `kind = "host_stream"`; other exec kinds are rejected by
    /// the daemon until their runtime boundaries are implemented.
    pub path: String,
    /// Target device node id. Defaults to 'local', the only fully
    /// implemented target in this release. Any other node id requires
    /// the federation Invoke transport and returns a typed
    /// 'federation_not_wired' error until that transport ships.
    #[arg(
        long = "node",
        short = 'n',
        value_name = "NODE_ID",
        default_value = "local"
    )]
    pub node: String,
}

pub fn run(args: DeployArgs) -> anyhow::Result<()> {
    let dir = std::path::Path::new(&args.path);
    anyhow::ensure!(dir.is_dir(), "{} is not a directory", args.path);
    anyhow::ensure!(
        !args.node.trim().is_empty(),
        "--node was given but empty; pass `local` for this device or a real node id"
    );

    eprint!(
        "  deploying {} to {} ... ",
        style(&args.path).cyan(),
        style(&args.node).cyan()
    );
    let resource_ref = resource_ref_for_local_path(dir, FilesystemResourceCapability::Read)
        .context("mint ability bundle ResourceRef")?;
    let subject_ura = resource_ref
        .get("resource_ura")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("minted ResourceRef did not include resource_ura"))?;
    let result = invoke_local_ability_with_subject(
        "ability.deploy",
        json!({
            "resource_ref": resource_ref,
            "node_id": args.node,
        }),
        Some(subject_ura),
    )
    .context("invoke ability.deploy")?;
    eprintln!("{}", style("✓").green());

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
