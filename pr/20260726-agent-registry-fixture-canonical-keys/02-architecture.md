# Architecture

## Boundary

`agent_registry::save_agents` is the single persistence boundary for durable
agent registry rows. It already validates that keys are canonical `AgentId`
strings.

## Design

Update tests and fixtures to use canonical registry keys at persistence and
lookup boundaries while continuing to pass short names to public CLI APIs.

This removes a legacy fixture model instead of adding a production fallback.

## Ownership

- Production validation remains in `agent_registry`.
- Test helpers own canonical key derivation for fixture rows.
- CLI tests assert against canonical storage while keeping public selector
  behavior unchanged.
