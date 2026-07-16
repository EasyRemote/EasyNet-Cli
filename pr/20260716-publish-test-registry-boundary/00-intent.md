# Publish Test Registry Boundary

## Goal

Remove the production-module `agent_registry` alias from
`ability.publish`/`ability.unpublish` while keeping the existing tests that
materialise throwaway agents.

## Concrete Use Case

The full cutover readiness run repeatedly compiles the crate and reports:

```text
unused import: crate::daemon::persistence::agent_registry as agents
```

The import is only needed by the test helper. Leaving it at production module
scope makes the publish path look like it still depends directly on procedural
agent registry persistence even though runtime owner-root resolution now goes
through `AgentAggregateRepository`.

## Non-Goals

- Do not change publish/unpublish behavior.
- Do not alter agent registry or aggregate persistence.
- Do not touch unrelated warning families in generated route files or other
  dirty working-tree modules.

## Acceptance Criteria

1. Production publish module no longer imports `agent_registry as agents`.
2. Test helper keeps explicit access to `agent_registry` for fixture setup.
3. Focused publish tests still pass.
4. Focused Rust compile no longer reports this unused import.
