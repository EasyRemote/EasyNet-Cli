# API Contract

No public CLI flags or output schemas change.

Internal contract:

- `AgentCommandGateway` must not be used for `agent.list`.
- `AgentStateReadGateway` is the only injectable read seam for daemon-owned
  Agent rows.
- Production `AgentStateReadGateway` must call `LocalRuntimeStateReadIssuer`.
