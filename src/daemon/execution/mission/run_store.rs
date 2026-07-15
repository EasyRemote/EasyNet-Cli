// EasyNet CLI — Agent Run Persistence
// ====================================
//
// File: src/agent/run_store.rs
// Description: Per-invocation persistence for agent runs. Every `agent send`
//              call creates a timestamped directory under the agent's
//              workspace containing prompt, response, and metadata.
//
// Layout:
//   ~/.easynet/agents/<agent>/runs/<YYYY-MM-DD_HHMMSS>/
//     ├── prompt.txt     — composed prompt (incl. context) sent to the agent
//     ├── response.md    — final agent reply, as markdown
//     └── meta.json      — timing, token counts, cost, model, exit status, invocation_id
//
// The stream event log (`trace.jsonl`) moved to the Timeline /
// PersistentLog in PR-7 Commit 2. `meta.json::invocation_id` is the
// cross-reference key: operators locate the event stream at
// `$AXON_INVOCATION_LOG_DIR/<invocation_id>.jsonl`. See
// `daemon::execution::mission::session::Session` and
// `daemon::execution::mission::timeline::TimelineWriter`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Local;
use serde::{Deserialize, Serialize};

use super::workspace;

/// A single persisted run directory. Holds per-run artefact paths
/// (`prompt.txt`, `response.md`, `meta.json`). The stream event
/// log lives in the Timeline (see module doc).
pub struct RunDir {
    pub path: PathBuf,
}

impl RunDir {
    /// Create a new timestamped run directory under the agent's workspace.
    pub fn create(agent_name: &str) -> anyhow::Result<Self> {
        let ws = workspace::workspace_dir(agent_name);
        let runs = ws.join("runs");
        fs::create_dir_all(&runs)?;
        let stamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
        let path = allocate_unique_run_dir(&runs, &stamp)?;
        Ok(Self { path })
    }

    /// Write the composed prompt to `prompt.txt`.
    ///
    /// Returns `Result` rather than swallowing — a missed prompt write means
    /// the run directory is no longer a faithful record, and a caller might
    /// reasonably choose to fail loudly rather than continue with a half-
    /// persisted run. The CLI dispatcher currently logs and proceeds.
    pub fn write_prompt(&self, prompt: &str) -> io::Result<()> {
        fs::write(self.path.join("prompt.txt"), prompt)
    }

    /// Write the final response (markdown) to `response.md`. See
    /// `write_prompt` for why this returns `Result`.
    pub fn write_response(&self, response: &str) -> io::Result<()> {
        fs::write(self.path.join("response.md"), response)
    }

    /// Write the run metadata to `meta.json`. Returns `Result`; a serialize
    /// failure indicates a programming error (RunMeta is plain data) and a
    /// write failure means the operator has lost the per-run audit record.
    pub fn write_meta(&self, meta: &RunMeta) -> io::Result<()> {
        let json = serde_json::to_string_pretty(meta)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(self.path.join("meta.json"), json + "\n")
    }

    /// Directory path for display.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Allocate a unique run directory for `stamp` under `runs`, retrying
/// with `-1`, `-2`, ... on collision.
///
/// Concurrency note: two calls landing in the same wall-clock second
/// must each get their own directory — interleaved trace lines and
/// stomped `response.md` files have no recovery path. A check-then-act
/// loop (`while exists() { ... } create_dir_all(...)`) is racy because
/// both racers can pass the existence check and then both `create_dir_all`
/// succeed silently (it treats "already exists" as OK). We instead use
/// `create_dir`, which fails atomically with `AlreadyExists` if the
/// directory already exists, and retry with the next suffix on conflict.
/// POSIX `mkdir` is `O_EXCL`-equivalent; Windows `CreateDirectoryW` has
/// the same atomicity.
fn allocate_unique_run_dir(runs: &Path, stamp: &str) -> anyhow::Result<PathBuf> {
    // Cap retries: a real second never sees more than a handful of
    // concurrent runs from one device, but the bound stops us from
    // spinning forever if (e.g.) the parent dir becomes unwritable
    // and we keep racing with our own permission errors.
    const MAX_SUFFIX_ATTEMPTS: u32 = 10_000;
    for suffix in 0..MAX_SUFFIX_ATTEMPTS {
        let path = if suffix == 0 {
            runs.join(stamp)
        } else {
            runs.join(format!("{stamp}-{suffix}"))
        };
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!(
        "could not allocate a unique run directory under {} after {MAX_SUFFIX_ATTEMPTS} attempts",
        runs.display()
    )
}

/// Metadata written to `meta.json` at the end of the run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunMeta {
    pub agent: String,
    pub agent_type: String,
    pub model: Option<String>,
    /// PersistentLog invocation_id for this run. Empty on a meta
    /// written by a pre-PR-7 binary (legacy records). A populated
    /// id lets an operator cross-reference the run directory with
    /// the on-disk event log at `$AXON_INVOCATION_LOG_DIR/<id>.jsonl`,
    /// which carries the P1-P6-compliant event stream (see
    /// `daemon::execution::mission::session::Session`). `#[serde(default)]`
    /// keeps deserialisation backward-compatible with meta.json files
    /// written before this field existed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invocation_id: String,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn temp_runs() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "easynet-runstore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn allocate_unique_run_dir_returns_distinct_paths_for_same_stamp() {
        // Sequential calls with the same stamp must each get their own
        // directory. The suffix scheme is the user-visible contract.
        let runs = temp_runs();
        let a = allocate_unique_run_dir(&runs, "2026-04-15_120000").unwrap();
        let b = allocate_unique_run_dir(&runs, "2026-04-15_120000").unwrap();
        let c = allocate_unique_run_dir(&runs, "2026-04-15_120000").unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert!(a.exists() && b.exists() && c.exists());
        let _ = fs::remove_dir_all(&runs);
    }

    #[test]
    fn allocate_unique_run_dir_survives_concurrent_callers() {
        // Regression guard for the TOCTOU race: a `while exists() { ... }
        // create_dir_all(...)` loop would let two racers both observe
        // "doesn't exist" and then both succeed in creating, returning the
        // same path to two callers. With `create_dir`'s atomic semantics,
        // every successful return must yield a distinct path.
        let runs = Arc::new(temp_runs());
        const N: usize = 16;
        let workers: Vec<_> = (0..N)
            .map(|_| {
                let r = Arc::clone(&runs);
                thread::spawn(move || allocate_unique_run_dir(&r, "2026-04-15_120000").unwrap())
            })
            .collect();
        let paths: Vec<PathBuf> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(
            unique.len(),
            N,
            "expected {N} distinct paths, got {unique:?}"
        );
        let _ = fs::remove_dir_all(&*runs);
    }
}
