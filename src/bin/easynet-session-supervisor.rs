// EasyNet CLI - PTY session supervisor executable
// =================================================
//
// File: src/bin/easynet-session-supervisor.rs
// Description: Entrypoint for the non-Runtime PTY handle custodian.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

fn main() -> anyhow::Result<()> {
    easynet_cli::daemon::execution::pty::supervisor::run()
}
