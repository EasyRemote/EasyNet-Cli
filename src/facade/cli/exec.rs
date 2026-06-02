// EasyNet CLI — `easynet ability exec`
// =====================================
//
// File: src/facade/cli/exec.rs
// Description: One-shot remote command execution. Local targets
//              dispatch to `process.exec` through the local daemon;
//              remote targets reuse the same
//              `federation.forward_invoke` path that powers
//              `easynet ability invoke --node`.
//
// What this CLI shim does
// -----------------------
//   1. Validate args (node + non-empty command).
//   2. Map argv → the `process.exec` JSON contract.
//   3. Route locally or remotely depending on the target.
//   4. Pipe stdout / stderr / exit_code from the response.
//
// `process.exec` is the structured execution surface (argv, no
// shell interpretation). That matches this CLI's historical
// semantics better than `shell.run`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use clap::Args;
use serde_json::{json, Value};

#[cfg(not(feature = "axon-pb"))]
use crate::support::local_invoke::federation_not_wired_error;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::{output, timeouts};

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Target device node id. Pass `local` for this device or a
    /// real node id once federation Invoke is wired.
    pub node: String,
    /// Per-call deadline in seconds. '0' inherits the runtime
    /// default. Default: 60 s ('support::timeouts::INVOKE_DEFAULT_SECS').
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// Command to execute (everything after '--'). Joined with
    /// spaces and passed to the handler verbatim; the handler
    /// chooses whether to shell-evaluate (defaults to NO — argv
    /// dispatch via 'process.exec' for safety).
    #[arg(last = true)]
    pub command: Vec<String>,
}

pub fn run(args: ExecArgs) -> anyhow::Result<()> {
    anyhow::ensure!(
        !args.command.is_empty(),
        "no command specified (use -- to separate)"
    );

    let timeout_ms = timeouts::effective_ms(args.timeout).map_err(anyhow::Error::msg)?;
    let payload = json!({
        "command": args.command[0].clone(),
        "args": args.command[1..].to_vec(),
        "timeout_ms": timeout_ms,
    });
    let result = if is_local_exec_target(&args.node) {
        invoke_local_ability("device.process.exec", payload).context("invoke process.exec")?
    } else {
        invoke_remote_process_exec(&args.node, payload)?
    };

    if result.get("ok").and_then(Value::as_bool) == Some(false) {
        let message = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("process.exec failed before spawn");
        let code = result
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("PROCESS_EXEC_FAILED");
        return Err(anyhow!("{code}: {message}"));
    }

    let stdout = decode_exec_stream(&result, "stdout");
    let stderr = decode_exec_stream(&result, "stderr");
    if !stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&stdout));
    }
    if !stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&stderr));
    }
    if let Some(code) = result.get("exit_code").and_then(Value::as_i64) {
        if code != 0 {
            anyhow::bail!("command exited with code {code}");
        }
    }
    output::success("done");
    Ok(())
}

fn is_local_exec_target(node: &str) -> bool {
    let trimmed = node.trim();
    if trimmed.is_empty() || trimmed == "local" {
        return true;
    }
    crate::persistence::config::load_credentials()
        .ok()
        .is_some_and(|creds| trimmed == creds.node_id)
}

fn decode_exec_stream(result: &Value, field: &str) -> Vec<u8> {
    let raw = result.get(field).and_then(Value::as_str).unwrap_or("");
    B64.decode(raw.as_bytes())
        .unwrap_or_else(|_| raw.as_bytes().to_vec())
}

#[cfg(feature = "axon-pb")]
fn invoke_remote_process_exec(node: &str, payload: Value) -> anyhow::Result<Value> {
    let target_ura = crate::support::remote_device::resolve_target_device_ura(node)?;
    let caller_ura = crate::support::remote_device::caller_device_ura_from_credentials();
    crate::support::federation_invoke::invoke_via_federation_forward(
        "process.exec",
        payload,
        &target_ura,
        caller_ura.as_deref(),
    )
    .with_context(|| {
        format!("invoke process.exec via federation.forward_invoke target={target_ura}")
    })
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_remote_process_exec(node: &str, _payload: Value) -> anyhow::Result<Value> {
    Err(federation_not_wired_error(&format!(
        "running a one-shot command on remote node {node:?}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_exec_target_accepts_literal_local() {
        assert!(is_local_exec_target("local"));
        assert!(is_local_exec_target(""));
    }

    #[test]
    fn decode_exec_stream_falls_back_to_plain_text() {
        let value = json!({"stdout": "hello"});
        assert_eq!(decode_exec_stream(&value, "stdout"), b"hello");
    }
}
