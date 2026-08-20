# Node Invocation Handle Seam Intent

Implement the Node/TypeScript Runtime Core submitted-invocation handle seam
described by `docs/spec/daemon-sdk-requirements-v1.md`.

## Scope

- Add typed `InvocationHandle`, `InvocationHandleEvent`, and
  `InvocationCancel` DTOs.
- Add `RuntimeClient` await/cancel/events/free-handle operations over injected
  transport methods.
- Bind submitted handles back to their `RuntimeClient` for ergonomic handle
  methods.
- Preserve terminal monotonicity at the SDK DTO boundary.

## Out Of Scope

- No daemon transport provider.
- No C ABI bridge.
- No canonical Invocation preparation or signing implementation in this slice.
- No conformance claim for `invocation/handle_terminal_monotonicity` until
  Node evidence covers prepare/sign/submit/await/cancel/events together.
