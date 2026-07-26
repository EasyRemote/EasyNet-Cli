# Decisions Log

- Decision: add a protocol-violation failure class instead of reusing invocation rejection.
  - Reason: a known failed/cancelled/timed-out invocation is a valid runtime outcome; an unknown wire state is a schema mismatch and must not be modeled as business state.
- Decision: make SPEC v2 enforce absence of synthetic unknown-state labels in remote invocation production code.
  - Reason: this is an architecture boundary, not only a unit behavior; product surfaces must never regain compatibility projections for unknown wire enum values.
