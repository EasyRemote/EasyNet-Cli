// EasyNet CLI
// ===========
//
// File: src/cli/invoke.rs
// Description: `easynet invoke <ability> [--node <id>] [--args JSON]`.
//
// Routing model (single source of truth):
//
//   easynet invoke <ability>                 # auto-route: runtime picks an
//                                            # activated install within the
//                                            # caller's tenant.
//   easynet invoke <ability> --node <id>     # pinned: execute on <id>.
//
// The response always carries `selected_node_id`, so callers can see where
// an auto-routed invocation landed. Federation topology is surfaced by
// `easynet device list`; we deliberately do NOT make `invoke` pay a
// second `list_nodes` RPC just to print a cosmetic label.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{bail, Context};
use clap::Args;
use serde_json::Value;

use crate::shared::{output, timeouts};

#[derive(Debug, Args)]
pub struct InvokeArgs {
    /// Ability (tool) name to invoke.
    pub ability: String,
    /// Pin invocation to a specific device. Omit to let the runtime
    /// auto-route across any activated install in your tenant — the
    /// response always carries `selected_node_id` so you can see where
    /// the call landed.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// JSON object passed to the ability as its arguments (e.g.
    /// `--args '{"prompt": "hi"}'`). Defaults to `{}` when omitted.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Per-call deadline in seconds. `0` inherits the runtime default.
    /// Default: 60 s, governed by `shared::timeouts::INVOKE_DEFAULT_SECS`.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
}

/// Routing decision for one `invoke` call. Keeping this as an enum (rather
/// than an `Option<&str>`) lets the caller branch on intent without
/// re-checking "is this empty" twice downstream, and gives the runtime a
/// clear pinned vs auto-route signal.
enum Route<'a> {
    Pinned(&'a str),
    Auto,
}

impl Route<'_> {
    fn node_arg(&self) -> &str {
        match self {
            Route::Pinned(id) => id,
            Route::Auto => "",
        }
    }
}

pub fn run(invoke_args: InvokeArgs) -> anyhow::Result<()> {
    // Decide routing up front so the rest of this function can treat
    // pinned vs auto as a single intent, not an empty-string sentinel.
    // An explicit `--node ""` is almost always a shell-expansion accident
    // (an unset variable expanded to empty) — reject it clearly instead
    // of silently falling through to auto-route.
    let route = match invoke_args.node.as_deref().map(str::trim) {
        None => Route::Auto,
        Some("") => {
            bail!(
                "--node was given but empty; omit the flag to auto-route, or pass a real node id"
            );
        }
        Some(s) => Route::Pinned(s),
    };

    let (br, rt) = crate::persistence::config::load_and_connect()?;
    let tenant = rt.tenant_or_default();

    let arguments: Value = match invoke_args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };

    let timeout_ms = timeouts::effective_ms(invoke_args.timeout).map_err(anyhow::Error::msg)?;

    if matches!(route, Route::Auto) {
        output::info(&format!(
            "auto-routing '{}' — runtime will pick an activated install",
            invoke_args.ability
        ));
    }

    let result = br
        .call_mcp_tool_with_timeout(
            tenant,
            &invoke_args.ability,
            route.node_arg(),
            &arguments,
            timeout_ms,
        )
        .with_context(|| match &route {
            Route::Pinned(id) => format!("invoke '{}' on {}", invoke_args.ability, id),
            Route::Auto => format!("invoke '{}' (auto-route)", invoke_args.ability),
        })?;

    let selected = result
        .get("selected_node_id")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");

    println!("{}", serde_json::to_string_pretty(&result)?);

    match route {
        Route::Auto => output::success(&format!(
            "{} → auto-routed to {}",
            invoke_args.ability, selected
        )),
        Route::Pinned(_) => {
            output::success(&format!("{} on {}", invoke_args.ability, selected));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_node_arg_surface_matches_intent() {
        assert_eq!(Route::Pinned("node-x").node_arg(), "node-x");
        assert_eq!(Route::Auto.node_arg(), "");
    }
}
