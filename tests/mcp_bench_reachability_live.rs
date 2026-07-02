//! Live reachability verifier for the operator's local `mcp-bench`
//! catalogue. Counts how many of the configured upstream servers
//! answer `tools/list` end-to-end, prints a per-server PASS/FAIL
//! triage list, and asserts the count holds against a baseline.
//!
//! Unlike `mcp_bench_round1_e2e` — which runs against an
//! in-process echo upstream and is part of the default test set —
//! this test depends on:
//!   * `~/.easynet/mcp_clients.json` existing on the host
//!   * each upstream's stdio command actually being installed
//!   * outbound network for any HTTP-backed upstream
//!
//! It is therefore gated behind `EASYNET_VERIFY_LOCAL_MCP_BENCH=1`
//! so a vanilla `cargo test` skips it cleanly; CI never trips it.
//!
//! Invocation:
//!
//!   EASYNET_VERIFY_LOCAL_MCP_BENCH=1 \
//!     cargo test --test mcp_bench_reachability_live -- --nocapture
//!
//! Output: one PASS / FAIL line per configured server with the
//! per-upstream failure reason on FAIL, then a `Summary` line that
//! captures the reachable count and total reflected tools. The
//! summary is the line operators / CI watchers pin; the per-line
//! breakdown is the triage feed for whichever upstream broke.
//!
//! Named-defer (haifeng review F-6 follow-up): this verifier truly
//! belongs in an `xtask` binary since it doesn't exercise an
//! in-process invariant. Keeping it in `tests/` with an explicit
//! `_live` suffix until the workspace grows an `xtask` crate.

#![cfg(unix)]

use std::path::PathBuf;

use easynet_cli::daemon::ability::builtins::integrations::mcp::reflective_registry::reflect_all;
use easynet_cli::daemon::execution::mcp_client::McpClientService;
use easynet_cli::runtime::ability_dispatch::AxonAbilityCatalog;

/// Minimum reachable-server count below which the verifier fails.
/// Pinned to the round-1 baseline so a regression in setup or in
/// the reflective registry surfaces as a test failure rather than
/// a silently smaller summary line.
const REACHABILITY_BASELINE: usize = 21;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_reachability_holds_baseline() {
    if std::env::var("EASYNET_VERIFY_LOCAL_MCP_BENCH").is_err() {
        eprintln!("skipping: set EASYNET_VERIFY_LOCAL_MCP_BENCH=1 to run");
        return;
    }
    // Per-tools-list timeout — long enough for slow startups (uv
    // first-run can pull deps), short enough that a hung upstream
    // doesn't wedge the whole verification.
    std::env::set_var("EASYNET_MCP_TOOLS_LIST_TIMEOUT_SECS", "30");

    let home = std::env::var("HOME").expect("HOME");
    let path = PathBuf::from(home).join(".easynet/mcp_clients.json");
    assert!(path.exists(), "{} missing", path.display());
    let svc = McpClientService::from_path(&path).expect("from_path must accept");
    let names = svc.server_names().await;
    eprintln!(
        "\n=== Reachability check across {} servers ===\n",
        names.len()
    );

    let mut reg = AxonAbilityCatalog::new();
    let owner = easynet_axon::ura::agent_ura("test-realm", "test-user", "mcp");
    let result = reflect_all(&svc, &mut reg, &owner).await;

    // Group failures by server.
    let mut failed_servers: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for f in &result.failed {
        if f.tool.is_none() {
            failed_servers
                .entry(f.server.clone())
                .or_insert_with(|| f.reason.clone());
        }
    }
    let mut tools_per_server: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for r in &result.registered {
        *tools_per_server.entry(r.server.clone()).or_insert(0) += 1;
    }

    eprintln!("--- per server ---");
    for name in &names {
        if let Some(reason) = failed_servers.get(name) {
            eprintln!("FAIL  {name}: {reason}");
        } else {
            let count = tools_per_server.get(name).copied().unwrap_or(0);
            eprintln!("PASS  {name}: {count} tools");
        }
    }

    let pass_count = names
        .iter()
        .filter(|n| !failed_servers.contains_key(*n))
        .count();
    let total_tools: usize = tools_per_server.values().sum();
    eprintln!(
        "\n=== Summary: {pass_count}/{} servers reachable, {total_tools} tools ===\n",
        names.len()
    );

    // Per-tool failures (collision / schema bug — not server-down).
    let per_tool_fails: Vec<_> = result.failed.iter().filter(|f| f.tool.is_some()).collect();
    if !per_tool_fails.is_empty() {
        eprintln!("--- per-tool failures ({}) ---", per_tool_fails.len());
        for f in per_tool_fails {
            eprintln!(
                "  {}::{}: {}",
                f.server,
                f.tool.as_deref().unwrap_or(""),
                f.reason
            );
        }
    }

    assert!(
        pass_count >= REACHABILITY_BASELINE,
        "reachable count {pass_count} regressed below the documented baseline of {REACHABILITY_BASELINE}"
    );
}
