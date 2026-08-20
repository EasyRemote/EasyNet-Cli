# Invariants

## Runtime Boundary

- The daemon sidecar frame contract is language-neutral:
  executable process receives one JSON frame on stdin and emits one JSON frame
  on stdout.
- The sidecar helper is provider-scoped daemon integration, not a canonical SDK
  root concept.
- Canonical runtime packages expose generic runtime primitives only.

## Safety

- Public templates must use provider-backed helpers instead of hand-written
  JSON protocol parsing.
- A language with no provider-backed helper must not be generated as a plugin
  template.
- Helper packages must not mint authority, synthesize proof facts, or bypass
  descriptor-bound invocation.

## Boundedness

- Sidecar template generation is a closed set controlled by a capability matrix.
- Adding a new language requires updating the matrix, helper surface, template,
  and negative gate in the same root-fork slice.
