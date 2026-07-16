# API Contract

## Public Surface

No public CLI, daemon RPC, or Axon Invocation API changes.

## Internal Contract

`TargetGate::new(...)` still constructs a cheap per-dispatch gate. Internally, it loads one Agent aggregate snapshot to construct `LocalAgentTargetIndex`.

## Error Contract

If the aggregate snapshot cannot be loaded, Agent URA self-target matching fails closed. The gate records an operational event with the load source classification and continues to evaluate daemon, hub, and device URA matches.

## Tenant Rules

The existing realm and user checks remain unchanged:

- Hosted identity match requires exact parsed URA tuple equality.
- Credential-plus-registry match requires credential realm/user equality and exact bare Agent ID membership.
