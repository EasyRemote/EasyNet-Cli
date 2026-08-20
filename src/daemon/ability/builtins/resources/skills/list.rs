// EasyNet CLI — skill inventory filesystem walk
// =====================================================================
//
// File: src/daemon/ability/builtins/resources/skills/list.rs
// Description: Shared filesystem walk behind `skill.list`.
//              This module is not registered as a standalone system
//              ability; it exists so `skill.list` and CLI
//              tests use one source of truth for installed skills.
//
// Why "skill list" is an ability, not a separate gRPC RPC
// -------------------------------------------------------
// Architecturally, every discoverable resource on a node enters through the
// daemon's canonical Invocation face. `skill.list` is registered in the live
// local ability catalog, published into the resolver snapshot, and dispatched
// by the selected route. MCP and CLI surfaces are callers of that same ability;
// neither owns a parallel skill-list transport or registry.
//
// Wire shape
// ----------
// The exported helper returns `{ "items": [InstalledSkillProjection, ...] }`.
// Each item carries:
//   * name, description, agent_id, resource_ura
//   * source = { kind, identifier, ref?, subpath? }
//   * content_hash (empty for global-pool skills)
//   * size_bytes, installed_at, last_checked_at?, upgrade_available
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentAggregateSnapshot, AgentHostedSkillOwnerProjection,
    AgentRegisteredWorkspace, AgentSkillLayout,
};
use crate::daemon::resources::skills::projection::{InstalledSkillProjection, SkillListResponse};
use crate::daemon::resources::skills::store::managed_skill_dir_for;

/// Skill inventory handler.
///
/// Args:
/// ```json
/// {
///   "owner_agent_id": "<local agent name>"?,
///   "agent_ura": "<canonical owner Agent URA>"?,
///   "subject_ura": "<owner Agent URA or skill package Resource URA>"?
/// }
/// ```
///
/// `owner_agent_id` is usually a local workspace selector. It may
/// also be `global:<pool>` for unscoped global-pool rows returned by
/// this ability. `agent_ura` and `subject_ura` are canonical query
/// scopes. If both forms are supplied they must resolve to the same
/// hosted agent.
///
/// Returns: `{ "items": [InstalledSkill, ...] }`. The InstalledSkill
/// shape matches `EasyNet/backend/internal/types/custom_types.go`
/// `InstalledSkill` so the backend can decode the response and
/// forward to the frontend without re-shaping fields. Each item
/// carries `agent_id`: managed/scoped rows carry the concrete local
/// agent id, while unscoped global-pool rows carry `global:<pool>` so
/// the list stays one row per global skill rather than multiplying by
/// the number of agents that can consume the pool.
pub(crate) fn handle(args: Value) -> anyhow::Result<Value> {
    let snapshot = AgentAggregateRepository::load_snapshot()?;
    let hosted_skill_owners = snapshot.hosted_skill_owner_projection();
    let scope = SkillListScope::from_args(&args, &hosted_skill_owners)?;
    let rows = SkillInventoryBuilder::new(&snapshot, &scope).collect()?;

    let items: Vec<InstalledSkillProjection> = rows
        .into_iter()
        .map(|r| {
            let resource_ura = scoped_skill_resource_ura(
                &hosted_skill_owners,
                scope.agent_ura_for_row(&r.agent_id),
                &r.agent_id,
                &r.name,
            );
            InstalledSkillProjection::from_record(r, resource_ura)
        })
        .collect();

    Ok(serde_json::to_value(SkillListResponse::from_items(items))?)
}

/// Builds the installed-skill inventory in one bounded pass.
///
/// This type owns the fan-out policy: managed agent installs are inherently
/// agent-scoped, while native global pools are scanned once and then either
/// emitted once globally or projected onto the selected agent. Keeping that
/// policy out of `list_handler` prevents future patches from reintroducing an
/// O(agents * global_pool_size) unscoped listing.
struct SkillInventoryBuilder<'a> {
    snapshot: &'a AgentAggregateSnapshot,
    scope: &'a SkillListScope,
    rows: Vec<crate::daemon::resources::skills::store::InstallRecord>,
    global_pool_cache: GlobalSkillPoolCache,
    emitted_unscoped_global_pools: BTreeSet<PathBuf>,
}

impl<'a> SkillInventoryBuilder<'a> {
    fn new(snapshot: &'a AgentAggregateSnapshot, scope: &'a SkillListScope) -> Self {
        Self {
            snapshot,
            scope,
            rows: Vec::new(),
            global_pool_cache: GlobalSkillPoolCache::default(),
            emitted_unscoped_global_pools: BTreeSet::new(),
        }
    }

    fn collect(
        mut self,
    ) -> anyhow::Result<Vec<crate::daemon::resources::skills::store::InstallRecord>> {
        if let Some(global_pool) = self.scope.global_pool() {
            self.collect_global_pool(global_pool)?;
            return Ok(self.rows);
        }

        for name in self.snapshot.registered_agent_names()? {
            if !self.scope.includes_agent(&name) {
                continue;
            }
            let workspace = self
                .snapshot
                .registered_agent_workspace(&name, "skill.list")?;
            self.collect_managed_installs(&name, &workspace)?;
            self.collect_global_pools(&name, workspace.skill_layout())?;
        }
        self.retain_skill_name_filter();
        Ok(self.rows)
    }

    fn retain_skill_name_filter(&mut self) {
        if let Some(skill_name) = &self.scope.skill_name {
            self.rows.retain(|row| row.name == *skill_name);
        }
    }

    fn collect_managed_installs(
        &mut self,
        agent_name: &str,
        workspace: &AgentRegisteredWorkspace,
    ) -> anyhow::Result<()> {
        let skills_dir = managed_skill_dir_for(workspace.root_path(), workspace.skill_layout());
        if !skills_dir.exists() {
            return Ok(());
        }
        let Ok(read) = std::fs::read_dir(&skills_dir) else {
            return Ok(());
        };
        for dir_entry in read.flatten() {
            self.collect_agent_skill_record(agent_name, workspace, &dir_entry.path())?;
        }
        Ok(())
    }

    fn collect_agent_skill_record(
        &mut self,
        agent_name: &str,
        workspace: &AgentRegisteredWorkspace,
        skill_dir: &Path,
    ) -> anyhow::Result<()> {
        let record_path = skill_dir.join(".easynet").join("install.json");
        if record_path.exists() {
            match crate::daemon::resources::skills::store::read_install_record(&record_path) {
                Ok(record) => self.rows.push(record),
                Err(err) => {
                    let path_display = format!("{}", skill_dir.display());
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = skill_list,
                        kind = entry_skipped,
                        level = "warn",
                        path = path_display,
                        error = err_msg,
                    );
                }
            }
            return Ok(());
        }

        let runtime_label = match workspace.skill_layout() {
            AgentSkillLayout::ClaudeCode => "claude-project",
            AgentSkillLayout::Codex => "codex-project",
            AgentSkillLayout::External => "external-project",
        };
        if let Some(record) =
            crate::daemon::resources::skills::store::native_project_skill_record_from_dir(
                agent_name,
                runtime_label,
                skill_dir,
            )?
        {
            self.rows.push(record);
        }
        Ok(())
    }

    fn collect_global_pools(
        &mut self,
        agent_name: &str,
        layout: AgentSkillLayout,
    ) -> anyhow::Result<()> {
        for (label, pool_dir) in
            crate::daemon::resources::skills::store::global_skill_pools_for(layout)
        {
            if self.scope.is_agent_scoped() {
                self.rows.extend(
                    self.global_pool_cache
                        .rows_for_agent(agent_name, label, &pool_dir)?,
                );
            } else if self.emitted_unscoped_global_pools.insert(pool_dir.clone()) {
                self.rows.extend(
                    self.global_pool_cache
                        .rows_for_global_pool(label, &pool_dir)?,
                );
            }
        }
        Ok(())
    }

    fn collect_global_pool(
        &mut self,
        global_pool: &crate::daemon::resources::skills::store::GlobalSkillPoolRef,
    ) -> anyhow::Result<()> {
        if let Some(skill_name) = self.scope.skill_name.as_deref() {
            self.rows.extend(
                self.global_pool_cache
                    .rows_for_global_skill(global_pool, skill_name)?,
            );
            return Ok(());
        }
        for pool_dir in global_pool.dirs() {
            if self.emitted_unscoped_global_pools.insert(pool_dir.clone()) {
                self.rows.extend(
                    self.global_pool_cache
                        .rows_for_global_pool(global_pool.label(), &pool_dir)?,
                );
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct GlobalSkillPoolCache {
    templates_by_dir:
        BTreeMap<PathBuf, Vec<crate::daemon::resources::skills::store::InstallRecord>>,
}

impl GlobalSkillPoolCache {
    fn rows_for_global_pool(
        &mut self,
        pool_label: &str,
        pool_dir: &std::path::Path,
    ) -> anyhow::Result<Vec<crate::daemon::resources::skills::store::InstallRecord>> {
        Ok(self
            .templates_for(pool_label, pool_dir)?
            .iter()
            .cloned()
            .map(|mut row| {
                row.agent_id = format!("global:{pool_label}");
                row
            })
            .collect())
    }

    fn rows_for_global_skill(
        &mut self,
        global_pool: &crate::daemon::resources::skills::store::GlobalSkillPoolRef,
        skill_name: &str,
    ) -> anyhow::Result<Vec<crate::daemon::resources::skills::store::InstallRecord>> {
        let Some(skill_dir) = global_pool.skill_dir(skill_name)? else {
            return Ok(Vec::new());
        };
        Ok(
            crate::daemon::resources::skills::store::global_skill_record_from_dir(
                &global_pool.owner_agent_id(),
                global_pool.label(),
                &skill_dir,
            )?
            .into_iter()
            .collect(),
        )
    }

    fn rows_for_agent(
        &mut self,
        agent_name: &str,
        pool_label: &str,
        pool_dir: &std::path::Path,
    ) -> anyhow::Result<Vec<crate::daemon::resources::skills::store::InstallRecord>> {
        Ok(self
            .templates_for(pool_label, pool_dir)?
            .iter()
            .cloned()
            .map(|mut row| {
                row.agent_id = agent_name.to_string();
                row
            })
            .collect())
    }

    fn templates_for(
        &mut self,
        pool_label: &str,
        pool_dir: &std::path::Path,
    ) -> anyhow::Result<&Vec<crate::daemon::resources::skills::store::InstallRecord>> {
        if !self.templates_by_dir.contains_key(pool_dir) {
            let mut templates = Vec::new();
            crate::daemon::resources::skills::store::scan_global_pool_into(
                "",
                pool_label,
                pool_dir,
                &mut templates,
            )?;
            self.templates_by_dir
                .insert(pool_dir.to_path_buf(), templates);
        }
        Ok(self
            .templates_by_dir
            .get(pool_dir)
            .expect("global skill pool template inserted"))
    }
}

struct SkillListScope {
    owner_agent_id: Option<String>,
    agent_ura: Option<String>,
    skill_name: Option<String>,
    global_pool: Option<crate::daemon::resources::skills::store::GlobalSkillPoolRef>,
}

impl SkillListScope {
    fn from_args(
        args: &Value,
        hosted_skill_owners: &AgentHostedSkillOwnerProjection,
    ) -> anyhow::Result<Self> {
        let object = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("skill.list: args must be a JSON object"))?;
        for key in object.keys() {
            match key.as_str() {
                "owner_agent_id" | "agent_ura" | "subject_ura" => {}
                other => anyhow::bail!("skill.list: unsupported field `{other}`"),
            }
        }

        let owner_agent_id = string_arg(object, "owner_agent_id");
        let agent_ura = string_arg(object, "agent_ura");
        let subject = string_arg(object, "subject_ura")
            .map(|subject| {
                crate::daemon::federation::read_model::owner_projection::project_agent_skill_subject(&subject)
                    .map_err(|e| anyhow::anyhow!("skill.list: {e}"))
            })
            .transpose()?;
        let global_pool = owner_agent_id
            .as_deref()
            .map(|owner| {
                crate::daemon::resources::skills::store::GlobalSkillPoolRef::parse_owner_id(
                    owner,
                    "skill.list",
                )
            })
            .transpose()?
            .flatten();
        if global_pool.is_some() && (agent_ura.is_some() || subject.is_some()) {
            anyhow::bail!(
                "skill.list: global owner_agent_id cannot be combined with agent_ura/subject_ura"
            );
        }
        let scoped_agent_ura = merge_agent_scope(agent_ura, subject.as_ref())?;
        let scoped_owner = scoped_agent_ura
            .as_deref()
            .map(|ura| owner_name_for_agent_ura(hosted_skill_owners, ura))
            .transpose()?;
        if let (Some(owner), Some(scoped_owner)) = (&owner_agent_id, &scoped_owner) {
            if owner != scoped_owner {
                anyhow::bail!(
                    "skill.list: owner_agent_id {owner:?} does not match agent_ura/subject_ura owner {scoped_owner:?}"
                );
            }
        }

        Ok(Self {
            owner_agent_id: global_pool
                .as_ref()
                .map(|pool| pool.owner_agent_id())
                .or(scoped_owner)
                .or(owner_agent_id),
            agent_ura: scoped_agent_ura,
            skill_name: subject.and_then(|scope| scope.skill_name),
            global_pool,
        })
    }

    fn agent_ura_for_row(&self, row_agent_id: &str) -> Option<&str> {
        match (&self.owner_agent_id, &self.agent_ura) {
            (Some(owner), Some(agent_ura)) if owner == row_agent_id => Some(agent_ura.as_str()),
            (None, Some(agent_ura)) => Some(agent_ura.as_str()),
            _ => None,
        }
    }

    fn is_agent_scoped(&self) -> bool {
        self.global_pool.is_none() && (self.owner_agent_id.is_some() || self.agent_ura.is_some())
    }

    fn includes_agent(&self, agent_name: &str) -> bool {
        if self.global_pool.is_some() {
            return false;
        }
        self.owner_agent_id
            .as_ref()
            .map(|filter| filter == agent_name)
            .unwrap_or(true)
    }

    fn global_pool(&self) -> Option<&crate::daemon::resources::skills::store::GlobalSkillPoolRef> {
        self.global_pool.as_ref()
    }
}

fn string_arg(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn merge_agent_scope(
    agent_ura: Option<String>,
    subject: Option<
        &crate::daemon::federation::read_model::owner_projection::AgentSkillSubjectProjection,
    >,
) -> anyhow::Result<Option<String>> {
    match (agent_ura, subject) {
        (Some(agent_ura), Some(subject)) if agent_ura != subject.agent_ura => {
            anyhow::bail!("skill.list: agent_ura and subject_ura owner must match")
        }
        (Some(agent_ura), _) => Ok(Some(agent_ura)),
        (None, Some(subject)) => Ok(Some(subject.agent_ura.clone())),
        (None, None) => Ok(None),
    }
}

fn owner_name_for_agent_ura(
    hosted_skill_owners: &AgentHostedSkillOwnerProjection,
    agent_ura: &str,
) -> anyhow::Result<String> {
    hosted_skill_owners
        .owner_name_for_agent_ura(agent_ura)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("skill.list: agent_ura {agent_ura:?} is not hosted here"))
}

fn scoped_skill_resource_ura(
    hosted_skill_owners: &AgentHostedSkillOwnerProjection,
    explicit_agent_ura: Option<&str>,
    agent_name: &str,
    skill_name: &str,
) -> Option<String> {
    let agent_ura = explicit_agent_ura.map(str::to_string).or_else(|| {
        hosted_skill_owners
            .hosted_ura_for(agent_name)
            .map(str::to_string)
    })?;
    crate::daemon::federation::read_model::owner_projection::skill_resource_ura(
        &agent_ura, skill_name,
    )
}

/// JSON Schema for the input.
pub fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "owner_agent_id": {
                "type": "string",
                "description": "Restrict the list to skills owned by this agent, or use global:<pool> for unscoped global-pool rows. Absent = every registered agent."
            },
            "agent_ura": {
                "type": "string",
                "description": "Canonical owner Agent URA. Filters to that hosted agent and derives resource_ura on returned rows."
            },
            "subject_ura": {
                "type": "string",
                "description": "Owner Agent URA or skill package Resource URA. A skill Resource URA filters to that single skill."
            }
        },
        "additionalProperties": false,
    })
}

/// Human-readable blurb for discovery surfaces.
pub fn list_description() -> &'static str {
    "List installed skills. Unscoped calls return managed agent installs plus one row \
     per global skill pool entry; agent-scoped calls project global pool entries onto \
     the selected agent and return InstalledSkill rows."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_list_dispatchable() {
        let res = handle(json!({})).expect("list handler ok");
        assert!(res.get("items").and_then(Value::as_array).is_some());
    }

    #[test]
    fn list_input_schema_is_object_with_optional_agent_id() {
        // The schema is the contract a UI renders. Pin the keys so a typo trips here.
        let s = list_input_schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        let props = s["properties"].as_object().expect("properties is object");
        assert!(props.contains_key("owner_agent_id"));
        assert!(props.contains_key("agent_ura"));
        assert!(props.contains_key("subject_ura"));
        assert_eq!(props["owner_agent_id"]["type"], "string");
        // No `required` array — owner_agent_id is optional (omitted = list
        // across every agent).
        assert!(s.get("required").is_none());
    }

    #[test]
    fn managed_skill_dir_for_claude_code_uses_native_project_dir_only() {
        let root = std::path::Path::new("/tmp/agent-root");
        let dir = managed_skill_dir_for(root, AgentSkillLayout::ClaudeCode);
        assert_eq!(dir, root.join(".claude").join("skills"));
        assert_ne!(
            dir,
            root.join("skills"),
            "claude-code must not scan retired root-level skills directory"
        );
    }

    #[test]
    fn managed_skill_dir_for_codex_profiles_uses_agents_project_dir() {
        let root = std::path::Path::new("/tmp/agent-root");
        let dir = managed_skill_dir_for(root, AgentSkillLayout::Codex);
        assert_eq!(dir, root.join(".agents").join("skills"));
        assert_ne!(
            dir,
            root.join("skills"),
            "codex managed skills must not use the retired root-level directory"
        );
    }

    #[test]
    fn managed_skill_dir_for_external_keeps_generic_skill_dir() {
        let root = std::path::Path::new("/tmp/agent-root");
        assert_eq!(
            managed_skill_dir_for(root, AgentSkillLayout::External),
            root.join("skills")
        );
    }

    #[test]
    fn list_handler_includes_bootstrapped_codex_project_skills_without_install_record() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let agent_name = "codex-native";
        let agent_ura = "easynet:///r/acme/agent/u1.codex-native";
        let agent_root = crate::daemon::persistence::config::agents_root().join(agent_name);
        let skill_dir = agent_root
            .join(".agents")
            .join("skills")
            .join("easynet-collaborate");
        std::fs::create_dir_all(&skill_dir).expect("native skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: easynet-collaborate\ndescription: Collaborate through EasyNet\n---\n",
        )
        .expect("native skill manifest");

        let mut registry = crate::daemon::persistence::agent_registry::AgentRegistry::default();
        let mut agent = crate::daemon::persistence::agent_registry::AgentEntry::new(
            crate::core::agent::spec::RuntimeKind::Codex,
            None,
        );
        agent.root_path = Some(agent_root);
        registry
            .agents
            .insert(format!("default/{agent_name}"), agent);
        crate::daemon::persistence::agent_registry::save_agents(&registry).expect("save registry");
        crate::daemon::persistence::local_agents::save(
            &crate::daemon::persistence::local_agents::LocalAgentsFile {
                host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
                hosted_agents: vec![crate::daemon::persistence::local_agents::HostedAgentEntry {
                    profile: "llm".to_string(),
                    name: agent_name.to_string(),
                    agent_ura: agent_ura.to_string(),
                    signing_authority: "hosted_by:easynet:///r/acme/device/dev-1".to_string(),
                    first_seen_at: "2026-01-01T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("save local agents");

        let response = handle(json!({
            "owner_agent_id": agent_name,
            "agent_ura": agent_ura
        }))
        .expect("skill list");
        let items = response["items"].as_array().expect("items");
        let native = items
            .iter()
            .find(|item| item["name"] == "easynet-collaborate")
            .expect("bootstrapped native skill must be visible");
        assert_eq!(native["agent_id"], agent_name);
        assert_eq!(native["description"], "Collaborate through EasyNet");
        assert_eq!(native["source"]["kind"], "custom");
        assert_eq!(native["source"]["identifier"], "codex-project");
        assert_eq!(
            native["resource_ura"],
            "easynet:///r/acme/resource/agent.u1.codex-native/skill/easynet-collaborate"
        );
    }

    #[test]
    fn global_skill_pool_cache_scans_once_and_projects_per_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_dir = dir.path().join("summarize");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: summarize\ndescription: Summarize text\n---\n",
        )
        .expect("skill md");

        let mut cache = GlobalSkillPoolCache::default();
        let first = cache
            .rows_for_agent("alice", "claude-global", dir.path())
            .expect("first scan");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].agent_id, "alice");
        assert_eq!(first[0].name, "summarize");

        std::fs::remove_file(skill_dir.join("SKILL.md")).expect("remove marker");
        let second = cache
            .rows_for_agent("bob", "claude-global", dir.path())
            .expect("cached projection");
        assert_eq!(
            second.len(),
            1,
            "second projection must reuse the cached scan instead of rescanning the pool"
        );
        assert_eq!(second[0].agent_id, "bob");
        assert_eq!(second[0].name, "summarize");
    }

    #[test]
    fn global_skill_pool_cache_projects_unscoped_pool_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_dir = dir.path().join("summarize");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: summarize\ndescription: Summarize text\n---\n",
        )
        .expect("skill md");

        let mut cache = GlobalSkillPoolCache::default();
        let rows = cache
            .rows_for_global_pool("claude-global", dir.path())
            .expect("global pool rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].agent_id, "global:claude-global");
        assert_eq!(rows[0].name, "summarize");
    }

    #[test]
    fn global_skill_pool_cache_rejects_corrupt_skill_package() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_dir = dir.path().join("broken");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(skill_dir.join("SKILL.md"), "# Missing declared name\n").expect("skill md");

        let mut cache = GlobalSkillPoolCache::default();
        let error = cache
            .rows_for_global_pool("claude-global", dir.path())
            .expect_err("corrupt skill package must not disappear");

        assert!(
            error.to_string().contains("must declare frontmatter name"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn list_handler_emits_unscoped_global_pool_once_across_agents() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let home = std::path::PathBuf::from(std::env::var("HOME").expect("home"));
        let skill_dir = home.join(".claude").join("skills").join("summarize");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: summarize\ndescription: Summarize text\n---\n",
        )
        .expect("skill md");

        let mut registry = crate::daemon::persistence::agent_registry::AgentRegistry::default();
        let alice_root = crate::daemon::persistence::config::agents_root().join("alice");
        let bob_root = crate::daemon::persistence::config::agents_root().join("bob");
        std::fs::create_dir_all(&alice_root).expect("alice root");
        std::fs::create_dir_all(&bob_root).expect("bob root");
        let mut alice = crate::daemon::persistence::agent_registry::AgentEntry::new(
            crate::core::agent::spec::RuntimeKind::ClaudeCode,
            None,
        );
        alice.root_path = Some(alice_root);
        let mut bob = crate::daemon::persistence::agent_registry::AgentEntry::new(
            crate::core::agent::spec::RuntimeKind::ClaudeCode,
            None,
        );
        bob.root_path = Some(bob_root);
        registry.agents.insert("default/alice".to_string(), alice);
        registry.agents.insert("default/bob".to_string(), bob);
        crate::daemon::persistence::agent_registry::save_agents(&registry).expect("save registry");

        let unscoped = handle(json!({})).expect("unscoped list");
        let unscoped_items = unscoped["items"].as_array().expect("items");
        let global_rows: Vec<_> = unscoped_items
            .iter()
            .filter(|row| row["name"] == "summarize")
            .collect();
        assert_eq!(
            global_rows.len(),
            1,
            "unscoped global skill listing must not multiply by agent count"
        );
        assert_eq!(global_rows[0]["agent_id"], "global:claude-global");

        let scoped = handle(json!({"owner_agent_id": "alice"})).expect("scoped list");
        let scoped_items = scoped["items"].as_array().expect("items");
        let scoped_rows: Vec<_> = scoped_items
            .iter()
            .filter(|row| row["name"] == "summarize")
            .collect();
        assert_eq!(scoped_rows.len(), 1);
        assert_eq!(scoped_rows[0]["agent_id"], "alice");
    }

    #[test]
    fn list_handler_filters_unscoped_global_pool_owner() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let home = std::path::PathBuf::from(std::env::var("HOME").expect("home"));
        let skill_dir = home.join(".claude").join("skills").join("summarize");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: summarize\ndescription: Summarize text\n---\n",
        )
        .expect("skill md");

        let filtered = handle(json!({"owner_agent_id": "global:claude-global"}))
            .expect("global-pool scoped list");
        let items = filtered["items"].as_array().expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["agent_id"], "global:claude-global");
        assert_eq!(items[0]["name"], "summarize");
    }

    #[test]
    fn list_handler_projects_resource_ura_without_extending_install_record_schema() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let agent_root = crate::daemon::persistence::config::agents_root().join("claude");
        let skill_dir = agent_root
            .join(".agents")
            .join("skills")
            .join("inspectable");
        std::fs::create_dir_all(skill_dir.join(".easynet")).expect("skill metadata dir");
        std::fs::write(
            skill_dir.join(".easynet").join("install.json"),
            r#"{
                "name": "inspectable",
                "agent_id": "claude",
                "source": {"kind": "github", "identifier": "owner/repo", "ref": "main"},
                "skill_tree_hash": "sha256:abc",
                "size_bytes": 42,
                "installed_at": "2026-04-23T00:00:00Z",
                "upgrade_available": false
            }"#,
        )
        .expect("install record");

        let mut registry = crate::daemon::persistence::agent_registry::AgentRegistry::default();
        let mut agent = crate::daemon::persistence::agent_registry::AgentEntry::new(
            crate::core::agent::spec::RuntimeKind::Codex,
            None,
        );
        agent.root_path = Some(agent_root);
        registry.agents.insert("default/claude".to_string(), agent);
        crate::daemon::persistence::agent_registry::save_agents(&registry).expect("save registry");
        crate::daemon::persistence::local_agents::save(
            &crate::daemon::persistence::local_agents::LocalAgentsFile {
                host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
                hosted_agents: vec![crate::daemon::persistence::local_agents::HostedAgentEntry {
                    profile: "llm".to_string(),
                    name: "claude".to_string(),
                    agent_ura: "easynet:///r/acme/agent/u1.claude".to_string(),
                    signing_authority: "hosted_by:easynet:///r/acme/device/dev-1".to_string(),
                    first_seen_at: "2026-01-01T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("save local agents");

        let response = handle(json!({"owner_agent_id": "claude"})).expect("skill list");
        let items = response["items"].as_array().expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "inspectable");
        assert_eq!(items[0]["content_hash"], "sha256:abc");
        assert_eq!(
            items[0]["resource_ura"],
            "easynet:///r/acme/resource/agent.u1.claude/skill/inspectable"
        );
        assert!(
            serde_json::from_value::<crate::daemon::resources::skills::store::InstallRecord>(
                items[0].clone()
            )
            .is_err(),
            "response-only resource_ura must not be accepted by the persistence schema"
        );
    }

    #[test]
    fn list_handler_rejects_corrupt_global_skill_package() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let home = std::path::PathBuf::from(std::env::var("HOME").expect("home"));
        let skill_dir = home.join(".claude").join("skills").join("broken");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(skill_dir.join("SKILL.md"), "# Missing declared name\n").expect("skill md");

        let error = handle(json!({"owner_agent_id": "global:claude-global"}))
            .expect_err("corrupt global skill package must fail inventory");

        assert!(
            error.to_string().contains("must declare frontmatter name"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn hosted_skill_owner_projection_resolves_rows_without_scanning_per_row() {
        let local = crate::daemon::persistence::local_agents::LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![crate::daemon::persistence::local_agents::HostedAgentEntry {
                profile: "llm".to_string(),
                name: "claude".to_string(),
                agent_ura: "easynet:///r/acme/agent/u1.claude".to_string(),
                signing_authority: "hosted_by:easynet:///r/acme/device/dev-1".to_string(),
                first_seen_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };
        let snapshot = crate::daemon::persistence::agent_aggregate::AgentAggregateSnapshot::new(
            crate::daemon::persistence::agent_registry::AgentRegistry::default(),
            local,
        );
        let projection = snapshot.hosted_skill_owner_projection();

        assert_eq!(
            projection.hosted_ura_for("claude"),
            Some("easynet:///r/acme/agent/u1.claude")
        );
        assert_eq!(
            projection.owner_name_for_agent_ura("easynet:///r/acme/agent/u1.claude"),
            Some("claude")
        );
        assert_eq!(projection.hosted_ura_for("codex"), None);
    }
}
