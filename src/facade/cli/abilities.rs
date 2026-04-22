// EasyNet CLI
// ===========
//
// File: src/cli/abilities.rs
// Description: `easynet abilities` — lists MCP tools/abilities across the TANet.
//
// Federation-wide single-RPC discovery:
// - When --node is omitted, a single list_mcp_tools(tenant, pattern, "") call
//   returns the deduplicated, federation-wide view. The runtime already merges
//   local installs with federated peer abilities (see interop_native/mcp.rs
//   list_tools). Each MCPToolEntry carries node_ids[] listing every node that
//   has this ability activated, so we expand one entry per (tool, node) pair
//   for the table output.
// - When --node is set, the call is scoped to that node.
//
// Output: table (ability name, node, version, state) or JSON.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{bail, Context};
use clap::Args;
use serde_json::Value;

use crate::support::{
    output::{self, OutputFormat},
};

#[derive(Debug, Args)]
pub struct AbilitiesArgs {
    /// Filter by node id (defaults to federation-wide view).
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// Glob pattern to filter by tool name (e.g. "image.*")
    #[arg(long, default_value = "")]
    pub pattern: String,
    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: AbilitiesArgs) -> anyhow::Result<()> {
    let (br, rt) = crate::persistence::config::load_and_connect()?;
    let tenant = rt.tenant_or_default();

    let scope = match args.node.as_deref().map(str::trim) {
        None => "",
        Some("") => bail!("--node was given but empty; pass a real node id or omit --node"),
        Some(s) => s,
    };
    let entries = br
        .list_mcp_tools(tenant, &args.pattern, scope)
        .context("list_mcp_tools")?;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    // Empty federation view — print a targeted hint and skip the table.
    // Rendering an empty table after a warning reads like "we found things
    // but can't show them," which is worse than a single clear message.
    if entries.is_empty() {
        output::warn("no abilities found in this TANet (try `easynet device list` first)");
        return Ok(());
    }

    let mut table = output::table(&["Ability", "Node", "Version", "Status"]);
    for entry in &entries {
        let name = entry
            .get("tool_name")
            .or_else(|| entry.get("ability_name"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        let ver = entry
            .get("ability_version")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let st = entry
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("ACTIVE");

        // Expand one row per node that has this ability activated.
        let mut node_ids: Vec<&str> = entry
            .get("node_ids")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        node_ids.sort_unstable();
        node_ids.dedup();

        if node_ids.is_empty() {
            // Fallbacks for older runtimes that don't populate node_ids:
            //   1. legacy per-entry node_id field
            //   2. the --node scope itself (when scope is non-empty)
            //   3. "-" placeholder (consistent with the other columns)
            let legacy = entry.get("node_id").and_then(Value::as_str);
            let single = legacy
                .or(Some(scope).filter(|s| !s.is_empty()))
                .unwrap_or("-");
            table.add_row(vec![name, single, ver, st]);
        } else {
            for nid in &node_ids {
                table.add_row(vec![name, nid, ver, st]);
            }
        }
    }

    println!("{table}");
    Ok(())
}
