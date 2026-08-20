# Invariants

- EAL agent member-call dispatch must consume one aggregate-owned registered Agent projection per dispatcher construction.
- A registry load failure must not be silently treated as success; operator-visible warning behavior remains.
- Missing Agent targets remain `not_found` through `dispatch_to_agent`.
- EAL agent steps continue to lower into daemon-owned Axon Invocation; no direct executor or chat path may be introduced.
- The Agent aggregate repository remains the only owner that reads durable Agent registry state for public execution-path projections.
