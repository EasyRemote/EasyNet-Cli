// EasyNet CLI
// ===========
//
// File: src/cli/stop.rs
// Description: `easynet runtime stop` — orderly shutdown of every process and
//              state file `easynet runtime start` left behind.
//
// Shape — what changed and why
// -----------------------------
// `stop::run` renders one staged lifecycle. `daemon::lifecycle` owns
// runtime shape selection and OS-facing process-stop transitions; this file
// maps typed outcomes to operator-visible progress.
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
//   6. cleanup-state   — remove runtime.json when it existed
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::daemon::control::discovery;
use crate::daemon::lifecycle::{
    LiveProcessStopOutcome, PidfileStopOutcome, RuntimeLifecycleService, RuntimeStopPlan,
    RuntimeStopProcessController, RuntimeStopShape,
};
use crate::daemon::persistence::config;

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
    let plan = StopPlan::from_runtime_plan(RuntimeLifecycleService::new().stop_plan()?, options);
    plan.execute()
}

/// Bundle the shape decision and stage execution. Methods on
/// `StopPlan` are the single sanctioned way to render any stop
/// stage; nothing outside this file should call StageRenderer or
/// process-stop lifecycle logic directly.
struct StopPlan {
    shape: RuntimeStopShape,
    discovery_pid: Option<u32>,
    cleanup_runtime_projection: bool,
    stop_timed_out: bool,
    options: StopOptions,
    process_controller: RuntimeStopProcessController,
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
            process_controller: RuntimeStopProcessController::new(),
            renderer: StageRenderer::new(),
        }
    }

    fn execute(mut self) -> anyhow::Result<()> {
        self.stage_revoke();
        self.stage_stop_daemon();
        self.stage_stop_discovered_daemon();
        self.stage_sweep_daemons();
        let discovery_cleanup = self.stage_cleanup_discovery();
        let cleanup = self.stage_cleanup_state();
        self.stage_desktop_companions();
        self.renderer.finish();
        discovery_cleanup?;
        cleanup?;
        let post = RuntimeLifecycleService::new().status()?;
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
            match crate::daemon::invocation::routing::remote_invoke::invoke_federation_revoke(
                &caller_ura,
                "device shutdown",
                &caller_ura,
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

    fn stage_desktop_companions(&mut self) {
        self.renderer.set_active("desktop-companions");
        let Ok(state) = crate::daemon::plugins::default_state() else {
            self.renderer
                .stage_skipped("desktop-companions", "(plugin state unavailable)");
            return;
        };
        let manager = match crate::daemon::plugins::DesktopCompanionManager::current() {
            Ok(manager) => manager,
            Err(error) => {
                self.renderer
                    .stage_skipped("desktop-companions", &format!("({error})"));
                return;
            }
        };
        let warnings = manager.stop_for_runtime_stop(state.index().packages());
        if warnings.is_empty() {
            self.renderer
                .stage_skipped("desktop-companions", "(no stop_on_runtime_stop companions)");
        } else {
            self.renderer
                .stage_skipped("desktop-companions", &format!("({})", warnings.join("; ")));
        }
    }

    /// Pidfile-driven SIGTERM on the easynet-daemon child.
    fn stage_stop_daemon(&mut self) {
        self.renderer.set_active("stop-daemon");
        match self
            .process_controller
            .stop_pidfile_process(&config::easynet_daemon_pid_path())
        {
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
        match self.process_controller.stop_discovered_daemon_process(pid) {
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
        let swept = self.process_controller.sweep_stray_easynet_daemons();
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
        let path = discovery::try_default_path()
            .map_err(|error| anyhow::anyhow!("resolve control discovery cleanup path: {error}"))?;
        if !path.exists() {
            self.renderer
                .stage_skipped("cleanup-discovery", "(no control.json)");
            return Ok(());
        }
        let report = RuntimeLifecycleService::new().status()?;
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
        let report = RuntimeLifecycleService::new().status()?;
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
