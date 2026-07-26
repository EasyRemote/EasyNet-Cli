Invariants:

1. `owner is not online` must not be projected as `ABILITY_NOT_FOUND`.
2. `ROUTE_NEGATIVE + NEGATIVE_REASON_NXDOMAIN + owner is not online` is a
   descriptor-owner offline terminal state.
3. Daemon route status should use availability transport status for owner
   liveness failures.
4. Go and Python direct-runtime providers must canonicalize both legacy
   `NOT_FOUND` and current `UNAVAILABLE` owner-offline payloads to
   `DESCRIPTOR_OWNER_OFFLINE`.
5. Plain descriptor absence remains `DESCRIPTOR_NOT_FOUND`.
