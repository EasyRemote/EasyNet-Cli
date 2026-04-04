// EasyNet CLI
// ===========
//
// File: src/cli/abilities.rs
// Description: `easynet abilities` — lists MCP tools/abilities across all federated nodes.
//
// Aggregation: iterates all known nodes, calls list_mcp_tools() per node, merges results.
// Output: table (ability name, node, version, state) or JSON.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;

use crate::shared::{self, config, output::{self, OutputFormat}};

#[derive(Debug, Args)]
pub struct AbilitiesArgs {
    /// Filter by node
    #[arg(long)]
    pub node: Option<String>,
    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: AbilitiesArgs) -> anyhow::Result<()> {
    let state = config::load()?;
    let br = shared::connect_bridge_to(&state.endpoint)?;
    let tenant = state.tenant_or_default();

    let target_nodes: Vec<String> = if let Some(ref n) = args.node {
        vec![n.clone()]
    } else {
        let nodes = br.list_nodes(tenant, None).context("list nodes")?;
        nodes
            .iter()
            .filter_map(|n| n.get("node_id").and_then(|v| v.as_str()).map(String::from))
            .collect()
    };

    let mut all: Vec<serde_json::Value> = Vec::new();
    for node_id in &target_nodes {
        if let Ok(tools) = br.list_mcp_tools(tenant, "", node_id) {
            for mut tool in tools {
                if let Some(m) = tool.as_object_mut() {
                    m.insert("node_id".into(), serde_json::json!(node_id));
                }
                all.push(tool);
            }
        }
    }

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&all)?);
        return Ok(());
    }

    let mut table = output::table(&["Ability", "Node", "Version", "Status"]);
    for a in &all {
        let name = a.get("tool_name").or(a.get("ability_name")).and_then(|v| v.as_str()).unwrap_or("-");
        let node = a.get("node_id").and_then(|v| v.as_str()).unwrap_or("-");
        let ver = a.get("ability_version").and_then(|v| v.as_str()).unwrap_or("-");
        let st = a.get("state").and_then(|v| v.as_str()).unwrap_or("ACTIVE");
        table.add_row(vec![name, node, ver, st]);
    }
    println!("{table}");
    Ok(())
}
