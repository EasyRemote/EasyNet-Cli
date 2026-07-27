## Goal

Remove the plugin manifest compatibility default that silently treats an ability without `call_mode` as RPC.

## Non-goals

- Do not change public invocation semantics.
- Do not add support for new plugin transports.
- Do not weaken sidecar/declarative loadability checks.

## Acceptance criteria

- Every `[[ability_metadata]]` row must declare `call_mode`.
- Missing `call_mode` fails during manifest parsing before descriptor publication or handler registration.
- Repository fixtures and tests use explicit call mode declarations.
- Plugin sidecar/declarative gates continue to use the manifest-declared call mode as the sole routing authority.
