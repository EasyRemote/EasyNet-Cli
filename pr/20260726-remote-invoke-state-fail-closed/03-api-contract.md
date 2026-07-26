# API Contract

No public API signature changes.

Behavioral tightening:

- Unknown remote `InvokeResponse.state` values now return a protocol violation error instead of `UNKNOWN_STATE_<n>`.
- Known non-completed states keep the existing error text shape.
