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

use std::fs;
use std::path::PathBuf;

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::shared::config;

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
        let mut path = root.join(format!("{stamp}_{safe_name}"));
        let mut suffix = 0u32;
        while path.exists() {
            suffix += 1;
            path = root.join(format!("{stamp}_{safe_name}-{suffix}"));
        }
        fs::create_dir_all(&path)?;
        // mark in-flight; deleted on completion
        let _ = fs::write(path.join("pid"), std::process::id().to_string());
        Ok(Self { path })
    }

    pub fn write_source(&self, source: &str) {
        let _ = fs::write(self.path.join("source.eal"), source);
    }
    pub fn write_ir(&self, ir_json: &str) {
        let _ = fs::write(self.path.join("ir.json"), ir_json);
    }
    pub fn write_trace(&self, trace_json: &str) {
        let _ = fs::write(self.path.join("trace.json"), trace_json);
    }
    pub fn write_meta(&self, meta: &MissionRunMeta) {
        if let Ok(s) = serde_json::to_string_pretty(meta) {
            let _ = fs::write(self.path.join("meta.json"), s + "\n");
        }
    }
    pub fn finish(&self) {
        let _ = fs::remove_file(self.path.join("pid"));
    }
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
    /// field name alone. See ARCHITECTURE.md §3 (self-evolution = graph)
    /// and §10 (non-CLI artefacts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_graph_traces: Option<Vec<serde_json::Value>>,
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
        }
    }

    #[test]
    fn create_writes_pid_and_finish_removes_it() {
        let _g = HomeGuard::new();
        let dir = MissionRunDir::create("smoke").expect("create");
        assert!(dir.path.join("pid").exists(), "pid file should exist after create");
        dir.finish();
        assert!(!dir.path.join("pid").exists(), "pid file should be gone after finish");
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
        assert!(runs.is_empty(), "found {:?}", runs.iter().map(|r| &r.id).collect::<Vec<_>>());
        // Sanity: the directory itself does exist.
        assert!(dir.path.exists());
    }

    #[test]
    fn list_runs_returns_recorded_meta_sorted_desc() {
        let _g = HomeGuard::new();
        for n in ["alpha", "beta", "gamma"] {
            let d = MissionRunDir::create(n).expect("create");
            d.write_meta(&make_meta(n));
            d.finish();
        }
        let runs = list_runs().expect("list");
        assert_eq!(runs.len(), 3);
        // ID prefix is the same timestamp; ordering then comes from the
        // collision suffix appended by `create`. Whichever ordering, the
        // contract is "sorted descending by id".
        for w in runs.windows(2) {
            assert!(w[0].id >= w[1].id, "not sorted desc: {} vs {}", w[0].id, w[1].id);
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
        d.write_meta(&make_meta("solo"));
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
        d.write_meta(&make_meta("running"));
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
        d.write_meta(&make_meta("done"));
        d.finish(); // remove pid → terminal
        let id = d.path.file_name().unwrap().to_string_lossy().to_string();

        match cancel_run(&id).expect("cancel") {
            CancelOutcome::AlreadyTerminal(r) => assert_eq!(r.meta.status, "ok"),
            CancelOutcome::Cancelled(_) => panic!("expected AlreadyTerminal"),
        }
    }
}

