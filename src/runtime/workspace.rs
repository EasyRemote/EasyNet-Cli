// EasyNet CLI — Agent Workspace Projection
// =========================================
//
// File: src/runtime/workspace.rs
// Description: Projects an `AgentDirectory` onto the on-disk layout
//              each runtime binary expects: `.mcp.json` for Claude
//              Code, `.codex/config.toml` + `.agents/skills/` for
//              Codex, and the shared `CLAUDE.md` / `AGENTS.md`
//              knowledge docs. This module materialises those
//              *derived* files; the *source* (agent.toml, per-agent
//              abilities and skills) lives under `AgentDirectory`
//              and is owned by `runtime::directory`.
//
// Why "projection"
// ----------------
// Before PR-3b.3 this file was a creator: `ensure_workspace` built
// the whole tree from the fat `AgentEntry`. That put policy ("which
// fields of an agent become which files on disk?") in two places —
// here and inside AgentDirectory — with nothing pinning the two to
// one truth. The reversal keeps the agent root a pure source of
// truth (agent.toml + abilities/ + skills/ + memory/ + runs/ +
// mcp_servers.json + .env) and makes this module derive the
// runtime-native files from that source on every invocation. A
// caller that has mutated `agent.toml` and wants a downstream
// runtime to see the change re-runs the projection; no state lives
// only in the derived files.
//
// Entry points
// ------------
// * `ensure_from_directory(dir)` — new primary entry. Takes an
//   `AgentDirectory` and writes the derived files into it.
// * `ensure_workspace(name, entry)` — backcompat shim for callers
//   that still hand over an `AgentEntry`. Resolves to an
//   `AgentDirectory` (preferring `entry.root_path`, falling back to
//   `config::agents_root().join(name)` per the consumer-side
//   fallback rule) and delegates to `ensure_from_directory`.
//
// Why keep the shim
// -----------------
// PR-3b.3 constrains its blast radius to this file and the
// dispatcher call site would otherwise be scope creep. Consumers
// (`runtime::dispatch::send_to_agent_with_depth`, tests) continue
// to hand over an `AgentEntry`; PR-3b.5 migrates them to hand over
// an `AgentDirectory` directly and the shim goes away.
//
// What this module owns / does NOT own
// ------------------------------------
// * Owns: writing `CLAUDE.md`, `AGENTS.md`, `.mcp.json`,
//   `.codex/config.toml`, `.agents/skills/*.md`, and the `git init`
//   step Codex requires.
// * Does NOT own: agent root creation (that's
//   `AgentDirectory::create`), agent.toml mutation, abilities
//   discovery, skill content.
//
// Claude Code discovers MCP servers from `.mcp.json` in project
// root and knowledge from `CLAUDE.md`. `-p` mode respects both.
//
// Codex discovers skills from `.agents/skills/` and config from
// `.codex/`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::{Path, PathBuf};

use super::toml_escape::toml_basic_string;
use crate::core::agent_spec::RuntimeKind;
use crate::persistence::config;
use crate::registry::agents::{AgentEntry, AgentType};
use crate::runtime::directory::AgentDirectory;

/// Project an `AgentDirectory` onto the runtime-native layout
/// the installed agent binary expects. Returns the agent root
/// path so callers (today, `dispatch::send_to_agent_with_depth`)
/// can use it as the subprocess `cwd`.
///
/// Idempotent. Safe to call on every dispatch — each derived
/// file is rewritten atomically so concurrent invocations never
/// observe a torn file, and the `git init` step is guarded on
/// absence of `.git/`.
///
/// Behaviour per runtime (branching on `AgentSpec::runtime`):
/// * `ClaudeCode`: writes `.mcp.json` + `CLAUDE.md` + `AGENTS.md`,
///   seeds `.claude/skills/easynet-collaborate/SKILL.md`.
/// * `Codex` / `CodexAppServer`: additionally writes
///   `.codex/config.toml` + `.agents/skills/easynet-collaborate/*`
///   and runs `git init` if `.git/` is absent.
pub fn ensure_from_directory(dir: &AgentDirectory) -> anyhow::Result<PathBuf> {
    // Guarantee the four spec-adjacent subdirs exist. Covers the
    // case where an operator deleted `runs/` to reclaim disk
    // between dispatches — `AgentDirectory::ensure_layout` is
    // idempotent.
    dir.ensure_layout()?;

    let root = dir.root().to_path_buf();
    let runtime = dir.spec().runtime;
    let agent_name = dir.spec().name.clone();
    let model = dir.spec().model.clone();

    // Codex requires a git repo at the cwd it runs in.
    // ClaudeCode doesn't need one, but creating it is harmless
    // and keeps the layout uniform across runtimes.
    //
    // A failed `git init` is non-fatal here — a ClaudeCode
    // dispatch happily runs without `.git/`, and even for
    // Codex the subsequent dispatch will fail with its own
    // diagnostic if the missing repo matters. We nevertheless
    // surface the failure on stderr so an operator tracing a
    // Codex "git repo required" error can correlate it back to
    // the workspace-projection step rather than having to
    // guess. A fully silent `let _ = ...output()` would force
    // the operator to instrument this line by hand.
    if !root.join(".git").exists() {
        match std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .output()
        {
            Ok(out) if !out.status.success() => {
                eprintln!(
                    "[easynet warn] git init at {} exited {}: {}",
                    root.display(),
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Err(e) => {
                eprintln!(
                    "[easynet warn] git init at {} failed to spawn: {e}",
                    root.display()
                );
            }
            Ok(_) => {}
        }
    }

    // Knowledge doc (shared). Written atomically so a concurrent
    // dispatch does not observe a torn file when the doc grows
    // or shrinks across releases.
    let knowledge = generate_knowledge_doc();
    config::atomic_write(&root.join("CLAUDE.md"), knowledge.as_bytes())?;
    config::atomic_write(&root.join("AGENTS.md"), knowledge.as_bytes())?;

    // `.mcp.json` — project-level MCP discovery for Claude Code
    // in `-p` mode.
    write_mcp_json(&root, &agent_name)?;

    match runtime {
        RuntimeKind::ClaudeCode => {} // `.mcp.json` + `CLAUDE.md` is enough.
        RuntimeKind::Codex | RuntimeKind::CodexAppServer => {
            write_codex_config(&root, model.as_deref(), &agent_name)?;
        }
    }

    // Seed the `easynet-collaborate` skill so a freshly-installed
    // agent knows how to discover and invoke abilities on the device /
    // federation before falling back to "I can't do that". Single
    // seed, single source of truth (`skills/easynet-collaborate/`
    // in this repo via include_str!).
    //
    // Two skills used to be seeded here (`easynet-ability-crud` for
    // authoring + `delegate` for using). They were merged: ability
    // authoring became `easynet-author` (user-installed, not seeded —
    // it's an event-driven need, not every-call muscle memory), and
    // `delegate` was renamed `easynet-collaborate` so the EasyNet
    // family has a uniform `easynet-` prefix and an action-shaped
    // name.
    //
    // Path is runtime-aware: claude-code reads `<root>/.claude/skills/`
    // (Anthropic project-local skill convention — load-bearing),
    // codex reads `<root>/.agents/skills/`. Earlier code wrote to
    // `<root>/skills/` for both, which Claude Code's loader did not
    // scan — the seed existed but never activated. Same path-routing
    // pattern as `skill.publish` in `runtime::agents::skill_publish_ability`.
    write_collaborate_seed(&root, runtime)?;

    // RFC-006-B v0.6 — seed the `easynet-pages-author` skill so
    // a freshly-installed agent can ship a real website on this
    // machine without any prior briefing. The skill walks the
    // agent through the project layout, the static + TOML-API
    // surface, and the `easynet pages create` deploy. Same path-
    // routing rule as `easynet-collaborate`: claude-code reads
    // `<root>/.claude/skills/`, codex reads `<root>/.agents/skills/`.
    write_pages_author_seed(&root, runtime)?;

    // Seed the `easynet-ability-author` skill so the same agent
    // that learns to publish pages can also write the *real*
    // backend that `kind="ability"` api manifests forward to.
    // Pair with pages-author: pages teaches "frontend +
    // declarative TOML manifest", ability-author teaches "deploy
    // a real ability the manifest points at". Together they
    // close the agent-driven full-stack loop silan asked for.
    write_ability_author_seed(&root, runtime)?;

    Ok(root)
}

/// Backcompat shim: resolve an `AgentEntry` to an `AgentDirectory`
/// and delegate. The shim is the only place in this file that
/// talks to the fat `AgentEntry` surface, so PR-3b.5 can remove
/// it in one move once every caller has been migrated.
///
/// Resolution order:
/// 1. `entry.root_path` — set on v2-migrated rows by
///    `registry::agents::migrate_one_entry`.
/// 2. `config::agents_root().join(agent_name)` — the fallback
///    used when the row is still shaped as pure v2
///    (`schema_version == 2`) but `root_path` hasn't been
///    populated yet. The consumer-side fallback rule keeps
///    PR-3b.3 from having to backport a writer-side fix to
///    PR-3b.2. `agents_root()` itself folds over the
///    new-vs-legacy `agents/` / `workspaces/` fallback from
///    PR-0b.
///
/// If the resolved path does not yet carry an `agent.toml`, we
/// materialise one from the `AgentEntry` via `spec_from_entry`.
/// This is the second migration bridge — a v2 row whose on-disk
/// spec has been deleted must still be usable; we reconstruct a
/// minimal spec from the surviving registry fields and write
/// it. Without this, the first dispatch after an `rm -rf`
/// accident would fail with "agent.toml missing" even though
/// every piece of data the spec carries is still present in the
/// registry.
pub fn ensure_workspace(agent_name: &str, entry: &AgentEntry) -> anyhow::Result<PathBuf> {
    let root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(agent_name));

    // Resolve or reconstruct the directory. The reconstruction
    // path is rare (agent.toml deleted manually) but we cover
    // it because the projection otherwise silently writes into
    // a directory that `AgentDirectory::open` would refuse on
    // the next CLI read — a confusing half-state.
    let directory = if root.join("agent.toml").exists() {
        AgentDirectory::open(&root)?
    } else {
        fs::create_dir_all(&root)?;
        let spec = spec_from_entry(agent_name, entry);
        // `AgentDirectory::create` refuses a pre-existing
        // non-empty root without agent.toml (the partial-failure
        // guard from PR-3b.1.5). That guard is what we want
        // here too — if a previous crashed dispatch left
        // subdirs but no spec, refuse and let the operator
        // clean up rather than silently claiming the skeleton.
        AgentDirectory::create(
            &crate::runtime::directory::Location::Local { root: root.clone() },
            spec,
        )?
    };

    ensure_from_directory(&directory)
}

/// Reconstruct a minimal `AgentSpec` from the surviving
/// `AgentEntry` fields. Used only by the backcompat shim when
/// an agent's on-disk `agent.toml` has been deleted manually.
/// Each mapping mirrors `registry::agents::migrate_one_entry`
/// so a double-path (migration then reconstruction) yields the
/// same spec.
fn spec_from_entry(agent_name: &str, entry: &AgentEntry) -> crate::core::agent_spec::AgentSpec {
    let runtime = match entry.agent_type {
        AgentType::ClaudeCode => RuntimeKind::ClaudeCode,
        AgentType::Codex => RuntimeKind::Codex,
        AgentType::CodexAppServer => RuntimeKind::CodexAppServer,
    };
    let mut spec = crate::core::agent_spec::AgentSpec::new(agent_name, runtime);
    spec.model = entry.model.clone();
    // Match migration's "only persist non-default timeout" rule.
    // Read the current default through the registry function rather
    // than hardcoding so a default bump (5 min → 1 hour, 2026-04)
    // does not silently turn this comparison into a "always persist"
    // path.
    if entry.timeout_secs != crate::registry::agents::default_timeout_for_new_rows() {
        spec.timeout_secs = Some(entry.timeout_secs);
    }
    if let Some(label) = &entry.label {
        spec.description = Some(label.clone());
    }
    spec
}

/// Legacy path helper retained for a small number of read-side
/// callers (`facade::cli::agent::read_latest_agent_usage`). The
/// source of truth for "where does agent N live on disk?" is
/// `AgentDirectory::root()`; this helper only answers the question
/// when no `AgentEntry` is in hand.
pub fn workspace_dir(agent_name: &str) -> PathBuf {
    config::agents_root().join(agent_name)
}

// ── .mcp.json — Claude Code project-level MCP discovery ─────────────────────

fn write_mcp_json(ws: &Path, agent_name: &str) -> anyhow::Result<()> {
    let (cmd, args, env) = build_mcp_entry(agent_name);

    let mcp_json = serde_json::json!({
        "mcpServers": {
            "easynet": {
                "command": cmd,
                "args": args,
                "env": env,
            }
        }
    });

    let json = serde_json::to_string_pretty(&mcp_json)? + "\n";
    // Atomic write — concurrent dispatches for the same agent must not
    // observe a torn `.mcp.json` (Claude Code's `-p` mode reads this on
    // every spawn).
    config::atomic_write(&ws.join(".mcp.json"), json.as_bytes())?;
    Ok(())
}

// ── Codex workspace ──────────────────────────────────────────────────────────

fn write_codex_config(ws: &Path, model: Option<&str>, agent_name: &str) -> anyhow::Result<()> {
    let codex_dir = ws.join(".codex");
    fs::create_dir_all(&codex_dir)?;

    // Every value handed to the TOML parser must go through
    // `toml_basic_string` — the previous open-coded `format!("\"{s}\"")`
    // emitted invalid TOML for any value containing `\` (e.g. Windows
    // paths) or embedded quotes, silently dropping the override. The
    // same encoder is used by `runtime::codex` so the runtime form (`-c`
    // overrides) and the on-disk form (this file) stay in lock-step.
    let (cmd, args, env) = build_mcp_entry(agent_name);
    let args_toml = args
        .iter()
        .map(|a| toml_basic_string(a))
        .collect::<Vec<_>>()
        .join(", ");

    let mut env_lines = String::new();
    if let serde_json::Value::Object(map) = &env {
        if !map.is_empty() {
            env_lines.push_str("\n[mcp_servers.easynet.env]\n");
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    env_lines.push_str(&format!("{k} = {}\n", toml_basic_string(s)));
                }
            }
        }
    }

    let model_line = model
        .map(|m| format!("model = {}\n", toml_basic_string(m)))
        .unwrap_or_default();

    let toml = format!(
        "{model}\n[mcp_servers.easynet]\ncommand = {cmd}\nargs = [{args}]\n{env}",
        model = model_line,
        cmd = toml_basic_string(&cmd),
        args = args_toml,
        env = env_lines,
    );

    // Atomic write so a concurrent `easynet agent send` for the same
    // agent can't observe a partially-written `config.toml`. Reuses the
    // race-safe primitive from `persistence::config` (iter-1 fix).
    config::atomic_write(&codex_dir.join("config.toml"), toml.as_bytes())?;
    Ok(())
}

/// Seed the `easynet-collaborate` skill so a freshly-installed
/// agent knows how to walk the discovery ladder (own abilities →
/// other agents on this device → published abilities on EasyNet)
/// before falling back to "I can't do that" or a generic web search.
///
/// Source of truth: `<repo>/skills/easynet-collaborate/SKILL.md`,
/// included at compile time. Operators editing the skill body edit
/// the markdown file; Rust does not need to recompile until the
/// next release cuts.
///
/// Path picking
/// ------------
/// Claude Code's skill loader scans `<cwd>/.claude/skills/` per
/// Anthropic's project-local convention. Codex reads
/// `<cwd>/.agents/skills/` (the historical Agent-Skills convention).
/// Writing to the right path is load-bearing — earlier seeds wrote
/// to `<cwd>/skills/` for both runtimes; the file existed but the
/// loader never matched it.
///
/// Mirrors the routing in
/// `runtime::agents::skill_publish_ability::skills_dir_for` so a
/// curator-published skill and a freshly-seeded skill both land in
/// the location the runtime actually scans.
fn write_collaborate_seed(ws: &Path, runtime: RuntimeKind) -> anyhow::Result<()> {
    let skill_dir = collaborate_seed_dir(ws, runtime);
    fs::create_dir_all(&skill_dir)?;
    config::atomic_write(&skill_dir.join("SKILL.md"), COLLABORATE_SKILL_MD.as_bytes())?;
    Ok(())
}

/// Pick the seed-skill directory for a given runtime. Pulled out so
/// the seed function and the workspace tests stay in sync.
fn collaborate_seed_dir(ws: &Path, runtime: RuntimeKind) -> PathBuf {
    let parent = match runtime {
        RuntimeKind::ClaudeCode => ws.join(".claude").join("skills"),
        RuntimeKind::Codex | RuntimeKind::CodexAppServer => ws.join(".agents").join("skills"),
    };
    parent.join("easynet-collaborate")
}

/// `easynet-collaborate` seed body — the source of truth lives at
/// `skills/easynet-collaborate/SKILL.md` in this repo. Included via
/// `include_str!` so editing the markdown is enough; no Rust recompile
/// is needed for a content change in the same release.
const COLLABORATE_SKILL_MD: &str = include_str!("../../skills/easynet-collaborate/SKILL.md");

/// Seed the `easynet-pages-author` skill (RFC-006-B v0.6) so a freshly-
/// installed agent knows how to write a website + tiny declarative
/// backend, then deploy it via `easynet pages create`. Mirrors the
/// `write_collaborate_seed` shape: `include_str!` body, runtime-aware
/// directory, atomic write.
fn write_pages_author_seed(ws: &Path, runtime: RuntimeKind) -> anyhow::Result<()> {
    let skill_dir = pages_author_seed_dir(ws, runtime);
    fs::create_dir_all(&skill_dir)?;
    config::atomic_write(&skill_dir.join("SKILL.md"), PAGES_AUTHOR_SKILL_MD.as_bytes())?;
    Ok(())
}

/// Pick the seed-skill directory for `easynet-pages-author` per
/// runtime. Claude Code: `<ws>/.claude/skills/easynet-pages-author/`.
/// Codex: `<ws>/.agents/skills/easynet-pages-author/`.
fn pages_author_seed_dir(ws: &Path, runtime: RuntimeKind) -> PathBuf {
    let parent = match runtime {
        RuntimeKind::ClaudeCode => ws.join(".claude").join("skills"),
        RuntimeKind::Codex | RuntimeKind::CodexAppServer => ws.join(".agents").join("skills"),
    };
    parent.join("easynet-pages-author")
}

/// `easynet-pages-author` seed body. Source of truth:
/// `skills/easynet-pages-author/SKILL.md`. Editing the markdown
/// recompiles the daemon binary at next build but does not require
/// a manual sync step.
const PAGES_AUTHOR_SKILL_MD: &str =
    include_str!("../../skills/easynet-pages-author/SKILL.md");

/// Seed `easynet-ability-author` — pairs with pages-author so a
/// single agent learns both ends of the full-stack loop.
fn write_ability_author_seed(ws: &Path, runtime: RuntimeKind) -> anyhow::Result<()> {
    let skill_dir = ability_author_seed_dir(ws, runtime);
    fs::create_dir_all(&skill_dir)?;
    config::atomic_write(
        &skill_dir.join("SKILL.md"),
        ABILITY_AUTHOR_SKILL_MD.as_bytes(),
    )?;
    Ok(())
}

fn ability_author_seed_dir(ws: &Path, runtime: RuntimeKind) -> PathBuf {
    let parent = match runtime {
        RuntimeKind::ClaudeCode => ws.join(".claude").join("skills"),
        RuntimeKind::Codex | RuntimeKind::CodexAppServer => ws.join(".agents").join("skills"),
    };
    parent.join("easynet-ability-author")
}

const ABILITY_AUTHOR_SKILL_MD: &str =
    include_str!("../../skills/easynet-ability-author/SKILL.md");

// ── Shared ───────────────────────────────────────────────────────────────────

/// Resolve the path of the `easynet` binary that the spawned
/// MCP server child should run.
///
/// Resolution order:
///   1. `current_exe()` IF the executable is named `easynet`
///      / `easynet-daemon` (the two binaries that actually
///      implement `mcp-server`). This is the production case:
///      the daemon spawned the call, so its own path is the
///      correct subprocess to relaunch.
///   2. Else, search `PATH` for a binary literally named
///      `easynet`. This catches the dev-time scenario where a
///      maintainer runs `cargo run --bin gen-ability-tomls` or
///      a smoke binary; current_exe() returns that test
///      binary's path, but we want claude's `.mcp.json` to
///      point at the real `easynet` install on the developer's
///      PATH (typically `/usr/local/bin/easynet` from
///      `cargo install easynet`).
///   3. Last resort: the literal string `"easynet"` and let
///      the spawn-time PATH search find it.
///
/// Why we don't use current_exe unconditionally
/// --------------------------------------------
/// During this audit conversation, `cargo run --bin
/// real-user-smoke` corrupted the developer's
/// `~/.easynet/workspaces/claude/.mcp.json` to point at the
/// smoke binary. The smoke binary doesn't implement
/// `mcp-server`, so the next claude.chat invocation that tries
/// to use an EasyNet MCP tool would fail silently. The check
/// against the binary's filename eliminates that whole class
/// of test-side-effect.
fn resolve_easynet_binary() -> String {
    let current = std::env::current_exe().ok();

    // Step 1: if current_exe is `easynet` (the CLI binary that DOES
    // parse `mcp serve`), use it directly. Doing this preserves the
    // dev-build path (target/debug/easynet) — the production
    // installer drops both binaries into /usr/local/bin so PATH
    // resolution would also work, but using the absolute path
    // skips the lookup and guarantees the same binary version
    // gets re-spawned for MCP.
    //
    // We deliberately do NOT use `easynet-daemon` even when the
    // current process IS easynet-daemon. `easynet-daemon` does not
    // parse subcommand args at all — it always boots the full
    // daemon (binds control.sock + runtime-dispatch.sock). When
    // `.mcp.json` invokes `easynet-daemon mcp serve …` to satisfy
    // the host AI client's MCP request, the spawned subprocess
    // forcibly removes the parent daemon's control.sock file
    // (transport.rs::bind_at line 146-147) and binds its own.
    // From that point forward, all chat dispatches land on the
    // ghost daemon (which has no chat handler registered) and
    // hang or close. Real behaviour observed: first chat works,
    // every subsequent chat returns "daemon closed the connection
    // before responding" instantly.
    //
    // The right binary for `.mcp.json` is always `easynet` (which
    // routes `mcp serve` through `cli::mcp_server::run`). Resolve
    // it from the current_exe's parent directory first (production
    // installs side-by-side); fall back to PATH lookup; final
    // fallback: bare name and let exec() find it.
    if let Some(p) = current.as_ref() {
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            if stem == "easynet" {
                if let Some(s) = p.to_str() {
                    return s.to_string();
                }
            }
            if stem == "easynet-daemon" {
                // We are the daemon writing .mcp.json. Look for the
                // sibling `easynet` binary so the spawned MCP subprocess
                // hits the CLI parser, not a second daemon.
                if let Some(parent) = p.parent() {
                    let sibling = parent.join("easynet");
                    if sibling.is_file() {
                        if let Some(s) = sibling.to_str() {
                            return s.to_string();
                        }
                    }
                }
            }
        }
    }

    // Step 2: search PATH for `easynet`.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("easynet");
            if candidate.is_file() {
                if let Some(s) = candidate.to_str() {
                    return s.to_string();
                }
            }
        }
    }

    // Step 3: last resort — let spawn-time PATH search find it.
    "easynet".to_string()
}

/// Build the (command, args, env) tuple for launching the EasyNet MCP
/// server as a subprocess of an agent. The launching agent's name is
/// threaded through so the MCP server knows which agent it belongs to,
/// for two purposes:
///
/// 1. The `--agent <name>` flag becomes the `from=` label in the
///    per-call audit line emitted by `mcp::handlers::send_to_agent`.
/// 2. `--enable-agent-dispatch` is set so the MCP server is allowed to
///    spawn other agents through the mission runtime. This is what
///    makes ontology §6.2 derivation 3 ("there is no second path") hold
///    *inside* an agent, not just at the CLI surface.
///
/// Defaulting `--enable-agent-dispatch` is a behaviour-semantics
/// change, not pure plumbing. The scoping rules that make it safe live
/// in `mcp::handlers::send_to_agent` (audit log, recursion guard,
/// future tenant check). The MCP server itself prints a one-line
/// stderr banner on startup so the operator sees that the workspace
/// MCP can spawn other agents.
pub(super) fn build_mcp_entry(agent_name: &str) -> (String, Vec<String>, serde_json::Value) {
    let cmd = resolve_easynet_binary();
    // The CLI subcommand is `easynet mcp serve` (a two-token
    // path, not a single hyphenated `mcp-server`). The earlier
    // shape was renamed when the `mcp` group split into
    // serve/status/install/skill-install — but the .mcp.json
    // writer was never updated, so every agent's workspace got
    // a broken MCP server config that fails with "unrecognized
    // subcommand" the moment Claude Code / Codex tries to spawn
    // it. Fixed in slice 27.
    let mut args = vec!["mcp".to_string(), "serve".to_string()];
    let mut env = serde_json::Map::new();

    // `easynet mcp serve` accepts only --tenant and --agent
    // (see facade/cli/mcp_server.rs::McpServerArgs). Two flags
    // we used to write — --endpoint and --enable-agent-dispatch
    // — were dropped in the P4.9 quarantine when MCP server
    // construction moved into the mcp profile. Writing them
    // here today causes claude/codex to spawn the subprocess
    // and immediately get "error: unexpected argument
    // '--endpoint'", which surfaces in claude's debug as
    // `mcp_servers: [{name: "easynet", status: "failed"}]`.
    // The audit conversation surfaced this; the fix below
    // matches the actual subcommand grammar.
    if let Ok(state) = config::load() {
        if let Some(t) = &state.tenant {
            args.push("--tenant".to_string());
            args.push(t.clone());
        }
    }

    // Identify the launching agent. The mcp serve handler uses
    // this to (a) label audit lines, (b) include the agent's
    // per-workspace abilities in the tool catalogue (slice
    // 28's G1 fix in profiles/mcp.rs::build_stdio_server).
    args.push("--agent".to_string());
    args.push(agent_name.to_string());

    if let Ok(lib) = std::env::var("EASYNET_DENDRITE_BRIDGE_LIB") {
        env.insert(
            "EASYNET_DENDRITE_BRIDGE_LIB".to_string(),
            serde_json::json!(lib),
        );
    }

    (cmd, args, serde_json::Value::Object(env))
}

fn generate_knowledge_doc() -> String {
    r#"# EasyNet Agent Workspace

## What you are

You are an agent in **EasyNet**, an agent-native distributed system. EasyNet's
network has only two first-class objects:

- **Agent** — the addressable actor (you).
- **Ability** — a public method endpoint exposed by an agent.

You expose abilities. You do **not** expose skills. Skills are private to you
and are unreachable from outside. Other agents can call your abilities; they
cannot read your skills, your memory, or your internal state. The same rule
applies in reverse: you can call other agents' abilities, but you cannot
reach into their skills.

This is the **encapsulation invariant**: no CLI command, no SDK call, and no
EAL construct may reach across an agent boundary into a private skill. If
you find yourself wanting to "use another agent's skill", the only valid
form is: that agent must wrap the skill as an ability. See
`docs/easynet_ontology.tex` §4 for the full rule.

## How you call other agents

Cross-agent calls go through the **mission runtime**. There is no second
path. The shortest possible call is:

```eal
mission "ask-claude" {
  let r = claude.chat(prompt: "hello")
}
```

`chat` is the default callable on every agent — analogous to `__call__` in
Python or `Object.toString()` in Java. Custom abilities are called the
same way:

```eal
mission "review-code" {
  let r = claude.review(file: "x.rs", strict: true)
}
```

You can run a mission via the `run_mission` MCP tool, or — when calling
just the default `chat` ability — via the `send_to_agent` MCP tool, which
is the wire-level form of `<agent>.chat(<prompt>)` and desugars to a
single-line External EAL mission internally.

### What this means for you

Calling another agent triggers a **network execution chain**. When you
write `<other_agent>.chat(...)` or use the `send_to_agent` MCP tool, the
Hub spawns the target agent through the mission runtime. The target agent
may itself call other agents, up to depth 2. This is the only path by
which agents talk to each other in EasyNet, and every call is logged in
the MCP server's stderr stream as `[easynet mcp dispatch] from=... to=...
depth=... mission=...`.

Use this awareness when planning multi-step tasks: a single sentence like
*"ask the reviewer agent to look at this"* is potentially a multi-process,
multi-second operation, not a local function call. If you don't actually
need cross-agent coordination, do the work yourself.

## MCP tools available to you

When the workspace MCP server is running, the following tools are exposed.
Use them for federation queries, ability lifecycle, remote execution,
orchestration, and agent-to-agent dispatch.

### Federation queries (read-only)

- `hub_status()` — Hub connectivity, node and ability counts.
- `list_devices()` — List federation devices.
- `get_device_detail(node_id)` — Device info plus installed abilities.
- `list_all_abilities(node_id?, name_pattern?)` — Discover abilities
  across nodes; `name_pattern` accepts substring or glob filters.
- `list_a2a_agents(tags?, owner_id?, limit?)` — Remote A2A agents.
- `get_a2a_agent_card(node_id)` — A2A agent card.

### Ability lifecycle (mutating)

- `deploy_ability(node_id, tool_name, command, description?)` — Publish,
  install, and activate an ability on a device.
- `uninstall_ability(node_id, install_id)` — Remove an ability.

### Remote execution

- `execute_command(node_id, command)` — One-shot remote command on a
  device.
- `invoke_ability(node_id, ability, arguments?)` — Invoke an ability on
  a federated node.
- `send_a2a_task(target_agent_id, skill_id, input_json?, ...)` — Send an
  A2A task to a remote agent.

### Orchestration

- `run_mission(eal_source, emit_ir_only?)` — Compile and execute an EAL
  program. This is the highest-leverage tool — every multi-step
  cross-agent or cross-device interaction is best expressed as a mission.

### Device management

- `manage_device(node_id, action)` — Drain or disconnect a device.

### Agent dispatch (this workspace only, see banner)

- `send_to_agent(agent, prompt, context?)` — Wire-level form of
  `<agent>.chat(<prompt>)`. Desugars to a single-line External EAL
  mission internally. Use this when you need to talk to another agent
  and don't want to write the EAL yourself.

## What you can not do

- You can **not** access another agent's skills. Skills are private.
- You can **not** call methods that are not declared as abilities. Only
  the public surface is callable.
- You can **not** introspect another agent's internal graph. The agent
  is a black box from the network's point of view.
- You can **not** rename, delete, or directly modify abilities deployed
  by other tenants. The CLI surface deliberately omits commands like
  `easynet ability update` because they conflate three distinct time
  scales (schema bumps, graph evolution, per-call execution).

If a future need arises that you can't satisfy with the above rules, the
only valid resolution is for the *other* agent to expose what you need
as a new ability. You don't get a back door.

## Patterns

### Ask another agent something

```eal
mission "ask-codex" {
  let r = codex.chat(prompt: "explain quicksort in 3 lines")
}
```

Or via the MCP tool:

```json
{
  "name": "send_to_agent",
  "arguments": { "agent": "codex", "prompt": "explain quicksort in 3 lines" }
}
```

Both produce identical execution: a single-line External EAL mission
through the mission runtime.

### Compose two agents in a pipeline

```eal
mission "draft-and-review" {
  let draft = claude.draft(topic: "rate limiter design")
  let review = codex.review(input: draft.output)
}
```

The dependency on `draft.output` is inferred — `review` runs after
`draft` automatically. Independent steps would run in parallel.

### Avoid recursive loops

The recursion guard caps cross-agent dispatch at depth 2. If you find
yourself wanting to call yourself recursively, restructure the mission
into explicit phases instead — the planner will handle parallelism for
you.

## Quick CLI reference

```bash
easynet agent send <agent> "<prompt>"   # sugar for one-line mission
easynet mission run <file.eal>          # run a multi-step mission
easynet mission list                    # show recorded mission runs
easynet ability list                    # discover available abilities
easynet device list                     # list hosting substrates
```
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the workspace MCP entry includes the agent dispatch
    /// flag and the launching agent name. This is the load-bearing
    /// behaviour from Step 5 of the implementation plan: every agent
    /// workspace must launch its MCP server with `--enable-agent-dispatch`
    /// and `--agent <name>` so cross-agent dispatch is always available
    /// from inside an agent.
    #[test]
    fn resolve_easynet_binary_does_not_use_test_runner_path() {
        // During `cargo test`, `current_exe()` returns the path of
        // the test runner binary (e.g.
        // `target/debug/deps/easynet_cli-<hash>`), NOT `easynet`.
        // Pre-fix this leaked into the developer's `.mcp.json`
        // and broke claude.chat's MCP discovery for any subsequent
        // call. The resolver must return either an actual
        // `easynet` path on PATH, or the bare string "easynet"
        // — never the test runner's path.
        let resolved = resolve_easynet_binary();
        let runner = std::env::current_exe().ok();
        if let Some(r) = runner {
            let r_str = r.to_string_lossy().to_string();
            assert_ne!(
                resolved, r_str,
                "resolve_easynet_binary returned the test runner's path: {r_str}; \
                 fix the resolver, see the slice-24 commit message"
            );
        }
        // The resolved path either ends in /easynet or is the
        // literal "easynet" fallback.
        let stem = std::path::Path::new(&resolved)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        assert!(
            stem == "easynet" || stem == "easynet-daemon" || resolved == "easynet",
            "resolved binary path has unexpected name: {resolved}"
        );
    }

    #[test]
    fn build_mcp_entry_uses_correct_two_token_subcommand() {
        // `easynet mcp serve` is the actual CLI path. Pre-fix
        // this wrote `["mcp-server", ...]` (a single hyphenated
        // token that the CLI dispatcher does not recognise),
        // which meant every agent's `.mcp.json` had a broken
        // MCP server config that failed with "unrecognized
        // subcommand" the moment Claude Code / Codex spawned
        // it. Lock the correct two-token path so a future
        // rename of the subcommand requires updating both this
        // writer and this test in lockstep.
        let (_, args, _) = build_mcp_entry("claude");
        assert_eq!(args.first().map(|s| s.as_str()), Some("mcp"));
        assert_eq!(args.get(1).map(|s| s.as_str()), Some("serve"));
        // The hyphenated form must NEVER appear.
        for a in &args {
            assert!(
                a != "mcp-server",
                "args must not contain the legacy `mcp-server` token: {args:?}"
            );
        }
    }

    #[test]
    fn build_mcp_entry_passes_agent_name_via_two_arg_flag() {
        let (cmd, args, _env) = build_mcp_entry("claude");
        assert!(!cmd.is_empty(), "command must be set");
        // The agent name must be passed as `--agent <name>` (two adjacent
        // args, not a single `--agent=name`).
        let agent_idx = args
            .iter()
            .position(|a| a == "--agent")
            .expect("args must contain --agent");
        assert_eq!(
            args.get(agent_idx + 1).map(|s| s.as_str()),
            Some("claude"),
            "--agent must be followed by the agent name"
        );
        // Sanity: args must NOT contain flags that the current
        // `mcp serve` subcommand doesn't accept. P4.9 dropped
        // --endpoint and --enable-agent-dispatch; writing them
        // here would cause every workspace MCP server to fail
        // with "unexpected argument" the moment claude/codex
        // spawned it.
        assert!(
            !args.iter().any(|a| a == "--endpoint"),
            "args must NOT contain --endpoint (removed in P4.9): {args:?}"
        );
        assert!(
            !args.iter().any(|a| a == "--enable-agent-dispatch"),
            "args must NOT contain --enable-agent-dispatch (removed in P4.9): {args:?}"
        );
    }

    #[test]
    fn build_mcp_entry_threads_different_agent_names() {
        let (_, args_a, _) = build_mcp_entry("alice");
        let (_, args_b, _) = build_mcp_entry("bob");
        let agent_a = args_a
            .iter()
            .position(|a| a == "--agent")
            .map(|i| &args_a[i + 1]);
        let agent_b = args_b
            .iter()
            .position(|a| a == "--agent")
            .map(|i| &args_b[i + 1]);
        assert_eq!(agent_a.map(|s| s.as_str()), Some("alice"));
        assert_eq!(agent_b.map(|s| s.as_str()), Some("bob"));
    }

    // ── ensure_from_directory (primary entry, projection) ────────────────

    use crate::core::agent_spec::{AgentSpec, RuntimeKind};
    use crate::runtime::directory::{AgentDirectory, Location};

    /// Build a throwaway `AgentDirectory` at a unique temp path
    /// with the given runtime. Returns the directory handle and
    /// its root path so the caller can assert on the derived
    /// files and clean up.
    fn scratch_dir(tag: &str, runtime: RuntimeKind) -> (AgentDirectory, PathBuf) {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("easynet-ws-{tag}-{pid}-{nanos}"));
        let dir = AgentDirectory::create(
            &Location::Local { root: root.clone() },
            AgentSpec::new("alice", runtime),
        )
        .unwrap();
        (dir, root)
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ensure_from_directory_writes_claude_knowledge_and_mcp_json() {
        let (dir, root) = scratch_dir("claude-proj", RuntimeKind::ClaudeCode);
        let returned = ensure_from_directory(&dir).unwrap();
        assert_eq!(returned, root);
        // Claude-path files.
        assert!(root.join("CLAUDE.md").is_file());
        assert!(root.join("AGENTS.md").is_file());
        assert!(root.join(".mcp.json").is_file());
        // Codex-only files must not be materialised for a
        // ClaudeCode runtime — the branch in the projection is
        // what enforces the size/shape of the workspace.
        assert!(!root.join(".codex/config.toml").exists());
        // Single seed: easynet-collaborate. Claude Code reads
        // <root>/.claude/skills/ per the Anthropic project-local
        // skill convention; that's the load-bearing path.
        assert!(root
            .join(".claude/skills/easynet-collaborate/SKILL.md")
            .is_file());
        // Negative: pre-rewrite the seeder also wrote to
        // <root>/skills/ and <root>/.agents/skills/ for both
        // runtimes; that produced files Claude Code's loader did
        // not scan. Confirm the legacy paths are gone for the
        // claude-code branch.
        assert!(!root.join("skills/easynet-collaborate/SKILL.md").exists());
        assert!(!root.join("skills/delegate/SKILL.md").exists());
        assert!(!root.join("skills/easynet-ability-crud/SKILL.md").exists());
        cleanup(&root);
    }

    #[test]
    fn ensure_from_directory_for_codex_also_writes_codex_config_and_skill() {
        let (dir, root) = scratch_dir("codex-proj", RuntimeKind::Codex);
        ensure_from_directory(&dir).unwrap();
        assert!(root.join("CLAUDE.md").is_file());
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join(".codex/config.toml").is_file());
        // Codex reads <root>/.agents/skills/ — the historical Agent-
        // Skills convention. Single seed lands there.
        assert!(root
            .join(".agents/skills/easynet-collaborate/SKILL.md")
            .is_file());
        // Negative: legacy paths are not written.
        assert!(!root.join("skills/easynet-collaborate/SKILL.md").exists());
        assert!(!root
            .join(".claude/skills/easynet-collaborate/SKILL.md")
            .exists());
        cleanup(&root);
    }

    #[test]
    fn collaborate_seed_has_canonical_skill_structure() {
        // The seed body is included from
        // skills/easynet-collaborate/SKILL.md at compile time. Pin
        // the load-bearing parts of the Anthropic-canonical skill
        // structure here so a regression to the markdown source
        // (dropping front matter, the activation section, the
        // "Use when" hint) trips this test instead of silently
        // shipping an inert seed.
        for required in [
            "name: easynet-collaborate",
            "description:",
            "allowed-tools:",
            "## When This Skill Activates",
            "<self>.discover",
            "<self>.invoke",
        ] {
            assert!(
                COLLABORATE_SKILL_MD.contains(required),
                "seed must contain {required:?} for activation / canonical structure"
            );
        }
        // The "Use when" hint is the activation-decision phrase the
        // Claude Code skill loader matches against incoming prompts.
        // Without it the skill lands on disk but never activates.
        let lower = COLLABORATE_SKILL_MD.to_ascii_lowercase();
        assert!(
            lower.contains("use when"),
            "description must contain 'Use when …' so the loader has an activation hint"
        );
    }

    #[test]
    fn ensure_from_directory_for_codex_app_server_mirrors_codex_shape() {
        // CodexAppServer uses the same runtime binary and the
        // same discovery files; the branch in the projection
        // must treat it identically. Pinning this test prevents
        // a future refactor that tightens the match from
        // accidentally splitting the two variants.
        let (dir, root) = scratch_dir("codex-as", RuntimeKind::CodexAppServer);
        ensure_from_directory(&dir).unwrap();
        assert!(root.join(".codex/config.toml").is_file());
        cleanup(&root);
    }

    #[test]
    fn projection_is_idempotent_on_subsequent_calls() {
        // Second call over the same directory must not fail,
        // must not panic, and must leave the derived files
        // byte-identical. A regression where, say, `.mcp.json`
        // embeds a timestamp would break this.
        //
        // HomeGuard is load-bearing: `build_mcp_entry` reads
        // `~/.easynet/runtime_state.json` via `config::load()`.
        // Under parallel `cargo test`, another test may create or
        // delete that file between our two projection calls,
        // producing two different .mcp.json files (the race flips
        // whether --endpoint/--tenant args appear). Scoping HOME
        // to a private tmp dir makes the "no runtime started"
        // branch deterministic.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let (dir, root) = scratch_dir("idem", RuntimeKind::Codex);
        ensure_from_directory(&dir).unwrap();
        let mcp_before = fs::read(root.join(".mcp.json")).unwrap();
        let codex_before = fs::read(root.join(".codex/config.toml")).unwrap();
        ensure_from_directory(&dir).unwrap();
        let mcp_after = fs::read(root.join(".mcp.json")).unwrap();
        let codex_after = fs::read(root.join(".codex/config.toml")).unwrap();
        assert_eq!(mcp_before, mcp_after);
        assert_eq!(codex_before, codex_after);
        cleanup(&root);
    }

    #[test]
    fn codex_config_embeds_model_from_spec_not_entry() {
        // The projection must read `model` from the
        // `AgentDirectory`'s `AgentSpec`, not from any
        // `AgentEntry` the shim might be carrying. We create a
        // directory with a spec that sets `model`, run the
        // projection, and verify the on-disk
        // `.codex/config.toml` picked it up. This is the
        // load-bearing test for "spec is source of truth".
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("easynet-ws-model-{pid}-{nanos}"));
        let mut spec = AgentSpec::new("alice", RuntimeKind::Codex);
        spec.model = Some("gpt-5-turbo".into());
        let dir = AgentDirectory::create(&Location::Local { root: root.clone() }, spec).unwrap();

        ensure_from_directory(&dir).unwrap();
        let codex_toml = fs::read_to_string(root.join(".codex/config.toml")).unwrap();
        assert!(
            codex_toml.contains("model = \"gpt-5-turbo\""),
            "codex config must carry model from spec; got:\n{codex_toml}"
        );
        cleanup(&root);
    }

    // ── ensure_workspace (shim) ──────────────────────────────────────────

    use crate::facade::cli::test_support::HomeGuard;
    use crate::registry::agents::{AgentEntry, AgentType};

    #[test]
    fn shim_resolves_entry_root_path_when_set() {
        // A v2-migrated row carries `root_path`. The shim must
        // prefer it over the default computation — otherwise a
        // project-local agent whose root is outside the global
        // tree would be projected into the wrong directory.
        let _g = HomeGuard::new();
        let root = std::env::temp_dir().join(format!(
            "easynet-ws-shim-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0),
        ));
        let mut entry = AgentEntry::new(AgentType::ClaudeCode, None);
        entry.root_path = Some(root.clone());

        let returned = ensure_workspace("alice", &entry).unwrap();
        assert_eq!(returned, root, "shim must honour entry.root_path");
        // Derived files must be under the chosen root.
        assert!(root.join("CLAUDE.md").is_file());
        cleanup(&root);
    }

    #[test]
    fn shim_falls_back_to_agents_root_when_entry_has_no_root_path() {
        // Fresh v2 rows (written by today's `run_add`, which
        // does not yet populate `root_path`) still work because
        // the shim falls back to `agents_root().join(name)`.
        // This is the consumer-side fallback the PR-3b.2
        // review flagged: rather than backport a writer-side
        // fix, the consumer handles the gap.
        let _g = HomeGuard::new();
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        assert!(entry.root_path.is_none());

        let returned = ensure_workspace("alice", &entry).unwrap();
        let expected = config::agents_root().join("alice");
        assert_eq!(returned, expected);
        assert!(expected.join("CLAUDE.md").is_file());
        assert!(expected.join("agent.toml").is_file());
        // The reconstructed spec must have the correct runtime;
        // a silent mismatch here would dispatch to the wrong
        // driver on the next call.
        let spec =
            AgentSpec::from_toml_str(&fs::read_to_string(expected.join("agent.toml")).unwrap())
                .unwrap();
        assert_eq!(spec.runtime, RuntimeKind::ClaudeCode);
    }

    #[test]
    fn shim_reuses_existing_agent_toml_without_overwrite() {
        // If the agent directory already has an `agent.toml`
        // the shim must open it rather than create afresh. A
        // create-on-every-dispatch would clobber user edits to
        // the spec (e.g. a custom `description`).
        let _g = HomeGuard::new();
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        // First call materialises agent.toml.
        ensure_workspace("alice", &entry).unwrap();
        let root = config::agents_root().join("alice");
        let spec_path = root.join("agent.toml");
        // Edit the spec on disk.
        let mut spec = AgentSpec::from_toml_str(&fs::read_to_string(&spec_path).unwrap()).unwrap();
        spec.description = Some("user-edited".into());
        fs::write(&spec_path, spec.to_toml_string().unwrap()).unwrap();

        // Second call must NOT overwrite the user's edit.
        ensure_workspace("alice", &entry).unwrap();
        let after = AgentSpec::from_toml_str(&fs::read_to_string(&spec_path).unwrap()).unwrap();
        assert_eq!(
            after.description.as_deref(),
            Some("user-edited"),
            "shim must not overwrite user edits to agent.toml"
        );
    }

    /// Parity: the two paths that turn an `AgentEntry` into an
    /// `AgentSpec` must produce the same mapping.
    ///
    /// * `spec_from_entry` (this file) — the reconstruction
    ///   path inside the `ensure_workspace` shim, hit when
    ///   `agent.toml` was deleted manually.
    /// * `registry::agents::migrate_one_entry` — the v1→v2
    ///   load-time migration path, hit once per legacy row.
    ///
    /// Both run on the same fat-field input. If one tightens a
    /// field mapping (e.g. "carry labels longer than 64 chars
    /// differently") without the other, a user who triggers
    /// the shim's reconstruction branch gets a different spec
    /// than one whose row was migrated at load time. That's
    /// the heisenbug class this test pins.
    ///
    /// We compare via the serialized TOML rather than via
    /// `AgentSpec` equality so a future field added to
    /// `AgentSpec` is also guaranteed to survive both paths —
    /// both sides must know about it.
    #[test]
    fn spec_from_entry_agrees_with_migrate_one_entry_mapping() {
        let _g = HomeGuard::new();

        // Build a fat v1-shaped entry with every mappable
        // field populated so the parity test covers each
        // mapping hop, not just the happy case.
        let mut entry = AgentEntry::new(AgentType::Codex, Some("gpt-5".into()));
        entry.label = Some("nightly auditor".into());
        entry.timeout_secs = 900;

        // Reconstruction path: spec_from_entry on the same
        // input.
        let spec_via_shim = spec_from_entry("alice", &entry);
        let toml_via_shim = spec_via_shim.to_toml_string().unwrap();

        // Migration path: seed a v1 registry carrying the same
        // entry, load it (which triggers the migration + writes
        // `agent.toml` under `agents_root()/alice/`), then read
        // the `agent.toml` back.
        let mut registry = crate::registry::agents::AgentRegistry::default();
        registry.agents.insert("alice".into(), entry.clone());
        // Write as v1 by force (schema_version=0) — the
        // AgentEntry::new helper stamps v2, but we want the
        // migration path to fire.
        let mut v1_entry = entry.clone();
        v1_entry.schema_version = 0;
        v1_entry.root_path = None;
        let mut v1_reg = crate::registry::agents::AgentRegistry::default();
        v1_reg.agents.insert("alice".into(), v1_entry);
        // Hand-serialize to bypass save_agents()'s validation
        // (which would stamp v2 on write); the migration is
        // what we're testing.
        let json = serde_json::to_string_pretty(&v1_reg).unwrap();
        let agents_path = config::state_dir().join("agents.json");
        fs::create_dir_all(config::state_dir()).unwrap();
        fs::write(&agents_path, json).unwrap();

        // Trigger migration.
        let loaded = crate::registry::agents::load_agents().unwrap();
        let root = loaded.agents["alice"].root_path.as_ref().unwrap();
        let toml_via_migrate = fs::read_to_string(root.join("agent.toml")).unwrap();

        // Parse both back to AgentSpec and compare field-by-field
        // rather than byte-by-byte; the two writers produce
        // equivalent-but-possibly-different formatting (both
        // correct TOML) and we care about semantic identity.
        let spec_shim = AgentSpec::from_toml_str(&toml_via_shim).unwrap();
        let spec_migrate = AgentSpec::from_toml_str(&toml_via_migrate).unwrap();
        assert_eq!(
            spec_shim, spec_migrate,
            "spec_from_entry and migrate_one_entry must produce the same spec \
             for the same AgentEntry input"
        );
    }

    #[test]
    fn shim_refuses_to_overwrite_partial_skeleton() {
        // The partial-skeleton guard from PR-3b.1.5 must still
        // fire through the shim. An operator whose earlier
        // `agent new` crashed before writing agent.toml must
        // not silently adopt the skeleton on the next
        // dispatch. The guard lives in `AgentDirectory::create`
        // and the shim reaches it through the "agent.toml
        // missing + root exists non-empty" branch.
        let _g = HomeGuard::new();
        let root = config::agents_root().join("alice");
        fs::create_dir_all(root.join("abilities")).unwrap();
        fs::write(root.join(".env"), "").unwrap();
        // No agent.toml. No root_path on entry, so the shim
        // computes the same path.
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        let err = ensure_workspace("alice", &entry)
            .expect_err("partial skeleton must be refused by shim path");
        assert!(
            format!("{err}").contains("half-finished") || format!("{err}").contains("previous")
        );
    }
}
