# Runtime Dispatch Latest Mode Boundary

## Intent

Remove the daemon runtime-dispatch request fallback that treated an omitted
`mode` field as `rpc`. The runtime-dispatch bridge must consume the latest
request shape explicitly and must not preserve stale input compatibility.

## Scope

- Require `mode` on runtime-dispatch request JSON.
- Keep accepted modes limited to `rpc` and `stream`.
- Reject missing or unknown modes as `BAD_REQUEST`.
- Extend the daemon latest-input gate so the fallback cannot be reintroduced.

## Non-Scope

- No public SDK API changes.
- No runtime-dispatch wire expansion.
- No Axon protocol or generated type changes.
