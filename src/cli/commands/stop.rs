// EasyNet CLI
// ===========
//
// File: src/cli/stop.rs
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
// The current version keeps the same stage object, but feeds it from
// `daemon::lifecycle::RuntimeStatusReport` instead of treating
// `runtime.json` as authority. That lets stop clean up migration
// states where the projection is missing but `control.json`, a PID,
// or an accepting socket proves the daemon is still alive.
//
// Stage order:
//   1. revoke          — federation.revoke against the daemon
//                        (best-effort; gated on axon-pb feature
//                        and on the runtime being DaemonOnly)
//   2. stop-daemon     — pidfile -> SIGTERM -> wait 3s on the
//                        easynet-daemon child
//   3. stop-discovered-daemon
//                     — SIGTERM daemon PID advertised in control.json
//   4. sweep-daemons   — pgrep `easynet-daemon` to catch ghosts
//                        whose pidfile was lost
//   5. cleanup-discovery
//                     — remove stale control.json after daemon exit
//   6. legacy-heartbeat-cleanup
//                     — stale retired heartbeat pidfile cleanup only
//   7. legacy-axon-cleanup
//                     — non-DaemonOnly only: SIGTERM the axon
//                        runtime PID (or lsof-discovered one)
//   8. cleanup-state   — remove runtime.json when it existed
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::daemon::control::discovery;
use crate::daemon::lifecycle::{RuntimeLifecycleService, RuntimeStopPlan, RuntimeStopShape};
use crate::daemon::persistence::config;
use crate::support::platform::net;

use crate::cli::presentation::stage::StageRenderer;

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
    let plan = StopPlan::from_runtime_plan(RuntimeLifecycleService::new().stop_plan(), options);
    plan.execute()
}

/// Bundle the shape decision and stage execution. Methods on
/// `StopPlan` are the single sanctioned way to render any stop
/// stage; nothing outside this file should call StageRenderer or
/// the low-level `stop_*` helpers directly.
struct StopPlan {
    shape: RuntimeStopShape,
    discovery_pid: Option<u32>,
    cleanup_runtime_projection: bool,
    stop_timed_out: bool,
    options: StopOptions,
    renderer: StageRenderer,
}

impl StopPlan {
    fn from_runtime_plan(plan: RuntimeStopPlan, options: StopOptions) -> Self {
        Self {
            shape: plan.shape().clone(),
            discovery_pid: plan.discovery_pid(),
            cleanup_runtime_projection: plan.should_cleanup_runtime_projection(),
            stop_timed_out: false,
            options,
            renderer: StageRenderer::new(),
        }
    }

    fn execute(mut self) -> anyhow::Result<()> {
        self.stage_revoke();
        self.stage_stop_daemon();
        self.stage_stop_discovered_daemon();
        self.stage_sweep_daemons();
        let discovery_cleanup = self.stage_cleanup_discovery();
        self.stage_legacy_heartbeat_cleanup();
        self.stage_legacy_axon_cleanup();
        let cleanup = self.stage_cleanup_state();
        self.renderer.finish();
        discovery_cleanup?;
        cleanup?;
        let post = RuntimeLifecycleService::new().status();
        if self.stop_timed_out && post.daemon().has_daemon_fact() {
            anyhow::bail!(
                "runtime stop timed out; daemon facts remain visible (status={})",
                post.status().as_wire_str()
            );
        }
        Ok(())
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
        if !matches!(self.shape, RuntimeStopShape::DaemonOnly) {
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
            let caller_ura = crate::core::ura::device_ura(&creds.realm, &creds.node_id);
            match crate::daemon::invocation::routing::federation_invoke::invoke_federation_revoke(
                &caller_ura,
                "device shutdown",
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

    /// Legacy janitor for the retired heartbeat helper pidfile.
    fn stage_legacy_heartbeat_cleanup(&mut self) {
        self.renderer.set_active("legacy-heartbeat-cleanup");
        match stop_pidfile_process(&config::heartbeat_pid_path()) {
            PidfileStopOutcome::Stopped { pid } => self
                .renderer
                .stage_ok(&format!("legacy-heartbeat-cleanup (pid {pid})")),
            PidfileStopOutcome::NoPidfile => self
                .renderer
                .stage_skipped("legacy-heartbeat-cleanup", "(no retired pidfile)"),
            PidfileStopOutcome::StalePidfile { pid } => self.renderer.stage_skipped(
                "legacy-heartbeat-cleanup",
                &format!("(pid {pid} already exited)"),
            ),
            PidfileStopOutcome::PidReuseRefused { pid } => self.renderer.stage_skipped(
                "legacy-heartbeat-cleanup",
                &format!("(pid {pid} no longer an easynet process)"),
            ),
            PidfileStopOutcome::TimedOut { pid } => {
                self.stop_timed_out = true;
                self.renderer.stage_failed(
                    "legacy-heartbeat-cleanup",
                    &format!("pid {pid} did not exit in time"),
                );
            }
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
            PidfileStopOutcome::TimedOut { pid } => {
                self.stop_timed_out = true;
                self.renderer
                    .stage_failed("stop-daemon", &format!("pid {pid} did not exit in time"));
            }
        }
    }

    /// Discovery-driven daemon shutdown. This is the repair path for
    /// migration states where `control.json` survived but the daemon
    /// pidfile did not.
    fn stage_stop_discovered_daemon(&mut self) {
        self.renderer.set_active("stop-discovered-daemon");
        let Some(pid) = self.discovery_pid else {
            self.renderer
                .stage_skipped("stop-discovered-daemon", "(no discovery pid)");
            return;
        };
        match stop_discovered_daemon_process(pid) {
            LiveProcessStopOutcome::Stopped { pid } => self
                .renderer
                .stage_ok(&format!("stop-discovered-daemon (pid {pid})")),
            LiveProcessStopOutcome::StalePid { pid } => self.renderer.stage_skipped(
                "stop-discovered-daemon",
                &format!("(pid {pid} already exited)"),
            ),
            LiveProcessStopOutcome::PidReuseRefused { pid } => self.renderer.stage_skipped(
                "stop-discovered-daemon",
                &format!("(pid {pid} no longer an easynet process)"),
            ),
            LiveProcessStopOutcome::TimedOut { pid } => {
                self.stop_timed_out = true;
                self.renderer.stage_failed(
                    "stop-discovered-daemon",
                    &format!("pid {pid} did not exit in time"),
                );
            }
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

    /// Remove stale daemon discovery after shutdown. If a daemon still
    /// appears live, keep `control.json` so operators do not lose the
    /// remaining process evidence.
    fn stage_cleanup_discovery(&mut self) -> anyhow::Result<()> {
        self.renderer.set_active("cleanup-discovery");
        let path = discovery::default_path();
        if !path.exists() {
            self.renderer
                .stage_skipped("cleanup-discovery", "(no control.json)");
            return Ok(());
        }
        let report = RuntimeLifecycleService::new().status();
        if report.daemon().has_daemon_fact() {
            self.renderer
                .stage_skipped("cleanup-discovery", "(daemon still appears live)");
            return Ok(());
        }
        match discovery::remove(&path) {
            Ok(()) => {
                self.renderer.stage_ok("cleanup-discovery");
                Ok(())
            }
            Err(e) => {
                self.renderer
                    .stage_failed("cleanup-discovery", &format!("{e}"));
                Err(e)
            }
        }
    }

    /// SIGTERM the legacy axon runtime PID. Skipped for DaemonOnly
    /// and Stateless shapes — there is no axon process to stop.
    fn stage_legacy_axon_cleanup(&mut self) {
        self.renderer.set_active("legacy-axon-cleanup");
        let (endpoint, pid) = match &self.shape {
            RuntimeStopShape::LegacyAxonRuntime { endpoint, pid } => (endpoint.clone(), *pid),
            _ => {
                self.renderer
                    .stage_skipped("legacy-axon-cleanup", "(daemon-only runtime)");
                return;
            }
        };
        let Some(pid) = pid else {
            self.renderer.stage_failed(
                "legacy-axon-cleanup",
                &format!("could not determine pid for endpoint {endpoint}"),
            );
            return;
        };
        if net::kill_and_wait(pid, std::time::Duration::from_secs(5)) {
            self.renderer
                .stage_ok(&format!("legacy-axon-cleanup (pid {pid})"));
        } else {
            self.stop_timed_out = true;
            self.renderer.stage_failed(
                "legacy-axon-cleanup",
                &format!("pid {pid} did not exit in time"),
            );
        }
    }

    /// Remove `runtime.json` when a projection existed at plan time.
    fn stage_cleanup_state(&mut self) -> anyhow::Result<()> {
        self.renderer.set_active("cleanup-state");
        if !self.cleanup_runtime_projection {
            self.renderer
                .stage_skipped("cleanup-state", "(no runtime.json)");
            return Ok(());
        }
        if self.stop_timed_out {
            self.renderer
                .stage_skipped("cleanup-state", "(stop timed out)");
            return Ok(());
        }
        let report = RuntimeLifecycleService::new().status();
        if report.daemon().has_daemon_fact() {
            self.renderer
                .stage_skipped("cleanup-state", "(daemon still appears live)");
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

/// Result of stopping a live process discovered outside a pidfile.
enum LiveProcessStopOutcome {
    StalePid { pid: u32 },
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

fn stop_discovered_daemon_process(pid: u32) -> LiveProcessStopOutcome {
    if !net::is_pid_alive(pid) {
        return LiveProcessStopOutcome::StalePid { pid };
    }
    if !net::is_easynet_process(pid) {
        return LiveProcessStopOutcome::PidReuseRefused { pid };
    }
    if net::kill_and_wait(pid, std::time::Duration::from_secs(3)) {
        LiveProcessStopOutcome::Stopped { pid }
    } else {
        LiveProcessStopOutcome::TimedOut { pid }
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
