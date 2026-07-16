# CodeGraph Notes

## Static Graph Evidence

- `src/daemon/ability/builtins/resources/skills/list.rs::handle` loads both `agent_registry::load_agents()` and `local_agents::load().ok()`.
- `HostedAgentUraIndex::from_local_agents` builds a local name-to-URA map from `LocalAgentsFile.hosted_agents`.
- `SkillListScope::from_args` resolves `agent_ura` and `subject_ura` by calling `owner_name_for_agent_ura` with `LocalAgentsFile`.
- `scoped_skill_resource_ura` only needs a lookup interface, not persistence rows.
- `src/daemon/persistence/agent_aggregate.rs` already owns aggregate snapshot loading and hosted identity projections.

## Convergence Decision

Add an aggregate-owned hosted skill owner projection and migrate `skill.list` to consume that projection. Add an executable convergence rule to keep the ability from learning `LocalAgentsFile` again.
