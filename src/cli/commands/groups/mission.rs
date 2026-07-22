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

use clap::{Args, Subcommand};
use console::style;

use crate::cli::commands::discuss as discuss_cmd;
use crate::cli::commands::mission_runs::{self, CancelOutcome};
use crate::cli::commands::think as think_cmd;
use crate::eal;
use crate::support::platform::output::{self, OutputFormat};

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
    /// Long-running task with worker+judge+curator sessions. The
    /// worker resumes its session_id across cycles to outgrow
    /// per-session step limits; the judge classifies whether
    /// anything from the run is worth saving as a published
    /// ability or private skill; the curator authors the
    /// deliverable and dispatches publish.
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
    /// Output format. 'table' keeps the human progress lines; 'json'
    /// emits the machine run record on stdout — including 'trace_id',
    /// the anchor 'easynet invocation watch --trace' attaches to
    /// (seven-axes T2.0). Conflicts with '--trace', which owns stdout.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table, conflicts_with = "trace")]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Maximum number of runs to show.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Output format. 'table' prints the human-readable run history;
    /// 'json' emits the meta records as a JSON array. Aligned with
    /// every other list/show command — see 'support::output::OutputFormat'.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
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
        // Mission patterns: `discuss` is a special-shaped external EAL
        // mission (multi-agent round-robin orchestration). `think` is
        // the long-running counterpart: a worker/judge/curator triple
        // that lets a task outgrow Claude Code's per-session step
        // limit and, when the judge identifies a sinkable lesson, has
        // the curator author + publish a real ability or skill. Both
        // live here because they are not agent-instance methods.
        //
        // History note: an earlier `think` was deleted (modern agent
        // runtimes already do think-act-observe internally inside
        // chat). This rewrite restores the verb with a different
        // contract: it is no longer a stand-in for chat's inner loop;
        // it is the loop that runs ABOVE chat to author durable
        // abilities/skills from cross-session experience.
        MissionAction::Discuss(a) => discuss_cmd::run(a),
        MissionAction::Think(a) => think_cmd::run(a),
    }
}

fn run_compile(args: CompileArgs) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(&args.file)?;
    let program = eal::parser::parse(&source)?;
    let ir = eal::runtime::planner::compile(&program)?;
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

    // Show a quick pre-run summary based on a parse-only pass. We compile
    // here purely to get the IR shape for the banner; the real execution
    // happens inside `mission.run`, which compiles again. The
    // double-compile is cheap (parser + planner are pure) and keeps the
    // banner / single-entry contract orthogonal.
    let program = eal::parser::parse(&source)?;
    let ir = eal::runtime::planner::compile(&program)?;
    let total_steps = ir.steps.len();
    let total_phases = ir.phases.len();
    // PR-10: walk every leaf call (block variants like `loop` / `chat`
    // nest IrSteps; count the distinct targets across all leaves).
    let mut leaves: Vec<&eal::runtime::ir::IrCall> = Vec::new();
    for s in &ir.steps {
        s.walk_calls(&mut leaves);
    }
    let node_count = leaves
        .iter()
        .map(|c| c.target.display_string())
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

    let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura()?;
    let value =
        crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer::invoke_root_for_subject_timeout(
        crate::daemon::ability::builtins::automation::mission::ABILITY_RUN,
        serde_json::json!({"source": source, "label": args.file}),
        &local_device_ura,
        std::time::Duration::from_secs(3600),
    )?;
    let result: crate::daemon::ability::builtins::automation::mission::MissionRunResponse =
        serde_json::from_value(value).map_err(|error| {
            anyhow::anyhow!("mission.run returned an invalid response: {error}")
        })?;
    let run_dir = std::path::PathBuf::from(&result.run_dir);

    if matches!(args.format, OutputFormat::Json) {
        // Machine record on stdout (progress already went to stderr).
        // `trace_id` is the runtime anchor of the whole run — the id
        // every step Invocation carries and the handle
        // `invocation watch --trace` attaches to (seven-axes T2.0).
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "run_id": result.run_id,
                "trace_id": result.meta.trace_id,
                "status": result.meta.status,
                "duration_ms": result.meta.duration_ms,
                "steps_total": result.meta.steps_total,
                "steps_completed": result.meta.steps_completed,
                "steps_failed": result.meta.steps_failed,
                "run_dir": result.run_dir,
            }))?
        );
        return Ok(());
    }

    eprintln!();
    eprintln!(
        "  {} {:.1}s, {node_count} targets, {total_phases} phases",
        style("done").green().bold(),
        result.meta.duration_ms as f64 / 1000.0,
    );
    eprintln!(
        "  {} {}",
        style("saved").dim(),
        style(&result.run_dir).cyan()
    );
    if args.trace {
        // Print the persisted trace.json (the runner has already written
        // it). This avoids re-serializing in-memory state and keeps the
        // CLI handler dependency-free of `ExecutionReport`.
        let trace_path = run_dir.join("trace.json");
        if let Ok(trace) = std::fs::read_to_string(&trace_path) {
            eprintln!();
            println!("{trace}");
        }
    }

    Ok(())
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let runs = mission_runs::list_runs()?;

    if args.format == OutputFormat::Json {
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
        output::info("No mission runs recorded yet. Run 'easynet mission run <file.eal>'.");
        return Ok(());
    }

    let mut table = output::table(&["ID", "Mission", "Status", "Steps", "Duration"]);
    for r in runs.iter().take(args.limit) {
        let status = if r.running {
            "running".to_string()
        } else {
            r.meta.status.to_string()
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
    eprintln!(
        "  {} {}",
        style("mission").dim(),
        style(&run.meta.name).bold()
    );
    output::detail("id", &run.id);
    output::detail("path", &run.path.display().to_string());
    output::detail(
        "status",
        if run.running {
            "running"
        } else {
            run.meta.status.as_str()
        },
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
