# CodeGraph Notes

`codegraph query "skill publish list AgentType AgentRegisteredWorkspace agent_registry aggregate projection skill layout" --path . -l 180` showed:

- `AgentRegisteredWorkspace::agent_type()` exposed `agent_registry::AgentType`.
- `skill.publish` selected managed and fallback skill paths from that raw type.
- `skill.list` iterated `snapshot.registry.agents` and called
  `required_root_path` directly.
- `store::global_skill_pools_for` and `global_skill_dir_for` accepted raw
  `AgentType`.

The root fork is type ownership, not path spelling: skill code needs a skill
layout projection, while registry row type is a persistence detail.
