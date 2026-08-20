# Architecture

Root abstraction problem:

The existing `AgentCommandGateway` is named as a command boundary but also owns
read-model access. That violates high cohesion: read authority and mutation
authority have different subject semantics.

Refactoring:

- Add `AgentStateReadGateway` for `agent.list`.
- Implement the production state reader with `LocalRuntimeStateReadIssuer`.
- Keep `AgentCommandGateway` for mutating agent abilities.
- Migrate CLI helper functions that need `DaemonAgentRow` to the state reader.

The result is a cleaner local product boundary: read projections bind to the
paired-user runtime-state subject, while mutating actions remain explicitly
command-shaped.
