// EasyNet CLI — Mission Group
// ===========================
//
// File: src/cli/groups/mission.rs
// Description: `easynet mission …` — full lifecycle for EAL programs.
//
// Verbs:
//   compile <file>    Parse + plan to Mission IR v2 (no execution)   (NEW)
//   run <file>        Compile and execute, recording history          (refactor of cli::mission)
//   list              Show recorded mission runs                      (NEW)
//   show <id>         Show one run's source / IR / trace / meta       (NEW)
//   cancel <id>       Mark an in-flight run as cancelled              (NEW, best-effort)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use chrono::Local;
use clap::{Args, Subcommand};
use console::style;

use crate::cli::mission_runs::{self, CancelOutcome, MissionRunDir, MissionRunMeta};
use crate::cli::{discuss as discuss_cmd, think as think_cmd};
use crate::eal;
use crate::shared::{config, output};

#[derive(Debug, Args)]
pub struct MissionArgs {
    #[command(subcommand)]
    pub action: MissionAction,
}

#[derive(Debug, Subcommand)]
pub enum MissionAction {
    /// Parse and plan an EAL program without executing it.
    Compile(CompileArgs),
    /// Compile and execute an EAL program (records run history).
    Run(RunArgs),
    /// List recorded mission runs.
    List(ListArgs),
    /// Show one mission run's full detail.
    Show(ShowArgs),
    /// Mark an in-flight mission run as cancelled.
    Cancel(CancelArgs),
    /// Multi-agent orchestration pattern (round-robin discussion).
    Discuss(discuss_cmd::DiscussArgs),
    /// Iterative planning loop pattern (think → act → observe → repeat).
    Think(think_cmd::ThinkArgs),
}

#[derive(Debug, Args)]
pub struct CompileArgs {
    /// Path to a `.eal` file.
    pub file: String,
    /// Pretty-print the planned Mission IR JSON to stdout.
    #[arg(long)]
    pub emit_ir: bool,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Path to a `.eal` file.
    pub file: String,
    /// Print the full execution trace JSON after completion.
    #[arg(long)]
    pub trace: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Maximum number of runs to show.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Emit raw JSON instead of the table view.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Run id (timestamp directory name) or unique prefix.
    pub id: String,
    /// Print the full execution trace JSON instead of the meta summary.
    #[arg(long)]
    pub trace: bool,
}

#[derive(Debug, Args)]
pub struct CancelArgs {
    /// Run id (timestamp directory name) or unique prefix.
    pub id: String,
}

pub fn run(args: MissionArgs) -> anyhow::Result<()> {
    match args.action {
        MissionAction::Compile(a) => run_compile(a),
        MissionAction::Run(a) => run_run(a),
        MissionAction::List(a) => run_list(a),
        MissionAction::Show(a) => run_show(a),
        MissionAction::Cancel(a) => run_cancel(a),
        // Mission patterns: both `discuss` and `think` are special-shaped
        // External EAL missions (multi-agent orchestration; iterative
        // planning loop). They are *not* agent-instance methods, so this
        // is their canonical home — the `agent` group only keeps them as
        // deprecated aliases. See ARCHITECTURE.md §8 #5.
        MissionAction::Discuss(a) => discuss_cmd::run(a),
        MissionAction::Think(a) => think_cmd::run(a),
    }
}

fn run_compile(args: CompileArgs) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(&args.file)?;
    let program = eal::parser::parse(&source)?;
    let ir = eal::planner::compile(&program)?;
    if args.emit_ir {
        println!("{}", serde_json::to_string_pretty(&ir)?);
    } else {
        eprintln!(
            "  {} {} ({} steps, {} phases)",
            style("✓").green(),
            style(&ir.name).bold(),
            ir.steps.len(),
            ir.phases.len()
        );
    }
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn run_run(args: RunArgs) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(&args.file)?;

    let program = eal::parser::parse(&source)?;
    let ir = eal::planner::compile(&program)?;

    let state = config::load()?;
    let tenant = state.tenant_or_default();

    let total_steps = ir.steps.len();
    let total_phases = ir.phases.len();
    let node_count = ir
        .steps
        .iter()
        .map(|s| s.target_node_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();

    eprintln!();
    eprintln!(
        "  {} {}",
        style(&ir.name).white().bold(),
        style(format!(
            "{total_steps} steps, {node_count} targets, {total_phases} phases"
        ))
        .dim(),
    );

    // Persist the run.
    let run_dir = MissionRunDir::create(&ir.name)?;
    run_dir.write_source(&source);
    if let Ok(ir_json) = serde_json::to_string_pretty(&ir) {
        run_dir.write_ir(&ir_json);
    }
    let started = std::time::Instant::now();
    let started_at = Local::now().to_rfc3339();

    let exec = eal::interpreter::execute_with_endpoint(&state.endpoint, tenant, &ir);

    let duration_ms = started.elapsed().as_millis() as u64;
    let mut meta = MissionRunMeta {
        name: ir.name.clone(),
        source_file: Some(args.file.clone()),
        started_at,
        duration_ms,
        status: "ok".into(),
        error: None,
        steps_total: total_steps,
        steps_completed: 0,
        steps_failed: 0,
        ability_graph_traces: None,
    };

    match &exec {
        Ok(report) => {
            meta.steps_completed = report.steps_completed;
            meta.steps_failed = report.steps_failed;
            // The interpreter returns Ok even when individual steps fail —
            // surface that as "partial" so the listing doesn't lie about
            // a run with broken steps.
            if report.steps_failed > 0 {
                meta.status = "partial".into();
            }
            if let Ok(trace_json) = serde_json::to_string_pretty(&report.trace) {
                run_dir.write_trace(&trace_json);
            }
            run_dir.write_meta(&meta);
            run_dir.finish();

            eprintln!();
            eprintln!(
                "  {} {:.1}s, {node_count} targets, {total_phases} phases",
                style("done").green().bold(),
                report.total_elapsed_ms as f64 / 1000.0,
            );
            eprintln!(
                "  {} {}",
                style("saved").dim(),
                style(run_dir.path.display().to_string()).cyan()
            );
            if args.trace {
                eprintln!();
                println!("{}", serde_json::to_string_pretty(&report.trace)?);
            }
            Ok(())
        }
        Err(e) => {
            meta.status = "error".into();
            meta.error = Some(e.to_string());
            run_dir.write_meta(&meta);
            run_dir.finish();
            Err(anyhow::anyhow!("mission run failed: {e}"))
        }
    }
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let runs = mission_runs::list_runs()?;

    if args.json {
        let pretty: Vec<serde_json::Value> = runs
            .iter()
            .take(args.limit)
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "running": r.running,
                    "meta": r.meta,
                    "path": r.path,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&pretty)?);
        return Ok(());
    }

    if runs.is_empty() {
        output::info("No mission runs recorded yet. Run `easynet mission run <file.eal>`.");
        return Ok(());
    }

    let mut table = output::table(&["ID", "Mission", "Status", "Steps", "Duration"]);
    for r in runs.iter().take(args.limit) {
        let status = if r.running {
            "running".to_string()
        } else {
            r.meta.status.clone()
        };
        let steps = format!("{}/{}", r.meta.steps_completed, r.meta.steps_total);
        let dur = format!("{:.1}s", r.meta.duration_ms as f64 / 1000.0);
        table.add_row(vec![&r.id, &r.meta.name, &status, &steps, &dur]);
    }
    println!("{table}");
    Ok(())
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let run = mission_runs::find_run(&args.id)?;
    eprintln!();
    eprintln!("  {} {}", style("mission").dim(), style(&run.meta.name).bold());
    output::detail("id", &run.id);
    output::detail("path", &run.path.display().to_string());
    output::detail(
        "status",
        if run.running { "running" } else { run.meta.status.as_str() },
    );
    output::detail(
        "duration",
        &format!("{:.1}s", run.meta.duration_ms as f64 / 1000.0),
    );
    output::detail(
        "steps",
        &format!("{}/{}", run.meta.steps_completed, run.meta.steps_total),
    );
    if let Some(err) = &run.meta.error {
        output::detail("error", err);
    }
    eprintln!();

    if args.trace {
        let trace_path = run.path.join("trace.json");
        if trace_path.exists() {
            let trace = std::fs::read_to_string(trace_path)?;
            println!("{trace}");
        } else {
            output::info("no trace.json recorded for this run");
        }
    }
    Ok(())
}

fn run_cancel(args: CancelArgs) -> anyhow::Result<()> {
    match mission_runs::cancel_run(&args.id)? {
        CancelOutcome::Cancelled(run) => {
            output::success(&format!("cancelled {}", run.id));
        }
        CancelOutcome::AlreadyTerminal(run) => {
            output::info(&format!(
                "{} is already terminal (status: {}); nothing to cancel",
                run.id, run.meta.status
            ));
        }
    }
    Ok(())
}
