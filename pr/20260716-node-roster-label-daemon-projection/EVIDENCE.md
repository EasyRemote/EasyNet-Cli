# Evidence

## Source Exploration

- `src/daemon/federation/read_model/a2a_labels.rs` states that
  `AgentRegistry` contributes roster metadata only, while ability identity,
  schema, description, transport, and visibility come from
  `LocalAbilityPublicationSnapshot`.
- `build_agents_envelope(registry, publication)` omits roster-only agents and
  projects only live RPC descriptors from the committed publication snapshot.
- `src/daemon/ability/builtins/integrations/a2a/bridge.rs` captures
  `LocalAbilityPublicationSnapshot` from the live `AxonAbilityCatalog` before
  returning the structured A2A envelope.
- `a2a.bridge.send_task` re-checks the live registry and descriptor publication,
  then dispatches through the daemon invocation routing target.
- `src/daemon/execution/mission/agent_ability_specs.rs` documents that
  hosted-agent descriptors are registered in the daemon canonical control-plane
  catalogue, bound to daemon Invocation, and published from the same live
  catalogue snapshot used by discovery.

## Root-Fork Closed

The previous spec wording assigned normative ownership to bridge label
registration and process-local adapter registration. Current code already uses
the daemon catalogue as the owner and treats labels as a projection. This slice
closes the documentation/source-of-truth fork without changing public behavior.

## Commands

Commands are recorded in `VERIFY.md` after execution.
