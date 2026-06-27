// EasyNet CLI
// ===========
//
// File: src/facade/cli/stop.rs
// Description: `easynet runtime stop` — orderly shutdown of every process and
//              state file `easynet runtime start` left behind.
//
// Shape — what changed and why
// -----------------------------
// Before this rewrite, `stop::run` was a tangle of three conditional
// branches (no-state fallback / DaemonOnly / full runtime), each
// open-coding its own subset of the same six operations. Operators
// saw a different log shape per branch and the maintainer paid a
// branch-tax every time a step was added.
//
// The rewrite keeps every single side effect of the old code and
// every guard (pid-alive checks, easynet-process verification,
// pgrep sweep, federation revoke gated on axon-pb feature). It only
// reorganises them into a `StopPlan` object whose `execute()` method
// walks a fixed sequence of stages, each rendered through the
// shared `StageRenderer` from `presentation::stage`. A stage that
// has nothing to do reports itself as `skipped("(reason)")` instead
// of disappearing; the operator reads the same column of stages on
// every shutdown.
//
// Stage order (matches the previous behaviour; not invented):
//   1. revoke          — federation.revoke against the daemon
//                        (best-effort; gated on axon-pb feature
//                        and on the runtime being DaemonOnly)
//   2. stop-heartbeat  — pidfile -> SIGTERM -> wait 3s on the
//                        heartbeat helper process
//   3. stop-daemon     — pidfile -> SIGTERM -> wait 3s on the
//                        easynet-daemon child
//   4. sweep-daemons   — pgrep `easynet-daemon` to catch ghosts
//                        whose pidfile was lost
//   5. stop-axon       — non-DaemonOnly only: SIGTERM the axon
//                        runtime PID (or lsof-discovered one)
//   6. cleanup-state   — remove runtime.json
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::persistence::config;
use crate::support::net;

use super::presentation::stage::StageRenderer;

#[derive(Debug, Args)]
pub struct StopArgs {}

pub fn run(args: StopArgs) -> anyhow::Result<()> {
    run_with_options(args, StopOptions::default())
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StopOptions {
    /// Internal caller has already attempted `federation.revoke`.
    ///
    /// Public `runtime stop` keeps revoke enabled; `self uninstall`
    /// disables it because uninstall owns an explicit hub-removal stage
    /// before it tears down local files.
    pub(crate) skip_revoke: bool,
}

pub(crate) fn run_with_options(_args: StopArgs, options: StopOptions) -> anyhow::Result<()> {
    let state = config::load().ok();
    let plan = StopPlan::from_state(state.as_ref(), options);
    plan.execute()
}

/// What we are about to shut down. Computed once from runtime.json
/// (or its absence) so every stage decision below reads the same
/// snapshot.
enum StopShape {
    /// No `runtime.json` on disk. There may still be live processes
    /// (an operator-spawned daemon, or a crashed start that left a
    /// daemon orphaned); the sweep stage handles that case. The
    /// axon-runtime and revoke stages are skipped because we have
    /// no endpoint to talk to.
    Stateless,
    /// `runtime.json` exists and reports a daemon-only runtime —
    /// no axon-runtime PID, just the IPC daemon. This is the
    /// shape every modern device boots into; the revoke stage
    /// runs against the daemon before we tear it down.
    DaemonOnly,
    /// `runtime.json` exists and reports the legacy embedded axon
    /// runtime. The axon PID gets its own stop stage; revoke is
    /// skipped because the historical path deferred deregister to
    /// the heartbeat process's exit handler.
    WithAxonRuntime { endpoint: String, pid: Option<u32> },
}

/// Bundle the shape decision and stage execution. Methods on
/// `StopPlan` are the single sanctioned way to render any stop
/// stage; nothing outside this file should call StageRenderer or
/// the low-level `stop_*` helpers directly.
struct StopPlan {
    shape: StopShape,
    options: StopOptions,
    renderer: StageRenderer,
}

impl StopPlan {
    fn from_state(state: Option<&config::RuntimeState>, options: StopOptions) -> Self {
        let shape = match state {
            None => StopShape::Stateless,
            Some(s) if matches!(s.runtime_kind, config::RuntimeKind::DaemonOnly) => {
                StopShape::DaemonOnly
            }
            Some(s) => {
                let pid = s
                    .pid
                    .or_else(|| net::discover_pid_from_endpoint(&s.endpoint));
                StopShape::WithAxonRuntime {
                    endpoint: s.endpoint.clone(),
                    pid,
                }
            }
        };
        Self {
            shape,
            options,
            renderer: StageRenderer::new(),
        }
    }

    fn execute(mut self) -> anyhow::Result<()> {
        self.stage_revoke();
        self.stage_stop_heartbeat();
        self.stage_stop_daemon();
        self.stage_sweep_daemons();
        self.stage_stop_axon_runtime();
        let cleanup = self.stage_cleanup_state();
        self.renderer.finish();
        cleanup
    }

    // ── Stages ────────────────────────────────────────────────────

    /// Best-effort `federation.revoke` against the daemon. Only
    /// meaningful when the daemon is still alive (DaemonOnly) AND
    /// the binary was compiled with `axon-pb`. Skipped in every
    /// other case — the message names which one so operators can
    /// tell "I have nothing to revoke" from "this build can't".
    fn stage_revoke(&mut self) {
        self.renderer.set_active("revoke");
        if self.options.skip_revoke {
            self.renderer
                .stage_skipped("revoke", "(already attempted by caller)");
            return;
        }
        if !matches!(self.shape, StopShape::DaemonOnly) {
            self.renderer
                .stage_skipped("revoke", "(only runs in daemon-only mode)");
            return;
        }
        #[cfg(feature = "axon-pb")]
        {
            let creds = match config::load_credentials() {
                Ok(c) => c,
                Err(_) => {
                    self.renderer.stage_skipped("revoke", "(no credentials)");
                    return;
                }
            };
            let caller_ura = crate::ura::device_ura(&creds.realm, &creds.node_id);
            match crate::services::invocation_transport::federation_invoke::invoke_federation_revoke(
                &caller_ura,
                "device shutdown",
                Some(&caller_ura),
            ) {
                Ok(_) => self.renderer.stage_ok("revoke"),
                Err(e) => self.renderer.stage_skipped("revoke", &format!("({e})")),
            }
        }
        #[cfg(not(feature = "axon-pb"))]
        {
            self.renderer
                .stage_skipped("revoke", "(axon-pb feature disabled)");
        }
    }

    /// Pidfile-driven SIGTERM on the heartbeat helper. Skipped
    /// when no pidfile exists OR the PID is stale.
    fn stage_stop_heartbeat(&mut self) {
        self.renderer.set_active("stop-heartbeat");
        match stop_pidfile_process(&config::heartbeat_pid_path()) {
            PidfileStopOutcome::Stopped { pid } => self
                .renderer
                .stage_ok(&format!("stop-heartbeat (pid {pid})")),
            PidfileStopOutcome::NoPidfile => self
                .renderer
                .stage_skipped("stop-heartbeat", "(no pidfile)"),
            PidfileStopOutcome::StalePidfile { pid } => self
                .renderer
                .stage_skipped("stop-heartbeat", &format!("(pid {pid} already exited)")),
            PidfileStopOutcome::PidReuseRefused { pid } => self.renderer.stage_skipped(
                "stop-heartbeat",
                &format!("(pid {pid} no longer an easynet process)"),
            ),
            PidfileStopOutcome::TimedOut { pid } => self
                .renderer
                .stage_failed("stop-heartbeat", &format!("pid {pid} did not exit in time")),
        }
    }

    /// Pidfile-driven SIGTERM on the easynet-daemon child.
    fn stage_stop_daemon(&mut self) {
        self.renderer.set_active("stop-daemon");
        match stop_pidfile_process(&config::easynet_daemon_pid_path()) {
            PidfileStopOutcome::Stopped { pid } => {
                self.renderer.stage_ok(&format!("stop-daemon (pid {pid})"))
            }
            PidfileStopOutcome::NoPidfile => {
                self.renderer.stage_skipped("stop-daemon", "(no pidfile)")
            }
            PidfileStopOutcome::StalePidfile { pid } => self
                .renderer
                .stage_skipped("stop-daemon", &format!("(pid {pid} already exited)")),
            PidfileStopOutcome::PidReuseRefused { pid } => self.renderer.stage_skipped(
                "stop-daemon",
                &format!("(pid {pid} no longer an easynet process)"),
            ),
            PidfileStopOutcome::TimedOut { pid } => self
                .renderer
                .stage_failed("stop-daemon", &format!("pid {pid} did not exit in time")),
        }
    }

    /// `pgrep -f easynet-daemon` belt-and-suspenders pass. Catches
    /// the "pidfile lost" case where an earlier stop crashed
    /// mid-write, or where an operator spawned `easynet-daemon`
    /// manually without going through `easynet runtime start`.
    fn stage_sweep_daemons(&mut self) {
        self.renderer.set_active("sweep-daemons");
        let swept = sweep_stray_easynet_daemons();
        if swept.is_empty() {
            self.renderer.stage_skipped("sweep-daemons", "(none found)");
        } else {
            let pids = swept
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            self.renderer
                .stage_ok(&format!("sweep-daemons (pid {pids})"));
        }
    }

    /// SIGTERM the legacy axon runtime PID. Skipped for DaemonOnly
    /// and Stateless shapes — there is no axon process to stop.
    fn stage_stop_axon_runtime(&mut self) {
        self.renderer.set_active("stop-axon");
        let (endpoint, pid) = match &self.shape {
            StopShape::WithAxonRuntime { endpoint, pid } => (endpoint.clone(), *pid),
            _ => {
                self.renderer
                    .stage_skipped("stop-axon", "(daemon-only runtime)");
                return;
            }
        };
        let Some(pid) = pid else {
            self.renderer.stage_failed(
                "stop-axon",
                &format!("could not determine pid for endpoint {endpoint}"),
            );
            return;
        };
        if net::kill_and_wait(pid, std::time::Duration::from_secs(5)) {
            self.renderer.stage_ok(&format!("stop-axon (pid {pid})"));
        } else {
            self.renderer
                .stage_failed("stop-axon", &format!("pid {pid} did not exit in time"));
        }
    }

    /// Remove `runtime.json`. Skipped when there was no state file
    /// to begin with — preserves the pre-rewrite behaviour where
    /// the "no state found" branch never called `config::remove`.
    fn stage_cleanup_state(&mut self) -> anyhow::Result<()> {
        self.renderer.set_active("cleanup-state");
        if matches!(self.shape, StopShape::Stateless) {
            self.renderer
                .stage_skipped("cleanup-state", "(no runtime.json)");
            return Ok(());
        }
        match config::remove() {
            Ok(()) => {
                self.renderer.stage_ok("cleanup-state");
                Ok(())
            }
            Err(e) => {
                self.renderer.stage_failed("cleanup-state", &format!("{e}"));
                Err(e)
            }
        }
    }
}

// ── Low-level helpers ────────────────────────────────────────────

/// Result of attempting to stop a process named by a pidfile.
/// Returned by [`stop_pidfile_process`]; the staged caller maps
/// each variant onto a `stage_ok` / `stage_skipped` / `stage_failed`
/// rendering. Splitting the outcome from the rendering keeps the
/// signaling logic free of UI concerns.
enum PidfileStopOutcome {
    NoPidfile,
    StalePidfile { pid: u32 },
    PidReuseRefused { pid: u32 },
    Stopped { pid: u32 },
    TimedOut { pid: u32 },
}

/// Pidfile -> liveness check -> easynet-process check -> SIGTERM
/// with a 3-second wait. Removes the pidfile after the attempt
/// regardless of outcome so a stale file from a crashed daemon
/// does not block the next `easynet runtime start`.
///
/// The pidfile race window between `read` and `kill` is narrow but
/// not zero; the `is_easynet_process` check after liveness mitigates
/// pid reuse on busy hosts. A production daemon would use pidfd or
/// a lockfile to close the window entirely — out of scope here.
fn stop_pidfile_process(pid_path: &std::path::Path) -> PidfileStopOutcome {
    let pid: u32 = match std::fs::read_to_string(pid_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
    {
        Some(p) => p,
        None => return PidfileStopOutcome::NoPidfile,
    };
    if !net::is_pid_alive(pid) {
        let _ = std::fs::remove_file(pid_path);
        return PidfileStopOutcome::StalePidfile { pid };
    }
    if !net::is_easynet_process(pid) {
        let _ = std::fs::remove_file(pid_path);
        return PidfileStopOutcome::PidReuseRefused { pid };
    }
    let stopped = net::kill_and_wait(pid, std::time::Duration::from_secs(3));
    let _ = std::fs::remove_file(pid_path);
    if stopped {
        PidfileStopOutcome::Stopped { pid }
    } else {
        PidfileStopOutcome::TimedOut { pid }
    }
}

/// Pgrep-style sweep that SIGTERMs every alive `easynet-daemon`
/// process other than this CLI itself. Returns the PIDs that were
/// successfully signalled, in pgrep iteration order. Best-effort:
/// silently skips PIDs that fail the easynet-process guard or that
/// did not exit within 3 seconds.
fn sweep_stray_easynet_daemons() -> Vec<u32> {
    let output_res = std::process::Command::new("pgrep")
        .args(["-f", "easynet-daemon"])
        .output();
    let candidates: Vec<u32> = match output_res {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .filter(|pid| *pid != std::process::id())
            .collect(),
        _ => return Vec::new(),
    };
    let mut swept = Vec::new();
    for pid in candidates {
        if !net::is_pid_alive(pid) || !net::is_easynet_process(pid) {
            continue;
        }
        if net::kill_and_wait(pid, std::time::Duration::from_secs(3)) {
            swept.push(pid);
        }
    }
    swept
}
