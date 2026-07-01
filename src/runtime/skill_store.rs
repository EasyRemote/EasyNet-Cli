// EasyNet CLI — runtime skill package store
// ==========================================
//
// File: src/runtime/skill_store.rs
// Description: Canonical filesystem implementation behind skill.* abilities.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::persistence::config;
use crate::registry::agents;

const GLOBAL_SKILL_OWNER_PREFIX: &str = "global:";

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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
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

/// Pure install helper: fetches the source, atomically moves into
/// the agent's skills/ dir, writes the install record, and returns
/// it. No stdout, no CLI dep. Used by `run_install` (CLI) and
/// `skill.install` ability (daemon ability dispatch).
///
/// `pub(crate)` because the only callers are in this crate
/// (run_install in this file + skill_install_ability handler in
/// runtime/agents). Public visibility would invite external
/// callers to bind to a helper that exists for in-tree wiring,
/// not as a stable downstream API.
///
/// Errors:
///   * agent not registered
///   * skill already installed (caller should run upgrade/remove first)
///   * fetch / unpack failures from `fetch_github`
///
/// Atomicity: fs::rename within the same filesystem is atomic; if
/// the temp dir is on a different FS the fall-back copy+remove is
/// not atomic but is at least all-or-nothing at the directory level.
pub(crate) fn install_skill(
    source: &str,
    agent: &str,
    pin: Option<&str>,
) -> anyhow::Result<InstallRecord> {
    let parsed = parse_source_url(source)?;
    let effective = SkillSource {
        ref_: pin.map(|s| s.to_string()).or(parsed.ref_.clone()),
        ..parsed
    };

    let registry = agents::load_agents()?;
    let entry = registry.agents.get(agent).ok_or_else(|| {
        anyhow::anyhow!("agent '{}' not registered; run 'easynet agent list'", agent)
    })?;
    let agent_root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(agent));
    if !agent_root.exists() {
        anyhow::bail!(
            "agent '{}' has no on-disk root at {}",
            agent,
            agent_root.display()
        );
    }

    let skills_dir = agent_root.join("skills");
    fs::create_dir_all(&skills_dir)?;

    // Workdir wrapped in an RAII guard so it's removed on every
    // exit path — including the early-return cases below
    // (target_dir-already-exists, fetch_github failure surfaced
    // via `?`). Pre-fix, fetch_github failures leaked the temp
    // dir; the guard makes cleanup unconditional.
    let workdir = TempDirGuard::create("easynet-skill-install")?;
    let fetch_result = fetch_github(&effective, workdir.path())?;

    let target_dir = skills_dir.join(&fetch_result.name);
    if target_dir.exists() {
        anyhow::bail!(
            "skill '{}' is already installed at {}; run 'skill upgrade' or 'skill remove' first",
            fetch_result.name,
            target_dir.display()
        );
    }

    if let Err(_e) = fs::rename(&fetch_result.unpacked, &target_dir) {
        copy_tree(&fetch_result.unpacked, &target_dir)?;
        let _ = fs::remove_dir_all(&fetch_result.unpacked);
    }
    // workdir cleanup happens in TempDirGuard's Drop.

    let tree_digest = hash_tree(&target_dir, &[".easynet"])?;
    let size_bytes = tree_size(&target_dir, &[".easynet"])?;

    let record = InstallRecord {
        name: fetch_result.name.clone(),
        description: skill_description_from_dir(&target_dir),
        agent_id: agent.to_string(),
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
    Ok(record)
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
    let (owner, repo) = src.identifier.split_once('/').ok_or_else(|| {
        anyhow::anyhow!(
            "github identifier must be owner/repo, got {:?}",
            src.identifier
        )
    })?;

    // Resolve "default branch" when no ref given.
    let ref_spec = src.ref_.clone().unwrap_or_else(|| "HEAD".to_string());

    // Tarball URL — no auth required for public repos.
    // `archive/<ref>.tar.gz` resolves branch names, tags, and SHAs.
    let tarball_url = format!("https://codeload.github.com/{owner}/{repo}/tar.gz/{ref_spec}");

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

pub(crate) fn global_skill_pools_for(
    agent_type: agents::AgentType,
) -> Vec<(&'static str, std::path::PathBuf)> {
    let home = config::home_dir();
    match agent_type {
        agents::AgentType::ClaudeCode => {
            vec![("claude-global", home.join(".claude").join("skills"))]
        }
        agents::AgentType::Codex | agents::AgentType::CodexAppServer => {
            vec![("codex-global", home.join(".agents").join("skills"))]
        }
        agents::AgentType::External => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalSkillPoolRef {
    label: String,
}

impl GlobalSkillPoolRef {
    pub(crate) fn parse_owner_id(owner_id: &str, verb: &str) -> anyhow::Result<Option<Self>> {
        let Some(label) = owner_id.strip_prefix(GLOBAL_SKILL_OWNER_PREFIX) else {
            return Ok(None);
        };
        Ok(Some(Self::from_label(label, verb)?))
    }

    pub(crate) fn from_label(label: &str, verb: &str) -> anyhow::Result<Self> {
        let label = label.trim();
        if label.is_empty() {
            anyhow::bail!("{verb}: global skill owner must include a pool label");
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            anyhow::bail!(
                "{verb}: global skill pool label {label:?} must use ASCII alphanumeric, '-' or '_'"
            );
        }
        let this = Self {
            label: label.to_string(),
        };
        if this.dirs().is_empty() {
            anyhow::bail!("{verb}: unknown global skill pool {label:?}");
        }
        Ok(this)
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn owner_agent_id(&self) -> String {
        format!("{GLOBAL_SKILL_OWNER_PREFIX}{}", self.label)
    }

    pub(crate) fn dirs(&self) -> Vec<std::path::PathBuf> {
        global_skill_pool_dirs_for_label(&self.label)
    }

    pub(crate) fn skill_dir(&self, skill_name: &str) -> Option<std::path::PathBuf> {
        for pool_dir in self.dirs() {
            if let Some(path) = skill_dir_in_global_pool(&pool_dir, skill_name) {
                return Some(path);
            }
        }
        None
    }
}

pub(crate) fn global_skill_dir_for(
    agent_type: agents::AgentType,
    skill_name: &str,
) -> Option<std::path::PathBuf> {
    for (_label, pool_dir) in global_skill_pools_for(agent_type) {
        if let Some(path) = skill_dir_in_global_pool(&pool_dir, skill_name) {
            return Some(path);
        }
    }
    None
}

fn global_skill_pool_dirs_for_label(pool_label: &str) -> Vec<std::path::PathBuf> {
    let agent_types = [
        agents::AgentType::ClaudeCode,
        agents::AgentType::Codex,
        agents::AgentType::CodexAppServer,
        agents::AgentType::External,
    ];
    let mut dirs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for agent_type in agent_types {
        for (label, pool_dir) in global_skill_pools_for(agent_type) {
            if label != pool_label || !seen.insert(pool_dir.clone()) {
                continue;
            }
            dirs.push(pool_dir);
        }
    }
    dirs
}

fn skill_dir_in_global_pool(pool_dir: &Path, skill_name: &str) -> Option<std::path::PathBuf> {
    let direct = pool_dir.join(skill_name);
    if direct
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| !name.starts_with('.'))
        && direct.is_dir()
        && looks_like_skill_dir(&direct)
    {
        return Some(direct);
    }

    let entries = fs::read_dir(pool_dir).ok()?;
    for dir_entry in entries.flatten() {
        let path = dir_entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        if !looks_like_skill_dir(&path) {
            continue;
        }
        let parsed_name =
            parse_skill_md_name(&path.join("SKILL.md")).unwrap_or_else(|| dir_name.clone());
        if parsed_name == skill_name || dir_name == skill_name {
            return Some(path);
        }
    }
    None
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
        if !looks_like_skill_dir(&path) {
            // Not a skill directory shape — could be an unrelated
            // user folder under ~/.claude. Silent skip.
            continue;
        }
        if let Some(record) = global_skill_record_from_dir(agent_name, pool_label, &path) {
            rows.push(record);
        }
    }
}

pub(crate) fn global_skill_record_from_dir(
    agent_name: &str,
    pool_label: &str,
    path: &std::path::Path,
) -> Option<InstallRecord> {
    if !path.is_dir() || !looks_like_skill_dir(path) {
        return None;
    }
    let dir_name = path.file_name().and_then(|s| s.to_str())?;
    if dir_name.starts_with('.') {
        return None;
    }
    // Best-effort metadata extraction. Frontmatter `name` wins
    // when present; otherwise the directory name is the fallback.
    let skill_md = path.join("SKILL.md");
    let parsed_name = parse_skill_md_name(&skill_md).unwrap_or_else(|| dir_name.to_string());
    let size_bytes = directory_size_bytes(path);
    let installed_at = file_mtime_iso(path).unwrap_or_default();

    Some(InstallRecord {
        name: parsed_name,
        description: skill_description_from_dir(path),
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
            subpath: Some(dir_name.to_string()),
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
    })
}

fn looks_like_skill_dir(path: &std::path::Path) -> bool {
    path.join("SKILL.md").exists() || path.join("skill.json").exists()
}

pub(crate) fn skill_description_from_dir(path: &std::path::Path) -> String {
    skill_description_from_markdown(&path.join("SKILL.md")).unwrap_or_default()
}

fn skill_description_from_markdown(path: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    if let Some(frontmatter) = skill_md_frontmatter(&content) {
        if let Some(description) = frontmatter_field(frontmatter, "description") {
            return Some(description);
        }
    }
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("---"))
        .map(|line| line.to_string())
}

/// Extract the `name:` field from a SKILL.md YAML frontmatter
/// block. Returns None on any parse failure — the caller falls back
/// to the directory name. We do a minimal hand parse rather than
/// pulling in a YAML crate because the frontmatter shape we care
/// about is one line: `name: <value>`.
fn parse_skill_md_name(skill_md: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(skill_md).ok()?;
    // Frontmatter is delimited by `---` lines at the top.
    let frontmatter = skill_md_frontmatter(&content)?;
    frontmatter_field(frontmatter, "name")
}

fn skill_md_frontmatter(content: &str) -> Option<&str> {
    let body = content.strip_prefix("---")?.strip_prefix('\n')?;
    let end = body.find("\n---")?;
    Some(&body[..end])
}

fn frontmatter_field(frontmatter: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
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

/// new ref into place, and returns the new InstallRecord on success.
/// On failure the backup is restored — the caller never observes a
/// half-upgraded state. No stdout, no CLI dep. Used by `run_upgrade`
/// (CLI) and `skill.upgrade` ability.
///
/// `pub(crate)` for the same reason as install_skill.
///
/// `target_ref`:
///   * `Some("v1.2.3")` — pin to a specific tag/SHA/branch
///   * `None` — track upstream HEAD (whatever fetch_github resolves)
pub(crate) fn upgrade_skill(
    name: &str,
    agent: &str,
    target_ref: Option<&str>,
) -> anyhow::Result<InstallRecord> {
    let registry = agents::load_agents()?;
    let entry = registry
        .agents
        .get(agent)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", agent))?;
    let agent_root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(agent));
    let skill_dir = agent_root.join("skills").join(name);
    let record_path = skill_dir.join(".easynet").join("install.json");
    let existing = read_install_record(&record_path)?;

    let resolved_target_ref = target_ref
        .map(|s| s.to_string())
        .or_else(|| existing.source.ref_.clone());

    // Workdir wrapped in TempDirGuard so cleanup happens on every
    // exit — including the early-return inside fetch_github (which
    // pre-fix leaked the temp dir on network failure).
    let workdir = TempDirGuard::create("easynet-skill-upgrade")?;
    let mut new_source = existing.source.clone();
    new_source.ref_ = resolved_target_ref.clone();
    let fetch = fetch_github(&new_source, workdir.path())?;

    let backup = agent_root
        .join("skills")
        .join(format!(".{}-backup-{}", name, rand_suffix()));
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
            description: skill_description_from_dir(&skill_dir),
            agent_id: agent.to_string(),
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

    match result {
        Ok(rec) => {
            let _ = fs::remove_dir_all(&backup);
            // workdir cleanup happens in TempDirGuard's Drop.
            Ok(rec)
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&skill_dir);
            let _ = fs::rename(&backup, &skill_dir);
            // workdir cleanup happens in TempDirGuard's Drop.
            Err(anyhow::anyhow!("upgrade failed, rolled back: {e}"))
        }
    }
}

/// Pure remove helper: deletes the skill directory and returns Ok
/// when the skill was present and the delete succeeded. No stdout,
/// no CLI dep. Used by `run_remove` (CLI) and `skill.remove`
/// ability.
///
/// `pub(crate)` for the same reason as install_skill.
///
/// Errors:
///   * agent not registered
///   * skill not installed (caller can choose to treat this as
///     idempotent at the ability layer if desired; we surface the
///     distinction here)
pub(crate) fn remove_skill(name: &str, agent: &str) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let entry = registry
        .agents
        .get(agent)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", agent))?;
    let agent_root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(agent));
    let skill_dir = agent_root.join("skills").join(name);
    if !skill_dir.exists() {
        anyhow::bail!("skill '{}' is not installed on agent '{}'", name, agent);
    }
    fs::remove_dir_all(&skill_dir)?;
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
    let text =
        fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
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

/// RAII wrapper around a temp directory used for fetch+stage of a
/// skill. Removal happens in `Drop` so every exit path from
/// install_skill / upgrade_skill — including `?` short-circuits
/// from fetch_github — runs the cleanup.
///
/// Pre-fix the temp dir was created with `fs::create_dir_all` and
/// removed manually at the end, so a failure inside fetch_github
/// (network error / bad ref / etc.) leaked the directory on every
/// failed install attempt. The guard makes that impossible.
///
/// Drop deliberately ignores the remove result: a temp dir that
/// can't be removed (rare; fs full / permission change mid-flight)
/// is a separate ops problem and should not panic in a `Drop`.
struct TempDirGuard {
    path: std::path::PathBuf,
}

impl TempDirGuard {
    /// Create a fresh temp dir under std::env::temp_dir() with a
    /// caller-supplied prefix. The full directory name is
    /// `<prefix>-<pid>-<rand>-<attempt>`; creation uses
    /// `create_dir` rather than `create_dir_all` so an existing
    /// path is treated as a collision and retried instead of
    /// accidentally sharing a staging area with another install.
    fn create(prefix: &str) -> std::io::Result<Self> {
        let base = std::env::temp_dir();
        for attempt in 0..32 {
            let path = base.join(format!(
                "{prefix}-{}-{}-{attempt}",
                std::process::id(),
                rand_suffix()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("could not allocate unique temp dir for prefix {prefix:?}"),
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}KB", n as f64 / 1024.0)
    } else {
        format!("{:.1}MB", n as f64 / (1024.0 * 1024.0))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_skill_pool_ref_parses_known_owner_and_rejects_bad_labels() {
        let _home = crate::facade::cli::test_support::HomeGuard::new();
        let pool = GlobalSkillPoolRef::parse_owner_id("global:claude-global", "skill.test")
            .expect("parse")
            .expect("global owner");

        assert_eq!(pool.label(), "claude-global");
        assert_eq!(pool.owner_agent_id(), "global:claude-global");
        assert_eq!(pool.dirs().len(), 1);

        let invalid = GlobalSkillPoolRef::parse_owner_id("global:../claude-global", "skill.test")
            .unwrap_err();
        assert!(
            invalid.to_string().contains("pool label"),
            "wrong error: {invalid}"
        );

        let unknown =
            GlobalSkillPoolRef::parse_owner_id("global:missing-pool", "skill.test").unwrap_err();
        assert!(
            unknown.to_string().contains("unknown global skill pool"),
            "wrong error: {unknown}"
        );
    }

    #[test]
    fn global_skill_pool_ref_resolves_directory_name_without_alias_scan() {
        let _home = crate::facade::cli::test_support::HomeGuard::new();
        let pool = GlobalSkillPoolRef::from_label("claude-global", "skill.test").unwrap();
        let skill_dir = config::home_dir()
            .join(".claude")
            .join("skills")
            .join("directory-name");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: frontmatter-alias\ndescription: Alias skill\n---\n",
        )
        .unwrap();

        assert_eq!(
            pool.skill_dir("directory-name").as_deref(),
            Some(skill_dir.as_path())
        );
        assert_eq!(
            pool.skill_dir("frontmatter-alias").as_deref(),
            Some(skill_dir.as_path())
        );
    }

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

    #[test]
    fn temp_dir_guard_allocates_distinct_staging_dirs() {
        let first = TempDirGuard::create("easynet-skill-test").unwrap();
        let second = TempDirGuard::create("easynet-skill-test").unwrap();
        assert_ne!(first.path(), second.path());
        assert!(first.path().is_dir());
        assert!(second.path().is_dir());
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
            description: "Alpha skill".into(),
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
            "wire must emit 'content_hash' (not the Rust field name): {wire}"
        );
        assert!(
            wire.contains("\"description\":\"Alpha skill\""),
            "wire must include the skill description: {wire}"
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
        assert_eq!(rec.description, "");
    }

    // ─── TempDirGuard ─────────────────────────────────────────────

    #[test]
    fn temp_dir_guard_creates_directory() {
        let guard = TempDirGuard::create("test-creates").unwrap();
        assert!(guard.path().exists(), "guard must create the directory");
        assert!(
            guard.path().is_dir(),
            "guard's path must be a directory, not a file"
        );
    }

    #[test]
    fn temp_dir_guard_removes_on_drop() {
        let path = {
            let guard = TempDirGuard::create("test-cleanup").unwrap();
            guard.path().to_path_buf()
        };
        // Exited the scope → guard dropped → directory must be gone.
        assert!(
            !path.exists(),
            "TempDirGuard.drop must remove the directory; still present at {}",
            path.display()
        );
    }

    #[test]
    fn temp_dir_guard_drop_tolerates_non_empty_directory() {
        // The guard is used to wrap a fetch+stage workdir that
        // contains files; drop must remove the whole tree, not
        // fail because the dir is non-empty.
        let path = {
            let guard = TempDirGuard::create("test-recursive").unwrap();
            std::fs::write(guard.path().join("a-file"), b"content").unwrap();
            std::fs::create_dir(guard.path().join("a-subdir")).unwrap();
            std::fs::write(guard.path().join("a-subdir/nested"), b"more").unwrap();
            guard.path().to_path_buf()
        };
        assert!(
            !path.exists(),
            "drop must remove the entire tree, not bail on non-empty"
        );
    }

    #[test]
    fn temp_dir_guard_concurrent_creates_dont_collide() {
        // The guard's name template includes pid + a per-call
        // random suffix; two creates with the same prefix must
        // produce distinct paths.
        let g1 = TempDirGuard::create("test-collide").unwrap();
        let g2 = TempDirGuard::create("test-collide").unwrap();
        assert_ne!(
            g1.path(),
            g2.path(),
            "two same-prefix creates must yield distinct paths"
        );
    }
}
