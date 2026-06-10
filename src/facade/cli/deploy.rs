// EasyNet CLI — `easynet ability deploy`
// =======================================
//
// File: src/facade/cli/deploy.rs
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

use crate::runtime::resources::filesystem::{
    resource_ref_for_local_path, FilesystemResourceCapability,
};
use crate::support::local_invoke::invoke_local_ability;
use crate::support::output;

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Path to the ability directory (must contain `ability.json`).
    /// The CLI converts it to a short-lived ResourceRef before invocation.
    pub path: String,
    /// Target device node id. Use 'local' to deploy onto this
    /// device's own ability registry; any other node id requires
    /// the federation Invoke transport (the handler returns a
    /// typed 'federation_not_wired' error in that case until it
    /// ships).
    #[arg(long = "node", short = 'n', value_name = "NODE_ID")]
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
    let result = invoke_local_ability(
        "ability.deploy",
        json!({
            "resource_ref": resource_ref,
            "node_id": args.node,
        }),
    )
    .context("invoke ability.deploy")?;
    eprintln!("{}", style("✓").green());

    if let Some(install_id) = result.get("install_id").and_then(|v| v.as_str()) {
        output::step(&format!("install_id: {install_id}"));
    }
    if let Some(ability_ura) = result.get("ability_ura").and_then(|v| v.as_str()) {
        output::success(&format!(
            "activated — {ability_ura} is live on {}",
            args.node
        ));
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}
