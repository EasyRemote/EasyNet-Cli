# Intent

## Goal

Make the `agent.stop` versus `agent.purge` lifecycle split executable in the
architecture convergence gate.

## Expected Effect

- Effect type: architecture convergence.
- Root fork addressed: destructive Agent lifecycle semantics leaking through the
  non-destructive `agent.stop` boundary.
- Concrete use case: callers and catalogs must be able to distinguish
  row/authority removal from destructive root-directory removal before invoking
  a lifecycle ability.

## Non-goals

- Do not change public `agent.stop` or `agent.purge` behavior in this slice.
- Do not add a compatibility path that accepts `purge` on `agent.stop`.
- Do not touch unrelated docs/spec/skills/packaging worktree changes.
