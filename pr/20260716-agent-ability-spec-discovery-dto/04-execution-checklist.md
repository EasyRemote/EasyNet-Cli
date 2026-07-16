# Execution Checklist

- [x] Confirm `agent_ability_specs` is crate-private.
- [x] Use `rg` and CodeGraph-style queries to find `AgentAbilitySpec` callers.
- [x] Confirm production callers do not consume `parameters()`.
- [x] Remove retained schema payload from the DTO.
- [x] Move schema assertions to manifest-source tests.
- [x] Run focused ability-spec and chat tests.
- [x] Run architecture and warning checks.
- [x] Record verification results.
