# Agent Registry-Only Workspace Resolver

## Goal

Separate registered-Agent workspace resolution from paired hosted-identity snapshots, then migrate every package or authoring caller that needs only durable registry state.

## Expected Effect

- Architecture convergence: one registry-only resolver owns `agents.json` workspace interpretation.
- Effect convergence: corrupt or unavailable `local-agents.json` cannot newly block skill packages, ability publication, or ability authoring.
- Product acceleration: callers receive the projection they need without reconstructing registry semantics or acquiring unrelated state.
