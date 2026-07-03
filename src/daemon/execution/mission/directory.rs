// EasyNet CLI — Agent Directory
// ==============================
//
// File: src/daemon/execution/mission/directory.rs
// Description: An `AgentDirectory` is the on-disk home of one
//              agent — the canonical source of truth for its
//              configuration, abilities, skills, memory, and per-run
//              artefacts. This module owns the layout convention and
//              the file-system operations that create or open one.
//
// Layout on disk
// --------------
//
//   <agent-root>/
//     agent.toml            — AgentSpec (required, source of truth)
//     abilities/            — ability manifests (PR-4 populates)
//     skills/               — private skills (contents agent-defined)
//     memory/               — long-running memory; agent-defined shape
//     mcp_servers.json      — per-agent MCP server list (opt-in)
//     .env                  — per-agent env vars, chmod 600 on unix
//     runs/                 — per-run artefact directories (run_store)
//     CLAUDE.md / AGENTS.md — populated by `runtime::workspace` when a
//                             runtime-native projection runs. Their
//                             presence is not an AgentDirectory invariant;
//                             workspace-layer calls ensure them before
//                             dispatch.
//     .mcp.json             — Claude Code discovery file (workspace-layer)
//     .codex/config.toml    — Codex discovery file (workspace-layer)
//     .agents/skills/       — Codex skill location (workspace-layer)
//     .git/                 — Codex requires a repo root; workspace
//                             layer `git init`s when absent.
//
// Why this module does not create the runtime-native files
// --------------------------------------------------------
// `.mcp.json`, `.codex/config.toml`, `.agents/skills/`, and the `git
// init` call are projections of the spec into runtime-native shapes.
// That is the job of `runtime::workspace` — and the reversal of
// workspace.rs from "creator" to "projector" is a separate PR. This
// module's only contract is the agent root layout itself.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::ability::spec::{default_chat_manifest, AbilityManifest};
use crate::core::agent::spec::AgentSpec;
use crate::daemon::persistence::config;

/// File-name suffix for ability manifests inside
/// `<agent-root>/abilities/`. Pinning the suffix in a constant makes
/// the enumeration logic in `list_ability_manifests` the single place
/// that knows the convention — everything else reads it from here.
pub const ABILITY_MANIFEST_SUFFIX: &str = ".ability.toml";

/// Whether `path` is a directory that has no entries. Used by
/// `AgentDirectory::create` to distinguish "operator pre-mkdir'd
/// an empty root" (acceptable) from "previous create failed and
/// left a skeleton behind" (must refuse). The check ignores
/// whether `path` is a symlink or a regular dir; it only answers
/// "can I safely populate this as if it were new".
fn is_empty_dir(path: &Path) -> anyhow::Result<bool> {
    let mut iter = fs::read_dir(path)?;
    Ok(iter.next().is_none())
}

/// Where an agent root lives in the filesystem — either globally
/// under `$EASYNET_HOME/agents/<name>/` or project-local at an
/// arbitrary absolute path the operator chose.
///
/// Semantics
/// ---------
/// * `Global { name }`  — resolved via `config::agents_root()`.
///   The single tree the registry defaults to; the natural choice
///   for "one user, many agents, same machine, no project
///   affinity." Resolution folds into the same `agents_root()`
///   helper that PR-0b added for reads-with-fallback, so a user
///   who has only the legacy `workspaces/` tree keeps working
///   until that deprecation window closes.
/// * `Local { root }`   — any absolute path. The typical shape is
///   `<repo>/my-agent/` — an agent that ships inside a code
///   project and lives or dies with it. Registry stores the
///   absolute path; a moved or deleted Local agent shows up in
///   `agent list` with a "path missing" badge (PR-3b.2).
///
/// We deliberately reject relative paths at construction: the
/// registry stores raw paths, and a `./foo`-relative record would
/// resolve differently depending on the `pwd` of whoever runs the
/// CLI next. Callers canonicalize once, here, and the error is
/// visible to the operator immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    #[cfg(test)]
    Global {
        name: String,
    },
    Local {
        root: PathBuf,
    },
}

impl Location {
    /// Resolve to the absolute path the directory lives at. For
    /// `Global`, this joins the agent name onto the
    /// `agents_root()` — which itself honors the
    /// new-or-legacy fallback introduced in PR-0b. For `Local`,
    /// we return the caller's already-absolute path verbatim.
    pub fn resolve(&self) -> PathBuf {
        match self {
            #[cfg(test)]
            Self::Global { name } => config::agents_root().join(name),
            Self::Local { root } => root.clone(),
        }
    }
}

/// One agent's on-disk root. Holding an `AgentDirectory` asserts
/// that the `agent.toml` inside has been loaded and validated; it
/// does NOT assert that every sibling directory (`abilities/`,
/// `skills/`, `memory/`, `runs/`) exists. The construction paths
/// create the skeletal subdirs up front, but an operator who
/// removes one between loads will see the directory come back on
/// the next `ensure_layout` call, not panic here.
#[derive(Debug, Clone)]
pub struct AgentDirectory {
    root: PathBuf,
    spec: AgentSpec,
}

impl AgentDirectory {
    /// Open an existing agent directory. The path must carry an
    /// `agent.toml`; absence is a hard error (not a degrade).
    ///
    /// We do NOT auto-create missing subdirectories here — a
    /// caller who wants the skeleton materialised should invoke
    /// `ensure_layout` explicitly. Separating the two keeps
    /// `open` a pure read: it can be called from diagnostic
    /// commands (`agent show`, `agent doctor`) without implicit
    /// side effects on disk.
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = path.as_ref().to_path_buf();
        let spec_path = root.join("agent.toml");
        let toml = fs::read_to_string(&spec_path).map_err(|e| {
            anyhow::anyhow!(
                "agent directory at {} is missing agent.toml: {e}",
                root.display()
            )
        })?;
        let spec = AgentSpec::from_toml_str(&toml)?;
        Ok(Self { root, spec })
    }

    /// Create a brand-new agent directory at `location` with
    /// `spec` as its initial `agent.toml`. Returns the freshly
    /// materialised `AgentDirectory`.
    ///
    /// Behaviour
    /// ---------
    /// * Root is created recursively; a pre-existing empty root
    ///   is tolerated (operators often `mkdir my-agent` before
    ///   `agent new ./my-agent`).
    /// * The four spec-adjacent subdirectories
    ///   (`abilities/`, `skills/`, `memory/`, `runs/`) are
    ///   created.
    /// * `.env` is created with `create_new(true)` and unix mode
    ///   0600. If an existing `.env` is already present we treat
    ///   that as a partial-failure signal (see below).
    /// * `agent.toml` is written LAST via
    ///   `persistence::config::atomic_write` so a concurrent
    ///   reader never observes a torn file. Writing it last also
    ///   means a failure *before* this step leaves a
    ///   distinguishable partial state that the next invocation
    ///   can detect and refuse.
    ///
    /// Refusal cases
    /// -------------
    /// * Root already contains an `agent.toml` — refuse, to
    ///   prevent `agent new` from silently clobbering an
    ///   existing agent.
    /// * Root exists, has no `agent.toml`, but has at least one
    ///   of the spec-adjacent subdirs (`abilities/`, `skills/`,
    ///   `memory/`, `runs/`) or a `.env` file — refuse: this is
    ///   the shape left by a previous `create` that failed
    ///   *after* mkdir but *before* agent.toml was written.
    ///   Silently reusing it would let a retry with a *different*
    ///   spec succeed on top of whatever skeleton the operator
    ///   thought they had abandoned. That class of bug is
    ///   invisible in happy-path testing and catastrophic when
    ///   it fires.
    /// * `Location::Local { root }` whose `root` is not
    ///   absolute — bail; see the `Location` doc for why.
    pub fn create(location: &Location, spec: AgentSpec) -> anyhow::Result<Self> {
        // Pre-flight. Validate the spec first so a syntactically
        // bad request never touches the filesystem.
        spec.validate()?;

        #[cfg(test)]
        if let Location::Local { root } = location {
            if !root.is_absolute() {
                anyhow::bail!(
                    "Location::Local requires an absolute path, got {}",
                    root.display()
                );
            }
        }
        #[cfg(not(test))]
        let Location::Local { root } = location;
        #[cfg(not(test))]
        if !root.is_absolute() {
            anyhow::bail!(
                "Location::Local requires an absolute path, got {}",
                root.display()
            );
        }

        let root = location.resolve();
        let spec_path = root.join("agent.toml");

        // ── Partial-failure detection ──
        //
        // We check this *before* mkdir. If the root already
        // exists we inspect it for the "previous create half
        // finished" shape: missing agent.toml but non-empty.
        // Three acceptable states:
        //   1. root doesn't exist — normal first-time create
        //   2. root exists and is empty — operator pre-mkdir'd
        //   3. root exists and has agent.toml — return
        //      `already exists` (the existing guard)
        //
        // Anything else is the partial-skeleton shape and we
        // refuse: the operator (or a retry script) must make an
        // explicit choice between "rm -rf and retry" or "open
        // the existing agent."
        if root.exists() {
            if spec_path.exists() {
                anyhow::bail!(
                    "agent.toml already exists at {}; refusing to overwrite. \
                     Use `agent show` to inspect or remove the directory first.",
                    spec_path.display()
                );
            }
            if !is_empty_dir(&root)? {
                anyhow::bail!(
                    "directory {} exists, is not empty, and has no agent.toml. \
                     This shape is the signature of a previous `create` that \
                     failed mid-way (e.g. out-of-disk before agent.toml could be \
                     written). Refusing to proceed to avoid silently adopting the \
                     half-finished skeleton under a different spec. \
                     Remove the directory manually (`rm -rf {}`) and retry.",
                    root.display(),
                    root.display()
                );
            }
        }

        fs::create_dir_all(&root)?;

        // Materialise the skeleton. Each subdir is independent;
        // we propagate the first failure but do not undo earlier
        // steps. A partially-populated root is not a silent
        // success: the next invocation's partial-skeleton check
        // above will trip, forcing the operator to clean up
        // explicitly. The alternative (attempt to undo) leaves
        // its own set of races — a concurrent reader might
        // observe the undo mid-flight and take the dir to be in
        // yet another ambiguous state.
        for child in ["abilities", "skills", "memory", "runs"] {
            fs::create_dir_all(root.join(child))?;
        }

        // Create an empty `.env` with owner-only permissions on
        // unix. `create_new(true)` is load-bearing: if a stale
        // `.env` from a previous half-finished run were present
        // the partial-skeleton check above would already have
        // refused, but using `create_new` is a second barrier
        // that would catch even a post-check race and a belt-
        // and-braces guard against accidentally adopting a weak-
        // permission .env from an earlier user.
        let env_path = root.join(".env");
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&env_path)?;
        }
        #[cfg(not(unix))]
        {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&env_path)?;
        }

        // Seed the default `chat` manifest before agent.toml is
        // written. Order matters for the same reason the subdirs
        // are materialised before the spec: a failure here leaves
        // the partial-skeleton shape that the top-of-function
        // check will refuse to adopt on retry. A successful
        // create therefore guarantees both agent.toml AND at
        // least one ability manifest are on disk — consumers of
        // `list_ability_manifests` can count on the directory
        // never being `{ agent.toml, abilities/=empty }`.
        let chat_manifest = default_chat_manifest();
        let chat_path = root
            .join("abilities")
            .join(format!("chat{ABILITY_MANIFEST_SUFFIX}"));
        config::atomic_write(&chat_path, chat_manifest.to_toml_string()?.as_bytes())?;

        // Write the spec last so a failure anywhere above leaves
        // the directory in the partial-skeleton state the check
        // at the top of this function detects. That cycle is
        // self-healing: an operator whose first attempt ran out
        // of disk gets a clear "half-finished skeleton; rm -rf
        // and retry" error on their second attempt rather than
        // a silent overwrite.
        let toml = spec.to_toml_string()?;
        config::atomic_write(&spec_path, toml.as_bytes())?;

        Ok(Self { root, spec })
    }

    /// Idempotently ensure every subdirectory this module
    /// promises exists. Useful after an `open()` on a
    /// directory that might be missing a subdir (e.g. the
    /// operator deleted `runs/` to reclaim disk), and the
    /// load-path before a write.
    ///
    /// Does NOT touch `agent.toml` — if the spec is absent,
    /// `open` would have failed earlier.
    pub fn ensure_layout(&self) -> anyhow::Result<()> {
        for child in ["abilities", "skills", "memory", "runs"] {
            fs::create_dir_all(self.root.join(child))?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn spec(&self) -> &AgentSpec {
        &self.spec
    }

    pub fn abilities_dir(&self) -> PathBuf {
        self.root.join("abilities")
    }

    #[cfg(test)]
    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    #[cfg(test)]
    pub fn memory_dir(&self) -> PathBuf {
        self.root.join("memory")
    }

    #[cfg(test)]
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    #[cfg(test)]
    pub fn env_path(&self) -> PathBuf {
        self.root.join(".env")
    }

    #[cfg(test)]
    pub fn mcp_servers_path(&self) -> PathBuf {
        self.root.join("mcp_servers.json")
    }

    /// Rewrite `agent.toml` from the in-memory spec. Atomic via
    /// the persistence layer's write helper. Callers who have
    /// mutated `self.spec` in memory invoke this to persist the
    /// change; we do not auto-persist on every field mutation
    /// because batched edits need to be one atomic write.
    pub fn save_spec(&self) -> anyhow::Result<()> {
        let toml = self.spec.to_toml_string()?;
        config::atomic_write(&self.root.join("agent.toml"), toml.as_bytes())?;
        Ok(())
    }

    /// Update the spec's `model` field and persist. The chokepoint
    /// for `easynet agent set --model …`. Keeping this as a
    /// dedicated method (rather than exposing `spec_mut()`) lets a
    /// future per-runtime model validator hook here without
    /// touching every call site.
    ///
    /// `model = None` clears the field, falling the agent back to
    /// the underlying CLI's own default model. That symmetry with
    /// `agent add` (where `--model` is optional and absence means
    /// "let the CLI pick") is the load-bearing reason we accept
    /// `Option` rather than `&str`.
    pub fn set_model(&mut self, model: Option<String>) -> anyhow::Result<()> {
        self.spec.model = model;
        self.save_spec()
    }

    /// Enumerate every `*.ability.toml` under `abilities/`, parsed
    /// and validated. Returned in sorted order by on-disk file name
    /// so `agent abilities` and `agent publish --dry-run` print
    /// stable output across invocations.
    ///
    /// Behavior on malformed manifests
    /// --------------------------------
    /// We deliberately do NOT silently skip a broken file. The
    /// whole point of having `abilities/` as the source of truth
    /// is that a manifest on disk is either load-bearing (it gets
    /// published) or removed. A half-loaded list — "we published 3
    /// of 4 abilities, ignored the malformed one" — is exactly the
    /// class of subtle drift between discovery and dispatch this
    /// module is designed to prevent. The first parse error bails
    /// with the offending path; the operator fixes it, or deletes
    /// the file.
    ///
    /// Non-manifest files in the directory (README.md, .DS_Store,
    /// …) are ignored because they do not match
    /// `ABILITY_MANIFEST_SUFFIX`. A subdirectory is also ignored
    /// — the layout is flat by design; nesting would require
    /// disambiguation we do not want.
    pub fn list_ability_manifests(&self) -> anyhow::Result<Vec<AbilityManifest>> {
        let dir = self.abilities_dir();
        if !dir.exists() {
            // The happy path creates the subdir, but an operator
            // who `rm -rf`'d it should get an empty list rather
            // than an IO error — the canonical "nothing declared"
            // shape.
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .map_err(|e| {
                anyhow::anyhow!("failed to read abilities directory {}: {e}", dir.display())
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("failed to enumerate {}: {e}", dir.display()))?;
        // Sorted by file name for stable output.
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let fname = match path.file_name().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };
            if !fname.ends_with(ABILITY_MANIFEST_SUFFIX) {
                continue;
            }
            let body = fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
            let manifest = AbilityManifest::from_toml_str(&body)
                .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))?;
            // The file-name stem (before `.ability.toml`) is the
            // operator's authoritative verb name — a manifest whose
            // internal `name` disagrees with its file stem would
            // cause "agent publish" to print a different tool name
            // than "ls abilities/" suggests. Refuse the divergence
            // early. The check is cheap and catches copy-paste
            // errors.
            let stem = &fname[..fname.len() - ABILITY_MANIFEST_SUFFIX.len()];
            if stem != manifest.name() {
                anyhow::bail!(
                    "ability manifest {} declares name = {:?} but its filename stem is {:?}; \
                     rename the file to `{}{ABILITY_MANIFEST_SUFFIX}` or update the \
                     `name` field to match",
                    path.display(),
                    manifest.name(),
                    stem,
                    manifest.name(),
                );
            }
            out.push(manifest);
        }
        Ok(out)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Tests pin the layout invariants: every subdir the module
    //! doc promises must be present after `create`, absent after
    //! the cleanup window, chmod-correct on unix, and round-trip
    //! through `open` without losing fields.

    use super::*;
    use crate::core::agent::spec::{AgentSpec, RuntimeKind};

    /// Build a throwaway Location::Local at a unique temp path.
    /// We use `mktemp`-style naming so concurrent tests never
    /// collide; fixture cleanup is best-effort (the OS reaps the
    /// OS temp dir on reboot if a test is killed).
    fn local_at_tmp(tag: &str) -> (Location, PathBuf) {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("easynet-dirtest-{tag}-{pid}-{nanos}"));
        (Location::Local { root: root.clone() }, root)
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    // ── happy path ──────────────────────────────────────────────────────────

    #[test]
    fn create_produces_full_skeleton_and_spec() {
        let (loc, root) = local_at_tmp("happy");
        let spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        let dir = AgentDirectory::create(&loc, spec.clone()).expect("create must succeed");
        // agent.toml present and parseable.
        let toml_on_disk = fs::read_to_string(root.join("agent.toml")).unwrap();
        let loaded = AgentSpec::from_toml_str(&toml_on_disk).unwrap();
        assert_eq!(loaded, spec);
        // Every promised subdir exists.
        for child in ["abilities", "skills", "memory", "runs"] {
            assert!(root.join(child).is_dir(), "{child}/ must be present");
        }
        // .env present.
        assert!(root.join(".env").is_file());
        // Default `chat.ability.toml` was seeded — no fresh agent
        // should show up with an empty abilities list.
        assert!(
            root.join("abilities")
                .join(format!("chat{ABILITY_MANIFEST_SUFFIX}"))
                .is_file(),
            "chat.ability.toml must be seeded on create"
        );
        // Accessors return the paths we expect.
        assert_eq!(dir.abilities_dir(), root.join("abilities"));
        assert_eq!(dir.skills_dir(), root.join("skills"));
        assert_eq!(dir.memory_dir(), root.join("memory"));
        assert_eq!(dir.runs_dir(), root.join("runs"));
        assert_eq!(dir.env_path(), root.join(".env"));
        assert_eq!(dir.mcp_servers_path(), root.join("mcp_servers.json"));
        assert_eq!(dir.root(), root);
        assert_eq!(dir.spec(), &spec);
        cleanup(&root);
    }

    #[test]
    fn create_seeds_default_chat_manifest_that_parses_back() {
        // The seeded manifest is not just present — it round-
        // trips through the parser. A malformed default would
        // fail every `agent publish` on a fresh agent.
        let (loc, root) = local_at_tmp("default-chat");
        let dir =
            AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode)).unwrap();
        let manifests = dir.list_ability_manifests().unwrap();
        assert_eq!(manifests.len(), 1, "exactly one seeded ability");
        assert_eq!(manifests[0].name(), "chat");
        assert_eq!(manifests[0].qualified_name("alice"), "alice.chat");
        cleanup(&root);
    }

    #[test]
    fn list_ability_manifests_returns_empty_when_directory_absent() {
        // An operator who `rm -rf abilities/` gets a clean empty
        // list, not an IO error. Tools can branch on the empty
        // list; an IO error would require special-casing.
        let (loc, root) = local_at_tmp("no-abilities");
        let dir =
            AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode)).unwrap();
        fs::remove_dir_all(root.join("abilities")).unwrap();
        let manifests = dir.list_ability_manifests().unwrap();
        assert!(manifests.is_empty());
        cleanup(&root);
    }

    #[test]
    fn list_ability_manifests_sorts_by_file_name_for_stable_output() {
        // Publish dry-run prints this list; order stability is
        // load-bearing for diff-friendly output (a CI pipeline
        // that saves the stdout and diffs it on the next run).
        let (loc, root) = local_at_tmp("sorted");
        let dir =
            AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode)).unwrap();
        // Seed abilities in reverse-name order to confirm the
        // sorter actually runs.
        for name in ["zulu", "alpha"] {
            let m = AbilityManifest::new(name, "x", serde_json::json!({"type": "object"})).unwrap();
            fs::write(
                root.join("abilities")
                    .join(format!("{name}{ABILITY_MANIFEST_SUFFIX}")),
                m.to_toml_string().unwrap(),
            )
            .unwrap();
        }
        let names: Vec<_> = dir
            .list_ability_manifests()
            .unwrap()
            .into_iter()
            .map(|m| m.name().to_string())
            .collect();
        assert_eq!(names, vec!["alpha", "chat", "zulu"]);
        cleanup(&root);
    }

    #[test]
    fn list_ability_manifests_ignores_non_ability_files_and_subdirs() {
        // README.md, a nested subdir, a stale editor-swap file —
        // all expected-on-disk but not manifests. They must not
        // trip the parser.
        let (loc, root) = local_at_tmp("noise");
        let dir =
            AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode)).unwrap();
        let abilities = root.join("abilities");
        fs::write(abilities.join("README.md"), "# notes").unwrap();
        fs::write(abilities.join(".DS_Store"), "").unwrap();
        fs::create_dir_all(abilities.join("nested")).unwrap();
        let manifests = dir.list_ability_manifests().unwrap();
        assert_eq!(manifests.len(), 1, "only the default chat survives");
        assert_eq!(manifests[0].name(), "chat");
        cleanup(&root);
    }

    #[test]
    fn list_ability_manifests_rejects_malformed_manifest() {
        // Silent-skip would create drift between "what ls shows"
        // and "what publish registers". Instead, refuse loud.
        let (loc, root) = local_at_tmp("broken");
        let dir =
            AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode)).unwrap();
        fs::write(
            root.join("abilities")
                .join(format!("broken{ABILITY_MANIFEST_SUFFIX}")),
            "not = valid = toml",
        )
        .unwrap();
        let err = dir
            .list_ability_manifests()
            .expect_err("malformed manifest must surface");
        assert!(format!("{err}").contains("broken"));
        cleanup(&root);
    }

    #[test]
    fn list_ability_manifests_rejects_filename_name_mismatch() {
        // Copy-paste gotcha: `cp chat.ability.toml voice.ability.toml`
        // and forget to update the `name` field. If we accepted it,
        // `agent publish` would print `alice.chat` twice with
        // different descriptions. Refuse.
        let (loc, root) = local_at_tmp("mismatch");
        let dir =
            AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode)).unwrap();
        let m = default_chat_manifest();
        fs::write(
            root.join("abilities")
                .join(format!("voice{ABILITY_MANIFEST_SUFFIX}")),
            m.to_toml_string().unwrap(),
        )
        .unwrap();
        let err = dir
            .list_ability_manifests()
            .expect_err("filename/name mismatch must surface");
        let msg = format!("{err}");
        assert!(
            msg.contains("voice") && msg.contains("chat"),
            "error must name both sides; got {msg}"
        );
        cleanup(&root);
    }

    #[test]
    fn open_round_trips_spec_from_disk() {
        // Create, forget, re-open. Loaded spec must equal the
        // one written.
        let (loc, root) = local_at_tmp("roundtrip");
        let mut spec = AgentSpec::new("bob", RuntimeKind::Codex);
        spec.description = Some("nightly audit".into());
        spec.timeout_secs = Some(60);
        let _ = AgentDirectory::create(&loc, spec.clone()).unwrap();
        let re = AgentDirectory::open(&root).unwrap();
        assert_eq!(re.spec(), &spec);
        cleanup(&root);
    }

    #[test]
    fn save_spec_persists_in_memory_mutation() {
        // Mutate in memory, save, re-open, observe the change.
        let (loc, root) = local_at_tmp("save");
        let spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        let mut dir = AgentDirectory::create(&loc, spec).unwrap();
        // Mutate via a helper — we expose raw spec as immutable;
        // operators do this today by writing to agent.toml
        // directly. The API for programmatic mutation is
        // deliberately narrow (open + edit + save_spec), not a
        // swarm of typed setters.
        let mut s = dir.spec().clone();
        s.description = Some("now with prose".into());
        dir.spec = s.clone();
        dir.save_spec().unwrap();
        let re = AgentDirectory::open(&root).unwrap();
        assert_eq!(re.spec().description.as_deref(), Some("now with prose"));
        cleanup(&root);
    }

    #[test]
    fn ensure_layout_recreates_missing_subdirs() {
        // Operator deletes `runs/` to reclaim disk — the next
        // ensure_layout must bring it back without forcing a
        // second `create` call (which would refuse because
        // agent.toml already exists).
        let (loc, root) = local_at_tmp("ensure");
        let dir =
            AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode)).unwrap();
        fs::remove_dir_all(root.join("runs")).unwrap();
        assert!(!root.join("runs").exists());
        dir.ensure_layout().unwrap();
        assert!(root.join("runs").is_dir());
        cleanup(&root);
    }

    // ── failure path ────────────────────────────────────────────────────────

    #[test]
    fn create_refuses_existing_agent_toml() {
        // Classic "user ran `agent new` on the wrong dir" guard.
        let (loc, root) = local_at_tmp("conflict");
        AgentDirectory::create(&loc, AgentSpec::new("a", RuntimeKind::ClaudeCode)).unwrap();
        let err = AgentDirectory::create(&loc, AgentSpec::new("a", RuntimeKind::ClaudeCode))
            .expect_err("second create must refuse");
        assert!(format!("{err}").contains("already exists"));
        cleanup(&root);
    }

    #[test]
    fn create_rejects_relative_local_path() {
        // A relative `Local { root }` would be resolved against
        // whatever pwd the next CLI invocation runs in; the
        // registry must only store paths that are self-contained.
        let loc = Location::Local {
            root: PathBuf::from("./agents/alice"),
        };
        let err = AgentDirectory::create(&loc, AgentSpec::new("a", RuntimeKind::ClaudeCode))
            .expect_err("relative Local path must error");
        assert!(format!("{err}").to_lowercase().contains("absolute"));
    }

    #[test]
    fn open_fails_on_missing_agent_toml() {
        let (_loc, root) = local_at_tmp("missing");
        fs::create_dir_all(&root).unwrap();
        // We deliberately do NOT put agent.toml in place.
        let err = AgentDirectory::open(&root).expect_err("missing agent.toml must error");
        assert!(format!("{err}").contains("agent.toml"));
        cleanup(&root);
    }

    #[test]
    fn open_fails_on_invalid_agent_toml() {
        // An agent.toml present but malformed must be rejected
        // at the read boundary, not silently loaded with a
        // partial spec.
        let (_loc, root) = local_at_tmp("invalid");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("agent.toml"), "not = a = valid = toml").unwrap();
        let err = AgentDirectory::open(&root).expect_err("invalid toml must error");
        assert!(
            !format!("{err}").is_empty(),
            "error string must be non-empty"
        );
        cleanup(&root);
    }

    #[test]
    fn create_rejects_invalid_spec_before_touching_disk() {
        // Pre-flight validation: a spec with a bad name must be
        // rejected before any mkdir runs, so there is no
        // orphaned directory to clean up.
        let (loc, root) = local_at_tmp("badspec");
        let mut spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        spec.name = "has/slash".into();
        let err = AgentDirectory::create(&loc, spec).expect_err("bad name must error");
        assert!(format!("{err}").contains("/"));
        // No directory created.
        assert!(
            !root.exists(),
            "create must not leave a root behind on pre-flight failure"
        );
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn env_file_has_owner_only_permissions_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let (loc, root) = local_at_tmp("envperm");
        AgentDirectory::create(&loc, AgentSpec::new("a", RuntimeKind::ClaudeCode)).unwrap();
        let meta = fs::metadata(root.join(".env")).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            ".env must be 0o600 immediately after create, got {mode:o}"
        );
        cleanup(&root);
    }

    #[test]
    fn create_tolerates_existing_empty_root_dir() {
        // An operator might `mkdir my-agent` before running
        // `agent new ./my-agent`. That should be tolerated — we
        // only refuse when an agent.toml already lives inside or
        // when the dir is non-empty (previous half-failure).
        let (loc, root) = local_at_tmp("preexistdir");
        fs::create_dir_all(&root).unwrap();
        AgentDirectory::create(&loc, AgentSpec::new("a", RuntimeKind::ClaudeCode))
            .expect("empty pre-existing dir must be accepted");
        cleanup(&root);
    }

    #[test]
    fn create_refuses_partial_skeleton_without_spec() {
        // A previous `create` that mkdir'd subdirs but died before
        // writing agent.toml must not be silently adopted by a
        // second attempt. This is the critical invariant that
        // prevents "second run wins with a different spec" — the
        // concrete bad outcome being a user who ran `agent new`
        // twice for the same name with different runtimes and
        // had the second run quietly succeed on top of the first
        // run's skeleton.
        let (loc, root) = local_at_tmp("partial");
        fs::create_dir_all(&root).unwrap();
        // Seed the shape of a failed prior create: a couple of
        // the four skeleton dirs present, but no agent.toml.
        fs::create_dir_all(root.join("abilities")).unwrap();
        fs::create_dir_all(root.join("runs")).unwrap();
        // A leftover .env of any permissions is also part of the
        // half-finished shape — the invariant we're testing is
        // "non-empty + no agent.toml → refuse", so we include it.
        fs::write(root.join(".env"), "").unwrap();

        let err = AgentDirectory::create(&loc, AgentSpec::new("claude", RuntimeKind::ClaudeCode))
            .expect_err("partial skeleton must be refused");
        let msg = format!("{err}");
        // Error must mention both the offending path and the
        // remediation so an operator can fix the state without
        // reading the source.
        assert!(
            msg.contains("half-finished") || msg.contains("previous"),
            "error must name the half-finished shape; got {msg}"
        );
        assert!(
            msg.contains("rm -rf") || msg.contains("remove"),
            "error must point at remediation; got {msg}"
        );
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn create_refuses_even_if_partial_skeleton_has_only_weak_env() {
        // Subtle variant of the partial-skeleton case: only a
        // weak-permission `.env` present, no subdirs. A naive
        // check that only looked at the four subdirs would miss
        // this and adopt an attacker-writable .env as this
        // agent's credential store. The non-empty check catches
        // it.
        use std::os::unix::fs::PermissionsExt;
        let (loc, root) = local_at_tmp("partial-env");
        fs::create_dir_all(&root).unwrap();
        let env_path = root.join(".env");
        fs::write(&env_path, "STOLEN_API_KEY=xyz").unwrap();
        // Mode 0o644 — group/other readable, the shape a hostile
        // process might have deposited here.
        fs::set_permissions(&env_path, fs::Permissions::from_mode(0o644)).unwrap();

        let err = AgentDirectory::create(&loc, AgentSpec::new("alice", RuntimeKind::ClaudeCode))
            .expect_err("pre-existing weak .env must cause refusal");
        assert!(
            format!("{err}").contains("half-finished") || format!("{err}").contains("previous")
        );
        cleanup(&root);
    }

    #[test]
    fn create_refuses_timeout_zero_in_spec() {
        // Validation is run pre-flight by `create`, so the
        // AgentSpec timeout-zero rejection must propagate here.
        // If a future refactor split out validation, this test
        // would catch the gap.
        let (loc, root) = local_at_tmp("timeout0");
        let mut spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        spec.timeout_secs = Some(0);
        let err = AgentDirectory::create(&loc, spec)
            .expect_err("timeout_secs=0 must be refused pre-flight");
        assert!(format!("{err}").contains("timeout"));
        assert!(
            !root.exists(),
            "no directory must be created on pre-flight failure"
        );
    }

    #[test]
    fn location_global_resolves_through_agents_root() {
        // `Global { name }` must fold through `config::agents_root()`
        // so the PR-0b fallback is honored. We verify by
        // constructing a Location and checking the resolved
        // path has the expected shape; the actual agents_root()
        // behavior is unit-tested in persistence::config.
        let g = Location::Global {
            name: "probe".into(),
        };
        let resolved = g.resolve();
        // Must end with `agents/probe` or `workspaces/probe`
        // (depending on which one exists on the dev machine).
        let last_two: Vec<_> = resolved
            .iter()
            .rev()
            .take(2)
            .map(|c| c.to_string_lossy().into_owned())
            .collect();
        assert_eq!(last_two[0], "probe");
        assert!(
            last_two[1] == "agents" || last_two[1] == "workspaces",
            "expected agents/probe or workspaces/probe, got {}",
            resolved.display()
        );
    }

    #[test]
    fn open_after_create_yields_same_spec_reference_value() {
        // The returned AgentDirectory from `create` must carry
        // exactly the spec it wrote to disk. A future refactor
        // that read the spec back from disk after write (to
        // normalize formatting, say) must still preserve
        // equality — otherwise callers would see a subtle
        // pre/post-write divergence.
        let (loc, root) = local_at_tmp("dirref");
        let spec = AgentSpec::new("alice", RuntimeKind::Codex);
        let dir = AgentDirectory::create(&loc, spec.clone()).unwrap();
        assert_eq!(dir.spec(), &spec);
        cleanup(&root);
    }
}
