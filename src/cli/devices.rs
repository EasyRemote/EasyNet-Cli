// EasyNet CLI
// ===========
//
// File: src/cli/devices.rs
// Description: `easynet devices` — lists all nodes across the federation.
//
// Output: colored table (●/○ indicators) or JSON. Filterable by state (online/offline).
// Data: DendriteBridge.list_nodes() returns federated peer nodes via Hub heartbeat sync.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::shared::{self, config, output};

#[derive(Debug, Args)]
pub struct DevicesArgs {
    /// Filter by state (online, offline)
    #[arg(long)]
    pub state: Option<String>,
    /// Output format (table, json)
    #[arg(long, default_value = "table")]
    pub format: String,
}

pub fn run(args: DevicesArgs) -> anyhow::Result<()> {
    let state = config::load()?;
    let br = shared::connect_bridge()?;
    let tenant = state.tenant_or_default();

    let nodes = br
        .list_nodes(tenant, None)
        .map_err(|e| anyhow::anyhow!("list nodes: {e}"))?;

    let filtered: Vec<_> = nodes
        .iter()
        .filter(|n| {
            args.state.as_deref().map_or(true, |filter| {
                let s = n.get("state").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
                match filter {
                    "online" => s == "HEALTHY" || s == "ONLINE",
                    "offline" => s != "HEALTHY" && s != "ONLINE",
                    _ => s.eq_ignore_ascii_case(filter),
                }
            })
        })
        .collect();

    if args.format == "json" {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    let mut table = output::table(&["", "Node", "State", "OS", "Trust"]);
    for n in &filtered {
        let node_id = n.get("node_id").and_then(|v| v.as_str()).unwrap_or("-");
        let node_state = n.get("state").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
        let os = n.get("os").and_then(|v| v.as_str()).unwrap_or("-");
        let arch = n.get("arch").and_then(|v| v.as_str()).unwrap_or("");
        let trust = n.get("trust_level").and_then(|v| v.as_str()).unwrap_or("-");
        let os_display = if arch.is_empty() { os.to_string() } else { format!("{os}/{arch}") };

        table.add_row(vec![
            output::node_indicator(node_state),
            node_id.to_string(),
            node_state.to_string(),
            os_display,
            trust.to_string(),
        ]);
    }
    println!("{table}");
    Ok(())
}
