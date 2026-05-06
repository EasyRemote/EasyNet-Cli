// EasyNet CLI — `easynet mission think` subcommand
// =====================================================
//
// File: src/facade/cli/think.rs
// Description: Thin CLI wrapper that drives `mission.think` through
//              the local-invoke path. Same shape as
//              `easynet mission discuss`: collect args, fire the
//              ability, render the result.
//
// Why this lives at the facade layer (not as an EAL helper)
// ---------------------------------------------------------
// `mission.think` is a long-running orchestration that may take
// minutes; the operator wants progress feedback (cycles ticking,
// final verdict, curator outcome) on stderr. EAL is the right
// surface for *programmatic* invocation; the CLI surface is the
// right place for *interactive* invocation. Both go through
// `invoke_local_ability`, so the one canonical handler runs in
// either case — there is no duplicated mission.think shell.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::support::local_invoke::invoke_local_ability;

#[derive(Debug, Args)]
pub struct ThinkArgs {
    /// The worker agent (and judge by default). Must be a registered
    /// agent — confirm with `easynet agent list`.
    #[arg(long)]
    pub agent: String,

    /// The task description. The worker session sees this verbatim
    /// on cycle 1 and a continue-hint on cycles 2+.
    #[arg(long)]
    pub prompt: String,

    /// Hard cap on worker+judge cycles. Default 5; runtime ceiling
    /// HARD_MAX_CYCLES (50). The worker session_id is resumed
    /// across cycles, so each cycle picks up where the previous
    /// one left off.
    #[arg(long, default_value_t = 5)]
    pub max_cycles: u32,

    /// Optional separate judge agent. Defaults to `--agent`.
    /// Sessions are independent regardless — this flag only changes
    /// which agent's chat ability and tool catalog the judge uses.
    #[arg(long)]
    pub judge: Option<String>,

    /// Emit the full JSON envelope (transcript + verdict + curator
    /// outcome) instead of the human-readable summary. Useful when
    /// chaining mission.think output into another tool.
    #[arg(long)]
    pub json: bool,

    /// Run the full worker+judge+curator pipeline but skip the
    /// final ability.publish / skill.publish dispatch. The
    /// curator's authored body is returned in the envelope so the
    /// operator can inspect what *would* be published before
    /// touching their workspace. Recommended on first runs against
    /// an unfamiliar prompt.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(args: ThinkArgs) -> anyhow::Result<()> {
    let mut payload = json!({
        "owner_agent_id": args.agent,
        "prompt": args.prompt,
        "max_cycles": args.max_cycles,
        "dry_run": args.dry_run,
    });
    if let Some(judge) = &args.judge {
        payload["judge_agent_id"] = json!(judge);
    }

    eprintln!();
    eprintln!("{}", style("EasyNet Mission Think").cyan().bold());
    eprintln!("{}", style("═".repeat(40)).dim());
    eprintln!("  Worker:  {}", style(&args.agent).yellow());
    if let Some(j) = &args.judge {
        if j != &args.agent {
            eprintln!("  Judge:   {}", style(j).yellow());
        }
    }
    eprintln!("  Cycles:  up to {}", args.max_cycles);
    eprintln!();

    let resp = invoke_local_ability("device.mission.think", payload).context("invoke mission.think")?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    render_summary(&resp);
    Ok(())
}

/// Render the human-readable summary. Tracks four facts the
/// operator most often wants:
///   1. how many cycles ran and why the loop stopped,
///   2. what the final verdict was (memory_type / scope),
///   3. whether a curator step ran and what it published,
///   4. the worker's final reply (top of the last transcript entry).
///
/// The full transcript is too verbose for stdout; operators who
/// want it pass `--json` and pipe to `jq`.
fn render_summary(resp: &Value) {
    let cycles = resp.get("cycles_used").and_then(Value::as_u64).unwrap_or(0);
    let term = resp
        .get("termination_reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    eprintln!(
        "  Result:  {} cycle{} ({})",
        cycles,
        if cycles == 1 { "" } else { "s" },
        term
    );

    if let Some(v) = resp.get("final_verdict").filter(|v| !v.is_null()) {
        let mt = v
            .get("memory_type")
            .and_then(Value::as_str)
            .unwrap_or("none");
        let scope = v.get("scope").and_then(Value::as_str).unwrap_or("");
        eprintln!(
            "  Verdict: memory_type={} scope={}",
            style(mt).bold(),
            scope
        );
        if let Some(what) = v.get("what_to_save").and_then(Value::as_str) {
            if !what.trim().is_empty() {
                eprintln!("           what: {what}");
            }
        }
    } else {
        eprintln!("  Verdict: {}", style("(no parseable verdict)").dim());
    }

    if let Some(c) = resp.get("curator").filter(|c| !c.is_null()) {
        let attempted = c.get("attempted").and_then(Value::as_bool).unwrap_or(false);
        let ok = c.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let is_dry_run = c.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
        if attempted && ok && is_dry_run {
            // Dry-run path: validation passed, body authored, no
            // publish dispatched. Print the body so the operator
            // can inspect it; this is the load-bearing UX of the
            // --dry-run flag.
            let target = c.get("target").and_then(Value::as_str).unwrap_or("");
            eprintln!(
                "  Curator: {} → {} (validation passed)",
                style("dry-run").yellow(),
                target
            );
            if let Some(body) = c.get("authored_body").and_then(Value::as_str) {
                eprintln!();
                eprintln!("{}", style("Authored body (would-publish):").dim());
                println!("{body}");
            }
        } else if attempted && ok {
            let target = c.get("target").and_then(Value::as_str).unwrap_or("");
            eprintln!("  Curator: {} → {}", style("published").green(), target);
            if let Some(pr) = c.get("publish_result") {
                if let Some(p) = pr.get("path").and_then(Value::as_str) {
                    eprintln!("           {p}");
                } else if let Some(d) = pr.get("skill_dir").and_then(Value::as_str) {
                    eprintln!("           {d}");
                }
            }
        } else if attempted {
            let stage = c.get("stage").and_then(Value::as_str).unwrap_or("?");
            let err = c.get("error").and_then(Value::as_str).unwrap_or("?");
            eprintln!("  Curator: {} (stage={stage}) {err}", style("failed").red());
            // Validation failures include the authored_body so the
            // operator can see *what* the curator wrote that got
            // rejected. Operationally critical: without this, a
            // hallucinated EAL reference fails silently from the
            // operator's POV.
            if stage == "validate" {
                if let Some(body) = c.get("authored_body").and_then(Value::as_str) {
                    eprintln!();
                    eprintln!("{}", style("Rejected body:").dim());
                    println!("{body}");
                }
            }
        }
    }

    // Final worker reply preview — last cycle, first 800 chars.
    if let Some(arr) = resp.get("transcript").and_then(Value::as_array) {
        if let Some(last) = arr.last() {
            if let Some(w) = last.get("worker").and_then(Value::as_str) {
                let preview = if w.len() > 800 { &w[..800] } else { w };
                eprintln!();
                eprintln!("{}", style("Worker (final):").dim());
                println!("{preview}");
                if w.len() > 800 {
                    eprintln!(
                        "{}",
                        style(format!("… (truncated; pass --json for full)")).dim()
                    );
                }
            }
        }
    }
}
