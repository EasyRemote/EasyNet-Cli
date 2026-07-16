# Boundary Proof

`agents.json` is the source of registered Agent runtime metadata and workspace
configuration. `local-agents.json` is a separate hosted-identity index.

| Consumer | Required state | Forbidden dependency |
| --- | --- | --- |
| Bootstrap plan | registered names, types, models | hosted Agent URAs |
| Curator catalog | registered owner workspace | hosted Agent URAs |
| Device catalog boot/replay | registered runtime entries | hosted Agent URAs |

`AgentAggregateRepository::load_registered_agent_registry_projection` calls
the registry persistence owner directly. It must not be implemented through
`load_snapshot`, whose paired read correctly fails when hosted identity is
unreadable. This prevents a bootstrap dependency cycle where identity repair
requires the identity file to be readable first.

Lifecycle mutation code remains the transaction owner for registry writes. Its
catalog replay consumer reads the post-transaction projection through the
repository rather than reaching into persistence itself.
