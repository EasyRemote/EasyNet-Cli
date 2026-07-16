# Boundary Proof

## Owner

`src/daemon/persistence/agent_aggregate.rs` owns hosted-Agent identity read projections.

## Boundary

`src/daemon/invocation/bidi/session_initiator/prelude.rs` may publish hosted-Agent advertisement entries to the Hub. It must not load `local-agents.json`, inspect `LocalAgentsFile`, inspect `hosted_agents`, or duplicate row de-duplication rules.

## Invariants

- The aggregate filters blank and `<unjoined>` Agent URAs before advertisement.
- Duplicate Agent URAs are advertised once.
- Synthetic `pages` and `files` rows are still derived only when realm and user segment are present and user segment is not `self`.
- The prelude receives stable `agent_ura` and `short_label` accessors from `AgentHostedAdvertiseEntry`.

