# Skill Publish Agent Aggregate Owner

## Goal

Route `skill.publish` owner root/type resolution through the Agent aggregate instead of loading `agents.json` inside the skill ability.

## Expected Effect

- Architecture convergence: Agent registry read ownership stays behind `AgentAggregateRepository`.
- Effect convergence: `skill.publish`, `skill.unpublish`, `skill.tree`, `skill.read_file`, and `skill.write_file` keep the same public behavior and error shape for registered and missing owners.
- Product acceleration: skill package operations receive a domain projection for registered owner workspace facts instead of parsing registry rows locally.
