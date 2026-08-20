# Intent

Move startup hosted-agent identity projection under the Agent lifecycle owner.

`cli start` currently builds the bootstrap plan and writes `local-agents.json`
directly before daemon boot. That makes startup a second production writer for
Agent identity projection, parallel to the lifecycle aggregate that already
owns registry and identity persistence for start/stop/purge/refresh.

## Expected effect

- Architecture convergence: startup identity projection enters the Agent
  lifecycle mutation guard and projection store.
- Public behavior unchanged: device startup still refreshes hosted identity
  projection before daemon boot.
- No compatibility fallback is added; the direct CLI writer is deleted.
