Boundary proof:

- `start_daemon_invocation_transport` owns the paired User signer provisioning and
  custody proof because it owns invocation admission/trust-anchor construction.
- `ready_runtime_discovery` owns the final control discovery write contract because
  it is the last step before `control-ready` and `daemon ready`.
- Therefore `ready_runtime_discovery` must validate that the required invocation
  readiness facts are present instead of treating capability flags as optional
  display metadata.

Rejected approach:
- Do not add a later SDK fallback that provisions a missing signer during
  descriptor resolution. That would keep two identity authorities alive.
