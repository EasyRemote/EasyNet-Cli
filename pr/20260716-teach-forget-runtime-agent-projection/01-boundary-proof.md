# Boundary Proof

`AuthorizedForget::from_request` already needs a coherent aggregate snapshot
because hosted identity authority and registered runtime state must be observed
from the same durable read view.

The governance workflow may request:

- an optional registered runtime entry, because a removed learner has no runtime
  convergence work left;
- an optional ability manifest path, because descriptor artifact cleanup should
  use the registered workspace when it still exists.

It must not know that those values live under `AgentAggregateSnapshot.registry`
or that the registry stores agents in a map. `agent_aggregate.rs` owns that
persistence shape and exposes named projections for the workflow.
