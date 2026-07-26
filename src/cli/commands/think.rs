// EasyNet CLI — `easynet mission think` subcommand
// =====================================================
//
// File: src/cli/think.rs
// Description: Thin CLI wrapper that drives `mission.think` through
//              the canonical local daemon system issuer. Same shape
//              as `easynet mission discuss`: collect args, fire the
//              ability, render the result.
//
// Why this lives at the facade layer (not as an EAL helper)
// ---------------------------------------------------------
// `mission.think` is a long-running orchestration that may take minutes; the
// operator wants progress feedback (cycles ticking, final verdict, curator
// outcome) on stderr. EAL is the right surface for *programmatic* invocation;
// the CLI surface is the right place for *interactive* invocation. The CLI
// binds the local daemon identity subject before entering transport so the one
// canonical handler runs with an explicit invocation tuple.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer;

#[derive(Debug, Args)]
pub struct ThinkArgs {
    /// The worker agent (and judge by default). Must be a registered
    /// agent — confirm with 'easynet agent list'.
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

    /// Optional separate judge agent. Defaults to '--agent'.
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
    let request = MissionThinkRequest::from_args(&args);
    let payload = request.to_payload();

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

    let resp = MissionThinkIssuer::invoke(payload)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    render_summary(&resp);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MissionThinkRequest {
    owner_agent_id: String,
    prompt: String,
    max_cycles: u32,
    judge_agent_id: Option<String>,
    dry_run: bool,
}

impl MissionThinkRequest {
    fn from_args(args: &ThinkArgs) -> Self {
        Self {
            owner_agent_id: args.agent.clone(),
            prompt: args.prompt.clone(),
            max_cycles: args.max_cycles,
            judge_agent_id: args.judge.clone(),
            dry_run: args.dry_run,
        }
    }

    fn to_payload(&self) -> Value {
        let mut payload = json!({
            "owner_agent_id": self.owner_agent_id,
            "prompt": self.prompt,
            "max_cycles": self.max_cycles,
            "dry_run": self.dry_run,
        });
        if let Some(judge) = &self.judge_agent_id {
            payload["judge_agent_id"] = json!(judge);
        }
        payload
    }
}

struct MissionThinkIssuer;

impl MissionThinkIssuer {
    fn invoke(args: Value) -> anyhow::Result<Value> {
        LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity("mission.think", args)
            .context("invoke mission.think")
    }
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
                        style("… (truncated; pass --json for full)".to_string()).dim()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_think_request_projects_cli_payload_without_judge() {
        let payload = MissionThinkRequest::from_args(&ThinkArgs {
            agent: "worker".to_string(),
            prompt: "ship the thing".to_string(),
            max_cycles: 7,
            judge: None,
            json: false,
            dry_run: true,
        })
        .to_payload();

        assert_eq!(
            payload,
            json!({
                "owner_agent_id": "worker",
                "prompt": "ship the thing",
                "max_cycles": 7,
                "dry_run": true,
            })
        );
    }

    #[test]
    fn mission_think_request_projects_optional_judge() {
        let payload = MissionThinkRequest::from_args(&ThinkArgs {
            agent: "worker".to_string(),
            prompt: "review architecture".to_string(),
            max_cycles: 3,
            judge: Some("judge".to_string()),
            json: true,
            dry_run: false,
        })
        .to_payload();

        assert_eq!(
            payload,
            json!({
                "owner_agent_id": "worker",
                "prompt": "review architecture",
                "max_cycles": 3,
                "dry_run": false,
                "judge_agent_id": "judge",
            })
        );
    }
}
