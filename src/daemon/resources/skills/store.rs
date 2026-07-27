// EasyNet CLI — runtime skill package store
// ==========================================
//
// File: src/daemon/resources/skills/store.rs
// Description: Canonical filesystem implementation behind skill.* abilities.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentRegisteredAgentLoadError, AgentRegisteredWorkspace,
    AgentRegisteredWorkspaceLookupError, AgentSkillLayout,
};
use crate::daemon::persistence::config;

const GLOBAL_SKILL_OWNER_PREFIX: &str = "global:";

/// The normalised install record persisted at
/// `<agent-managed-skills-dir>/<name>/.easynet/install.json`. One file
/// per installed skill; the file is the source of truth for `list` /
/// `upgrade` / `remove`.
///
/// Persistence model only. Public skill ability / CLI response fields
/// are owned by `projection.rs` so the store does not carry product
/// or legacy wire names.
///
/// The install tree digest is persisted as `skill_tree_hash`. The
/// public `content_hash` response name is intentionally projected at
/// the API boundary instead of being baked into this canonical store
/// record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Disk name is `skill_tree_hash`. Public response compatibility
    /// with `content_hash` belongs to `InstalledSkillProjection`.
    pub skill_tree_hash: String,
    pub size_bytes: u64,
    pub installed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    #[serde(default)]
    pub upgrade_available: bool,
}

impl InstallRecord {
    pub(crate) fn validate_canonical_persistence(&self) -> anyhow::Result<()> {
        let hash = self.skill_tree_hash.trim();
        anyhow::ensure!(
            hash == self.skill_tree_hash,
            "install record skill_tree_hash must be canonical without surrounding whitespace"
        );
        anyhow::ensure!(
            hash.starts_with("sha256:"),
            "install record skill_tree_hash must include sha256: algorithm prefix"
        );
        anyhow::ensure!(
            hash.len() > "sha256:".len(),
            "install record skill_tree_hash must include a digest"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
/// the agent runtime's managed skills dir, writes the install record,
/// and returns it. No stdout, no CLI dep. Used by `run_install` (CLI)
/// and `skill.install` ability (daemon ability dispatch).
///
/// `pub(crate)` because the only callers are in this crate
/// (run_install in this file + the skill.install system ability
/// handler). Public visibility would invite external
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

    let workspace = resolve_skill_agent_workspace(agent, SkillMutation::Install)?;
    let agent_root = workspace.root_path();
    if !agent_root.exists() {
        anyhow::bail!(
            "agent '{}' has no on-disk root at {}",
            agent,
            agent_root.display()
        );
    }

    let skills_dir = managed_skill_dir_for(agent_root, workspace.skill_layout());
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
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    anyhow::ensure!(
        entries.len() == 1 && entries[0].is_dir(),
        "expected exactly one top-level directory in archive, got {} entries",
        entries.len()
    );
    Ok(entries.remove(0))
}

pub(crate) fn global_skill_pools_for(
    layout: AgentSkillLayout,
) -> Vec<(&'static str, std::path::PathBuf)> {
    let home = config::home_dir();
    match layout {
        AgentSkillLayout::ClaudeCode => {
            vec![("claude-global", home.join(".claude").join("skills"))]
        }
        AgentSkillLayout::Codex => {
            vec![("codex-global", home.join(".agents").join("skills"))]
        }
        AgentSkillLayout::External => Vec::new(),
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

    pub(crate) fn skill_dir(&self, skill_name: &str) -> anyhow::Result<Option<std::path::PathBuf>> {
        for pool_dir in self.dirs() {
            if let Some(path) = skill_dir_in_global_pool(&pool_dir, skill_name)? {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }
}

pub(crate) fn global_skill_dir_for(
    layout: AgentSkillLayout,
    skill_name: &str,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    for (_label, pool_dir) in global_skill_pools_for(layout) {
        if let Some(path) = skill_dir_in_global_pool(&pool_dir, skill_name)? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn global_skill_pool_dirs_for_label(pool_label: &str) -> Vec<std::path::PathBuf> {
    let layouts = [
        AgentSkillLayout::ClaudeCode,
        AgentSkillLayout::Codex,
        AgentSkillLayout::External,
    ];
    let mut dirs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for layout in layouts {
        for (label, pool_dir) in global_skill_pools_for(layout) {
            if label != pool_label || !seen.insert(pool_dir.clone()) {
                continue;
            }
            dirs.push(pool_dir);
        }
    }
    dirs
}

fn skill_dir_in_global_pool(
    pool_dir: &Path,
    skill_name: &str,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let direct = pool_dir.join(skill_name);
    if direct
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| !name.starts_with('.'))
        && direct.is_dir()
        && looks_like_skill_dir(&direct)
    {
        let declared = required_global_skill_declared_name(&direct)?;
        if declared == skill_name {
            return Ok(Some(direct));
        }
    }

    let entries = match fs::read_dir(pool_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => anyhow::bail!("read global skill pool {}: {err}", pool_dir.display()),
    };
    for dir_entry in entries {
        let dir_entry = dir_entry.map_err(|err| {
            anyhow::anyhow!("scan global skill pool {}: {err}", pool_dir.display())
        })?;
        let path = dir_entry.path();
        if !path.is_dir() {
            continue;
        }
        match path.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.starts_with('.') => {}
            _ => continue,
        };
        if !looks_like_skill_dir(&path) {
            continue;
        }
        if required_global_skill_declared_name(&path)? == skill_name {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Walk a global skill pool and append one synthetic InstallRecord per declared
/// skill package. Global pool identity is semantic: `SKILL.md` frontmatter
/// `name` is the only public package name authority. Physical directory names
/// remain source subpaths and never become fallback skill identities.
///
/// We do not propagate IO errors from the walk: a global pool that
/// is missing or unreadable should not fail the whole listing —
/// the EasyNet-managed half (Source 1) can still surface.
pub(crate) fn scan_global_pool_into(
    agent_name: &str,
    pool_label: &str,
    pool_dir: &std::path::Path,
    rows: &mut Vec<InstallRecord>,
) -> anyhow::Result<()> {
    if !pool_dir.is_dir() {
        return Ok(());
    }
    let entries = match fs::read_dir(pool_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "[warn] global skill pool {} unreadable: {e}",
                pool_dir.display()
            );
            anyhow::bail!("global skill pool {} unreadable: {e}", pool_dir.display());
        }
    };
    for dir_entry in entries {
        let dir_entry = dir_entry.map_err(|err| {
            anyhow::anyhow!("scan global skill pool {}: {err}", pool_dir.display())
        })?;
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
        if let Some(record) = global_skill_record_from_dir(agent_name, pool_label, &path)? {
            rows.push(record);
        }
    }
    Ok(())
}

pub(crate) fn global_skill_record_from_dir(
    agent_name: &str,
    pool_label: &str,
    path: &std::path::Path,
) -> anyhow::Result<Option<InstallRecord>> {
    if !path.is_dir() || !looks_like_skill_dir(path) {
        return Ok(None);
    }
    let Some(dir_name) = path.file_name().and_then(|s| s.to_str()) else {
        return Ok(None);
    };
    if dir_name.starts_with('.') {
        return Ok(None);
    }
    let metadata = required_global_skill_metadata(path)?;
    let size_bytes = directory_size_bytes(path);
    let installed_at = file_mtime_iso(path)?;

    Ok(Some(InstallRecord {
        name: metadata.name,
        description: metadata.description,
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
    }))
}

fn looks_like_skill_dir(path: &std::path::Path) -> bool {
    path.join("SKILL.md").exists() || path.join("skill.json").exists()
}

struct SkillMarkdownMetadata {
    name: String,
    description: String,
}

fn required_global_skill_declared_name(path: &std::path::Path) -> anyhow::Result<String> {
    Ok(required_global_skill_metadata(path)?.name)
}

fn required_global_skill_metadata(path: &std::path::Path) -> anyhow::Result<SkillMarkdownMetadata> {
    let skill_md = path.join("SKILL.md");
    let content = fs::read_to_string(&skill_md)
        .map_err(|err| anyhow::anyhow!("read {}: {err}", skill_md.display()))?;
    let name = parse_skill_md_name_from_content(&content)?.ok_or_else(|| {
        anyhow::anyhow!(
            "global skill package {} must declare frontmatter name in SKILL.md",
            path.display()
        )
    })?;
    let description = skill_description_from_markdown_content(&content);
    Ok(SkillMarkdownMetadata { name, description })
}

pub(crate) fn skill_description_from_dir(path: &std::path::Path) -> String {
    skill_description_from_markdown(&path.join("SKILL.md")).unwrap_or_default()
}

fn skill_description_from_markdown(path: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    Some(skill_description_from_markdown_content(&content))
}

fn skill_description_from_markdown_content(content: &str) -> String {
    if let Some(frontmatter) = skill_md_frontmatter(&content) {
        if let Some(description) = frontmatter_field(frontmatter, "description") {
            return description;
        }
    }
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("---"))
        .map(|line| line.to_string())
        .unwrap_or_default()
}

fn parse_skill_md_name_from_content(content: &str) -> anyhow::Result<Option<String>> {
    // Frontmatter is delimited by `---` lines at the top.
    let Some(frontmatter) = skill_md_frontmatter(content) else {
        return Ok(None);
    };
    Ok(frontmatter_field(frontmatter, "name"))
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
fn file_mtime_iso(path: &std::path::Path) -> anyhow::Result<String> {
    let meta = fs::metadata(path)
        .map_err(|err| anyhow::anyhow!("read metadata {}: {err}", path.display()))?;
    let modified = meta
        .modified()
        .map_err(|err| anyhow::anyhow!("read modified time {}: {err}", path.display()))?;
    let dt: chrono::DateTime<chrono::Utc> = modified.into();
    Ok(dt.to_rfc3339())
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
    let workspace = resolve_skill_agent_workspace(agent, SkillMutation::Upgrade)?;
    let agent_root = workspace.root_path();
    let skills_dir = managed_skill_dir_for(agent_root, workspace.skill_layout());
    let skill_dir = skills_dir.join(name);
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

    let backup = skills_dir.join(format!(".{}-backup-{}", name, rand_suffix()));
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
    let workspace = resolve_skill_agent_workspace(agent, SkillMutation::Remove)?;
    let skill_dir =
        managed_skill_dir_for(workspace.root_path(), workspace.skill_layout()).join(name);
    if !skill_dir.exists() {
        anyhow::bail!("skill '{}' is not installed on agent '{}'", name, agent);
    }
    fs::remove_dir_all(&skill_dir)?;
    Ok(())
}

// ─── helpers ─────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum SkillMutation {
    Install,
    Upgrade,
    Remove,
}

impl SkillMutation {
    fn operation(self) -> &'static str {
        match self {
            Self::Install => "skill.install",
            Self::Upgrade => "skill.upgrade",
            Self::Remove => "skill.remove",
        }
    }

    fn missing_owner_error(self, agent: &str) -> anyhow::Error {
        match self {
            Self::Install => {
                anyhow::anyhow!("agent '{}' not registered; run 'easynet agent list'", agent)
            }
            Self::Upgrade | Self::Remove => anyhow::anyhow!("agent '{}' not registered", agent),
        }
    }
}

fn resolve_skill_agent_workspace(
    agent: &str,
    mutation: SkillMutation,
) -> anyhow::Result<AgentRegisteredWorkspace> {
    match AgentAggregateRepository::load_registered_agent_workspace(agent, mutation.operation()) {
        Ok(workspace) => Ok(workspace),
        Err(AgentRegisteredAgentLoadError::Lookup(
            AgentRegisteredWorkspaceLookupError::Missing { .. },
        )) => Err(mutation.missing_owner_error(agent)),
        Err(error) => Err(error.into_source_or_self()),
    }
}

/// Canonical managed skill directory for an Agent runtime workspace.
///
/// This is the single projection used by `skill.install`,
/// `skill.publish`, `skill.list`, `skill.upgrade`, and `skill.remove`.
/// Runtime-specific loaders are the authority: Claude Code reads
/// `.claude/skills`, Codex/Codex App Server read `.agents/skills`, and only
/// External runtimes keep the generic `<root>/skills` convention.
pub(crate) fn managed_skill_dir_for(root: &Path, layout: AgentSkillLayout) -> PathBuf {
    match layout {
        AgentSkillLayout::ClaudeCode => root.join(".claude").join("skills"),
        AgentSkillLayout::Codex => root.join(".agents").join("skills"),
        AgentSkillLayout::External => root.join("skills"),
    }
}

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
    let record: InstallRecord = serde_json::from_str(&text)?;
    record.validate_canonical_persistence()?;
    Ok(record)
}

pub(crate) fn write_install_record(skill_dir: &Path, record: &InstallRecord) -> anyhow::Result<()> {
    record.validate_canonical_persistence()?;
    let meta_dir = skill_dir.join(".easynet");
    fs::create_dir_all(&meta_dir)?;
    let path = meta_dir.join("install.json");
    let json = serde_json::to_string_pretty(record)?;
    config::atomic_write(&path, json.as_bytes())?;
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
    use crate::daemon::persistence::agent_registry as agents;

    #[test]
    fn resolve_skill_agent_workspace_projects_registered_workspace() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let root = config::home_dir().join("agents").join("alice");
        let mut entry = agents::AgentEntry::new(agents::AgentType::ClaudeCode, None);
        entry.root_path = Some(root.clone());
        let mut registry = agents::AgentRegistry::default();
        registry.agents.insert("alice".to_string(), entry);
        agents::save_agents(&registry).expect("save registry");

        let workspace =
            resolve_skill_agent_workspace("alice", SkillMutation::Install).expect("workspace");
        assert_eq!(workspace.root_path(), root);
        assert_eq!(workspace.skill_layout(), AgentSkillLayout::ClaudeCode);
    }

    #[test]
    fn resolve_skill_agent_workspace_preserves_command_specific_missing_owner_errors() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        for (mutation, expected) in [
            (
                SkillMutation::Install,
                "agent 'missing' not registered; run 'easynet agent list'",
            ),
            (SkillMutation::Upgrade, "agent 'missing' not registered"),
            (SkillMutation::Remove, "agent 'missing' not registered"),
        ] {
            let err = resolve_skill_agent_workspace("missing", mutation)
                .expect_err("missing owner must fail");
            assert!(err.to_string().contains(expected), "wrong error: {err}");
        }
    }

    #[test]
    fn managed_skill_dir_for_codex_uses_runtime_project_skill_root() {
        let root = std::path::Path::new("/tmp/agent-root");
        assert_eq!(
            managed_skill_dir_for(root, AgentSkillLayout::Codex),
            root.join(".agents").join("skills")
        );
        assert_ne!(
            managed_skill_dir_for(root, AgentSkillLayout::Codex),
            root.join("skills"),
            "codex managed installs must not land in the retired audit-only root"
        );
    }

    #[test]
    fn global_skill_pool_ref_parses_known_owner_and_rejects_bad_labels() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
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
    fn global_skill_pool_ref_resolves_declared_name_not_directory_alias() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
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
            pool.skill_dir("directory-name").unwrap().as_deref(),
            None,
            "global pool lookup must not treat physical directory names as public skill identity"
        );
        assert_eq!(
            pool.skill_dir("frontmatter-alias").unwrap().as_deref(),
            Some(skill_dir.as_path())
        );
    }

    #[test]
    fn global_skill_record_requires_declared_frontmatter_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let unnamed = dir.path().join("directory-name");
        fs::create_dir_all(&unnamed).unwrap();
        fs::write(unnamed.join("SKILL.md"), "# No declared name\n").unwrap();

        let error = global_skill_record_from_dir("alice", "claude-global", &unnamed).unwrap_err();
        assert!(
            error.to_string().contains("must declare frontmatter name"),
            "wrong error: {error}"
        );

        fs::write(
            unnamed.join("SKILL.md"),
            "---\nname: declared-name\ndescription: Declared\n---\n# Declared\n",
        )
        .unwrap();
        let record = global_skill_record_from_dir("alice", "claude-global", &unnamed)
            .expect("record")
            .expect("record present");

        assert_eq!(record.name, "declared-name");
        assert_eq!(record.source.subpath.as_deref(), Some("directory-name"));
    }

    #[test]
    fn global_skill_dir_lookup_requires_declared_name_even_for_direct_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let direct = dir.path().join("declared-name");
        fs::create_dir_all(&direct).unwrap();
        fs::write(direct.join("SKILL.md"), "# Missing frontmatter name\n").unwrap();

        let error = skill_dir_in_global_pool(dir.path(), "declared-name").unwrap_err();
        assert!(
            error.to_string().contains("must declare frontmatter name"),
            "wrong error: {error}"
        );

        fs::remove_dir_all(&direct).unwrap();

        let renamed = dir.path().join("physical-package");
        fs::create_dir_all(&renamed).unwrap();
        fs::write(
            renamed.join("SKILL.md"),
            "---\nname: declared-name\ndescription: Declared\n---\n",
        )
        .unwrap();

        assert_eq!(
            skill_dir_in_global_pool(dir.path(), "declared-name")
                .unwrap()
                .as_deref(),
            Some(renamed.as_path())
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
        // content produce different skill tree hashes, making
        // upgrade and install-integrity comparisons unstable.
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
        // must not participate in the skill tree hash — otherwise
        // every skill's hash depends on its own install timestamp.
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
            "adding .easynet/ must not change the skill tree hash"
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

    // ─── canonical install-record persistence schema ─────────────
    //
    // The persisted field is `skill_tree_hash` (semantic name — it is
    // NOT AXIOM §6.1 Q6's `ability_snapshot.content_hash`). Public
    // response compatibility with `content_hash` belongs to
    // `InstalledSkillProjection`, never to the canonical store record.

    #[test]
    fn install_record_serialize_emits_skill_tree_hash_on_disk() {
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
            wire.contains("\"skill_tree_hash\":\"sha256:deadbeef\""),
            "install record must persist the canonical 'skill_tree_hash': {wire}"
        );
        assert!(
            wire.contains("\"description\":\"Alpha skill\""),
            "install record must include the skill description: {wire}"
        );
        assert!(
            !wire.contains("\"content_hash\""),
            "install record must not persist the public projection field: {wire}"
        );
    }

    #[test]
    fn install_record_deserialize_reads_skill_tree_hash_from_disk() {
        // Simulates reading the canonical install.json persistence
        // record. Public `content_hash` is a projection-only field.
        let wire = r#"{
            "name": "alpha",
            "agent_id": "alice",
            "source": {
                "kind": "github",
                "identifier": "a/b"
            },
            "skill_tree_hash": "sha256:wire",
            "size_bytes": 99,
            "installed_at": "2026-04-23T00:00:00Z",
            "upgrade_available": false
        }"#;
        let rec: InstallRecord = serde_json::from_str(wire).unwrap();
        assert_eq!(rec.skill_tree_hash, "sha256:wire");
        assert_eq!(rec.description, "");
    }

    #[test]
    fn read_install_record_rejects_unprefixed_skill_tree_hash() {
        let guard = TempDirGuard::create("install-record-read-unprefixed").unwrap();
        let path = guard.path().join("install.json");
        std::fs::write(
            &path,
            r#"{
                "name": "alpha",
                "agent_id": "alice",
                "source": {
                    "kind": "github",
                    "identifier": "a/b"
                },
                "skill_tree_hash": "wire",
                "size_bytes": 99,
                "installed_at": "2026-04-23T00:00:00Z",
                "upgrade_available": false
            }"#,
        )
        .unwrap();
        let error = read_install_record(&path)
            .expect_err("canonical read must reject unprefixed skill_tree_hash");
        assert!(
            error
                .to_string()
                .contains("skill_tree_hash must include sha256:"),
            "expected canonical hash prefix error: {error}"
        );
    }

    #[test]
    fn write_install_record_rejects_unprefixed_skill_tree_hash() {
        let guard = TempDirGuard::create("install-record-write-unprefixed").unwrap();
        let record = InstallRecord {
            name: "alpha".into(),
            description: "Alpha skill".into(),
            agent_id: "alice".into(),
            source: SkillSource {
                kind: "github".into(),
                identifier: "a/b".into(),
                ref_: None,
                subpath: None,
            },
            skill_tree_hash: "deadbeef".into(),
            size_bytes: 42,
            installed_at: "2026-04-23T00:00:00Z".into(),
            last_checked_at: None,
            upgrade_available: false,
        };
        let error = write_install_record(guard.path(), &record)
            .expect_err("canonical write must reject unprefixed skill_tree_hash");
        assert!(
            error
                .to_string()
                .contains("skill_tree_hash must include sha256:"),
            "expected canonical hash prefix error: {error}"
        );
        assert!(
            !guard.path().join(".easynet").join("install.json").exists(),
            "invalid records must not be persisted"
        );
    }

    #[test]
    fn install_record_rejects_legacy_content_hash_on_disk() {
        let wire = r#"{
            "name": "alpha",
            "agent_id": "alice",
            "source": {
                "kind": "github",
                "identifier": "a/b"
            },
            "content_hash": "sha256:legacy",
            "size_bytes": 99,
            "installed_at": "2026-04-23T00:00:00Z",
            "upgrade_available": false
        }"#;
        let error = serde_json::from_str::<InstallRecord>(wire)
            .expect_err("legacy content_hash must fail closed in persistence");
        assert!(
            error.to_string().contains("content_hash"),
            "strict schema error should name the legacy field: {error}"
        );
    }

    #[test]
    fn install_record_rejects_unknown_top_level_fields() {
        let wire = r#"{
            "name": "alpha",
            "agent_id": "alice",
            "source": {
                "kind": "github",
                "identifier": "a/b"
            },
            "skill_tree_hash": "sha256:wire",
            "size_bytes": 99,
            "installed_at": "2026-04-23T00:00:00Z",
            "upgrade_available": false,
            "legacy_content_hash": "sha256:legacy"
        }"#;
        let error = serde_json::from_str::<InstallRecord>(wire)
            .expect_err("unknown install record fields must fail closed");
        assert!(
            error.to_string().contains("legacy_content_hash"),
            "strict schema error should name the unknown field: {error}"
        );
    }

    #[test]
    fn install_record_rejects_unknown_source_fields() {
        let wire = r#"{
            "name": "alpha",
            "agent_id": "alice",
            "source": {
                "kind": "github",
                "identifier": "a/b",
                "legacy_ref": "main"
            },
            "skill_tree_hash": "sha256:wire",
            "size_bytes": 99,
            "installed_at": "2026-04-23T00:00:00Z",
            "upgrade_available": false
        }"#;
        let error = serde_json::from_str::<InstallRecord>(wire)
            .expect_err("unknown nested source fields must fail closed");
        assert!(
            error.to_string().contains("legacy_ref"),
            "strict nested schema error should name the unknown field: {error}"
        );
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
