# Invariants

- A `status_file` health companion must have exactly one authoritative status-file path in its plan.
- Platform supervisors may observe files, but they must not own manifest path semantics.
- Missing or invalid heartbeat content remains a health error, not a daemon boot failure.
- Status-file cleanup is best-effort during platform removal and must not hide supervisor cleanup errors.
- Axon SDKs remain free of desktop companion lifecycle ownership.
