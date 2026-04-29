// EasyNet CLI — `easynet ability deploy`
// =======================================
//
// File: src/facade/cli/deploy.rs
// Description: Publish an ability bundle to a target node.
//
// Per the ability-only ontology, deploying an ability is itself an
// ability invocation: the operator invokes `fleet.deploy_ability`
// on the local daemon, passing the local path and target node id.
// The daemon-side handler reads the bundle, validates the manifest,
// computes the integrity digest, and publishes through the
// federation transport. Single-node case (the only deployable
// target is `local`) lands the bundle into the local registry; the
// multi-node fan-out lights up the day federation Invoke ships.
//
// What this CLI shim does
// -----------------------
//   1. Validate args locally (path exists, node id non-empty).
//   2. Map args → JSON request body.
//   3. invoke_local_ability("fleet.deploy_ability", body).
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

use crate::support::local_invoke::invoke_local_ability;
use crate::support::output;

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Path to the ability directory (must contain `ability.json`).
    pub path: String,
    /// Target device node id. Use `local` to deploy onto this
    /// device's own ability registry; any other node id requires
    /// the federation Invoke transport (the handler returns a
    /// typed `federation_not_wired` error in that case until it
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
    let result = invoke_local_ability(
        "fleet.deploy_ability",
        json!({
            "path": args.path,
            "node_id": args.node,
        }),
    )
    .context("invoke fleet.deploy_ability")?;
    eprintln!("{}", style("✓").green());

    if let Some(install_id) = result.get("install_id").and_then(|v| v.as_str()) {
        output::step(&format!("install_id: {install_id}"));
    }
    if let Some(name) = result.get("ability_name").and_then(|v| v.as_str()) {
        output::success(&format!("activated — {name} is live on {}", args.node));
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}
