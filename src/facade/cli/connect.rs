// EasyNet CLI
// ===========
//
// File: src/cli/connect.rs
// Description: `easynet connect` — convenience alias for `easynet start --foreground`.
//
// Protocol Responsibility:
// - Delegates entirely to `start::run()` with foreground=true.
// - Provides a simpler mental model: "join pairs, connect goes online."
//
// Implementation Approach:
// - Delegates to start::run() via StartArgs::for_connect().
// - Hub/tenant are always read from credentials inside start.rs — connect does not
//   need to resolve them, avoiding redundant credential reads.
// - The --no-mcp flag is forwarded to control stdio MCP server behavior.
//
// Architectural Position:
// - Thin UX alias. All runtime, registration, and heartbeat logic lives in start.rs.
// - Kept as a separate command to preserve the join → connect two-step user flow.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use super::start;

#[derive(Debug, Args)]
pub struct ConnectArgs {
    /// Human-readable label for this device
    #[arg(long)]
    pub label: Option<String>,
    /// Pre-shared join token
    #[arg(long)]
    pub token: Option<String>,
    /// Disable Hub-level MCP server on stdio
    #[arg(long)]
    pub no_mcp: bool,
    /// Allow insecure (plaintext) connections to the Hub
    #[arg(long)]
    pub insecure: bool,
}

pub fn run(args: ConnectArgs) -> anyhow::Result<()> {
    let mut start_args = start::StartArgs::for_connect(args.no_mcp);
    start_args.label = args.label;
    start_args.token = args.token;
    start_args.insecure = args.insecure;
    start::run(start_args)
}
