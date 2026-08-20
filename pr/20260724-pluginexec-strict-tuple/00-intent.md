# Intent

## Goal

Remove provider sidecar helper tuple defaulting so declarative exec plugins
receive the exact daemon-admitted canonical invocation frame instead of a frame
repaired by language helpers.

## Non-goals

- Do not move EasyNet provider sidecar concepts into the canonical SDK root.
- Do not add product-specific behavior to the SDK.
- Do not add compatibility fallbacks for old sidecar frames.
- Do not open stream/bidi helper states; exec invoke remains the only
  provider-backed helper contract in this turn.

## Acceptance criteria

- Go, Python, Rust, Java, and Node helper packages require explicit
  `causal_context` and `args` fields.
- Missing `causal_context`/`args` returns a protocol error before invoking a
  plugin handler.
- The SPEC v2 helper matrix gate rejects optional/default tuple repair.
- Focused helper tests pass across supported helper languages.
