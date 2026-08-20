# Intent

## Goal

Remove fallback vocabulary from runtime failure-code classification. The
classifier does not preserve a legacy path; it chooses a caller-provided default
state-machine code when no more specific runtime/admission code is proven.

## Non-goals

- Do not change emitted failure codes.
- Do not change receipt, mission, join, or bidi terminal behavior.
- Do not alter Axon error stage/security-class mapping.

## Acceptance criteria

- `FailureCodeClassifier` API names the caller-provided value as a default code.
- Direct callers use default-code naming.
- Tests describe default-code behavior without fallback vocabulary.
- SPEC v2 rejects reintroduction of fallback naming in this classifier.
