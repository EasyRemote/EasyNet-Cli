# Intent

## Goal

Remove product-visible runtime error compatibility leakage where descriptor,
route, or caller-signer failures can surface as low-level implementation
details instead of canonical runtime error classes.

## Non-goals

- Do not weaken signer custody or descriptor-bound invocation requirements.
- Do not add stale-data compatibility fallbacks for product UIs.
- Do not make the SDK product-specific to compensate for daemon state defects.

## Acceptance criteria

- One concrete production leakage path is removed or structurally gated.
- The owning layer projects canonical runtime state instead of raw transport or
  key-service strings.
- A deterministic regression check proves the retired leakage cannot return.
