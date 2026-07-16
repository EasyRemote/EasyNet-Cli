# Invariants

1. `control.sock` remains boot/status/diagnostics only.
2. Product ability calls enter through daemon Invocation, not JSON control or a
   callback socket.
3. Local execution is an in-process daemon policy using Axon `LocalRuntime`;
   docs must not describe a second runtime-local-tool callback as current.
4. Runtime lifecycle has one product process kind: `DaemonOnly`.
5. `runtime.json` is a projection. Daemon process/discovery facts are the
   liveness authority.
6. Federation/status state is projected from `JoinConnectionSnapshot`; no
   parallel `FederationInitOutcome` state machine is canonical.
