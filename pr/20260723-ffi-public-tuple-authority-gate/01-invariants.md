# Invariants

1. `caller_ura`, `callee_ura`, and `subject_ura` are semantic tuple facts, not optional routing hints.
2. Public FFI invocation rejects all-zero placeholder principals before daemon transport.
3. Public FFI invocation rejects malformed or contradictory delegation/session authority metadata before daemon transport.
4. FFI validation reuses the canonical daemon authority metadata parser; it must not duplicate a second authority grammar.
5. The public API shape remains compatible: callers still submit the same JSON tuple, but invalid tuples fail earlier with deterministic ABI errors.
