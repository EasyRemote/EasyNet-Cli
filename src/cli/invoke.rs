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

use anyhow::Context;
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
    /// Timeout in seconds (0 = runtime default)
    #[arg(long, default_value_t = 0)]
    pub timeout: u64,
}

pub fn run(invoke_args: InvokeArgs) -> anyhow::Result<()> {
    let state = config::load()?;
    let br = shared::connect_bridge_to(&state.endpoint)?;
    let tenant = state.tenant_or_default();

    let arguments: serde_json::Value = match invoke_args.args.as_deref() {
        Some(s) => serde_json::from_str(s)?,
        None => serde_json::json!({}),
    };

    let timeout_ms = if invoke_args.timeout == 0 { None } else { Some(invoke_args.timeout * 1000) };
    let result = br
        .call_mcp_tool_with_timeout(tenant, &invoke_args.ability, &invoke_args.node, &arguments, timeout_ms)
        .context("invoke")?;

    println!("{}", serde_json::to_string_pretty(&result)?);
    output::success(&format!("{} on {}", invoke_args.ability, invoke_args.node));
    Ok(())
}
