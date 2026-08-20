# Boundary proof

## Root abstraction problem

Provider helpers were made strict, but the daemon sidecar frame type still
carried `#[serde(default)]` on `causal_context` and `args`. That kept a lower
layer compatibility path alive: old or incomplete sidecar frames could still be
decoded with synthesized tuple fields before reaching helper-level validation.

## Owning boundary

`src/daemon/plugins/sidecar/frame.rs` owns the host-to-sidecar wire model. It
must represent the daemon-admitted invocation envelope exactly. The provider
helpers only project this frame; they must not compensate for host-side
defaulting, and the host type must not compensate for incomplete input.

## Canonical invariant

- The daemon sidecar invocation envelope requires explicit `caller_ura`,
  `callee_ura`, `ability_ura`, `subject_ura`, `invocation_nonce`,
  `causal_context`, and `args`.
- Missing tuple fields are decode errors.
- Causal-context examples use canonical runtime shape, e.g. `{"form":"none"}`.
- No sidecar-specific causal context aliases such as `trace_id` or `root` are
  treated as canonical examples.

## Compatibility removed

Removed host-side decode compatibility:

- missing `causal_context` -> JSON null/default
- missing `args` -> JSON null/default

This keeps daemon frame ownership aligned with the strict provider helper
contract from the previous iteration.
