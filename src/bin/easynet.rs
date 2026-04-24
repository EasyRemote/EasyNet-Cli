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
// The parsed clap App → `facade::cli::run(...)` and nothing else.
// If a subcommand needs the daemon (PR-ATTACH / PR-PERM / etc.),
// its handler inside the library takes care of spawning or
// connecting to `easynet-daemon` via the IPC layer.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Parser;
use easynet_cli::facade::cli::{run, App};

fn main() -> anyhow::Result<()> {
    let app = App::parse();
    run(app.command)
}
