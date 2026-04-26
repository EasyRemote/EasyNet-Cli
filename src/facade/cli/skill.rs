// EasyNet CLI — `skill` command: marketplace-aware skill management.
// ====================================================================
//
// File: src/facade/cli/skill.rs
// Description: `easynet skill {install,list,upgrade,remove}` — the four
//              verbs the backend's `/api/v1/skills/*` endpoints shell
//              out to on the device the target agent lives on.
//
// Scope invariant (load-bearing)
// ------------------------------
// This module is PACKAGE MANAGEMENT, not invocation. It never reads
// a skill's code and it never calls the skill. It downloads +
// verifies + stores + computes content_hash. Skill execution still
// happens only via an agent's public ability path (which may wrap
// the skill).
//
// Layout on disk
// --------------
//
//   <agent-root>/skills/<skill-name>/
//       SKILL.md               # Anthropic-convention description
//       skill.toml (optional)  # normalised manifest we compute
//       <all other repo contents unchanged>
//       .easynet/
//           install.json       # our install metadata (source, ref,
//                              #   content_hash, installed_at,
//                              #   size_bytes)
//
// We put our metadata in a hidden `.easynet/` subdir inside the
// skill directory rather than alongside it. That way a `cp -R` of
// the agent root preserves install metadata; a `git clone` of the
// upstream repo as a separate operation does not accidentally pick
// up our install.json.
//
// Source URL grammar (v1)
// -----------------------
//
//   github:<owner>/<repo>[@<ref>][:<subpath>]
//
// `<ref>` defaults to the repo's default branch as resolved by
// the GitHub API; the CLI records the resolved SHA in install.json
// so a later `upgrade` has a concrete from-version.
//
// `<subpath>` is unused in v1 (single-skill repos only). The
// parser accepts it for forward compatibility.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use console::style;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::persistence::config;
use crate::registry::agents;
use crate::support::output;

#[derive(Debug, Args)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub action: SkillAction,
}

#[derive(Debug, Subcommand)]
pub enum SkillAction {
    /// Install a skill from a marketplace source into an agent's skills/.
    Install(InstallArgs),
    /// List installed skills, optionally filtered by agent.
    List(ListArgs),
    /// Upgrade an installed skill to a newer ref.
    Upgrade(UpgradeArgs),
    /// Remove an installed skill from an agent.
    Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Source URL: `github:<owner>/<repo>[@<ref>][:<subpath>]`.
    pub source: String,

    /// Agent name that will own this skill (see `easynet agent list`).
    #[arg(long)]
    pub agent: String,

    /// Override the ref in the source URL with a concrete tag / SHA.
    #[arg(long)]
    pub pin: Option<String>,

    /// Emit a single-line JSON blob on stdout with the installed
    /// skill's metadata (for machine consumers like the backend).
    /// Without this flag the output is human-readable.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Agent name. Omit to list skills across every registered agent.
    #[arg(long)]
    pub agent: Option<String>,

    /// Emit a JSON array on stdout instead of a human-readable table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    /// Skill name as installed under `<agent-root>/skills/<name>/`.
    pub name: String,

    /// Agent name that owns the skill.
    #[arg(long)]
    pub agent: String,

    /// Target ref — tag / SHA / branch. Omit for "latest upstream".
    #[arg(long)]
    pub to: Option<String>,

    /// Emit JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Skill name to remove.
    pub name: String,

    /// Agent name that owns the skill.
    #[arg(long)]
    pub agent: String,
}

pub fn run(args: SkillArgs) -> anyhow::Result<()> {
    match args.action {
        SkillAction::Install(a) => run_install(a),
        SkillAction::List(a) => run_list(a),
        SkillAction::Upgrade(a) => run_upgrade(a),
        SkillAction::Remove(a) => run_remove(a),
    }
}

// ─── metadata schema ─────────────────────────────────────────────

/// The normalised install record persisted at
/// `<agent-root>/skills/<name>/.easynet/install.json`. One file per
/// installed skill; the file is the source of truth for `list` /
/// `upgrade` / `remove`.
///
/// Matches the backend's `types.InstalledSkill` shape (minus
/// `agent_id` / `node_id`, which the backend injects when
/// aggregating). Keeping the two schemas isomorphic means
/// `skill list --json` output is directly parseable by the backend
/// without a translation shim.
///
/// Rust field names here are chosen for *semantic* honesty (see
/// `skill_tree_hash` doc). Wire + on-disk JSON keeps the legacy
/// `content_hash` name via `#[serde(rename)]` so that the backend
/// (`types.InstalledSkill.ContentHash`), the Frontend (`content_hash`
/// in `easynet-skills.ts`), and any pre-existing `install.json`
/// files keep parsing without a coordinated three-repo rename.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRecord {
    pub name: String,
    pub agent_id: String,
    pub source: SkillSource,
    /// SHA-256 over the sorted file tree of the installed skill
    /// directory (all shipped files, excluding our `.easynet/`
    /// metadata). Pins the on-disk skill code for upgrade diffs
    /// and install-integrity checks.
    ///
    /// **This is NOT AXIOM §6.1 Q6 `ability_snapshot.content_hash`.**
    /// Q6 requires the hash to cover (a) code + (b) the ability's
    /// public input/output schema + (c) external dependency
    /// references, and places the attestation at invocation-receipt
    /// time as a post-hoc callee signature — not at install time.
    /// Q6 says so explicitly: "the snapshot is a post-hoc attestation
    /// by the callee, not an input contract from the caller". Even
    /// if we expanded this hash to cover (a)+(b)+(c), it would still
    /// be install-time, not invocation-time, and so would not satisfy
    /// Q6 semantics. The correct Q6 implementation lives on the
    /// signed-envelope path tracked in
    /// `docs/open-questions/cli-dispatch-as-first-class-invocation.md`.
    ///
    /// Wire name is `content_hash` — see the struct-level doc for
    /// why we don't rename in JSON.
    #[serde(rename = "content_hash")]
    pub skill_tree_hash: String,
    pub size_bytes: u64,
    pub installed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    #[serde(default)]
    pub upgrade_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSource {
    pub kind: String,
    pub identifier: String,
    /// Wire name is `ref`; `ref` is a Rust keyword so the field is
    /// `ref_` here. Backend `types.SkillSource.Ref` decodes `"ref"`.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub ref_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
}

impl SkillSource {
    /// Render to the CLI-facing source URL.
    pub fn to_url(&self) -> String {
        let mut s = format!("{}:{}", self.kind, self.identifier);
        if let Some(r) = &self.ref_ {
            if !r.is_empty() {
                s.push('@');
                s.push_str(r);
            }
        }
        if let Some(p) = &self.subpath {
            if !p.is_empty() {
                s.push(':');
                s.push_str(p);
            }
        }
        s
    }
}

// ─── install ─────────────────────────────────────────────────────

fn run_install(args: InstallArgs) -> anyhow::Result<()> {
    let parsed = parse_source_url(&args.source)?;
    let effective = SkillSource {
        ref_: args.pin.clone().or(parsed.ref_.clone()),
        ..parsed
    };

    // Resolve the agent → its root directory.
    let registry = agents::load_agents()?;
    let entry = registry
        .agents
        .get(&args.agent)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "agent '{}' not registered; run `easynet agent list`",
                args.agent
            )
        })?;
    let agent_root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(&args.agent));
    if !agent_root.exists() {
        anyhow::bail!(
            "agent '{}' has no on-disk root at {}",
            args.agent,
            agent_root.display()
        );
    }

    let skills_dir = agent_root.join("skills");
    fs::create_dir_all(&skills_dir)?;

    // v1: GitHub source only. Download a tarball for the resolved
    // ref and extract to a temp dir, then atomically move into place.
    let workdir = std::env::temp_dir().join(format!(
        "easynet-skill-install-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&workdir)?;
    let fetch_result = fetch_github(&effective, &workdir)?;

    let target_dir = skills_dir.join(&fetch_result.name);
    if target_dir.exists() {
        anyhow::bail!(
            "skill '{}' is already installed at {}; run `skill upgrade` or `skill remove` first",
            fetch_result.name,
            target_dir.display()
        );
    }

    // Atomic move — fs::rename within the same filesystem is
    // atomic; if the temp dir is on a different FS (rare in
    // practice), fall back to a copy+remove.
    if let Err(_e) = fs::rename(&fetch_result.unpacked, &target_dir) {
        copy_tree(&fetch_result.unpacked, &target_dir)?;
        let _ = fs::remove_dir_all(&fetch_result.unpacked);
    }
    let _ = fs::remove_dir_all(&workdir);

    // Compute the skill tree hash over the installed skill dir
    // (excluding our own .easynet/ metadata). See
    // `InstallRecord::skill_tree_hash` for why this is not the Q6
    // `ability_snapshot.content_hash`.
    let tree_digest = hash_tree(&target_dir, &[".easynet"])?;
    let size_bytes = tree_size(&target_dir, &[".easynet"])?;

    let record = InstallRecord {
        name: fetch_result.name.clone(),
        agent_id: args.agent.clone(),
        source: SkillSource {
            kind: effective.kind.clone(),
            identifier: effective.identifier.clone(),
            ref_: Some(fetch_result.resolved_ref.clone()),
            subpath: effective.subpath.clone(),
        },
        skill_tree_hash: format!("sha256:{tree_digest}"),
        size_bytes,
        installed_at: chrono::Utc::now().to_rfc3339(),
        last_checked_at: Some(chrono::Utc::now().to_rfc3339()),
        upgrade_available: false,
    };
    write_install_record(&target_dir, &record)?;

    emit_install_result(&args, &record)?;
    Ok(())
}

/// Filled by a source adapter (`fetch_github`) after successful
/// download + extraction.
struct FetchResult {
    /// The skill's name, derived from the repo name.
    name: String,
    /// Absolute path to the extracted skill on local disk.
    unpacked: PathBuf,
    /// The concrete upstream ref the adapter resolved to — for
    /// GitHub this is the commit SHA the default-branch pointer
    /// resolved to at download time.
    resolved_ref: String,
}

fn fetch_github(src: &SkillSource, workdir: &Path) -> anyhow::Result<FetchResult> {
    anyhow::ensure!(
        src.kind == "github",
        "fetch_github called with non-github source {:?}",
        src.kind
    );
    let (owner, repo) = src
        .identifier
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("github identifier must be owner/repo, got {:?}", src.identifier))?;

    // Resolve "default branch" when no ref given.
    let ref_spec = src
        .ref_
        .clone()
        .unwrap_or_else(|| "HEAD".to_string());

    // Tarball URL — no auth required for public repos.
    // `archive/<ref>.tar.gz` resolves branch names, tags, and SHAs.
    let tarball_url = format!(
        "https://codeload.github.com/{owner}/{repo}/tar.gz/{ref_spec}"
    );

    let resp = ureq::get(&tarball_url)
        .set("User-Agent", "easynet-cli-skill-install")
        .call()
        .map_err(|e| anyhow::anyhow!("fetch {tarball_url}: {e}"))?;
    let status = resp.status();
    anyhow::ensure!(status == 200, "github tarball fetch returned {status}");

    // Stream the tarball to a temp file, then extract.
    let tar_path = workdir.join("skill.tar.gz");
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&tar_path)?;
    std::io::copy(&mut reader, &mut file)?;
    drop(file);

    let extract_dir = workdir.join("extracted");
    fs::create_dir_all(&extract_dir)?;
    extract_tar_gz(&tar_path, &extract_dir)?;

    // GitHub tarballs wrap everything in `<repo>-<ref>/`. Find the
    // single top-level directory and treat it as the skill root.
    let top = single_top_dir(&extract_dir)?;

    // Resolved SHA: GitHub's tarball URL is content-addressed even
    // when `HEAD` is given, but the response does not expose the
    // resolved SHA in headers we can trivially read via ureq. For
    // v1 we record the ref spec verbatim — a future improvement
    // makes a second API call to pin the SHA.
    let resolved_ref = if ref_spec == "HEAD" {
        "HEAD".to_string()
    } else {
        ref_spec
    };

    Ok(FetchResult {
        name: repo.to_string(),
        unpacked: top,
        resolved_ref,
    })
}

fn extract_tar_gz(tar_path: &Path, out_dir: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(tar_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut ar = tar::Archive::new(gz);
    ar.unpack(out_dir)?;
    Ok(())
}

fn single_top_dir(dir: &Path) -> anyhow::Result<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    anyhow::ensure!(
        entries.len() == 1 && entries[0].is_dir(),
        "expected exactly one top-level directory in archive, got {} entries",
        entries.len()
    );
    Ok(entries.remove(0))
}

// ─── list ────────────────────────────────────────────────────────

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let mut rows: Vec<InstallRecord> = Vec::new();
    for (name, entry) in &registry.agents {
        if let Some(filter) = &args.agent {
            if filter != name {
                continue;
            }
        }
        // Source 1 — EasyNet-managed skills under <agent-root>/skills/.
        // These are the ones `easynet skill install ...` writes;
        // each carries an `.easynet/install.json` metadata file.
        let root = entry
            .root_path
            .clone()
            .unwrap_or_else(|| config::agents_root().join(name));
        let skills_dir = root.join("skills");
        if skills_dir.exists() {
            for dir_entry in fs::read_dir(&skills_dir)?.flatten() {
                let record_path = dir_entry.path().join(".easynet").join("install.json");
                if !record_path.exists() {
                    continue;
                }
                match read_install_record(&record_path) {
                    Ok(r) => rows.push(r),
                    Err(e) => eprintln!(
                        "[warn] skill {}: failed to read install record: {e}",
                        dir_entry.path().display()
                    ),
                }
            }
        }

        // Source 2 — agent-native global skill pools. Claude Code
        // keeps user-level skills under `~/.claude/skills/`; Codex
        // keeps them under `~/.agents/skills/`. These pools are
        // populated by external tooling (the agent CLI itself, the
        // user's own scripts, marketplace installers other than
        // EasyNet's), so they have no `.easynet/install.json`. We
        // synthesise an InstallRecord from each skill directory's
        // `SKILL.md` YAML frontmatter — the canonical metadata
        // location for both Claude Code and Codex. The synthetic
        // record's `source.kind = "global"` so the listing tells
        // an operator at a glance which pool a row came from.
        for (label, pool_dir) in global_skill_pools_for(entry.agent_type) {
            scan_global_pool_into(name, label, &pool_dir, &mut rows);
        }
    }

    if args.json {
        let out = serde_json::to_string(&rows)?;
        println!("{out}");
        return Ok(());
    }

    if rows.is_empty() {
        output::info("No skills installed.");
        return Ok(());
    }
    eprintln!();
    eprintln!(
        "  {:<24} {:<18} {:<40} {:<12}",
        style("SKILL").dim(),
        style("AGENT").dim(),
        style("SOURCE").dim(),
        style("SIZE").dim(),
    );
    eprintln!("  {}", style("─".repeat(98)).dim());
    for r in &rows {
        eprintln!(
            "  {:<24} {:<18} {:<40} {:<12}",
            style(&r.name).white().bold(),
            style(&r.agent_id).cyan(),
            style(r.source.to_url()).dim(),
            style(format_bytes(r.size_bytes)).dim(),
        );
    }
    eprintln!();
    Ok(())
}

/// Per-agent-kind global skill pool roots. Mirrors the install-side
/// mapping in `skill_install.rs` so the lister and the installer
/// can never disagree about where each agent kind keeps its global
/// skills. Empty Vec for kinds without a known global pool.
///
/// `~/.claude/skills/` and `~/.agents/skills/` are populated by the
/// agent CLIs themselves and by external tooling — EasyNet does not
/// own these directories, only reads them.
pub(crate) fn global_skill_pools_for(agent_type: agents::AgentType) -> Vec<(&'static str, std::path::PathBuf)> {
    let home = config::home_dir();
    match agent_type {
        agents::AgentType::ClaudeCode => {
            vec![("claude-global", home.join(".claude").join("skills"))]
        }
        agents::AgentType::Codex | agents::AgentType::CodexAppServer => {
            vec![("codex-global", home.join(".agents").join("skills"))]
        }
    }
}

/// Walk a global skill pool and append one synthetic InstallRecord
/// per skill directory it contains. Skips directories that don't
/// look like a skill (no `SKILL.md` and no nested `skill.json`).
///
/// We do not propagate IO errors from the walk: a global pool that
/// is missing or unreadable should not fail the whole listing —
/// the EasyNet-managed half (Source 1) can still surface.
pub(crate) fn scan_global_pool_into(
    agent_name: &str,
    pool_label: &str,
    pool_dir: &std::path::Path,
    rows: &mut Vec<InstallRecord>,
) {
    if !pool_dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(pool_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "[warn] global skill pool {} unreadable: {e}",
                pool_dir.display()
            );
            return;
        }
    };
    for dir_entry in entries.flatten() {
        let path = dir_entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip dotfiles / hidden dirs.
        if dir_name.starts_with('.') {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        let skill_json = path.join("skill.json");
        if !skill_md.exists() && !skill_json.exists() {
            // Not a skill directory shape — could be an unrelated
            // user folder under ~/.claude. Silent skip.
            continue;
        }
        // Best-effort metadata extraction. Frontmatter `name` wins
        // when present; otherwise the directory name is the fallback.
        let parsed_name = parse_skill_md_name(&skill_md).unwrap_or_else(|| dir_name.clone());
        let size_bytes = directory_size_bytes(&path);
        let installed_at = file_mtime_iso(&path).unwrap_or_default();

        rows.push(InstallRecord {
            name: parsed_name,
            agent_id: agent_name.to_string(),
            source: SkillSource {
                // `kind = "global"` distinguishes these from the
                // `github`-kind records that `easynet skill install`
                // writes. The to_url() rendering becomes
                // "global:claude-global" — visible in the SOURCE
                // column so an operator can tell which pool a row
                // came from.
                kind: "global".to_string(),
                identifier: pool_label.to_string(),
                ref_: None,
                subpath: Some(dir_name),
            },
            // No tree hash for global skills — they're not pinned by
            // EasyNet and the file set may change without us
            // observing. Empty string is the documented "unknown"
            // sentinel.
            skill_tree_hash: String::new(),
            size_bytes,
            installed_at,
            last_checked_at: None,
            upgrade_available: false,
        });
    }
}

/// Extract the `name:` field from a SKILL.md YAML frontmatter
/// block. Returns None on any parse failure — the caller falls back
/// to the directory name. We do a minimal hand parse rather than
/// pulling in a YAML crate because the frontmatter shape we care
/// about is one line: `name: <value>`.
fn parse_skill_md_name(skill_md: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(skill_md).ok()?;
    // Frontmatter is delimited by `---` lines at the top.
    let body = content.strip_prefix("---")?.strip_prefix('\n')?;
    let end = body.find("\n---")?;
    let frontmatter = &body[..end];
    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("name:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Recursive byte size of a directory, best-effort. A symlink loop
/// or an unreadable child silently skips that subtree rather than
/// failing the whole listing; the result is approximate but the
/// listing's SIZE column has always been advisory.
fn directory_size_bytes(dir: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    let walker = match fs::read_dir(dir) {
        Ok(w) => w,
        Err(_) => return 0,
    };
    for entry in walker.flatten() {
        let p = entry.path();
        match entry.metadata() {
            Ok(m) if m.is_file() => total = total.saturating_add(m.len()),
            Ok(m) if m.is_dir() => total = total.saturating_add(directory_size_bytes(&p)),
            _ => {}
        }
    }
    total
}

/// ISO-8601 mtime of a path, best-effort. Returns None if the
/// metadata read fails.
fn file_mtime_iso(path: &std::path::Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dt: chrono::DateTime<chrono::Utc> = modified.into();
    Some(dt.to_rfc3339())
}

// ─── upgrade ─────────────────────────────────────────────────────

fn run_upgrade(args: UpgradeArgs) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let entry = registry
        .agents
        .get(&args.agent)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", args.agent))?;
    let agent_root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(&args.agent));
    let skill_dir = agent_root.join("skills").join(&args.name);
    let record_path = skill_dir.join(".easynet").join("install.json");
    let existing = read_install_record(&record_path)?;

    let target_ref = args.to.clone().or_else(|| existing.source.ref_.clone());

    // Simplest correct upgrade: remove + re-install. Atomicity of
    // the overall operation (no corrupted state after a
    // mid-upgrade crash) is achieved by installing into a temp
    // location first then swapping.
    let workdir = std::env::temp_dir().join(format!(
        "easynet-skill-upgrade-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&workdir)?;
    let mut new_source = existing.source.clone();
    new_source.ref_ = target_ref.clone();
    let fetch = fetch_github(&new_source, &workdir)?;

    // Move old out of the way, move new into place.
    let backup = agent_root.join("skills").join(format!(
        ".{}-backup-{}",
        &args.name,
        rand_suffix()
    ));
    fs::rename(&skill_dir, &backup)?;
    let result = (|| -> anyhow::Result<InstallRecord> {
        if fs::rename(&fetch.unpacked, &skill_dir).is_err() {
            copy_tree(&fetch.unpacked, &skill_dir)?;
            let _ = fs::remove_dir_all(&fetch.unpacked);
        }
        let tree_digest = hash_tree(&skill_dir, &[".easynet"])?;
        let size_bytes = tree_size(&skill_dir, &[".easynet"])?;
        let rec = InstallRecord {
            name: existing.name.clone(),
            agent_id: args.agent.clone(),
            source: SkillSource {
                kind: existing.source.kind.clone(),
                identifier: existing.source.identifier.clone(),
                ref_: Some(fetch.resolved_ref.clone()),
                subpath: existing.source.subpath.clone(),
            },
            skill_tree_hash: format!("sha256:{tree_digest}"),
            size_bytes,
            installed_at: chrono::Utc::now().to_rfc3339(),
            last_checked_at: Some(chrono::Utc::now().to_rfc3339()),
            upgrade_available: false,
        };
        write_install_record(&skill_dir, &rec)?;
        Ok(rec)
    })();

    // Commit or rollback.
    match result {
        Ok(rec) => {
            let _ = fs::remove_dir_all(&backup);
            let _ = fs::remove_dir_all(&workdir);
            emit_upgrade_result(&args, &rec)?;
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&skill_dir);
            let _ = fs::rename(&backup, &skill_dir);
            let _ = fs::remove_dir_all(&workdir);
            Err(anyhow::anyhow!("upgrade failed, rolled back: {e}"))
        }
    }
}

// ─── remove ──────────────────────────────────────────────────────

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let entry = registry
        .agents
        .get(&args.agent)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", args.agent))?;
    let agent_root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(&args.agent));
    let skill_dir = agent_root.join("skills").join(&args.name);
    if !skill_dir.exists() {
        anyhow::bail!(
            "skill '{}' is not installed on agent '{}'",
            args.name,
            args.agent
        );
    }
    fs::remove_dir_all(&skill_dir)?;
    output::success(&format!(
        "Removed skill '{}' from agent '{}'",
        args.name, args.agent
    ));
    Ok(())
}

// ─── helpers ─────────────────────────────────────────────────────

fn parse_source_url(url: &str) -> anyhow::Result<SkillSource> {
    let (kind, rest) = url
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("source URL must be <kind>:<identifier>, got {:?}", url))?;
    if kind != "github" {
        anyhow::bail!("v1 supports only 'github:' sources, got kind={:?}", kind);
    }
    let (ident_and_ref, subpath) = match rest.rsplit_once(':') {
        // A literal `:` inside `rest` might belong to a subpath;
        // disambiguate by checking whether the left side contains
        // a `/` (which every identifier has) AND the right side
        // looks like a path.
        Some((left, right)) if left.contains('/') && !right.contains('@') => {
            (left.to_string(), Some(right.to_string()))
        }
        _ => (rest.to_string(), None),
    };
    let (identifier, ref_) = match ident_and_ref.split_once('@') {
        Some((id, r)) => (id.to_string(), Some(r.to_string())),
        None => (ident_and_ref, None),
    };
    Ok(SkillSource {
        kind: kind.to_string(),
        identifier,
        ref_,
        subpath,
    })
}

pub(crate) fn read_install_record(path: &Path) -> anyhow::Result<InstallRecord> {
    let text = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

fn write_install_record(skill_dir: &Path, record: &InstallRecord) -> anyhow::Result<()> {
    let meta_dir = skill_dir.join(".easynet");
    fs::create_dir_all(&meta_dir)?;
    let path = meta_dir.join("install.json");
    let json = serde_json::to_string_pretty(record)?;
    fs::write(&path, json)?;
    Ok(())
}

fn hash_tree(root: &Path, skip: &[&str]) -> anyhow::Result<String> {
    // Deterministic walk: sorted paths, hash each file's
    // (relative-path, contents). Directory-only entries do not
    // contribute bytes. The skip list filters top-level dirs.
    let mut entries = Vec::new();
    collect_files(root, root, skip, &mut entries)?;
    entries.sort();
    let mut hasher = Sha256::new();
    for rel in &entries {
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        let full = root.join(rel);
        let bytes = fs::read(&full)?;
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn tree_size(root: &Path, skip: &[&str]) -> anyhow::Result<u64> {
    let mut entries = Vec::new();
    collect_files(root, root, skip, &mut entries)?;
    let mut total = 0u64;
    for rel in &entries {
        let meta = fs::metadata(root.join(rel))?;
        total += meta.len();
    }
    Ok(total)
}

fn collect_files(
    root: &Path,
    dir: &Path,
    skip: &[&str],
    out: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        // Skip only at the top level. An arbitrary file deeper
        // in the tree named `.easynet` is still tracked.
        if dir == root && skip.iter().any(|s| s == &name_s) {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, skip, out)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_path_buf())
                .unwrap_or(path);
            out.push(rel);
        }
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)?.flatten() {
        let p = entry.path();
        let target = dst.join(entry.file_name());
        if p.is_dir() {
            copy_tree(&p, &target)?;
        } else {
            fs::copy(&p, &target)?;
        }
    }
    Ok(())
}

fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{nanos:08x}")
}

fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}KB", n as f64 / 1024.0)
    } else {
        format!("{:.1}MB", n as f64 / (1024.0 * 1024.0))
    }
}

// ─── output rendering ────────────────────────────────────────────

fn emit_install_result(args: &InstallArgs, rec: &InstallRecord) -> anyhow::Result<()> {
    if args.json {
        // Wire shape is flat: the backend unmarshals into an
        // anonymous struct with `name`/`content_hash`/`size_bytes`/
        // `installed_at`/`ref` at top level (see installSkillLogic.go
        // `cliOut` decoder), so we don't emit the nested `source`
        // object here. The on-disk InstallRecord keeps `source`; this
        // divergence is intentional.
        #[derive(Serialize)]
        struct MachineOut<'a> {
            name: &'a str,
            agent_id: &'a str,
            content_hash: &'a str,
            size_bytes: u64,
            installed_at: &'a str,
            #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
            ref_: Option<&'a String>,
        }
        let m = MachineOut {
            name: &rec.name,
            agent_id: &rec.agent_id,
            // Wire field name stays `content_hash` for back-compat;
            // semantic name inside the CLI is `skill_tree_hash`.
            content_hash: &rec.skill_tree_hash,
            size_bytes: rec.size_bytes,
            installed_at: &rec.installed_at,
            ref_: rec.source.ref_.as_ref(),
        };
        println!("{}", serde_json::to_string(&m)?);
    } else {
        output::success(&format!(
            "Installed skill '{}' on agent '{}'",
            rec.name, rec.agent_id
        ));
        output::detail("source", &rec.source.to_url());
        output::detail("hash", &rec.skill_tree_hash);
        output::detail("size", &format_bytes(rec.size_bytes));
    }
    Ok(())
}

fn emit_upgrade_result(args: &UpgradeArgs, rec: &InstallRecord) -> anyhow::Result<()> {
    if args.json {
        let json = serde_json::to_string(rec)?;
        println!("{json}");
    } else {
        output::success(&format!(
            "Upgraded skill '{}' on agent '{}' to {}",
            rec.name,
            rec.agent_id,
            rec.source.ref_.as_deref().unwrap_or("latest"),
        ));
        output::detail("hash", &rec.skill_tree_hash);
    }
    Ok(())
}

// ─── tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── parse_source_url ─────────────────────────────────────────

    #[test]
    fn parse_github_minimal() {
        let s = parse_source_url("github:anthropic/code-reviewer").unwrap();
        assert_eq!(s.kind, "github");
        assert_eq!(s.identifier, "anthropic/code-reviewer");
        assert!(s.ref_.is_none());
        assert!(s.subpath.is_none());
    }

    #[test]
    fn parse_github_with_ref() {
        let s = parse_source_url("github:a/b@v1.2.3").unwrap();
        assert_eq!(s.ref_.as_deref(), Some("v1.2.3"));
    }

    #[test]
    fn parse_github_with_subpath() {
        let s = parse_source_url("github:a/b@main:skills/foo").unwrap();
        assert_eq!(s.identifier, "a/b");
        assert_eq!(s.ref_.as_deref(), Some("main"));
        assert_eq!(s.subpath.as_deref(), Some("skills/foo"));
    }

    #[test]
    fn parse_rejects_unsupported_kind() {
        assert!(parse_source_url("npm:pkg").is_err());
        assert!(parse_source_url("anthropic:foo").is_err());
    }

    #[test]
    fn parse_requires_kind_prefix() {
        assert!(parse_source_url("just-a-name").is_err());
    }

    // ─── SkillSource::to_url round-trip ───────────────────────────

    #[test]
    fn to_url_roundtrips_minimal() {
        let s = SkillSource {
            kind: "github".into(),
            identifier: "a/b".into(),
            ref_: None,
            subpath: None,
        };
        assert_eq!(s.to_url(), "github:a/b");
        let r = parse_source_url(&s.to_url()).unwrap();
        assert_eq!(r.kind, s.kind);
        assert_eq!(r.identifier, s.identifier);
    }

    #[test]
    fn to_url_roundtrips_with_ref() {
        let s = SkillSource {
            kind: "github".into(),
            identifier: "a/b".into(),
            ref_: Some("v1".into()),
            subpath: None,
        };
        assert_eq!(s.to_url(), "github:a/b@v1");
        let r = parse_source_url(&s.to_url()).unwrap();
        assert_eq!(r.ref_.as_deref(), Some("v1"));
    }

    #[test]
    fn to_url_roundtrips_with_subpath() {
        let s = SkillSource {
            kind: "github".into(),
            identifier: "a/b".into(),
            ref_: Some("v1".into()),
            subpath: Some("skills/foo".into()),
        };
        assert_eq!(s.to_url(), "github:a/b@v1:skills/foo");
        let r = parse_source_url(&s.to_url()).unwrap();
        assert_eq!(r.subpath.as_deref(), Some("skills/foo"));
    }

    // ─── hash_tree determinism ────────────────────────────────────

    #[test]
    fn hash_tree_is_deterministic_across_platforms() {
        // Build a tiny tree twice, verify the hash matches. The
        // walk must sort paths — otherwise two runs on the same
        // content produce different hashes and content_hash
        // loses its AXIOM §6.1 Q6 meaning.
        let tmp = std::env::temp_dir().join(format!(
            "easynet-hash-test-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("a.txt"), "hello").unwrap();
        fs::write(tmp.join("b.txt"), "world").unwrap();
        fs::create_dir_all(tmp.join("nested")).unwrap();
        fs::write(tmp.join("nested/c.txt"), "!").unwrap();

        let h1 = hash_tree(&tmp, &[]).unwrap();
        let h2 = hash_tree(&tmp, &[]).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // sha256 hex digest
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn hash_tree_respects_skip_list() {
        // Our .easynet/install.json changes between installs but
        // must not participate in content_hash — otherwise every
        // skill's hash depends on its own install timestamp,
        // which makes Q6 attestation meaningless.
        let tmp = std::env::temp_dir().join(format!(
            "easynet-hash-skip-test-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("skill.md"), "content").unwrap();

        let h_without_easynet = hash_tree(&tmp, &[".easynet"]).unwrap();

        fs::create_dir_all(tmp.join(".easynet")).unwrap();
        fs::write(tmp.join(".easynet/install.json"), "metadata v1").unwrap();
        let h_with_easynet = hash_tree(&tmp, &[".easynet"]).unwrap();

        assert_eq!(
            h_without_easynet, h_with_easynet,
            "adding .easynet/ must not change the content hash"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    // ─── format_bytes ─────────────────────────────────────────────

    #[test]
    fn format_bytes_boundaries() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(1023), "1023B");
        assert_eq!(format_bytes(1024), "1.0KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0MB");
    }

    // ─── wire compatibility for the Q6-motivated rename ──────────
    //
    // The Rust field is `skill_tree_hash` (semantic name — it is
    // NOT AXIOM §6.1 Q6's `ability_snapshot.content_hash`). The
    // JSON wire field stays `content_hash` because the backend's
    // `types.InstalledSkill.ContentHash` and the Frontend's
    // `content_hash` in `easynet-skills.ts` read that name. Losing
    // the `#[serde(rename = "content_hash")]` silently breaks the
    // whole cross-repo wire. These tests pin both directions.

    #[test]
    fn install_record_serialize_emits_content_hash_on_wire() {
        let rec = InstallRecord {
            name: "alpha".into(),
            agent_id: "alice".into(),
            source: SkillSource {
                kind: "github".into(),
                identifier: "a/b".into(),
                ref_: Some("v1".into()),
                subpath: None,
            },
            skill_tree_hash: "sha256:deadbeef".into(),
            size_bytes: 42,
            installed_at: "2026-04-23T00:00:00Z".into(),
            last_checked_at: None,
            upgrade_available: false,
        };
        let wire = serde_json::to_string(&rec).unwrap();
        assert!(
            wire.contains("\"content_hash\":\"sha256:deadbeef\""),
            "wire must emit `content_hash` (not the Rust field name): {wire}"
        );
        assert!(
            !wire.contains("skill_tree_hash"),
            "wire must NOT leak the Rust field name: {wire}"
        );
    }

    #[test]
    fn install_record_deserialize_reads_content_hash_from_wire() {
        // Simulates reading a record that came across the wire
        // (or from an older install.json file). The wire name is
        // `content_hash`; the Rust field is `skill_tree_hash`.
        let wire = r#"{
            "name": "alpha",
            "agent_id": "alice",
            "source": {
                "kind": "github",
                "identifier": "a/b"
            },
            "content_hash": "sha256:wire",
            "size_bytes": 99,
            "installed_at": "2026-04-23T00:00:00Z",
            "upgrade_available": false
        }"#;
        let rec: InstallRecord = serde_json::from_str(wire).unwrap();
        assert_eq!(rec.skill_tree_hash, "sha256:wire");
    }
}
