// EasyNet CLI
// ===========
//
// File: src/cli/exec.rs
// Description: `easynet exec <node> -- <command>` — one-shot remote command execution.
//
// Mechanism: invokes the `session_bridge` MCP tool on the target node via Hub relay.
// The command string is passed as JSON argument; stdout/stderr/exit_code returned.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use console::style;

use crate::shared::{self, config, output};

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Target node ID
    pub node: String,
    /// Command to execute (everything after --)
    #[arg(last = true)]
    pub command: Vec<String>,
}

pub fn run(args: ExecArgs) -> anyhow::Result<()> {
    anyhow::ensure!(!args.command.is_empty(), "no command specified (use -- to separate)");

    let state = config::load()?;
    let br = shared::connect_bridge()?;
    let tenant = state.tenant_or_default();
    let cmd_str = args.command.join(" ");

    eprintln!(
        "{} tunnel via {} (E2E encrypted)",
        style("┌").dim(),
        state.hub.as_deref().unwrap_or("local")
    );

    let result = br
        .call_mcp_tool_with_args(tenant, "session_bridge", &args.node, &serde_json::json!({ "command": cmd_str }))
        .map_err(|e| anyhow::anyhow!("exec: {e}"))?;

    if let Some(stdout) = result.get("stdout").and_then(|v| v.as_str()) {
        print!("{stdout}");
    }
    if let Some(stderr) = result.get("stderr").and_then(|v| v.as_str()) {
        if !stderr.is_empty() { eprint!("{stderr}"); }
    }
    if let Some(code) = result.get("exit_code").and_then(|v| v.as_i64()) {
        if code != 0 {
            anyhow::bail!("command exited with code {code}");
        }
    }
    output::success("done");
    Ok(())
}
