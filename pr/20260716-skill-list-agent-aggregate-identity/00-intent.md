# Skill List Agent Aggregate Identity

## Goal

Route `skill.list` hosted-owner scope resolution through the Agent aggregate instead of loading `local-agents.json` beside the registry.

## Expected Effect

- Architecture convergence: `skill.list` receives one aggregate snapshot for registry rows and hosted Agent identity.
- Effect convergence: `agent_ura` scoped requests still resolve to the local hosted owner name, and skill resource URAs still use hosted Agent URAs.
- Product acceleration: skill inventory traversal no longer carries a private hosted identity index.
