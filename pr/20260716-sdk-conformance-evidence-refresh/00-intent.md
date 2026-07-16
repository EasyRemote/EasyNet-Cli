# Intent

SDK adapter reports pin the exact source bytes that contain each declared
selector. Source changes had left multiple report records stale, so the
conformance runner correctly stopped before executing any language suite.

This slice introduces one explicit maintenance owner for those derived
digests. It refreshes only the SHA-256 values of already-declared evidence;
the Rust runner remains the sole owner of report schema, selector binding, and
live execution attestation.
