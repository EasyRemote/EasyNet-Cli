// EasyNet CLI — MemoryLoader (chat context)
// ==========================================
//
// File: src/runtime/system/context_loaders/memory.rs
// Description: A `ContextLoader` that reads the agent's memory
//              directory (`<agent-root>/memory/*.md`) and emits the
//              N most-recent entries as a markdown block the chat
//              handler injects before the LLM call.
//
// On-disk format (v1)
// -------------------
// One markdown file per memory entry under `<agent-root>/memory/`.
// File extension is `.md` (anything else is ignored). Sort order is
// mtime descending — the most recently touched memory wins. Memory
// entries are agent-scoped: an "alice" chat call only sees
// `~/.easynet/agents/alice/memory/*.md`, never `bob`'s.
//
// Why mtime + cap (rather than reading everything)
// ------------------------------------------------
// An LLM prompt with hundreds of memory files would blow the
// context budget and bury the relevant signal. Capping at the
// MAX_ENTRIES most-recent files is the simplest "recency = relevance"
// heuristic; a future loader can refine this with semantic search
// without changing the trait surface.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::runtime::agents::chat_ability::ContextLoader;

/// Default cap on rendered memory entries. Below the chat handler's
/// "context block stays focused" budget; an operator with hundreds
/// of memories should call `system.memory.list` (a future ability)
/// rather than expect chat to dump them all.
pub const MAX_ENTRIES_RENDERED: usize = 5;

/// Per-entry character cap. A single rambling memory file should
/// not blow the budget for the rest of the chain. Truncated entries
/// get an explicit "…(truncated)" suffix so the LLM knows it saw a
/// partial payload.
pub const MAX_CHARS_PER_ENTRY: usize = 2000;

/// Resolved on first call: `~/.easynet/agents/<agent>/memory`. We
/// do not cache this across calls because operator state-dir env
/// changes (`EASYNET_HOME`, `XDG_*`) should take effect immediately
/// for the test harness.
fn agent_memory_dir(agent_name: &str) -> PathBuf {
    crate::persistence::config::agents_root()
        .join(agent_name)
        .join("memory")
}

pub struct MemoryLoader {
    max_entries: usize,
    max_chars_per_entry: usize,
}

impl MemoryLoader {
    pub fn new() -> Self {
        Self {
            max_entries: MAX_ENTRIES_RENDERED,
            max_chars_per_entry: MAX_CHARS_PER_ENTRY,
        }
    }

    /// Test/tuning constructor. Production wires `new()`.
    #[cfg(test)]
    pub fn with_caps(max_entries: usize, max_chars_per_entry: usize) -> Self {
        Self {
            max_entries,
            max_chars_per_entry,
        }
    }
}

impl Default for MemoryLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextLoader for MemoryLoader {
    fn name(&self) -> &str {
        "memory"
    }

    fn load(
        &self,
        agent_name: &str,
        _session_id: &str,
    ) -> anyhow::Result<Option<String>> {
        let dir = agent_memory_dir(agent_name);
        if !dir.exists() {
            return Ok(None);
        }
        // Collect (path, mtime) tuples, then sort desc.
        let mut entries: Vec<(PathBuf, SystemTime)> = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| {
            anyhow::anyhow!("read memory dir {}: {e}", dir.display())
        })? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // Filter by extension; anything not .md (a stray
            // .DS_Store, an editor swap file, …) is skipped silently.
            let is_md = path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("md"))
                .unwrap_or(false);
            if !is_md {
                continue;
            }
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((path, mtime));
        }
        if entries.is_empty() {
            return Ok(None);
        }
        // Newest first.
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        let total = entries.len();
        let truncated = total > self.max_entries;
        entries.truncate(self.max_entries);

        let mut out = String::new();
        out.push_str("## Recent memory\n\n");
        for (path, _mtime) in entries {
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unnamed");
            let body = match fs::read_to_string(&path) {
                Ok(b) => b,
                Err(e) => {
                    out.push_str(&format!(
                        "### {title}\n\n_(failed to read: {e})_\n\n"
                    ));
                    continue;
                }
            };
            let cap = self.max_chars_per_entry;
            let (body_to_emit, truncated_body) = if body.chars().count() > cap {
                let truncated: String = body.chars().take(cap).collect();
                (truncated, true)
            } else {
                (body, false)
            };
            out.push_str(&format!("### {title}\n\n"));
            out.push_str(&body_to_emit);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            if truncated_body {
                out.push_str("\n_…(truncated)_\n");
            }
            out.push('\n');
        }
        if truncated {
            out.push_str(&format!(
                "_…and {} older memory file(s) beyond the {}-entry cap._\n",
                total - self.max_entries,
                self.max_entries
            ));
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;

    fn write_memory(agent: &str, name: &str, body: &str) -> PathBuf {
        let dir = crate::persistence::config::agents_root()
            .join(agent)
            .join("memory");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.md"));
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn loader_returns_none_when_no_memory_dir() {
        let _g = HomeGuard::new();
        let loader = MemoryLoader::new();
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_returns_none_for_empty_memory_dir() {
        let _g = HomeGuard::new();
        let dir = crate::persistence::config::agents_root()
            .join("alice")
            .join("memory");
        fs::create_dir_all(&dir).unwrap();
        let loader = MemoryLoader::new();
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_renders_recent_memories_in_mtime_desc_order() {
        let _g = HomeGuard::new();
        // Write three with explicit mtime spread so the sort is deterministic
        // even on filesystems with second-level mtime granularity.
        let p1 = write_memory("alice", "first", "FIRST_CONTENT");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let p2 = write_memory("alice", "second", "SECOND_CONTENT");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let p3 = write_memory("alice", "third", "THIRD_CONTENT");
        let _ = (p1, p2, p3);

        let loader = MemoryLoader::new();
        let text = loader.load("alice", "s-1").unwrap().unwrap();
        // Newest first → THIRD before FIRST.
        let pos_third = text.find("THIRD_CONTENT").expect("third must appear");
        let pos_first = text.find("FIRST_CONTENT").expect("first must appear");
        assert!(
            pos_third < pos_first,
            "newest mtime must render first: {text}"
        );
    }

    #[test]
    fn loader_caps_at_max_entries_and_notes_truncation() {
        let _g = HomeGuard::new();
        let loader = MemoryLoader::with_caps(2, 1000);
        for i in 0..5 {
            write_memory("alice", &format!("m{i}"), &format!("body-{i}"));
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let text = loader.load("alice", "s-1").unwrap().unwrap();
        assert!(text.contains("older memory file(s) beyond"));
    }

    #[test]
    fn loader_truncates_overlong_entry_with_marker() {
        let _g = HomeGuard::new();
        let loader = MemoryLoader::with_caps(5, 50);
        let big = "X".repeat(200);
        write_memory("alice", "big", &big);
        let text = loader.load("alice", "s-1").unwrap().unwrap();
        assert!(text.contains("_…(truncated)_"));
    }

    #[test]
    fn loader_ignores_non_md_files() {
        let _g = HomeGuard::new();
        let dir = crate::persistence::config::agents_root()
            .join("alice")
            .join("memory");
        fs::create_dir_all(&dir).unwrap();
        // Stray non-md files: must be ignored silently.
        fs::write(dir.join(".DS_Store"), "junk").unwrap();
        fs::write(dir.join("notes.txt"), "should not appear").unwrap();
        let loader = MemoryLoader::new();
        // Empty after filtering — load returns None.
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_is_agent_scoped() {
        let _g = HomeGuard::new();
        write_memory("alice", "alice-only", "ALICE_SECRET");
        write_memory("bob", "bob-only", "BOB_SECRET");
        let loader = MemoryLoader::new();
        let alice_text = loader.load("alice", "s-1").unwrap().unwrap();
        assert!(alice_text.contains("ALICE_SECRET"));
        assert!(!alice_text.contains("BOB_SECRET"));
    }
}
