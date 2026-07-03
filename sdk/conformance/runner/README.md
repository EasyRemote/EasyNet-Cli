# Conformance Runner

This directory defines the runner contract for language SDK conformance.

The runner is intentionally not implemented as a single language harness yet.
Each SDK facade may provide its own adapter, but it must load the same cases
from `../cases`, the same fixtures from `../fixtures`, and emit equivalent
machine-readable results.

Minimum result record:

```json
{
  "case_id": "invocation/complete_tuple",
  "language": "rust",
  "profile": "runtime_core",
  "status": "passed",
  "error_code": null
}
```

Skipped required cases block a `language-stable` claim.
