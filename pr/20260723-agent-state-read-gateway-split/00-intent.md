# Intent

Split read-only Agent state projection from the generic Agent command gateway.

`AgentCommandGateway::invoke` currently carries both mutating Agent actions
(`agent.start`, `agent.refresh`, `agent.ability.put`) and read-only state
projection (`agent.list`). That keeps a legacy local-invoke seam alive: product
code can accidentally read runtime state through the daemon-self tuple shortcut
instead of the paired-user runtime-state read subject.

This slice introduces a dedicated Agent state read gateway and migrates
`agent.list` consumers to it. Mutating Agent commands keep their existing action
gateway.
