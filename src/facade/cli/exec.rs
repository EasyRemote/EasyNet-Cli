// EasyNet CLI — `easynet ability exec`
// =====================================
//
// File: src/facade/cli/exec.rs
// Description: One-shot remote command execution. Per the
//              ability-only ontology this is `fleet.exec_remote`
//              invoked on the local daemon; the daemon either
//              runs the command in-process (when `node == local`)
//              or forwards through federation transport (when
//              the target is a remote node id).
//
// What this CLI shim does
// -----------------------
//   1. Validate args (node + non-empty command).
//   2. Map args → JSON.
//   3. invoke_local_ability("fleet.exec_remote", body).
//   4. Pipe stdout / stderr / exit_code from the response.
//
// Local case: handler delegates to `process.exec`. Remote case:
// handler forwards via federation Invoke (returns typed
// federation_not_wired until that transport ships).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use serde_json::json;

use crate::support::local_invoke::invoke_local_ability;
use crate::support::{output, timeouts};

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Target device node id. Pass `local` for this device or a
    /// real node id once federation Invoke is wired.
    pub node: String,
    /// Per-call deadline in seconds. `0` inherits the runtime
    /// default. Default: 60 s (`support::timeouts::INVOKE_DEFAULT_SECS`).
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// Command to execute (everything after `--`). Joined with
    /// spaces and passed to the handler verbatim; the handler
    /// chooses whether to shell-evaluate (defaults to NO — argv
    /// dispatch via `process.exec` for safety).
    #[arg(last = true)]
    pub command: Vec<String>,
}

pub fn run(args: ExecArgs) -> anyhow::Result<()> {
    anyhow::ensure!(
        !args.command.is_empty(),
        "no command specified (use -- to separate)"
    );

    let timeout_ms = timeouts::effective_ms(args.timeout).map_err(anyhow::Error::msg)?;
    let result = invoke_local_ability(
        "fleet.exec_remote",
        json!({
            "node_id": args.node,
            "command": args.command,
            "timeout_ms": timeout_ms,
        }),
    )
    .context("invoke fleet.exec_remote")?;

    if let Some(stdout) = result.get("stdout").and_then(|v| v.as_str()) {
        print!("{stdout}");
    }
    if let Some(stderr) = result.get("stderr").and_then(|v| v.as_str()) {
        if !stderr.is_empty() {
            eprint!("{stderr}");
        }
    }
    if let Some(code) = result.get("exit_code").and_then(serde_json::Value::as_i64) {
        if code != 0 {
            anyhow::bail!("command exited with code {code}");
        }
    }
    output::success("done");
    Ok(())
}
