// EasyNet Daemon — thin bin wrapper
// ==================================
//
// File: src/bin/easynet-daemon.rs
// Description: Long-running daemon entry point. v10.5 R1 PR-DAEMON
//              introduces this bin as the permanent home of the
//              heartbeat loop + Axon node identity + (future) local
//              IPC server + system.* ability publisher.
//
// v1 behaviour — "scheme X"
// -------------------------
// The plan pins a single-daemon architecture: one process handles
// pair, heartbeat, agent runtime hosting, ability publishing, and
// local IPC. v1 starts that process by calling the library's
// existing `facade::cli::heartbeat::run_daemon`. Later PRs extend
// that function (or the library adds a sibling `run_daemon_v2`)
// with IPC server, schedule tick, and system ability dispatch.
//
// No business logic lives in this file. It is the process entry
// point, nothing more.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use easynet_cli::facade::cli::run_daemon;

fn main() -> anyhow::Result<()> {
    run_daemon()
}
