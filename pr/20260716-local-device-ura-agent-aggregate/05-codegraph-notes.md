# CodeGraph Notes

Static graph inputs:

- `rg -n "local_agents::load|lookup_hosted_ura|LocalAgentsFile|AgentAggregateRepository::load_snapshot|load_hosted_identity_snapshot|hosted_llm_agent_ura" src --glob '*.rs'`
- `rg -n "local_device_ura|persisted_local_device_ura|host_device_agent_ura|clipboard_tracker::spawn|clipboard-tracker|load_hosted_identity_status" src/daemon src/cli tests --glob '*.rs'`
- `bash tools/scripts/check-architecture-convergence.sh`

Findings:

- The current convergence gate is green, so the next slice must extend coverage
  rather than satisfy an existing failing rule.
- `local_invocation::persisted_local_device_ura()` and
  `clipboard_tracker::spawn()` are the two cohesive production readers that
  only need host device URA identity state and do not perform lifecycle
  mutation.
- Lifecycle and bootstrap writers still legitimately own mutation of
  `LocalAgentsFile`; they are outside this read-projection slice.
