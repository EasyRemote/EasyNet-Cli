# Hosted Advertise Prelude Agent Aggregate

## Goal

Route hosted-Agent session advertisement rows through the Agent aggregate hosted-identity projection instead of letting the bidi session prelude read `local-agents.json` directly.

## Expected Effect

- Architecture convergence: one hosted-Agent identity read owner remains, `AgentAggregateRepository`.
- Effect convergence: advertised rows keep the existing persisted-agent and synthetic `pages`/`files` behavior.
- Product acceleration: session startup code becomes a publisher of aggregate rows, not a second persistence parser.

