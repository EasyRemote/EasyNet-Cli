// EasyNet CLI — Agent Run Persistence
// ====================================
//
// File: src/agent/run_store.rs
// Description: Per-invocation persistence for agent runs. Every `agent send`
//              call creates a timestamped directory under the agent's
//              workspace containing prompt, response, trace, and metadata.
//
// Layout:
//   ~/.easynet/workspaces/<agent>/runs/<YYYY-MM-DD_HHMMSS>/
//     ├── prompt.txt     — composed prompt (incl. context) sent to the agent
//     ├── response.md    — final agent reply, as markdown
//     ├── trace.jsonl    — raw stream events (one JSON object per line)
//     └── meta.json      — timing, token counts, cost, model, exit status
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Local;
use serde::{Deserialize, Serialize};

use super::workspace;

/// A single persisted run directory. Cheap to clone (Arc-like file handles).
pub struct RunDir {
    pub path: PathBuf,
    /// Lazily-opened append handle for trace.jsonl.
    trace: Mutex<Option<File>>,
}

impl RunDir {
    /// Create a new timestamped run directory under the agent's workspace.
    pub fn create(agent_name: &str) -> anyhow::Result<Self> {
        let ws = workspace::workspace_dir(agent_name);
        let runs = ws.join("runs");
        fs::create_dir_all(&runs)?;

        // Use a timestamp with a dash separator so it sorts correctly and
        // works as a directory name on every filesystem we care about.
        let stamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
        let mut path = runs.join(&stamp);
        // Collision-proof: if the same second already has a directory,
        // append `-1`, `-2`, etc.
        let mut suffix = 0u32;
        while path.exists() {
            suffix += 1;
            path = runs.join(format!("{stamp}-{suffix}"));
        }
        fs::create_dir_all(&path)?;

        Ok(Self {
            path,
            trace: Mutex::new(None),
        })
    }

    /// Write the composed prompt to `prompt.txt`.
    pub fn write_prompt(&self, prompt: &str) {
        let _ = fs::write(self.path.join("prompt.txt"), prompt);
    }

    /// Write the final response (markdown) to `response.md`.
    pub fn write_response(&self, response: &str) {
        let _ = fs::write(self.path.join("response.md"), response);
    }

    /// Append a single raw stream-event line to `trace.jsonl`.
    /// Idempotent — the file is opened on first use and kept for the run.
    pub fn append_trace_line(&self, line: &str) {
        let mut guard = match self.trace.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if guard.is_none() {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path.join("trace.jsonl"))
            {
                Ok(f) => *guard = Some(f),
                Err(_) => return,
            }
        }
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }

    /// Write the run metadata (stats + cost + exit status) to `meta.json`.
    pub fn write_meta(&self, meta: &RunMeta) {
        if let Ok(json) = serde_json::to_string_pretty(meta) {
            let _ = fs::write(self.path.join("meta.json"), json + "\n");
        }
    }

    /// Directory path for display.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Metadata written to `meta.json` at the end of the run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunMeta {
    pub agent: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub started_at: String,
    pub duration_ms: u64,
    pub exit_status: String, // "ok" | "error" | "timeout"
    pub error: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub num_turns: u64,
    pub total_cost_usd: f64,
}
