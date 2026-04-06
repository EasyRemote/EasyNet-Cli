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
    pub status: String, // "ok" | "error" | "running" | "cancelled"
    pub error: Option<String>,
    pub steps_total: usize,
    pub steps_completed: usize,
    pub steps_failed: usize,
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
    let runs = list_runs()?;
    let exact = runs.iter().find(|r| r.id == id);
    if let Some(r) = exact {
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

/// Mark a run cancelled. Best-effort: only updates meta.json + removes pid.
pub fn cancel_run(id: &str) -> anyhow::Result<MissionRunSummary> {
    let mut run = find_run(id)?;
    if !run.running && run.meta.status != "ok" && run.meta.status != "error" {
        // already terminal — leave as-is
    } else if run.running {
        run.meta.status = "cancelled".to_string();
        let _ = fs::remove_file(run.path.join("pid"));
        if let Ok(s) = serde_json::to_string_pretty(&run.meta) {
            let _ = fs::write(run.path.join("meta.json"), s + "\n");
        }
        run.running = false;
    }
    Ok(run)
}

fn sanitize_for_path(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

