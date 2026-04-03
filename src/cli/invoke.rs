// EasyNet CLI
// ===========
//
// File: src/cli/invoke.rs
// Description: `easynet invoke <node> <ability> [--args JSON]` — direct ability invocation.
//
// Routes through Hub federation to the target node's activated ability.
// Result is pretty-printed JSON.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::shared::{self, config, output};

#[derive(Debug, Args)]
pub struct InvokeArgs {
    /// Target node ID
    pub node: String,
    /// Ability name
    pub ability: String,
    /// JSON arguments
    #[arg(long)]
    pub args: Option<String>,
}

pub fn run(invoke_args: InvokeArgs) -> anyhow::Result<()> {
    let state = config::load()?;
    let br = shared::connect_bridge()?;
    let tenant = state.tenant_or_default();

    let arguments: serde_json::Value = match invoke_args.args.as_deref() {
        Some(s) => serde_json::from_str(s)?,
        None => serde_json::json!({}),
    };

    let result = br
        .call_mcp_tool_with_args(tenant, &invoke_args.ability, &invoke_args.node, &arguments)
        .map_err(|e| anyhow::anyhow!("invoke: {e}"))?;

    println!("{}", serde_json::to_string_pretty(&result)?);
    output::success(&format!("{} on {}", invoke_args.ability, invoke_args.node));
    Ok(())
}
