# Architecture

`failed_dispatch_result` is a terminal-result constructor, not a compatibility
adapter. It belongs at the bidi dispatch boundary because it binds transport
failures to the canonical `DispatchResult` terminal state.

This refactor keeps the helper cohesive:

- transport-specific failure sites choose a default failure code;
- `SessionFailure` owns normalization/classification;
- `DispatchResult` remains the only terminal carrier returned to pending maps.

The SPEC v2 gate is extended at this exact boundary so future edits cannot
reintroduce fallback-shaped naming in the production bidi terminal helper.
