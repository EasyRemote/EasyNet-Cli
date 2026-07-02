// EasyNet CLI — Mission Run History
// =================================
//
// File: src/cli/mission_runs.rs
// Description: On-disk persistence for EAL mission executions, mirroring the
//              shape of the agent run store but rooted at
//              `~/.easynet/missions/runs/`. Each run has its own timestamped
//              directory containing the source program, the compiled IR, the
//              full execution trace, and a meta.json summary.
//
// Layout:
//   ~/.easynet/missions/runs/<YYYY-MM-DD_HHMMSS>/
//     ├── source.eal     — the .eal program text
//     ├── ir.json        — Mission IR v2 (compiler output)
//     ├── trace.json     — full execution trace
//     ├── meta.json      — name, status, duration, step counts
//     │                    (written with status=running at create, so
//     │                    in-flight runs are visible in listings)
//     └── heartbeat      — touched every HEARTBEAT_INTERVAL by the run's
//                          pump thread; liveness = freshness, so a run
//                          whose process died stops looking alive within
//                          HEARTBEAT_STALE_AFTER (F-022: the pid file's
//                          "presence == in-flight" lied forever after a
//                          crash)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::persistence::config;
use crate::runtime::context::ParentInvocationContext;

pub fn root_dir() -> PathBuf {
    config::state_dir().join("missions").join("runs")
}

/// The mission-run store: every run.json-family operation is anchored to
/// ONE missions root, resolved exactly once at construction. The object
/// exists so embedders and tests anchor to an explicit directory instead
/// of steering the free functions through the process-global HOME — the
/// F-056 race was sixteen tests mutating HOME to retarget [`root_dir`].
pub struct MissionRunStore {
    root: PathBuf,
}

impl MissionRunStore {
    /// Production entry: the canonical missions root under the state dir.
    /// The free-function facade below resolves through this, so the env
    /// is consulted only here.
    pub fn open_default() -> Self {
        Self { root: root_dir() }
    }

    /// Anchor to an explicit root (tests pass a TempDir path; no env).
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Cadence of the run's heartbeat pump.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// A heartbeat older than this (3× the cadence) means the owning
/// process is gone — the run reads as not-running even if its meta
/// still says `running` (an interrupted run).
const HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(15);
/// Pump-thread stop poll. Bounds both shutdown latency and the
/// drift between a stop signal and the last touch.
const HEARTBEAT_STOP_POLL: Duration = Duration::from_millis(500);

/// Background toucher for the run's `heartbeat` file. Owned by
/// [`MissionRunDir`]: the thread lives exactly as long as the run
/// object, so process death (the F-022 failure mode the pid file
/// could not express) stops the heartbeat within one interval.
struct HeartbeatPump {
    stop: Arc<AtomicBool>,
}

impl HeartbeatPump {
    fn start(file: PathBuf) -> Self {
        // First touch is SYNCHRONOUS: `create` returning means the run
        // already reads alive. Deferring it to the spawned thread made
        // "freshly created run" momentarily indistinguishable from an
        // interrupted one (and made the F-056 test family racy once the
        // HOME lock no longer serialized it incidentally). Content is
        // forensic (humans reading the dir); freshness checks use mtime.
        let _ = fs::write(&file, Local::now().to_rfc3339());
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let _ = std::thread::Builder::new()
            .name("mission-heartbeat".into())
            .spawn(move || {
                let mut since_touch = Duration::ZERO; // first touch already done
                while !flag.load(Ordering::Relaxed) {
                    if since_touch >= HEARTBEAT_INTERVAL {
                        let _ = fs::write(&file, Local::now().to_rfc3339());
                        since_touch = Duration::ZERO;
                    }
                    std::thread::sleep(HEARTBEAT_STOP_POLL);
                    since_touch += HEARTBEAT_STOP_POLL;
                }
            });
        Self { stop }
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for HeartbeatPump {
    fn drop(&mut self) {
        self.stop();
    }
}

/// True when the run directory's heartbeat is fresh enough to call
/// the run alive.
fn heartbeat_fresh(run_path: &Path) -> bool {
    fs::metadata(run_path.join("heartbeat"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|age| age < HEARTBEAT_STALE_AFTER)
        .unwrap_or(false)
}

pub struct MissionRunDir {
    pub path: PathBuf,
    pump: Option<HeartbeatPump>,
}

impl MissionRunStore {
    pub fn create(&self, name: &str) -> anyhow::Result<MissionRunDir> {
        fs::create_dir_all(&self.root)?;
        let stamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
        let safe_name = sanitize_for_path(name);
        let path = allocate_unique_run_dir(&self.root, &stamp, &safe_name)?;
        let run = MissionRunDir {
            pump: Some(HeartbeatPump::start(path.join("heartbeat"))),
            path,
        };
        // Initial meta with status=running: in-flight runs are visible
        // in listings instead of appearing only after completion. The
        // completion path overwrites this with the terminal record.
        // Best-effort, like every write on this surface.
        if let Err(e) = run.write_meta(&MissionRunMeta {
            name: name.to_string(),
            // The watch anchor must exist while the run is ALIVE —
            // attaching `invocation watch --trace` mid-run is the
            // entire point — so the in-flight meta already carries it.
            trace_id: run
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            started_at: Local::now().to_rfc3339(),
            status: MissionRunStatus::Running,
            ..Default::default()
        }) {
            eprintln!(
                "[easynet warn] mission run {}: write initial meta failed ({e})",
                run.path.display()
            );
        }
        Ok(run)
    }
}

impl MissionRunDir {
    /// Facade for the production root; see [`MissionRunStore::create`].
    pub fn create(name: &str) -> anyhow::Result<Self> {
        MissionRunStore::open_default().create(name)
    }

    pub fn write_source(&self, source: &str) -> std::io::Result<()> {
        fs::write(self.path.join("source.eal"), source)
    }
    pub fn write_ir(&self, ir_json: &str) -> std::io::Result<()> {
        fs::write(self.path.join("ir.json"), ir_json)
    }
    pub fn write_trace(&self, trace_json: &str) -> std::io::Result<()> {
        fs::write(self.path.join("trace.json"), trace_json)
    }
    pub fn write_meta(&self, meta: &MissionRunMeta) -> std::io::Result<()> {
        let s = serde_json::to_string_pretty(meta)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(self.path.join("meta.json"), s + "\n")
    }
    pub fn finish(&self) {
        if let Some(pump) = &self.pump {
            pump.stop();
        }
        let _ = fs::remove_file(self.path.join("heartbeat"));
    }
}

/// Allocate a unique mission run directory for `stamp_name` under `root`,
/// retrying with `-1`, `-2`, ... on collision.
///
/// Concurrency note: the same TOCTOU bug the iter-2 `runtime::run_store`
/// fix addressed applies here verbatim — `while exists() { ... }
/// create_dir_all(...)` lets two racers both pass the existence check
/// and then both succeed (since `create_dir_all` treats "already
/// exists" as OK), corrupting each other's source.eal / trace.json /
/// meta.json. We use `create_dir`'s atomic `O_EXCL`-equivalent and
/// retry on `AlreadyExists` instead.
fn allocate_unique_run_dir(
    root: &std::path::Path,
    stamp: &str,
    safe_name: &str,
) -> anyhow::Result<PathBuf> {
    const MAX_SUFFIX_ATTEMPTS: u32 = 10_000;
    for suffix in 0..MAX_SUFFIX_ATTEMPTS {
        let path = if suffix == 0 {
            root.join(format!("{stamp}_{safe_name}"))
        } else {
            root.join(format!("{stamp}_{safe_name}-{suffix}"))
        };
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!(
        "could not allocate a unique mission run directory under {} after {MAX_SUFFIX_ATTEMPTS} attempts",
        root.display()
    )
}

/// Mission run lifecycle — the stored state machine for `meta.json`
/// (F-022 / T5.3: stringly status plus pid-file liveness was the
/// "disk file as state machine" debt). Serialized lowercase, which is
/// byte-identical to the historical string literals, so every
/// existing run directory on disk parses unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissionRunStatus {
    /// Written at create; a terminal status overwrites it at
    /// completion. `Running` in a meta whose heartbeat went stale is
    /// an interrupted run (see [`MissionRunSummary::is_interrupted`]).
    #[default]
    Running,
    Ok,
    Partial,
    Error,
    Cancelled,
}

impl MissionRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MissionRunStatus::Running => "running",
            MissionRunStatus::Ok => "ok",
            MissionRunStatus::Partial => "partial",
            MissionRunStatus::Error => "error",
            MissionRunStatus::Cancelled => "cancelled",
        }
    }

    /// Terminal states never transition again; `Running` is the only
    /// non-terminal state.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, MissionRunStatus::Running)
    }
}

impl std::fmt::Display for MissionRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissionRunMeta {
    pub name: String,
    pub source_file: Option<String>,
    /// Envelope-stamped trace id shared by every step Invocation of
    /// this run (seven-axes T2.0). Equal to the run-directory id *by
    /// construction* (see `MissionContextGuard::enter`); stored
    /// explicitly so the `invocation watch --trace` surface depends
    /// on a recorded contract, not on that coincidence. A mission has
    /// no root Invocation of its own — it is a script, and the trace
    /// is the only runtime identity a CLI-launched run has (spec
    /// §0.1-1). Empty on metas written before this field existed.
    #[serde(default)]
    pub trace_id: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub status: MissionRunStatus,
    pub error: Option<String>,
    pub steps_total: usize,
    pub steps_completed: usize,
    pub steps_failed: usize,

    /// Per-cross-agent-ability-call execution summaries. Each entry
    /// captures what the target agent's ability graph did to satisfy one
    /// call (which sub-abilities it invoked, which memory it touched,
    /// which workflow path it took). Empty for runs that only invoked
    /// device abilities (which have no graph).
    ///
    /// The schema is intentionally `Value` here: this is a landing slot
    /// for the upcoming ability-graph trace format, not the format
    /// itself. Naming the field `ability_graph_traces` (rather than e.g.
    /// `internal_eal_summaries`) is the deliberate teaching point —
    /// it tells the next reader that an ability has a graph, by the
    /// field name alone. See docs/easynet_ontology.pdf §3
    /// (self-evolution = graph) and §10 (non-CLI artefacts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_graph_traces: Option<Vec<serde_json::Value>>,

    /// Parent invocation context captured for EAL/plugin-driven mission runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_context: Option<ParentInvocationContext>,
}

/// One row in the mission history listing.
pub struct MissionRunSummary {
    pub id: String,
    pub path: PathBuf,
    pub meta: MissionRunMeta,
    /// Alive RIGHT NOW: the run directory's heartbeat is fresh.
    pub running: bool,
}

impl MissionRunSummary {
    /// The run's process died without writing a terminal status —
    /// meta still says `running` but the heartbeat went stale. The
    /// exact state F-022's pid file misrendered as forever-running.
    #[cfg(test)]
    pub fn is_interrupted(&self) -> bool {
        self.meta.status == MissionRunStatus::Running && !self.running
    }
}

/// Facade for the production root; see [`MissionRunStore::list_runs`].
pub fn list_runs() -> anyhow::Result<Vec<MissionRunSummary>> {
    MissionRunStore::open_default().list_runs()
}

/// Facade for the production root; see [`MissionRunStore::find_run`].
pub fn find_run(id: &str) -> anyhow::Result<MissionRunSummary> {
    MissionRunStore::open_default().find_run(id)
}

impl MissionRunStore {
    pub fn list_runs(&self) -> anyhow::Result<Vec<MissionRunSummary>> {
        let root = &self.root;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            let meta_path = path.join("meta.json");
            let meta: MissionRunMeta = match fs::read_to_string(&meta_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
            {
                Some(m) => m,
                None => continue,
            };
            let running = heartbeat_fresh(&path);
            out.push(MissionRunSummary {
                id,
                path,
                meta,
                running,
            });
        }
        out.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(out)
    }

    pub fn find_run(&self, id: &str) -> anyhow::Result<MissionRunSummary> {
        // Reject blank ids — otherwise `starts_with("")` would match every run
        // and silently return the first one (or bail "ambiguous"), neither of
        // which is helpful.
        let id = id.trim();
        if id.is_empty() {
            anyhow::bail!("mission run id is empty");
        }

        let runs = self.list_runs()?;
        // Exact match short-circuits the prefix search so an id that happens
        // to also be a prefix of a longer id ("a" vs "ab") still resolves
        // unambiguously.
        if let Some(r) = runs.iter().find(|r| r.id == id) {
            return Ok(MissionRunSummary {
                id: r.id.clone(),
                path: r.path.clone(),
                meta: r.meta.clone(),
                running: r.running,
            });
        }
        // Allow id prefix as a convenience.
        let matches: Vec<&MissionRunSummary> =
            runs.iter().filter(|r| r.id.starts_with(id)).collect();
        if matches.len() == 1 {
            let r = matches[0];
            return Ok(MissionRunSummary {
                id: r.id.clone(),
                path: r.path.clone(),
                meta: r.meta.clone(),
                running: r.running,
            });
        }
        if matches.len() > 1 {
            anyhow::bail!(
                "ambiguous run id '{id}' — matches: {}",
                matches
                    .iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        anyhow::bail!("no mission run found for id '{id}'")
    }

    /// Mark a run cancelled if (and only if) its stored status is still
    /// non-terminal. The gate is the STATUS, not the liveness bool: an
    /// interrupted run (meta says running, heartbeat stale) is settled to
    /// `Cancelled` instead of being unreachable forever. A live run's
    /// completion write may still race this — same advisory semantics the
    /// surface always had.
    pub fn cancel_run(&self, id: &str) -> anyhow::Result<CancelOutcome> {
        let mut run = self.find_run(id)?;
        if run.meta.status.is_terminal() {
            return Ok(CancelOutcome::AlreadyTerminal(run));
        }
        run.meta.status = MissionRunStatus::Cancelled;
        let _ = fs::remove_file(run.path.join("heartbeat"));
        if let Ok(s) = serde_json::to_string_pretty(&run.meta) {
            let _ = fs::write(run.path.join("meta.json"), s + "\n");
        }
        run.running = false;
        Ok(CancelOutcome::Cancelled(run))
    }
}

/// Outcome of a `cancel_run` call. Lets callers report accurately whether
/// they actually changed anything.
pub enum CancelOutcome {
    Cancelled(MissionRunSummary),
    AlreadyTerminal(MissionRunSummary),
}

/// Facade for the production root; see [`MissionRunStore::cancel_run`].
pub fn cancel_run(id: &str) -> anyhow::Result<CancelOutcome> {
    MissionRunStore::open_default().cancel_run(id)
}

// ── In-process mission entry point ─────────────────────────────────────────
//
// `run_mission_inproc` is THE single in-process entry point for executing an
// EAL mission source string. Every CLI verb, every agent dispatch path,
// every MCP handler that needs to run a mission MUST call this function.
// Adding a second mission execution path is a load-bearing PR violation —
// see docs/easynet_ontology.tex §6.2 derivation 3 ("there is no second
// path"). The grep check `grep -rn 'fn run_mission' src/` should report
// exactly one production hit on this name.
//
// Layering note: this function lives in `cli/mission_runs.rs` (rather than
// a new `eal/mission_runner.rs`) because it is intimately coupled with
// `MissionRunDir` and `MissionRunMeta`, which are persistence concerns
// owned by this module. Moving it elsewhere would split the persistence
// logic without a corresponding gain.
//
// The former MCP mission handler bypass has been collapsed onto this entry:
// `runtime::system_abilities::automation::mission` and `runtime::executors::eal` both
// delegate here. Keep this comment in sync with the grep invariant above; a
// second production mission execution path is a release blocker, not a TODO.

/// Options for `run_mission_inproc`. Kept narrow on purpose: anything that
/// is not strictly required by both the CLI `mission run` path and the
/// `agent send` desugar path lives elsewhere (CLI flag handling, telemetry
/// rendering, etc.).
#[derive(Debug, Clone, Default)]
pub struct MissionRunOpts {
    /// Human-readable label that becomes part of the run-dir name and the
    /// `MissionRunMeta.source_file` field. For `mission run <file.eal>`
    /// this is the file path; for `agent send <name> "..."` this is the
    /// constructed mission name (`agent-send`).
    pub source_label: Option<String>,
    /// Reserved for future per-run trace export (e.g. `--trace <path>` on
    /// `agent send`). Currently unused — `run_mission_inproc` always
    /// writes the full trace into the run dir, and callers can read it
    /// from there.
    #[allow(dead_code)]
    pub trace_path: Option<PathBuf>,
    /// Parent AXIOM invocation context when a mission is executing as an
    /// ability implementation.
    ///
    /// What this is NOT: a second invocation constructor. The daemon stores
    /// and propagates this value so child dispatch can preserve the parent
    /// caller/subject/causal tuple while Axon remains the owner of canonical
    /// invocation and receipt construction.
    pub invocation_context: Option<ParentInvocationContext>,
}

/// Result of a mission run. Returned by `run_mission_inproc`.
///
/// Several fields are read by `cli/agent.rs::run_send` (Step 4 in the
/// implementation plan) but not yet by `mission run`'s CLI handler. The
/// `#[allow(dead_code)]` annotations below cover that gap until Step 4
/// lands; remove them once `run_send` consumes the fields.
#[derive(Debug)]
pub struct MissionRunResult {
    /// On-disk run directory under `~/.easynet/missions/runs/<id>`.
    pub run_dir: PathBuf,
    /// Run id (the trailing component of `run_dir`). This is what the
    /// dispatch invariant assertion (Step 9) compares against
    /// `EASYNET_MISSION_ID`, and what `mission run --format json`
    /// reports alongside `meta.trace_id`.
    pub run_id: String,
    /// Persisted run metadata.
    pub meta: MissionRunMeta,
    /// Captured outputs of `let`-bound steps. The key is the binding
    /// name; the value is the step result parsed back from JSON (or
    /// stored as a JSON string if it does not parse).
    #[allow(dead_code)]
    pub bound_vars: HashMap<String, serde_json::Value>,
    /// True if the run completed without aborting. False if any step
    /// failed and the mission terminated early.
    #[allow(dead_code)]
    pub ok: bool,
}

/// One match from `find_implicit_agent_fallback`. Carries the step id,
/// the colliding name, and the ability so the bail! message can suggest
/// the exact member-call form the user should write instead.
#[derive(Debug)]
struct ImplicitAgentFallback {
    step_id: String,
    colliding_name: String,
    ability: String,
}

/// Walk an IR once and detect any `IrTarget::Device { node_id }` where
/// `node_id` collides with a daemon-registered agent. Returns `Ok(None)`
/// if no conflict, `Ok(Some(_))` for the first conflict found, `Err` only
/// if the daemon agent view can't be loaded.
///
/// This implements the EAL surface invariant: traditional
/// `call ... on ...` is strictly device-only. There is no implicit
/// agent fallback. See `docs/AGENT_IDENTITY.md` and the EAL surface
/// invariant comment in `src/eal/parser.rs`.
fn find_implicit_agent_fallback(
    ir: &crate::eal::ir::MissionIr,
) -> anyhow::Result<Option<ImplicitAgentFallback>> {
    use crate::core::agent_id::{AgentId, DEFAULT_TENANT};
    use crate::eal::ir::IrTarget;

    let agent_rows = crate::cli::daemon_agent_view::list_agents()?;

    // Build the set of registered agent identifiers in their canonical
    // forms, plus the bare-name fallback for default-tenant agents
    // (so legacy `agents.json` files keyed on `"claude"` still trigger
    // the conflict check).
    let mut registered: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in agent_rows {
        let raw_key = row.name;
        // Try to parse the key as an AgentId. Both shorthand and full
        // form are accepted. Add both surface forms to the set so
        // either matches a colliding device node id.
        if let Ok(id) = AgentId::parse(&raw_key) {
            registered.insert(id.name.clone());
            if id.tenant != DEFAULT_TENANT {
                registered.insert(format!("{}/{}", id.tenant, id.name));
            } else {
                // For default-tenant agents, full form is also a valid
                // surface — `default/claude` matches `claude` matches
                // `default/claude`.
                registered.insert(format!("{}/{}", DEFAULT_TENANT, id.name));
            }
        }
    }

    // PR-10: the implicit-agent-fallback check only applies to flat
    // `Call` steps. Block variants' targets are resolved inside the
    // block's lowering; they never surface as `call ... on <name>`.
    let mut leaves: Vec<&crate::eal::ir::IrCall> = Vec::new();
    for s in &ir.steps {
        s.walk_calls(&mut leaves);
    }
    for step in leaves {
        if let IrTarget::Device { node_id } = &step.target {
            if registered.contains(node_id) {
                return Ok(Some(ImplicitAgentFallback {
                    step_id: step.step_id.clone(),
                    colliding_name: node_id.clone(),
                    ability: step.ability.as_str().to_string(),
                }));
            }
        }
    }
    Ok(None)
}

/// THE single in-process entry point for executing an EAL mission source
/// string. See module-level comment above for the load-bearing invariant.
pub fn run_mission_inproc(source: &str, opts: MissionRunOpts) -> anyhow::Result<MissionRunResult> {
    // Compile.
    let program = crate::eal::parser::parse(source)?;
    let ir = crate::eal::planner::compile(&program)?;

    // Reject "implicit agent fallback" — `call "x" on "<agent-name>"`
    // in EAL traditional form, where `<agent-name>` collides with a
    // registered agent. The traditional `call ... on ...` form is
    // strictly device-only by language design (see parser.rs and
    // ir.rs invariant comments). Without this check, the user's
    // intent to call an agent silently becomes a phantom-device
    // dispatch that fails with a confusing "node not found" error.
    //
    // The check happens here, in `run_mission_inproc`, because:
    //   - the planner is registry-free by design (Step 2 invariant 2:
    //     no `is_agent` string check at lower-time);
    //   - the dispatcher is registry-aware but only for routing, not
    //     for sanity checks;
    //   - this is the single in-process mission entry point, so the
    //     check covers every production mission.
    //
    // The check is a one-pass walk over the IR before persistence,
    // so a rejection produces a hard error before any disk artifact
    // (run dir, trace, meta) is created.
    if let Some(conflict) = find_implicit_agent_fallback(&ir)? {
        anyhow::bail!(
            "step '{step_id}' uses traditional form `call ... on \"{name}\"` \
             but \"{name}\" is a registered agent. The traditional form is \
             strictly device-only — to invoke an agent, use member-call form: \
             `let r = {name}.{ability}(...)`. See docs/AGENT_IDENTITY.md.",
            step_id = conflict.step_id,
            name = conflict.colliding_name,
            ability = conflict.ability,
        );
    }

    // Persist source + IR. The writes are best-effort: a missed
    // source.eal / ir.json means the on-disk audit record is incomplete,
    // but the mission still runs. We log so an operator inspecting a
    // partially-populated run dir can attribute the gap.
    let run_dir = MissionRunDir::create(&ir.name)?;
    if let Err(e) = run_dir.write_source(source) {
        eprintln!(
            "[easynet warn] mission run {}: write source.eal failed ({e})",
            run_dir.path.display()
        );
    }
    if let Ok(ir_json) = serde_json::to_string_pretty(&ir) {
        if let Err(e) = run_dir.write_ir(&ir_json) {
            eprintln!(
                "[easynet warn] mission run {}: write ir.json failed ({e})",
                run_dir.path.display()
            );
        }
    }
    let run_id = run_dir
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    // Execute. The mission-context env var is set here so the dispatch
    // invariant in `runtime::dispatch::send_to_agent_with_depth` can verify
    // that every cross-agent call originates from a real mission run dir.
    // The RAII guard restores the previous value (or removes the var)
    // even if the interpreter panics.
    let _ctx = MissionContextGuard::enter(&run_id, opts.invocation_context.clone());

    let state = crate::persistence::config::load()?;
    let tenant = state.tenant_or_default();
    let started = std::time::Instant::now();
    let started_at = chrono::Local::now().to_rfc3339();

    let exec = crate::eal::interpreter::execute_with_endpoint_for_trace(
        &state.endpoint,
        tenant,
        &ir,
        run_id.clone(),
    );

    let duration_ms = started.elapsed().as_millis() as u64;

    let total_steps = ir.steps.len();
    let mut meta = MissionRunMeta {
        name: ir.name.clone(),
        source_file: opts.source_label.clone(),
        trace_id: run_id.clone(),
        started_at,
        duration_ms,
        status: MissionRunStatus::Ok,
        error: None,
        steps_total: total_steps,
        steps_completed: 0,
        steps_failed: 0,
        ability_graph_traces: None,
        invocation_context: opts.invocation_context.clone(),
    };

    match exec {
        Ok(report) => {
            meta.duration_ms = report.total_elapsed_ms;
            meta.steps_completed = report.steps_completed;
            meta.steps_failed = report.steps_failed;
            if !report.trace.ability_graph.is_empty() {
                meta.ability_graph_traces = Some(report.trace.ability_graph.clone());
            }
            // The interpreter returns Ok even when individual steps fail
            // — surface that as "partial" so the listing doesn't lie about
            // a run with broken steps.
            if report.steps_failed > 0 {
                meta.status = MissionRunStatus::Partial;
            }
            if let Ok(trace_json) = serde_json::to_string_pretty(&report.trace) {
                if let Err(e) = run_dir.write_trace(&trace_json) {
                    eprintln!(
                        "[easynet warn] mission run {}: write trace.json failed ({e})",
                        run_dir.path.display()
                    );
                }
            }
            if let Err(e) = run_dir.write_meta(&meta) {
                eprintln!(
                    "[easynet warn] mission run {}: write meta.json failed ({e})",
                    run_dir.path.display()
                );
            }
            run_dir.finish();

            // Convert ExecutionReport.outputs (HashMap<String, String>)
            // into HashMap<String, Value> by parsing each as JSON. If a
            // value isn't valid JSON, fall back to wrapping it as a JSON
            // string. This makes `bound_vars["__reply"]` directly usable
            // by the `agent send` desugar path.
            let bound_vars: HashMap<String, serde_json::Value> = report
                .outputs
                .into_iter()
                .map(|(k, raw)| {
                    // `unwrap_or_else` would be wasted here — parsing is
                    // the whole operation; if it fails we fall back to
                    // wrapping `raw` as a plain JSON string. Evaluating
                    // the fallback eagerly costs nothing.
                    let v = serde_json::from_str::<serde_json::Value>(&raw)
                        .unwrap_or(serde_json::Value::String(raw));
                    (k, v)
                })
                .collect();

            Ok(MissionRunResult {
                run_dir: run_dir.path.clone(),
                run_id,
                meta,
                bound_vars,
                ok: report.steps_failed == 0,
            })
        }
        Err(e) => {
            meta.status = MissionRunStatus::Error;
            meta.error = Some(e.to_string());
            if let Err(write_err) = run_dir.write_meta(&meta) {
                eprintln!(
                    "[easynet warn] mission run {}: write meta.json failed ({write_err})",
                    run_dir.path.display()
                );
            }
            run_dir.finish();
            Err(anyhow::anyhow!("mission run failed: {e}"))
        }
    }
}

// ── Mission context guard ──────────────────────────────────────────────────
//
// Installs the active `DispatchContext` for the duration of a mission run on
// the typed in-process channel only. Concurrent missions on different threads
// get independent contexts; worker pools receive the context by explicit
// handoff, not by process-global mutation.
//
// `EASYNET_MISSION_ID` is reserved for the cross-process boundary. When the
// runtime spawns an external agent CLI, `DispatchContext::serialize_to_env`
// writes the child command's env map; the parent process env is never mutated.
// See `runtime::context` for the design rationale.
/// RAII scope for the mission's typed dispatch context.
///
/// Audit invariant: NOTHING in-process writes `EASYNET_MISSION_ID`.
/// Writers exist only on the child command's env map
/// (`DispatchContext::serialize_to_env` at the spawn boundary); the
/// cross-process *read* path is `DispatchContext::from_env()` at the
/// child's entry. If you find yourself wanting a process-env writer,
/// install a typed `DispatchContext` instead — the env var is the
/// subprocess boundary, not a general-purpose channel.
struct MissionContextGuard {
    _ctx: crate::runtime::context::ContextGuard,
}

impl MissionContextGuard {
    fn enter(run_id: &str, invocation_context: Option<ParentInvocationContext>) -> Self {
        // Typed thread-local only (F-028 / T5.4): nothing in-process
        // writes EASYNET_MISSION_ID anymore. The interpreter hands the
        // context to its rayon workers explicitly, and the subprocess
        // boundary injects env vars through
        // `DispatchContext::serialize_to_env` on the child's command —
        // never through the parent's own environment, which is
        // process-global and stomped under concurrent missions.
        //
        // The run_dir field is filled in best-effort from the canonical
        // mission-runs root; if the dir is missing the dispatch
        // invariant check surfaces that separately (Stage 2
        // anti-forgery).
        let ctx =
            crate::runtime::context::DispatchContext::for_mission(run_id, root_dir().join(run_id))
                .with_parent_invocation(invocation_context);
        Self {
            _ctx: crate::runtime::context::enter(ctx),
        }
    }
}

fn sanitize_for_path(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if s.is_empty() {
        "mission".into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::test_support::HomeGuard;

    // ── sanitize_for_path: pure, parallel-safe ─────────────────────────────

    #[test]
    fn sanitize_handles_normal_names() {
        assert_eq!(sanitize_for_path("smoke-fail"), "smoke-fail");
        assert_eq!(sanitize_for_path("hello_world_42"), "hello_world_42");
    }

    #[test]
    fn sanitize_replaces_unsafe_chars() {
        assert_eq!(sanitize_for_path("a b/c"), "a-b-c");
        assert_eq!(sanitize_for_path("名字"), "mission"); // all replaced+trimmed
    }

    #[test]
    fn sanitize_falls_back_when_empty() {
        assert_eq!(sanitize_for_path(""), "mission");
        assert_eq!(sanitize_for_path("---"), "mission");
        assert_eq!(sanitize_for_path("///"), "mission");
    }

    #[test]
    fn allocate_unique_run_dir_returns_distinct_paths_for_same_stamp() {
        // Sequential calls with the same stamp must each get their own
        // directory. The suffix scheme is the user-visible contract for
        // mission-run discovery (`easynet mission list`).
        let root = std::env::temp_dir().join(format!(
            "easynet-mission-runs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        fs::create_dir_all(&root).unwrap();
        let a = allocate_unique_run_dir(&root, "2026-04-15_120000", "x").unwrap();
        let b = allocate_unique_run_dir(&root, "2026-04-15_120000", "x").unwrap();
        let c = allocate_unique_run_dir(&root, "2026-04-15_120000", "x").unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert!(a.exists() && b.exists() && c.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn allocate_unique_run_dir_survives_concurrent_callers() {
        // Regression guard for the TOCTOU race the iter-2 `run_store`
        // fix already addressed for agent runs. The previous
        // `while exists() { ... } create_dir_all(...)` pattern let two
        // racing missions both win the same directory; with
        // `create_dir`'s atomic semantics every successful allocation
        // must yield a distinct path.
        use std::sync::Arc;
        use std::thread;
        let root = Arc::new(std::env::temp_dir().join(format!(
            "easynet-mission-runs-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        )));
        fs::create_dir_all(&*root).unwrap();
        const N: usize = 16;
        let workers: Vec<_> = (0..N)
            .map(|_| {
                let r = Arc::clone(&root);
                thread::spawn(move || {
                    allocate_unique_run_dir(&r, "2026-04-15_120000", "x").unwrap()
                })
            })
            .collect();
        let paths: Vec<PathBuf> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(
            unique.len(),
            N,
            "expected {N} distinct paths, got {unique:?}"
        );
        let _ = fs::remove_dir_all(&*root);
    }

    /// W2 acceptance gate (spec §4): metas written before `trace_id`
    /// existed must keep deserializing — the field defaults to empty,
    /// it never gates parsing.
    #[test]
    fn pre_trace_id_meta_still_deserializes() {
        let old = r#"{
            "name": "legacy", "source_file": null,
            "started_at": "2026-01-01T00:00:00+00:00",
            "duration_ms": 7, "status": "ok", "error": null,
            "steps_total": 1, "steps_completed": 1, "steps_failed": 0
        }"#;
        let meta: MissionRunMeta = serde_json::from_str(old).expect("legacy meta parses");
        assert_eq!(meta.trace_id, "", "absent field defaults, never errors");
        assert_eq!(meta.status, MissionRunStatus::Ok);
    }

    /// The in-flight meta written at `create()` time already carries
    /// the trace anchor — `invocation watch --trace` must be able to
    /// attach while the run is alive, not only after completion.
    #[test]
    fn in_flight_meta_carries_trace_anchor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(dir.path().to_path_buf());
        let run = store.create("anchor-check").expect("create run");
        let raw = fs::read_to_string(run.path.join("meta.json")).expect("read in-flight meta");
        let meta: MissionRunMeta = serde_json::from_str(&raw).expect("parse in-flight meta");
        let run_id = run.path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(meta.trace_id, run_id);
        assert_eq!(meta.status, MissionRunStatus::Running);
    }

    fn make_meta(name: &str, status: MissionRunStatus) -> MissionRunMeta {
        MissionRunMeta {
            name: name.into(),
            source_file: Some(format!("/tmp/{name}.eal")),
            trace_id: String::new(),
            started_at: "2026-04-06T12:00:00+00:00".into(),
            duration_ms: 42,
            status,
            error: None,
            steps_total: 3,
            steps_completed: 3,
            steps_failed: 0,
            ability_graph_traces: None,
            invocation_context: None,
        }
    }

    #[test]
    fn mission_context_guard_never_touches_process_env() {
        let _g = HomeGuard::new();
        std::env::remove_var("EASYNET_MISSION_ID");
        {
            let _guard = MissionContextGuard::enter("env-free-run", None);
            assert!(
                std::env::var("EASYNET_MISSION_ID").is_err(),
                "the in-process channel is the thread-local, never the env"
            );
            assert_eq!(
                crate::runtime::context::current().map(|c| c.mission_id),
                Some("env-free-run".to_string())
            );
        }
        assert!(crate::runtime::context::current().is_none());
    }

    #[test]
    fn status_serde_matches_historical_literals() {
        // Every run directory written before the enum existed stores
        // these exact lowercase strings — they must keep parsing.
        for (s, expect) in [
            ("\"ok\"", MissionRunStatus::Ok),
            ("\"error\"", MissionRunStatus::Error),
            ("\"partial\"", MissionRunStatus::Partial),
            ("\"running\"", MissionRunStatus::Running),
            ("\"cancelled\"", MissionRunStatus::Cancelled),
        ] {
            let parsed: MissionRunStatus = serde_json::from_str(s).expect(s);
            assert_eq!(parsed, expect);
            assert_eq!(serde_json::to_string(&parsed).unwrap(), s);
        }
        // Unknown literals are rejected at parse time — no silent
        // string passthrough (AXIOM 22.2 family: no stringly state).
        assert!(serde_json::from_str::<MissionRunStatus>("\"bogus\"").is_err());
    }

    #[test]
    fn create_starts_heartbeat_and_finish_removes_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        let dir = store.create("smoke").expect("create");
        assert!(
            dir.path.join("heartbeat").exists(),
            "heartbeat file should exist after create"
        );
        // The initial meta makes the in-flight run visible to listings.
        let meta: MissionRunMeta = serde_json::from_str(
            &fs::read_to_string(dir.path.join("meta.json")).expect("initial meta written"),
        )
        .expect("initial meta parses");
        assert_eq!(meta.status, MissionRunStatus::Running);
        assert!(heartbeat_fresh(&dir.path), "fresh heartbeat reads alive");
        dir.finish();
        assert!(
            !dir.path.join("heartbeat").exists(),
            "heartbeat file should be gone after finish"
        );
        assert!(!heartbeat_fresh(&dir.path));
    }

    #[test]
    fn interrupted_run_reads_dead_not_running_forever() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        let dir = store.create("crashy").expect("create");
        // Simulate process death: heartbeat stops being touched and
        // goes stale (we backdate it rather than waiting 15 s).
        if let Some(pump) = &dir.pump {
            pump.stop();
        }
        let hb = dir.path.join("heartbeat");
        let stale = std::time::SystemTime::now() - (HEARTBEAT_STALE_AFTER * 2);
        let f = fs::File::options().write(true).open(&hb).expect("open hb");
        f.set_modified(stale).expect("backdate");
        drop(f);

        let id = dir.path.file_name().unwrap().to_string_lossy().to_string();
        let run = store.find_run(&id).expect("find");
        assert!(!run.running, "stale heartbeat must not read as running");
        assert!(
            run.is_interrupted(),
            "running-status + dead heartbeat = interrupted"
        );
        // And cancel can settle it (the pid file made this state
        // permanently un-cancellable).
        match store.cancel_run(&id).expect("cancel") {
            CancelOutcome::Cancelled(r) => {
                assert_eq!(r.meta.status, MissionRunStatus::Cancelled)
            }
            CancelOutcome::AlreadyTerminal(_) => panic!("interrupted run must be settleable"),
        }
    }

    #[test]
    fn mission_context_guard_preserves_parent_invocation_context() {
        let _g = HomeGuard::new();
        let _guard = MissionContextGuard::enter(
            "mission-parent-context",
            Some(
                ParentInvocationContext::from_json_value(serde_json::json!({
                    "caller": "easynet:///r/acme/agent/alice",
                    "subject": "easynet:///r/acme/resource/doc",
                    "causal_context": {"kind": "none"},
                }))
                .expect("typed parent invocation context"),
            ),
        );
        let ctx = crate::runtime::context::current().expect("mission context installed");
        assert_eq!(
            ctx.parent_invocation.as_ref().unwrap().subject.as_deref(),
            Some("easynet:///r/acme/resource/doc")
        );
    }

    #[test]
    fn create_collision_appends_suffix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        // Two runs with the same timestamp (same name, no real time gap).
        // The second one must land on a `-1` suffix instead of clobbering.
        let a = store.create("clash").expect("a");
        let b = store.create("clash").expect("b");
        assert_ne!(a.path, b.path);
        assert!(b.path.to_string_lossy().contains("-1"));
    }

    #[test]
    fn list_runs_is_empty_when_root_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        // No mission runs created under this fresh root.
        let runs = store.list_runs().expect("list");
        assert!(runs.is_empty());
    }

    #[test]
    fn list_runs_skips_dirs_without_meta() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        // `create()` now writes an initial running meta (in-flight
        // runs are visible by design), so a meta-less directory can
        // only be foreign junk — fabricate one directly.
        let junk = store.root().join("2026-01-01_000000_junk");
        fs::create_dir_all(&junk).expect("mkdir junk");
        fs::write(junk.join("source.eal"), "mission noop {}").expect("seed file");
        let runs = store.list_runs().expect("list");
        assert!(
            runs.is_empty(),
            "found {:?}",
            runs.iter().map(|r| &r.id).collect::<Vec<_>>()
        );
        // Sanity: the directory itself does exist.
        assert!(junk.exists());

        // And a freshly created run IS visible, as running.
        let dir = store.create("inflight").expect("create");
        let runs = store.list_runs().expect("list");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].meta.status, MissionRunStatus::Running);
        assert!(runs[0].running, "fresh heartbeat reads alive");
        dir.finish();
    }

    #[test]
    fn list_runs_returns_recorded_meta_sorted_desc() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        for n in ["alpha", "beta", "gamma"] {
            let d = store.create(n).expect("create");
            d.write_meta(&make_meta(n, MissionRunStatus::Ok))
                .expect("write_meta");
            d.finish();
        }
        let runs = store.list_runs().expect("list");
        assert_eq!(runs.len(), 3);
        // ID prefix is the same timestamp; ordering then comes from the
        // collision suffix appended by `create`. Whichever ordering, the
        // contract is "sorted descending by id".
        for w in runs.windows(2) {
            assert!(
                w[0].id >= w[1].id,
                "not sorted desc: {} vs {}",
                w[0].id,
                w[1].id
            );
        }
    }

    #[test]
    fn find_run_rejects_empty_id() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        assert!(store.find_run("").is_err());
        assert!(store.find_run("   ").is_err());
    }

    #[test]
    fn find_run_finds_exact_then_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        let d = store.create("solo").expect("create");
        d.write_meta(&make_meta("solo", MissionRunStatus::Ok))
            .expect("write_meta");
        d.finish();

        let id = d.path.file_name().unwrap().to_string_lossy().to_string();
        // exact
        let r = store.find_run(&id).expect("exact");
        assert_eq!(r.id, id);

        // prefix
        let prefix = &id[..id.len() - 4];
        let r = store.find_run(prefix).expect("prefix");
        assert_eq!(r.id, id);

        // missing
        assert!(store.find_run("does-not-exist").is_err());
    }

    #[test]
    fn cancel_run_flips_in_flight_to_cancelled() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        let d = store.create("running").expect("create");
        d.write_meta(&make_meta("running", MissionRunStatus::Running))
            .expect("write_meta");
        // intentionally do NOT call finish — the run stays in flight.
        let id = d.path.file_name().unwrap().to_string_lossy().to_string();

        match store.cancel_run(&id).expect("cancel") {
            CancelOutcome::Cancelled(r) => {
                assert_eq!(r.meta.status, MissionRunStatus::Cancelled);
                assert!(!r.running);
            }
            CancelOutcome::AlreadyTerminal(_) => panic!("expected Cancelled"),
        }
        // heartbeat file is gone now.
        assert!(!d.path.join("heartbeat").exists());
    }

    #[test]
    fn cancel_run_noop_on_terminal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = MissionRunStore::with_root(tmp.path().join("runs"));
        let d = store.create("done").expect("create");
        d.write_meta(&make_meta("done", MissionRunStatus::Ok))
            .expect("write_meta");
        d.finish(); // stop heartbeat → run no longer reads alive
        let id = d.path.file_name().unwrap().to_string_lossy().to_string();

        match store.cancel_run(&id).expect("cancel") {
            CancelOutcome::AlreadyTerminal(r) => assert_eq!(r.meta.status, MissionRunStatus::Ok),
            CancelOutcome::Cancelled(_) => panic!("expected AlreadyTerminal"),
        }
    }

    // ── EAL surface invariant: no implicit agent fallback ──────────────────
    //
    // The traditional EAL `call ... on ...` form is STRICTLY device-only.
    // Member-call form `agent.ability(...)` is the ONLY way to invoke an
    // agent. Writing `call "x" on "<agent-name>"` where the name collides
    // with a registered agent is an error, not a fallback.
    //
    // These tests are the load-bearing anti-regression for that invariant.
    // If a future contributor accidentally re-introduces implicit agent
    // fallback (e.g. by making the dispatcher do `is_agent` lookups, or
    // by deleting the `find_implicit_agent_fallback` check), one of these
    // tests will fail. The test name encodes the invariant so it is
    // searchable.
    //
    // See `docs/AGENT_IDENTITY.md` and the EAL surface invariant comment
    // in `src/eal/parser.rs`.

    /// Helper: register an agent in a temp HOME so the conflict-detection
    /// has something to find.
    fn register_test_agent(name: &str) {
        use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            name.to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        crate::registry::agents::save_agents(&registry).expect("save test agent");
    }

    #[test]
    fn no_implicit_agent_fallback_traditional_form_with_agent_name_is_rejected() {
        let _g = HomeGuard::new();
        register_test_agent("claude");

        // Traditional form addressing a registered agent name → must
        // be rejected by run_mission_inproc, not silently classified
        // as a Device.
        let source = r#"
            mission "regression" {
                let r = call "chat" on "claude" with { prompt = "hi" }
            }
        "#;
        let result = run_mission_inproc(
            source,
            MissionRunOpts {
                source_label: Some("regression".into()),
                trace_path: None,
                invocation_context: None,
            },
        );
        let err = result.expect_err("traditional form on agent name must be rejected");
        let msg = format!("{err}");
        // Error message must:
        //   1. name the colliding agent
        //   2. point at the member-call form as the correct alternative
        //   3. mention the docs
        assert!(
            msg.contains("\"claude\""),
            "error must name the colliding agent: got {msg}"
        );
        assert!(
            msg.contains("member-call form"),
            "error must point at member-call form: got {msg}"
        );
        assert!(
            msg.contains("claude.chat"),
            "error must suggest the exact member-call form: got {msg}"
        );
        assert!(
            msg.contains("AGENT_IDENTITY.md"),
            "error must reference the docs: got {msg}"
        );
    }

    #[test]
    fn no_implicit_agent_fallback_member_call_form_is_accepted() {
        let _g = HomeGuard::new();
        register_test_agent("claude");

        // Member-call form addressing the same agent name → must
        // compile successfully (the rejection is specific to the
        // traditional form, not to the agent name itself).
        let source = r#"
            mission "regression" {
                let r = claude.chat(prompt: "hi")
            }
        "#;
        // We don't care if execution succeeds (it would try to spawn a
        // real claude binary, which the test env may not have). We
        // only care that compile + the conflict check pass. So we
        // call the planner directly to bypass the spawn-and-execute
        // path.
        let program = crate::eal::parser::parse(source).expect("parse");
        let ir = crate::eal::planner::compile(&program).expect("compile");
        let conflict = find_implicit_agent_fallback(&ir).expect("registry load");
        assert!(
            conflict.is_none(),
            "member-call form must NOT trigger the implicit-fallback check"
        );
    }

    #[test]
    fn no_implicit_agent_fallback_traditional_form_with_device_name_is_accepted() {
        let _g = HomeGuard::new();
        register_test_agent("claude");

        // Traditional form addressing a device-style name (NOT an agent)
        // → must compile cleanly. The conflict check is name-specific,
        // not form-specific.
        let source = r#"
            mission "regression" {
                let r = call "shell.exec" on "node-1" with { command = "ls" }
            }
        "#;
        let program = crate::eal::parser::parse(source).expect("parse");
        let ir = crate::eal::planner::compile(&program).expect("compile");
        let conflict = find_implicit_agent_fallback(&ir).expect("registry load");
        assert!(
            conflict.is_none(),
            "device-style name must not trigger the conflict check: got {conflict:?}"
        );
    }
}
