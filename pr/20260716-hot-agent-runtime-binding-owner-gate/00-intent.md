# Hot Agent Runtime Binding Owner Gate

## Root Fork

Hot-added hosted agents need one authority source when materializing runtime
rows. The production registrar currently derives `HostedAgentRuntimeBinding`
from `catalog.enroll_persisted_hot_agent_authority(name)` and the enrollment's
`authority_root()`, but the architecture checker does not protect that path.
A later change could reintroduce direct `local-agents.json` display-name lookup
beside the catalogue authority enrollment.

## Expected Effect

Architecture convergence. Hot-agent runtime registration uses the same
catalogue-owned authority enrollment that protects advertise/invoke authority;
it does not rebuild hosted identity lookup from persistence file shapes.

## Boundary Invariants

1. `register_agent_replacing` must call
   `enroll_persisted_hot_agent_authority(name)`.
2. `HostedAgentRuntimeBinding` must use `enrollment.authority_root()` as the
   runtime authority root.
3. The production binding path must not call `local_agents::load`,
   `lookup_hosted_ura`, `lookup_hosted_agent_by_name`, or perform a separate
   aggregate lookup after authority enrollment.
4. Runtime row materialization remains owned by `HotAgentRegistrar`; the
   catalogue authority enrollment owns the runtime authority proof.

## Verification

- Add `R34B_HOT_AGENT_RUNTIME_BINDING_AGGREGATE_FORK` to the architecture
  convergence checker.
- Add a negative fixture where `register_agent_replacing` reads
  `local-agents.json` directly for the runtime binding.
- Run the checker and checker self-tests.

Commands run:

- `bash -n tools/scripts/check-architecture-convergence.sh`
- `bash -n tests/scripts/test_check_architecture_convergence.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
