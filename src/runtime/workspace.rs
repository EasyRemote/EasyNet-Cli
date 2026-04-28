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
/// * `ClaudeCode`: writes `.mcp.json` + `CLAUDE.md` + `AGENTS.md`.
/// * `Codex` / `CodexAppServer`: additionally writes
///   `.codex/config.toml` + `.agents/skills/easynet-ability-crud/*`
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

    // Seed the ability-CRUD skill for every runtime. Claude Code
    // discovers it via --plugin-dir <root>/skills/<name>/ (see
    // drivers::claude_code::invoke); Codex via the legacy
    // .agents/skills/ convention. Writing to both makes the skill
    // visible no matter which runtime spawns.
    //
    // Pre-fix this seeded ONLY for Codex, and the content named
    // MCP tools (deploy_ability, run_mission, list_devices) that
    // do not exist in the current MCP surface. Surfaced when an
    // audit asked whether agents knew how to author abilities at
    // all — claude workspaces had no skill, codex had a skill that
    // pointed at ghost tools.
    write_ability_crud_skill(&root)?;
    // Pair skill: how to USE existing abilities, walking a
    // discovery ladder (self → device → easynet) before falling
    // back to "I can't do that". The CRUD skill teaches how to
    // GROW the network; this one teaches how to consume it.
    write_delegate_skill(&root)?;

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
    if entry.timeout_secs != 300 {
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

fn write_codex_config(
    ws: &Path,
    model: Option<&str>,
    agent_name: &str,
) -> anyhow::Result<()> {
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

/// Seed the `easynet-ability-crud` skill so a freshly-installed
/// agent knows how to author / validate / deploy / invoke / remove
/// abilities through EasyNet's actual MCP surface and CLI.
///
/// We write to TWO locations on purpose:
///
///   * `<workspace>/skills/easynet-ability-crud/SKILL.md` — picked
///     up by Claude Code via `--plugin-dir <workspace>/skills/...`
///     (drivers::claude_code::invoke walks this directory at spawn
///     time and adds one --plugin-dir per plugin-shaped subdir).
///   * `<workspace>/.agents/skills/easynet-ability-crud/SKILL.md`
///     — the legacy Codex/Agent-Skills convention.
///
/// Both files have identical content. The duplication costs ~3 KiB
/// per workspace and means the same skill works under whichever
/// runtime ends up dispatching, without a runtime branch here.
fn write_ability_crud_skill(ws: &Path) -> anyhow::Result<()> {
    write_seed_skill(ws, "easynet-ability-crud", ABILITY_CRUD_SKILL_MD)
}

/// Seed the `delegate` skill so a freshly-installed agent knows how
/// to walk the discovery ladder (own abilities → other agents on
/// this device → published abilities on EasyNet) before falling back
/// to "I can't do that" or a generic web search.
///
/// The skill replaces the previous `easynet-collaboration` seed,
/// which only covered the device tier and tied the workflow to
/// specific tool names. The new content frames discovery as a
/// general-purpose ladder so the skill stays useful when the
/// federation tier ships and when alternative discover providers are
/// installed.
///
/// Same dual-location pattern as `write_ability_crud_skill` — see
/// that function's doc comment for why both `skills/` and
/// `.agents/skills/` get a copy.
fn write_delegate_skill(ws: &Path) -> anyhow::Result<()> {
    write_seed_skill(ws, "delegate", DELEGATE_SKILL_MD)
}

/// Write the same SKILL.md body into both seed-skill locations
/// (`skills/<name>/SKILL.md` and `.agents/skills/<name>/SKILL.md`).
/// Pulled out so every seed function shares the identical layout
/// rules — a future skill that lands in only one of the two paths
/// would be invisible to whichever runtime expected the other.
fn write_seed_skill(ws: &Path, name: &str, body: &str) -> anyhow::Result<()> {
    for relative in ["skills", ".agents/skills"] {
        let skill_dir = ws.join(relative).join(name);
        fs::create_dir_all(&skill_dir)?;
        config::atomic_write(&skill_dir.join("SKILL.md"), body.as_bytes())?;
    }
    Ok(())
}

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

    // Step 1: current_exe is `easynet` or `easynet-daemon`.
    if let Some(p) = current.as_ref() {
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            if stem == "easynet" || stem == "easynet-daemon" {
                if let Some(s) = p.to_str() {
                    return s.to_string();
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

/// Default skill seeded into every freshly-provisioned workspace.
///
/// The frontmatter follows the Agent-Skills convention (Anthropic
/// Skills, Claude Code plugins, Codex `.agents/skills/`): `name`,
/// `description`, `allowed-tools`, `when_to_use`. The body is
/// structured as Goal → numbered Steps with explicit Success
/// criteria — the layout AliveCode's `skillify` template uses, and
/// the layout the EasyNet ability scaffolder itself emits when it
/// writes `SKILL.md` next to a new ability.
///
/// Tool names referenced inside MUST be tools the workspace's MCP
/// server actually advertises (from `easynet mcp serve --agent <n>
/// tools/list`). The stale predecessor of this file referenced
/// `deploy_ability`, `run_mission`, `list_devices`, etc — none of
/// which exist in the current MCP surface. Audit caught that an
/// agent reading the seed skill would try those names and get a
/// MethodNotFound on every call.
const ABILITY_CRUD_SKILL_MD: &str = r#"---
name: easynet-ability-crud
description: Author, register, invoke, and remove agent-owned EasyNet abilities by writing `.ability.toml` files into the workspace `abilities/` directory.
allowed-tools:
  - mcp__easynet
  - Bash(easynet:*)
  - Bash(ls:*)
  - Bash(cat:*)
  - Read
  - Write
  - Edit
when_to_use: |
  Use when the user asks you to learn a new ability, publish an ability, or
  invoke / remove an existing one in your own workspace. Trigger phrases:
  "learn how to <do X>", "make a <verb> ability", "save this as an ability",
  "publish <name>", "what abilities do I have", "remove the <name> ability".
---

# EasyNet Ability Authoring

You are an EasyNet agent. Every ability you own lives as one TOML file
under `<your-workspace>/abilities/<verb>.ability.toml`. The daemon
hot-reloads new files — you do not have to restart anything to make a
new ability callable. Once registered, every other agent on the same
node can call it via `easynet.invoke` (or `easynet.run`'s EAL with
the `<your-agent-name>.<verb>(...)` member-call form), and you can
call it on yourself.

## Two kinds of ability — pick the right one

EasyNet supports two execution paths, chosen by whether your
`<verb>.ability.toml` declares an `[exec]` section.

### A. Deterministic ability — `[exec] kind = "shell"` (PREFER THIS)

When the work is "run THIS exact command and return its output" —
hitting an HTTP endpoint, formatting a date, transforming text via a
trusted binary — write a `[exec]` section. The daemon spawns the argv
DIRECTLY. No LLM is in the loop, the call returns in milliseconds,
and the output is byte-for-byte identical every time.

Example (`weather.ability.toml`):

```toml
schema_version = "1"
name = "weather"
description = "Fetch the current weather for a location via wttr.in."

[input_schema]
type = "object"
additionalProperties = false
required = ["location"]
[input_schema.properties.location]
type = "string"
description = "City name, URL-safe."

[exec]
kind = "shell"
argv = [
  "curl",
  "--silent",
  "--fail",
  "--max-time", "5",
  "https://wttr.in/{{ location }}?format=%l:+%C+%t+%w+%h",
]
```

How `argv` substitution works:
- `{{ name }}` is replaced with the value of `args["name"]` for each
  call. Whitespace inside the braces is tolerated (`{{ name }}` and
  `{{name}}` both work).
- Strings substitute as their bare value; other JSON types
  substitute as JSON (e.g. `{{ count }}` with `count = 42` becomes
  `42`, with `count = [1,2]` becomes `[1,2]`).
- `argv` is a vector — each element is passed as ONE argv slot.
  Because the daemon does NOT shell-interpret the line, a value that
  contains a space or `;` cannot break out into a second token.
  Command injection is structurally impossible — no escaping needed.
- A missing `args` key referenced by `{{ name }}` errors loudly
  before the subprocess is spawned.

The envelope returned by a shell ability is:

```json
{ "result": "<stdout, utf-8 trimmed>",
  "fulfilled_by": "shell",
  "exit_code": 0,
  "elapsed_ms": 47 }
```

Fail loud: a non-zero exit code becomes a dispatch error (the
caller's `easynet.invoke` returns Err). Don't paper over upstream
failures with `|| true`.

### B. LLM-driven ability — no `[exec]` section

Use this ONLY when the work genuinely needs your reasoning — drafting
copy, summarising, classifying open-ended text. The daemon falls back
to your `<agent>.chat` handler, which embeds the manifest's
`description` field as the contract you must fulfil. This path costs
seconds (LLM cold start + tool search + reply), is non-deterministic,
and is rate-limited by your provider quota.

**Default to A. Only choose B when A genuinely cannot capture the
contract** (e.g. "summarise this article" — there is no curl that
does that).

## Steps

### 1. Discover existing abilities

Call `meta.list_abilities` (or its alias `easynet.discover`) to see
every ability registered on this node, yours and other agents'.
Cross-reference before you write — duplicating a peer's ability is
wasteful, and your callers can already invoke theirs.

### 2. Decide kind A vs kind B

Ask yourself in order:

1. Is there ONE shell command that does the whole job? → kind A.
2. Is there a single HTTP endpoint that returns the answer? → kind A
   with `argv = ["curl", ...]` until the dedicated HTTP executor
   ships.
3. Does the work require reading-then-deciding (LLM judgment)? →
   kind B.

If you are choosing B, document WHY in the manifest's `description`
so a future maintainer can revisit when a deterministic path becomes
available.

### 3. Author the manifest

Write directly to `<your-workspace>/abilities/<verb>.ability.toml`
using the `Write` tool. The verb portion of the file name (before
`.ability.toml`) MUST equal the `name` field inside.

Required top-level fields:
- `schema_version = "1"`
- `name` — bare verb, no agent prefix. The qualified name on the
  wire is built as `<your-agent-name>.<verb>` automatically.
- `description` — one paragraph. Surfaced in `meta.list_abilities`
  so other agents can decide whether to invoke you.
- `[input_schema]` — JSON Schema, top-level `type = "object"`. Be
  strict: list `required` fields, set `additionalProperties = false`,
  bound string lengths and number ranges where you can. A loose
  schema admits malformed calls that crash mid-execution.

Optional:
- `[output_schema]` — only for abilities with a typed return.
  Omit for chat-style results.
- `timeout_seconds` — per-call ceiling. Default is the daemon-wide
  setting; pin it here for an ability that must finish fast (e.g.
  health probes) or one that legitimately runs longer.
- `[exec]` — see kind A above.

### 4. Verify the manifest parses

```bash
ls -la <your-workspace>/abilities/
cat <your-workspace>/abilities/<verb>.ability.toml
```

Then call `meta.list_abilities` again — your new ability MUST appear
in the list under `<your-agent-name>.<verb>`. If it does not, the
TOML failed to parse; the daemon log (`~/.easynet/logs/easynet-daemon.log`)
will name the offending file and the parse error.

### 5. Smoke-test

Call your ability through `easynet.invoke`:

```
easynet.invoke({
  "ability": "<verb>",
  "args": { ... }   // shape per your input_schema
})
```

(Or, equivalently, ask another agent to invoke
`<your-agent-name>.<verb>` via `easynet.run`'s EAL.)

For a kind A ability, you should see `fulfilled_by: "shell"` and the
call should return in well under a second. For a kind B ability,
`fulfilled_by: "agent_chat"` and several seconds of latency are
expected.

### 6. Iterate on the contract

If a caller's args don't match your `input_schema` you'll get a
validation error before the executor runs — tighten or relax the
schema until valid calls go through and invalid ones fail clearly.
Edit the TOML in place; the next call uses the new manifest, no
restart.

### 7. Remove

Delete the file:

```bash
rm <your-workspace>/abilities/<verb>.ability.toml
```

The next call to `<your-agent-name>.<verb>` will return
`not_found`. There is no "soft delete" — be sure no caller still
needs the ability before removing it.

## Rules

- One ability = one file under `abilities/`. Do not nest, do not
  combine multiple manifests in one TOML.
- The `name` field MUST match the file stem. A drift here makes
  `<agent>.<file-stem>` and `<agent>.<name>` two different wire
  identifiers — a confusing half-state.
- Never invent MCP tool names. If a tool you want is not in
  `meta.list_abilities`, the right move is to author it (Steps 3-5),
  not to call a name and hope it routes.
- For kind A: prefer `argv` arrays to shell strings. The daemon does
  not run `sh -c`; values with spaces or metacharacters stay in one
  argv slot, by design.
- For kind B: embed the contract in `description` verbatim. The
  chat-translation path puts that text in front of the LLM as the
  brief; a vague description produces vague results.
- The agent's own `<self>.chat` is intentionally NOT exposed in the
  MCP tool list (recursion guard). Reach for sibling agents' chats
  via `easynet.invoke({ target: "<peer>", ability: "chat", ... })`.
"#;

/// Seed text for the `delegate` skill. Teaches the agent to walk a
/// three-tier discovery ladder (self abilities → other agents on the
/// same device → abilities published to the EasyNet federation)
/// before saying "I can't do that". Paired with `easynet-ability-crud`
/// (the authoring counterpart) so an agent that finds a gap can
/// either consume an existing ability or grow a new one.
///
/// Naming
/// ------
/// The skill is named `delegate` rather than `easynet-collaboration`
/// because cross-agent borrowing is only one tier of the ladder; the
/// general action is "delegate this work to a more specific tool",
/// which spans calling your own published ability, a peer's, or one
/// from the federation. A short verb-shaped name reads well in the
/// LLM's tool-picker context.
///
/// Tool names referenced inside MUST resolve on the live ability
/// registry — `<self>.discover` and `<self>.invoke` are owner-namespaced
/// per the ability-only model: every agent owns its own copy of the
/// self bundle, with `<self>` substituted by the runtime. The
/// `<self>.discover` ability accepts a `scope` argument
/// (`self | device | easynet`) and surfaces typed errors when a tier
/// is not yet wired (notably `federation_not_available` for `easynet`
/// before the federation layer ships).
const DELEGATE_SKILL_MD: &str = r#"---
name: delegate
description: Walk a three-tier discovery ladder (your own abilities → other agents on this device → abilities published to EasyNet) and delegate work to whichever existing ability fits, before falling back to "I can't do that" or a generic web search.
allowed-tools:
  - mcp__easynet
when_to_use: |
  Trigger this skill BEFORE you fall back to "I can't do that", a
  generic web search, or a fabricated answer. Concretely:
    - The user asks for live data you cannot produce on your own
      (weather, exchange rates, news, anything that needs a fresh
      fetch you don't already have a tool for).
    - The user names a domain you have no skill for ("transcribe
      audio", "render a CAD drawing", "compile this Rust").
    - The user explicitly addresses another agent ("ask codex", "have
      claude do it", "the other agent").
    - You see a task you'd normally do, but an existing published
      ability would obviously be cheaper, faster, or more precise.
  Do NOT trigger when: the user wants your opinion, references your
  own previous turn, or asks for purely conversational output.
---

# Delegate

You are part of an EasyNet device. Three tiers of abilities are
reachable from inside your chat — try them in this order before you
reach for plain reasoning or a web search.

The whole loop runs through two MCP tools, already in your tool list
under the `easynet` server:

  - `mcp__easynet__<self>.discover` — search for an ability that fits.
  - `mcp__easynet__<self>.invoke`   — call it once you've picked one.

`<self>` is your own agent name (claude, codex, …). The two tools are
yours; they delegate downstream as needed. Do NOT start a second MCP
server, do NOT shell out to a peer's CLI, do NOT assume an ability
exists without searching first.

## Discovery ladder

Walk the tiers in order; stop at the first tier that returns a usable
match.

### Tier 1 — your own abilities (fastest, fully trusted)

```
Tool: mcp__easynet__<self>.discover
Args: { "scope": "self", "query": "<short task description>" }
```

Use this first. If the user asks for something close to a verb you've
already published as `<self>.<verb>`, you don't need anyone else.

### Tier 2 — other agents on this device (fast, same user)

```
Tool: mcp__easynet__<self>.discover
Args: { "scope": "device", "query": "<short task description>" }
```

Other agents on this same device share the user's trust boundary.
Their abilities default to device-visibility, so you can see and
invoke any peer ability whose author chose `[access].visibility`
of `device` or `public`.

### Tier 3 — EasyNet federation (network call, lower trust)

```
Tool: mcp__easynet__<self>.discover
Args: { "scope": "easynet", "query": "<short task description>" }
```

Other users have published `[access].visibility = "public"` abilities
to the federation. Reaching this tier costs a network round-trip and
the publisher is outside your user's trust boundary — prefer Tier 1
or 2 when they have a usable match.

> Note: until the federation layer ships, `scope: "easynet"` returns
> the typed error `federation_not_available`. When you see that
> error, stop the ladder gracefully and fall back to telling the
> user honestly that no published ability exists for this. Do not
> fabricate.

## Discover output (all tiers share this shape)

```
Returns:
{
  "candidates": [
    {
      "qualified_name": "claude.weather",
      "owner": "claude",
      "ability": "weather",
      "description": "Fetch current weather...",
      "score": 0.93,
      "reason": "title match + tag match",
      "input_schema": { "type": "object", "required": ["location"], ... },
      "visibility": "device",
      "trust": "same_device",
      "fulfilled_by": "shell"
    },
    ...
  ]
}
```

`score` is the discover provider's relevance score (0..1). `reason` is
a short explanation. `fulfilled_by`, when present, distinguishes a
deterministic executor (`shell`, sub-second) from an agent chat
handler (`agent_chat`, several seconds, non-deterministic).

**Filtering rule (apply yourself):**

  - Pick the candidate whose `description` and `input_schema` best
    match the user's intent.
  - Skip your own `<self>.chat` — that's how callers reach you, not
    how you reach others.
  - Skip daemon-internal namespaces unless you specifically need
    them (`runtime.*`, `fleet.*`, `mcp.bridge.*`, `a2a.bridge.*`).
  - Host primitives (`fs.*`, `shell.*`, `http.*`, `process.exec`)
    are fine when you need a raw operation rather than a domain
    ability.

Tip: cache the discover result for the current turn — the registry
doesn't change mid-turn unless someone explicitly adds or removes
an ability.

## Choosing a discover provider

By default `<self>.discover` is the daemon's builtin keyword matcher
(BM25-lite over name/description/tags). If a more specialised
discover provider is installed (e.g. `userx.semantic_discover`), it
shows up in your ability list under `*.discover`. Pick the most
specific provider for your task — semantic providers handle vague
queries better; keyword providers are predictable and fast.

## Invoke

```
Tool: mcp__easynet__<self>.invoke
Args: {
  "ability": "weather",
  "target":  "claude",
  "args":    { "location": "Beijing" }
}

Returns (success):
{
  "result": "Beijing: Clear +18°C ↗5km/h 58%",
  "fulfilled_by": "shell",
  "exit_code": 0,
  "elapsed_ms": 312
}
```

Field rules:

  - `ability` — required, the bare verb portion (no agent prefix).
  - `target` — optional. Omit to call your own ability of that
    name; pass `"<peer>"` to call the peer's. The full wire name
    `<target>.<ability>` is built for you.
  - `args` — must satisfy the target ability's `input_schema`. If
    you got the schema wrong the daemon returns a validation error
    BEFORE the executor runs.

Read the inner result out of `result`. That's what you compose your
final answer around. **Do not paste the raw envelope back to the
user** — they want a natural reply, not a JSON dump.

## Decision flow

```
user asks for <X>
   │
   ├── Tier 1: <self>.discover(scope:"self", query:"<X>")
   │     │
   │     └── match? ──yes──► <self>.invoke({ ability, args })          ← fastest path
   │
   ├── Tier 2: <self>.discover(scope:"device", query:"<X>")
   │     │
   │     └── match? ──yes──► <self>.invoke({ ability, target, args })
   │
   ├── Tier 3: <self>.discover(scope:"easynet", query:"<X>")
   │     │
   │     ├── federation_not_available? ──► stop, fall through
   │     │
   │     └── match? ──yes──► <self>.invoke({ ability, target, args })
   │
   └── all tiers exhausted with no match:
         (a) attempt with your own reasoning if it's safe to do so, OR
         (b) offer to author the missing ability via the
             `easynet-ability-crud` skill, OR
         (c) tell the user honestly that no published ability exists
             for this. Do not fabricate.
```

## Example — User asks "what's the weather in Beijing?"

```
Tier 1 (you):  mcp__easynet__<self>.discover
                 { "scope": "self", "query": "weather" }
Tier 1 result: { "candidates": [] }            ← nothing of yours fits

Tier 2 (you):  mcp__easynet__<self>.discover
                 { "scope": "device", "query": "weather" }
Tier 2 result: { "candidates":
                   [{ "qualified_name": "claude.weather",
                      "owner": "claude",
                      "ability": "weather",
                      "score": 0.92,
                      "fulfilled_by": "shell",
                      "description": "Fetch current weather..." }] }

Invoke (you):  mcp__easynet__<self>.invoke
                 { "ability": "weather",
                   "target":  "claude",
                   "args":    { "location": "Beijing" } }
Invoke result: { "result": "Beijing: Clear +18°C ↗5km/h 58%",
                 "fulfilled_by": "shell",
                 "elapsed_ms": 312 }

You reply:     "Beijing is 18°C with clear skies right now."
```

## What NOT to do

- ❌ Skip the ladder. Always run `<self>.discover` before invoking;
  abilities come and go and your tool list is a stale snapshot.
- ❌ Jump straight to `scope:"easynet"`. Tier 3 is a network call to
  someone outside your user's trust boundary — earn your way there
  by exhausting Tier 1 and Tier 2 first.
- ❌ Invoke `<peer>.chat`. That's just chatting at the other agent,
  which is wasteful, recursive, and bypasses the deterministic
  ability they actually published.
- ❌ Loop discover → invoke more than ~3 times in one turn. If
  nothing fits, surface that to the user.
- ❌ Promise the user a result without actually calling. Always
  invoke before replying.
- ❌ Ignore an `Err` from `<self>.invoke`. Either retry once with
  better arguments (read the validation message), or tell the user
  the call failed and why.

## Pairing with ability authoring

If the user says "from now on, when I ask for weather, use claude" —
that's a publishing request. Hand off to `easynet-ability-crud`:
that skill walks you through writing the `<verb>.ability.toml`
manifest (with `[exec] kind = "shell"` whenever the work is a fixed
command, and `[access].visibility = "device"` when peers should be
able to discover it). The two skills compose: `delegate` consumes
the network; `easynet-ability-crud` grows it.
"#;

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
        // Ability-CRUD skill is seeded for EVERY runtime now (P6 audit
        // fix): pre-fix the seeder ran only for Codex, leaving Claude
        // workspaces with no idea that EasyNet exposed an ability
        // CRUD surface at all. Both runtimes get the same SKILL.md
        // at <root>/skills/easynet-ability-crud/SKILL.md so Claude
        // Code's --plugin-dir scan picks it up.
        assert!(root
            .join("skills/easynet-ability-crud/SKILL.md")
            .is_file());
        // The legacy Codex path is also written for both runtimes —
        // harmless duplication that keeps the seed visible no matter
        // which runtime convention the consumer follows.
        assert!(root
            .join(".agents/skills/easynet-ability-crud/SKILL.md")
            .is_file());
        // The `delegate` skill is the consume-side counterpart of
        // the CRUD (author-side) skill — both halves of the
        // discovery+publish loop must land in the same workspace so
        // an agent that can't solve a task knows to walk the
        // self → device → easynet ladder before giving up.
        assert!(root
            .join("skills/delegate/SKILL.md")
            .is_file());
        assert!(root
            .join(".agents/skills/delegate/SKILL.md")
            .is_file());
        cleanup(&root);
    }

    #[test]
    fn ensure_from_directory_for_codex_also_writes_codex_config_and_skill() {
        let (dir, root) = scratch_dir("codex-proj", RuntimeKind::Codex);
        ensure_from_directory(&dir).unwrap();
        assert!(root.join("CLAUDE.md").is_file());
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join(".codex/config.toml").is_file());
        assert!(root
            .join(".agents/skills/easynet-ability-crud/SKILL.md")
            .is_file());
        // Codex workspace also gets the Claude-style skills/ path
        // so the seed survives a runtime swap (e.g. an operator
        // reuses the workspace to test claude-code).
        assert!(root
            .join("skills/easynet-ability-crud/SKILL.md")
            .is_file());
        // Both halves of the discovery+publish pair must land
        // regardless of runtime — same rationale as the claude-code
        // test above.
        assert!(root
            .join("skills/delegate/SKILL.md")
            .is_file());
        assert!(root
            .join(".agents/skills/delegate/SKILL.md")
            .is_file());
        cleanup(&root);
    }

    #[test]
    fn ability_crud_skill_md_references_only_real_mcp_tools() {
        // The seed text MUST NOT name MCP tools that don't exist on
        // the live workspace MCP server — pre-fix the const named
        // `deploy_ability`, `run_mission`, `list_devices`, etc, none
        // of which are advertised today, so any agent following the
        // skill's instructions hit MethodNotFound on every call.
        // Audit caught this when we probed `tools/list` and compared.
        let dead = [
            "deploy_ability",
            "run_mission",
            "list_devices",
            "invoke_ability",
            "execute_command",
            "list_all_abilities",
            "uninstall_ability",
            "hub_status",
        ];
        for name in dead {
            assert!(
                !ABILITY_CRUD_SKILL_MD.contains(name),
                "seed skill still references the stale tool name {name:?}; \
                 pick a name from `easynet mcp serve --agent <n> tools/list` instead"
            );
        }
        // Sanity: keywords that MUST appear in the seed (proof the
        // new content has substance and stays aligned with the
        // current authoring path — write a `.ability.toml` directly
        // into the workspace, with `[exec]` for deterministic kinds
        // and chat fallback for LLM-driven kinds).
        for keyword in [
            "meta.list_abilities",
            ".ability.toml",
            "[exec]",
            "kind = \"shell\"",
            "argv",
            "easynet.invoke",
        ] {
            assert!(
                ABILITY_CRUD_SKILL_MD.contains(keyword),
                "seed skill must walk the agent through {keyword:?}; missing"
            );
        }
    }

    #[test]
    fn delegate_skill_md_uses_current_discovery_ladder() {
        // The skill replaces the older `easynet-collaboration` text
        // that pinned the workflow to `easynet.discover` /
        // `easynet.invoke` (a single-tier device-only model). The
        // rewrite (a) drops the AXON-RFC-001 P1.5 victims
        // (`mcp.bridge.*`) along with any remaining references to
        // the canonical-but-non-owner-namespaced `easynet.<verb>`
        // form, and (b) walks the agent through a three-tier ladder
        // (self → device → easynet) using the owner-namespaced
        // `<self>.discover` / `<self>.invoke` self bundle.
        let dead = [
            "mcp.bridge.call_tool",
            "mcp.bridge.list_tools",
            "deploy_ability",
            "run_mission",
            // Pre-rewrite the SKILL.md hardcoded `easynet.discover`
            // and `easynet.invoke`. The owner-namespaced form
            // (`<self>.discover` / `<self>.invoke`) is the canonical
            // one under the ability-only model — every agent owns
            // its own self bundle and the bare `easynet.*` names are
            // not registered as such.
            "easynet.discover",
            "easynet.invoke",
        ];
        for name in dead {
            assert!(
                !DELEGATE_SKILL_MD.contains(name),
                "delegate skill still references the stale tool {name:?}; \
                 the ladder must walk through `<self>.discover` / `<self>.invoke`"
            );
        }
        for keyword in [
            "<self>.discover",
            "<self>.invoke",
            "scope",
            "\"self\"",
            "\"device\"",
            "\"easynet\"",
            "federation_not_available",
            "candidates",
        ] {
            assert!(
                DELEGATE_SKILL_MD.contains(keyword),
                "delegate skill must walk the agent through {keyword:?}; missing"
            );
        }
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
        let dir = AgentDirectory::create(
            &Location::Local { root: root.clone() },
            spec,
        )
        .unwrap();

        ensure_from_directory(&dir).unwrap();
        let codex_toml =
            fs::read_to_string(root.join(".codex/config.toml")).unwrap();
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
        let spec = AgentSpec::from_toml_str(
            &fs::read_to_string(expected.join("agent.toml")).unwrap(),
        )
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
        let mut spec = AgentSpec::from_toml_str(
            &fs::read_to_string(&spec_path).unwrap(),
        )
        .unwrap();
        spec.description = Some("user-edited".into());
        fs::write(&spec_path, spec.to_toml_string().unwrap()).unwrap();

        // Second call must NOT overwrite the user's edit.
        ensure_workspace("alice", &entry).unwrap();
        let after = AgentSpec::from_toml_str(
            &fs::read_to_string(&spec_path).unwrap(),
        )
        .unwrap();
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
        assert!(format!("{err}").contains("half-finished")
            || format!("{err}").contains("previous"));
    }
}
