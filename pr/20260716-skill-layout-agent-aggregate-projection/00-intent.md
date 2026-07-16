# Skill Layout Agent Aggregate Projection

## Goal

Move skill package placement decisions off raw `AgentRegistry::AgentType` and
onto an Agent aggregate projection consumed by `skill.publish`, `skill.list`,
and shared skill store helpers.

## Concrete Use Case

Claude Code agents load managed skills from `<agent-root>/.claude/skills`,
while Codex-style agents use `<agent-root>/skills` and different global skill
pools. The skill abilities need that placement fact, not the registry row type
or registry persistence shape.

## Expected Effect

- Skill surfaces consume `AgentSkillLayout` from the aggregate boundary.
- `skill.list` asks the aggregate snapshot for registered workspace facts by
  owner name instead of iterating registry rows directly.
- Existing public behavior, response fields, and skill paths stay unchanged.
