# Invariants

- Daemon-local loopback calls still use `_system.local` as the caller.
- The daemon's runtime-published identity still comes only from control
  discovery; no default device or all-zero fallback may be introduced.
- Hosted-agent authority facts remain local runtime facts and must not be
  described as product-owned protocol authority.
- `LocalInvokeStatusCode` remains a one-to-one projection from tonic status
  codes; duplicate arms are removed without changing mapped values.
