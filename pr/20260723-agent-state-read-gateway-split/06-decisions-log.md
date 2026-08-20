# Decisions Log

- Decision: do not migrate mutating Agent commands to the runtime-state read
  issuer.
  Rationale: actions and reads have different authority semantics; mechanical
  migration would hide the same design error under a different helper.
- Decision: keep a test injection seam for Agent state reads.
  Rationale: product tests need to prove missing daemon projections fail before
  filesystem fallback or mutation.
