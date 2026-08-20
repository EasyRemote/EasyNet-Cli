# Decisions Log

- Decision: use a metadata-only explicit authority fixture rather than adding a
  LocalRuntime.
  Rationale: the registration smoke test asserts handler presence only; adding
  runtime state would broaden the fixture without improving the proof.
