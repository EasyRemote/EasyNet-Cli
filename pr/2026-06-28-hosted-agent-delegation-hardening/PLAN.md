# Hosted Agent Delegation Hardening

## Invariants

1. CLI processes never sign hosted-agent delegation authority for the daemon.
2. Unsigned hosted-agent delegation requests are accepted only after trusted loopback admission.
3. Ability handlers observe only signed hosted-agent delegation metadata.
4. Public signed callers cannot inject either signed hosted-agent delegation tokens or unsigned delegation requests.
5. The signed delegation token remains bound to the Axon invocation caller, callee, subject, nonce, and public route ability.

## Boundary Proof

The CLI owns invocation intent. The daemon owns local hosted-agent authority because it is the process that owns the embedded Axon runtime, local hosted-agent placement, and `_system.local` process capability. The transport layer is therefore the only valid conversion point from an unsigned local request into signed handler metadata.

## Implementation Order

1. Add a typed hosted-agent delegation request value object.
2. Add a daemon transport signer that consumes unsigned loopback metadata and emits signed metadata.
3. Change CLI local invocation helpers to send request metadata instead of signed delegation tokens.
4. Reject unsigned request metadata on public ingress.
5. Add focused tests for the new state transition.
