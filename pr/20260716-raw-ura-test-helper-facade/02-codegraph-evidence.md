# CodeGraph-style evidence

The failing guard identified five raw formatter roots:

```text
src/daemon/execution/mission/invocation_gateway.rs
src/daemon/federation/read_model/owner_projection.rs
src/daemon/persistence/federation_revoke.rs
src/daemon/persistence/agent_lifecycle.rs
src/daemon/ability/builtins/governance/access_control.rs
```

Source exploration confirmed they are inline test helpers or fixtures inside
production modules. The architectural owner remains `crate::core::ura`, which
re-exports Axon-owned builders such as `agent_ura`, `device_ura`,
`hub_ability_ura`, and `resource_dot_ura`.
