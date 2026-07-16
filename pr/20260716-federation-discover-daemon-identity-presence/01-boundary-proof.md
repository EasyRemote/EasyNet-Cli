# Boundary Proof

## Owners

- `local_invocation::local_daemon_ura` owns daemon loopback identity selection.
- `remote_invoke::invoke_federation_discover_filtered` owns the CLI transport
  envelope for local daemon `federation.discover`.
- `UnaryDispatcher::dispatch_federation_discover` owns daemon-side merging of
  local presence and federated directory projections.

## Invariants

- Hub mode must not use `local_device_ura()`'s unpaired fallback for daemon
  loopback calls.
- Device and both modes keep device URA loopback behavior when control discovery
  carries a node id.
- Hub loopback discover uses the owner ability URA as subject; device loopback
  keeps the daemon/device URA subject.
- Local presence entries are filtered by requested `agent_ura` and deduplicated
  against federated directory results by Agent URA.
- Remote federated entries remain visible; local presence only fills the local
  daemon's realm view.
