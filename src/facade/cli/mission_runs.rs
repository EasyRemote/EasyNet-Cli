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
//     └── pid             — empty file: presence means the run is in-flight
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::persistence::config;
use crate::runtime::context::ParentInvocationContext;

pub fn root_dir() -> PathBuf {
    config::state_dir().join("missions").join("runs")
}

pub struct MissionRunDir {
    pub path: PathBuf,
}

impl MissionRunDir {
    pub fn create(name: &str) -> anyhow::Result<Self> {
        let root = root_dir();
        fs::create_dir_all(&root)?;
        let stamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
        let safe_name = sanitize_for_path(name);
        let path = allocate_unique_run_dir(&root, &stamp, &safe_name)?;
        // Mark in-flight; deleted on completion. Best-effort: if the
        // pid file fails to write the run still proceeds (the file is
        // a debugging aid, not load-bearing for correctness).
        if let Err(e) = fs::write(path.join("pid"), std::process::id().to_string()) {
            eprintln!(
                "[easynet warn] mission run {}: write pid failed ({e})",
                path.display()
            );
        }
        Ok(Self { path })
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
        let _ = fs::remove_file(self.path.join("pid"));
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissionRunMeta {
    pub name: String,
    pub source_file: Option<String>,
    pub started_at: String,
    pub duration_ms: u64,
    pub status: String, // "ok" | "error" | "partial" | "running" | "cancelled"
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
    pub running: bool,
}

pub fn list_runs() -> anyhow::Result<Vec<MissionRunSummary>> {
    let root = root_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&root)? {
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
        let running = path.join("pid").exists();
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

pub fn find_run(id: &str) -> anyhow::Result<MissionRunSummary> {
    // Reject blank ids — otherwise `starts_with("")` would match every run
    // and silently return the first one (or bail "ambiguous"), neither of
    // which is helpful.
    let id = id.trim();
    if id.is_empty() {
        anyhow::bail!("mission run id is empty");
    }

    let runs = list_runs()?;
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
    let matches: Vec<&MissionRunSummary> = runs.iter().filter(|r| r.id.starts_with(id)).collect();
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

/// Outcome of a `cancel_run` call. Lets callers report accurately whether
/// they actually changed anything.
pub enum CancelOutcome {
    Cancelled(MissionRunSummary),
    AlreadyTerminal(MissionRunSummary),
}

/// Mark a run cancelled if (and only if) it is currently in-flight.
/// Best-effort: only updates meta.json + removes pid.
pub fn cancel_run(id: &str) -> anyhow::Result<CancelOutcome> {
    let mut run = find_run(id)?;
    if !run.running {
        return Ok(CancelOutcome::AlreadyTerminal(run));
    }
    run.meta.status = "cancelled".to_string();
    let _ = fs::remove_file(run.path.join("pid"));
    if let Ok(s) = serde_json::to_string_pretty(&run.meta) {
        let _ = fs::write(run.path.join("meta.json"), s + "\n");
    }
    run.running = false;
    Ok(CancelOutcome::Cancelled(run))
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
// `runtime::agents::mission_ability` and `runtime::agents::eal_executor` both
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
    /// `EASYNET_MISSION_ID`.
    #[allow(dead_code)]
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

    let agent_rows = crate::facade::cli::daemon_agent_view::list_agents()?;

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
///
/// TODO(layering): when this function grows past ~150 lines, split into:
///   compile_source(source) -> MissionIr
///   execute_ir(ir, opts)   -> MissionRunResult
///   dispatch_step(step)     -> StepResult (already in interpreter)
/// The single-entry contract still holds at the level of
/// `run_mission_inproc`; the split is purely internal.
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

    let exec = crate::eal::interpreter::execute_with_endpoint(&state.endpoint, tenant, &ir);

    let duration_ms = started.elapsed().as_millis() as u64;

    let total_steps = ir.steps.len();
    let mut meta = MissionRunMeta {
        name: ir.name.clone(),
        source_file: opts.source_label.clone(),
        started_at,
        duration_ms,
        status: "ok".into(),
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
                meta.status = "partial".into();
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
            meta.status = "error".into();
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
// Installs the active `DispatchContext` for the duration of a mission
// run on TWO channels:
//
//   1. The typed thread-local in `runtime::context` — the primary
//      in-process channel. Concurrent missions on different threads
//      get independent contexts (no cross-thread stomping).
//   2. The `EASYNET_MISSION_ID` env var — the cross-process channel.
//      When the mission interpreter spawns an external agent CLI as a
//      child process, the child inherits the env var and reconstructs
//      the typed context from it on entry.
//
// Both channels are reset on Drop (panic-safe). The env-var write is the
// remaining piece of process-global state in this codebase; it is here
// because spawning a subprocess is the only operation that crosses the
// thread-local boundary, and the env is the only mechanism that crosses
// the process boundary. See `runtime::context` for the design rationale.
struct MissionContextGuard {
    prev_env: Option<String>,
    _ctx: crate::runtime::context::ContextGuard,
}

impl MissionContextGuard {
    fn enter(run_id: &str, invocation_context: Option<ParentInvocationContext>) -> Self {
        let prev_env = std::env::var("EASYNET_MISSION_ID").ok();
        std::env::set_var("EASYNET_MISSION_ID", run_id);
        // Install the typed thread-local. The run_dir field is filled
        // in best-effort from the canonical mission-runs root; if the
        // dir is missing the dispatch invariant check will surface that
        // separately (the Stage 2 anti-forgery check).
        let ctx =
            crate::runtime::context::DispatchContext::for_mission(run_id, root_dir().join(run_id))
                .with_parent_invocation(invocation_context);
        let ctx_guard = crate::runtime::context::enter(ctx);
        Self {
            prev_env,
            _ctx: ctx_guard,
        }
    }
}

impl Drop for MissionContextGuard {
    fn drop(&mut self) {
        // Thread-local restore happens automatically via `_ctx`'s Drop;
        // we only need to clean up the env var the SDK can't see into.
        //
        // Audit invariant: this `Drop` impl, together with `enter()`
        // above, is the **only place in the codebase that writes
        // `EASYNET_MISSION_ID`**. The cross-process *read* path is
        // `runtime::context::DispatchContext::from_env()`. If you find
        // yourself wanting another writer, install a typed
        // `DispatchContext` instead — the env var is the subprocess
        // boundary, not a general-purpose channel.
        match self.prev_env.take() {
            Some(p) => std::env::set_var("EASYNET_MISSION_ID", p),
            None => std::env::remove_var("EASYNET_MISSION_ID"),
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
    use crate::facade::cli::test_support::HomeGuard;

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

    fn make_meta(name: &str) -> MissionRunMeta {
        MissionRunMeta {
            name: name.into(),
            source_file: Some(format!("/tmp/{name}.eal")),
            started_at: "2026-04-06T12:00:00+00:00".into(),
            duration_ms: 42,
            status: "ok".into(),
            error: None,
            steps_total: 3,
            steps_completed: 3,
            steps_failed: 0,
            ability_graph_traces: None,
            invocation_context: None,
        }
    }

    #[test]
    fn create_writes_pid_and_finish_removes_it() {
        let _g = HomeGuard::new();
        let dir = MissionRunDir::create("smoke").expect("create");
        assert!(
            dir.path.join("pid").exists(),
            "pid file should exist after create"
        );
        dir.finish();
        assert!(
            !dir.path.join("pid").exists(),
            "pid file should be gone after finish"
        );
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
        let _g = HomeGuard::new();
        // Two runs with the same timestamp (same name, no real time gap).
        // The second one must land on a `-1` suffix instead of clobbering.
        let a = MissionRunDir::create("clash").expect("a");
        let b = MissionRunDir::create("clash").expect("b");
        assert_ne!(a.path, b.path);
        assert!(b.path.to_string_lossy().contains("-1"));
    }

    #[test]
    fn list_runs_is_empty_when_root_missing() {
        let _g = HomeGuard::new();
        // No mission runs created in this clean HOME.
        let runs = list_runs().expect("list");
        assert!(runs.is_empty());
    }

    #[test]
    fn list_runs_skips_dirs_without_meta() {
        let _g = HomeGuard::new();
        let dir = MissionRunDir::create("noisy").expect("create");
        // No write_meta call → list_runs must skip this directory.
        let runs = list_runs().expect("list");
        assert!(
            runs.is_empty(),
            "found {:?}",
            runs.iter().map(|r| &r.id).collect::<Vec<_>>()
        );
        // Sanity: the directory itself does exist.
        assert!(dir.path.exists());
    }

    #[test]
    fn list_runs_returns_recorded_meta_sorted_desc() {
        let _g = HomeGuard::new();
        for n in ["alpha", "beta", "gamma"] {
            let d = MissionRunDir::create(n).expect("create");
            d.write_meta(&make_meta(n)).expect("write_meta");
            d.finish();
        }
        let runs = list_runs().expect("list");
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
        let _g = HomeGuard::new();
        assert!(find_run("").is_err());
        assert!(find_run("   ").is_err());
    }

    #[test]
    fn find_run_finds_exact_then_prefix() {
        let _g = HomeGuard::new();
        let d = MissionRunDir::create("solo").expect("create");
        d.write_meta(&make_meta("solo")).expect("write_meta");
        d.finish();

        let id = d.path.file_name().unwrap().to_string_lossy().to_string();
        // exact
        let r = find_run(&id).expect("exact");
        assert_eq!(r.id, id);

        // prefix
        let prefix = &id[..id.len() - 4];
        let r = find_run(prefix).expect("prefix");
        assert_eq!(r.id, id);

        // missing
        assert!(find_run("does-not-exist").is_err());
    }

    #[test]
    fn cancel_run_flips_in_flight_to_cancelled() {
        let _g = HomeGuard::new();
        let d = MissionRunDir::create("running").expect("create");
        d.write_meta(&make_meta("running")).expect("write_meta");
        // intentionally do NOT call finish — pid file stays in place.
        let id = d.path.file_name().unwrap().to_string_lossy().to_string();

        match cancel_run(&id).expect("cancel") {
            CancelOutcome::Cancelled(r) => {
                assert_eq!(r.meta.status, "cancelled");
                assert!(!r.running);
            }
            CancelOutcome::AlreadyTerminal(_) => panic!("expected Cancelled"),
        }
        // pid file is gone now.
        assert!(!d.path.join("pid").exists());
    }

    #[test]
    fn cancel_run_noop_on_terminal() {
        let _g = HomeGuard::new();
        let d = MissionRunDir::create("done").expect("create");
        d.write_meta(&make_meta("done")).expect("write_meta");
        d.finish(); // remove pid → terminal
        let id = d.path.file_name().unwrap().to_string_lossy().to_string();

        match cancel_run(&id).expect("cancel") {
            CancelOutcome::AlreadyTerminal(r) => assert_eq!(r.meta.status, "ok"),
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
