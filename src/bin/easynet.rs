// EasyNet CLI — thin bin wrapper
// ==============================
//
// File: src/bin/easynet.rs
// Description: Thin entry point for the `easynet` user-facing CLI.
//
// v10.5 R1: PR-DAEMON split the historical single-bin crate into
// a library + two bins. All dispatch, subcommand logic, and
// business code live in `easynet_cli` (the library); this bin is
// just `App::parse()` plus one call into the library's facade.
//
// Why keep a bin at all
// ---------------------
// The daemon-aware client (loading `libeasynet_cli`) does not use
// this entry point; it links the library directly. This bin serves
// the classic `easynet <command>` invocations that existed before
// PR-DAEMON — `easynet agent list`, `easynet skill install`,
// `easynet mission run`, etc. — preserving a one-shell-call UX and
// avoiding a hard dependency on the daemon for operations that
// don't need it.
//
// What this bin contains
// ----------------------
// The parsed clap App → `cli::run(...)` and nothing else.
// If a subcommand needs the daemon (PR-ATTACH / PR-PERM / etc.),
// its handler inside the library takes care of spawning or
// connecting to `easynet-daemon` via the IPC layer.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{CommandFactory, FromArgMatches};
use easynet_cli::cli::{presentation::banner, run, App};

/// Hard wrap point for all `--help` output. Set on the root
/// `Command` and recursively on every subcommand below — clap's
/// `derive` macro accepts neither `term_width` nor `max_term_width`
/// as `#[command(...)]` attributes (silently ignored), so the only
/// way to enforce a width cap is via the builder API after the
/// derive-built `Command` tree is in hand.
///
/// Why 100: matches the conventional CLI width (kubectl, gh, cargo);
/// the screen-wrap orphans silan flagged (`ub`, `stener.`, `ng if`)
/// disappear because clap takes over wrapping at 100 cols and
/// indents continuation lines under the description column. Wider
/// terminals get a left margin of whitespace, which is the standard
/// trade-off for predictable layout.
const HELP_TERM_WIDTH: usize = 100;

fn main() -> anyhow::Result<()> {
    // Top-level `--help` (`easynet`, `easynet --help`, `easynet -h`,
    // `easynet help`) gets a decorated banner above clap's help —
    // ASCII logo, creator blessing, live daemon/hub status. Any
    // other invocation (subcommand `--help`, real subcommand call,
    // version flag) skips the banner so it does not pollute scripts
    // or noise up `easynet device --help`.
    if is_top_level_help_invocation() {
        print!("{}", banner::render_top_level_banner());
    }

    // Force a 100-column wrap point on every command in the tree.
    // The derive macro built the Command from `App` already; we
    // post-process it to inject `term_width` because the derive
    // parser doesn't recognise that attribute.
    let cmd = apply_help_layout(App::command());
    let matches = cmd.get_matches();
    let app = App::from_arg_matches(&matches)?;
    run(app.command)
}

/// Recursively set `term_width(HELP_TERM_WIDTH)` on a command and
/// every one of its subcommands. clap's tree-of-Commands is built
/// by the derive expansion; this is the only way to flip a setting
/// uniformly across all leaves without sprinkling builder calls
/// throughout the source.
///
/// `mut_subcommands` only touches direct children, not their
/// children, so we walk the tree explicitly.
fn apply_help_layout(cmd: clap::Command) -> clap::Command {
    cmd.term_width(HELP_TERM_WIDTH)
        .mut_subcommands(apply_help_layout)
}

/// Detects whether argv is exactly the top-level `--help` form.
/// Args considered top-level help:
///   `easynet`               (no args at all — clap will print help)
///   `easynet --help` / `-h`
///   `easynet help`          (the auto-generated `help` subcommand,
///                            but only with no further argument —
///                            `easynet help device` should NOT show
///                            the banner)
fn is_top_level_help_invocation() -> bool {
    let argv: Vec<String> = std::env::args().collect();
    match argv.len() {
        // Bare `easynet` — clap will emit help due to no subcommand.
        1 => true,
        2 => {
            let a = argv[1].as_str();
            a == "--help" || a == "-h" || a == "help"
        }
        _ => false,
    }
}
