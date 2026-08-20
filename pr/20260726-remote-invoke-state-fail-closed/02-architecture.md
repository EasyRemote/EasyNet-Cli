# Architecture

Remote unary invocation has one terminal state decoder:

1. Decode the Axon wire `InvocationState`.
2. Accept `Completed` as success.
3. Report known non-completed states with structured error code/message.
4. Reject unknown integer states as protocol violations.

This keeps wire-state interpretation in the canonical remote invocation adapter instead of leaking fallback labels into product surfaces.
