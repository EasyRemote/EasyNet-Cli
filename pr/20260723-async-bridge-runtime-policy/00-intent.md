# Intent

## Goal

Remove fallback vocabulary from the canonical sync/async bridge. The bridge
does not preserve a legacy execution path; it selects an explicit runtime policy
when no usable ambient Tokio runtime is available.

## Non-goals

- Do not change sync bridge behavior.
- Do not change MCP reflection, device ability management, LocalRuntime, or
  smoke-test public behavior.
- Do not introduce an alias from the retired `NoRuntimeFallback` name.

## Acceptance criteria

- The bridge policy type is named as a runtime policy, not a fallback.
- Direct call sites pass the renamed policy explicitly.
- Async bridge docs and tests describe policy decisions without fallback
  vocabulary.
- SPEC v2 rejects reintroduction of the retired policy name and comments.
