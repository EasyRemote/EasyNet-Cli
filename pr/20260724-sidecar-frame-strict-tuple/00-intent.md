# Intent

## Goal

Remove daemon sidecar frame defaulting so the host-side frame model requires the
same complete canonical invocation tuple that provider helpers now require.

## Non-goals

- Do not add a sidecar-specific causal-context model.
- Do not move provider sidecar semantics into the canonical SDK root.
- Do not preserve decode compatibility for missing `causal_context` or `args`.

## Acceptance criteria

- `SidecarInvocationEnvelope` no longer uses serde defaults for
  `causal_context` or `args`.
- Sidecar frame tests use canonical causal-context examples.
- Missing `causal_context` and missing `args` fail at daemon frame decode.
- SPEC v2 rejects daemon sidecar frame defaulting.
