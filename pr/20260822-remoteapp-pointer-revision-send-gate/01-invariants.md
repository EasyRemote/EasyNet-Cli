# Invariants

1. Daemon stale-revision rejection remains authoritative.
2. Frontend `rdSendInput` must fail closed before data-channel send when a
   pointer/wheel frame misses the current target geometry revision.
3. Keyboard readiness remains controlled by daemon-projected input readiness.
4. This is source/test evidence only; successful real OS input injection E2E is
   still required before product completion.
