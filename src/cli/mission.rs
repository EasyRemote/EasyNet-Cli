// EasyNet CLI
// ===========
//
// File: src/cli/mission.rs
// Description: `easynet mission run <file.eal>` — compile and execute EAL programs.
//
// Pipeline:
//   .eal source → parser → analyzer → planner → Mission IR v2 → interpreter
//
// Modes:
//   --emit-ir   Output Mission IR v2 JSON without executing (compilation verification).
//   --trace     Output full ExecutionTrace JSON after execution (audit log).
//
// The interpreter uses BridgeDispatcher for true parallel dispatch within phases.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};

use crate::eal;
use crate::shared::{config, output};

#[derive(Debug, Args)]
pub struct MissionArgs {
    #[command(subcommand)]
    command: MissionCommand,
}

#[derive(Debug, Subcommand)]
enum MissionCommand {
    /// Execute an EAL program
    Run(RunArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Path to a `.eal` file.
    file: String,
    /// Emit Mission IR v2 JSON without executing
    #[arg(long)]
    emit_ir: bool,
    /// Output full execution trace as JSON after completion
    #[arg(long)]
    trace: bool,
}

pub fn run(args: MissionArgs) -> anyhow::Result<()> {
    match args.command {
        MissionCommand::Run(run_args) => run_mission(run_args),
    }
}

fn run_mission(args: RunArgs) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(&args.file)?;

    let program = eal::parser::parse(&source)?;
    let ir = eal::planner::compile(&program)?;

    if args.emit_ir {
        println!("{}", serde_json::to_string_pretty(&ir)?);
        return Ok(());
    }

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

    output::info(&format!(
        "mission: {} ({total_steps} steps, {node_count} nodes, {total_phases} phases)",
        ir.name
    ));

    let report = eal::interpreter::execute_with_endpoint(&state.endpoint, tenant, &ir)?;

    output::info("");
    output::success(&format!(
        "Mission completed — {:.1}s across {node_count} nodes ({total_phases} phases)",
        report.total_elapsed_ms as f64 / 1000.0,
    ));

    if args.trace {
        eprintln!();
        println!("{}", serde_json::to_string_pretty(&report.trace)?);
    }

    Ok(())
}
