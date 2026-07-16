# Stream/Bidi Lifecycle Cancellation Evidence

## Source Exploration

- `src/daemon/invocation/dispatch/cancellation.rs` already implements the
  descriptor-bound `invocation.cancel` authority. Its data dependency is
  `DescriptorBoundEnvelope + InvocationHandle`; it is not inherently unary.
- `dispatch_shim::dispatch_rpc` registers unary handles and marks them terminal
  after draining finalization.
- `LocalAxonSessionDispatcher::handle_carrier_v1_stream_open` opens an Axon
  `StreamingInvocationHandle` but only records a transport `CancellationToken`
  under `remote_stream_sessions`.
- `LocalAxonSessionDispatcher::register_remote_bidi` splits an Axon
  `BidiInvocationHandle` into input/output halves and stores only the input
  sender under `remote_bidi_sessions`.
- Axon stream and bidi handles both expose `handle() -> &InvocationHandle`, so
  the daemon can register the exact runtime lifecycle without reconstructing
  signed invocation material.

## Boundary Decision

The cancellation registry is the canonical lifecycle index. Session transport
maps (`remote_stream_sessions`, `remote_bidi_sessions`) remain routing state
for EOF/input forwarding and must not become cancellation authorities.
