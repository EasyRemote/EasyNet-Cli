# Mission Child Target Agent Aggregate Intent

## Objective

Move Mission child-target proof reads off direct Agent registry and
local hosted-agent persistence reads. Mission execution should consume
Agent aggregate projections when it validates EAL target collisions and
resolves hosted Agent child Invocation callees.

## Expected Effect

- Architecture convergence: Agent aggregate owns Agent target read
  projections for Mission execution.
- Cleaner proof chain: Mission child Invocation construction no longer
  understands `agents.json` or `local-agents.json` file layouts.
- Public behavior stability: traditional EAL `call ... on ...` remains
  device-only, and hosted Agent child calls still resolve exact Agent URAs.
