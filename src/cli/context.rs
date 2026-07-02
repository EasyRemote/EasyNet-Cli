// EasyNet CLI — `easynet context` (clipboard tracking + folder mappings)
// =======================================================================
//
// File: src/cli/context.rs
// Description: Operator controls for the Context surface.
//
//   easynet context clipboard on|off|status   — toggle clipboard history
//   easynet context add <path> [--name NAME]  — map a project folder
//   easynet context remove <name-or-path>     — unmap
//   easynet context list                      — mapped folders + tracking state
//
// These commands write `~/.easynet/context/` directly rather than
// invoking daemon abilities: mapping a folder grants filesystem read
// access and toggling capture is a privacy switch — both stay local,
// operator-initiated acts that must work even when the daemon is
// down. The daemon's clipboard tracker re-reads the config every
// tick, so toggles take effect within seconds without a restart.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use console::style;

use crate::persistence::context_store;

#[derive(Debug, Args)]
pub struct ContextArgs {
    #[command(subcommand)]
    pub action: ContextAction,
}

#[derive(Debug, Subcommand)]
pub enum ContextAction {
    /// Toggle or inspect clipboard history tracking (text + screenshots).
    Clipboard(ClipboardArgs),
    /// Map a folder into the Context surface (browsable from the console).
    Add(AddArgs),
    /// Remove a mapped folder by name or path.
    Remove(RemoveArgs),
    /// List mapped folders and the clipboard tracking state.
    List,
}

#[derive(Debug, Args)]
pub struct ClipboardArgs {
    #[command(subcommand)]
    pub action: ClipboardAction,
}

#[derive(Debug, Subcommand)]
pub enum ClipboardAction {
    /// Start capturing clipboard history on this device.
    On,
    /// Stop capturing.
    Off,
    /// Show whether tracking is enabled and how many clips are stored.
    Status,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Folder to map (must exist).
    pub path: String,
    /// Display name (defaults to the directory name).
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Mapped folder name or path.
    pub key: String,
}

pub fn run(args: ContextArgs) -> anyhow::Result<()> {
    match args.action {
        ContextAction::Clipboard(a) => run_clipboard(a),
        ContextAction::Add(a) => run_add(a),
        ContextAction::Remove(a) => run_remove(a),
        ContextAction::List => run_list(),
    }
}

fn run_clipboard(args: ClipboardArgs) -> anyhow::Result<()> {
    match args.action {
        ClipboardAction::On => {
            context_store::set_clipboard_tracking(true)?;
            println!(
                "{} clipboard tracking enabled (daemon picks it up within a few seconds)",
                style("✓").green()
            );
        }
        ClipboardAction::Off => {
            context_store::set_clipboard_tracking(false)?;
            println!("{} clipboard tracking disabled", style("✓").green());
        }
        ClipboardAction::Status => {
            let tracking = context_store::clipboard_tracking();
            let clips = context_store::list_clips(200);
            println!(
                "tracking: {}   stored clips: {}{}",
                if tracking {
                    style("on").green().to_string()
                } else {
                    style("off").dim().to_string()
                },
                clips.len(),
                if clips.len() == 200 { "+" } else { "" },
            );
        }
    }
    Ok(())
}

fn run_add(args: AddArgs) -> anyhow::Result<()> {
    let mapping = context_store::add_folder(&args.path, args.name.as_deref())?;
    println!(
        "{} mapped {} {} {}",
        style("✓").green(),
        style(&mapping.name).white().bold(),
        style("→").dim(),
        style(&mapping.path).cyan(),
    );
    Ok(())
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let removed = context_store::remove_folder(&args.key)?;
    println!(
        "{} unmapped {} ({})",
        style("✓").green(),
        style(&removed.name).white().bold(),
        style(&removed.path).dim(),
    );
    Ok(())
}

fn run_list() -> anyhow::Result<()> {
    let tracking = context_store::clipboard_tracking();
    println!(
        "clipboard tracking: {}",
        if tracking {
            style("on").green().to_string()
        } else {
            style("off").dim().to_string()
        }
    );
    let folders = context_store::list_folders();
    if folders.is_empty() {
        println!("no mapped folders — add one with `easynet context add <path>`");
        return Ok(());
    }
    println!("{:<24} PATH", "NAME");
    for f in folders {
        let missing = if std::path::Path::new(&f.path).is_dir() {
            String::new()
        } else {
            format!("  {}", style("(missing)").red())
        };
        println!("{:<24} {}{}", f.name, f.path, missing);
    }
    Ok(())
}
