// EasyNet CLI
// ===========
//
// File: src/cli/exec.rs
// Description: `easynet exec <node> -- <command>` — one-shot remote command execution
//              on a federated device, relayed through the Hub.
//
// Protocol Responsibility:
// - Invokes the built-in `session_bridge` MCP tool with action="exec" on the target node.
// - The Hub relays the call to the target device's Axon runtime, which spawns a shell,
//   runs the command, and returns {stdout, stderr, exit_code}.
// - Requires EASYNET_SESSION_BRIDGE_EXEC_ENABLED=1 on the target device (security gate).
//
// Implementation Approach:
// - Uses call_mcp_tool_with_args for synchronous request-response (no streaming).
// - Parses result from top-level or nested result_json (runtime version variance).
// - Non-zero exit codes propagate as CLI errors.
//
// Usage Contract:
// - Target node must be online and have exec enabled (via `easynet config exec on`).
// - Command is shell-evaluated on the remote device — supports pipes, redirects, etc.
// - For interactive sessions, use the web terminal; exec is strictly one-shot.
//
// Architectural Position:
// - Imperative counterpart to deploy/invoke: exec runs arbitrary commands,
//   while invoke calls registered abilities. Both route through session_bridge.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;

use crate::shared::{self, output};

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Target node ID
    pub node: String,
    /// Timeout in seconds (0 = no timeout)
    #[arg(long, default_value_t = 60)]
    pub timeout: u64,
    /// Command to execute (everything after --)
    #[arg(last = true)]
    pub command: Vec<String>,
}

/// Join command arguments into a shell-safe string.
/// Arguments containing spaces or shell metacharacters are single-quoted.
fn join_command(parts: &[String]) -> String {
    parts.iter().map(|p| {
        if p.contains(|c: char| c.is_ascii_whitespace() || "\"'\\$`|&;(){}[]<>?*#!~".contains(c)) {
            format!("'{}'", p.replace('\'', "'\"'\"'"))
        } else {
            p.clone()
        }
    }).collect::<Vec<_>>().join(" ")
}

pub fn run(args: ExecArgs) -> anyhow::Result<()> {
    anyhow::ensure!(!args.command.is_empty(), "no command specified (use -- to separate)");

    let (br, state) = shared::connect_bridge()?;
    let tenant = state.tenant_or_default();
    let cmd_str = join_command(&args.command);

    eprintln!(
        "{} tunnel via {}",
        style("┌").dim(),
        state.hub.as_deref().unwrap_or("local")
    );

    let node = &args.node;
    let timeout_secs = args.timeout;
    let call_args = serde_json::json!({
        "action": "exec",
        "command": cmd_str,
    });

    let timeout_ms = if timeout_secs == 0 { None } else { Some(timeout_secs * 1000) };
    let result = br
        .call_mcp_tool_with_timeout(tenant, "session_bridge", node, &call_args, timeout_ms)
        .context("exec")?;

    // Result may be at top level or nested in result_json.
    let payload = result.get("result_json").unwrap_or(&result);

    // Check for session_bridge error response first (e.g., exec disabled).
    if payload.get("ok") == Some(&serde_json::json!(false)) {
        let err = payload
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error (no error field in response)");
        anyhow::bail!("exec: {err}");
    }
    if let Some(stdout) = payload.get("stdout").and_then(|v| v.as_str()) {
        print!("{stdout}");
    }
    if let Some(stderr) = payload.get("stderr").and_then(|v| v.as_str()) {
        if !stderr.is_empty() { eprint!("{stderr}"); }
    }
    if let Some(code) = payload.get("exit_code").and_then(serde_json::Value::as_i64) {
        if code != 0 {
            anyhow::bail!("command exited with code {code}");
        }
    }
    output::success("done");
    Ok(())
}
