# Verification Plan

```bash
cargo test agent_ability_specs --lib
cargo test format_skills_hint --lib
cargo test --features axon-pb --lib --no-run
bash tools/scripts/check-architecture-convergence.sh
git diff --check -- src/daemon/execution/mission/agent_ability_specs.rs pr/20260716-agent-ability-spec-discovery-dto
```

The no-run compile should not report unused `AgentAbilitySpec::parameters` or
unused `parameters` field diagnostics.

# Results

- `cargo test agent_ability_specs --lib`: 18 passed.
- `cargo test format_skills_hint --lib`: 2 passed.
- `cargo test --features axon-pb --lib --no-run`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`:
  `architecture-convergence: OK`.
- `bash tools/scripts/check-runtime-abilities-manifest-boundary.sh`:
  `check-runtime-abilities-manifest-boundary: ok`.
- `rg 'AgentAbilitySpec|parameters' target/agent-ability-spec-no-run.log`: no
  matches; the retained-parameter warning is absent.
- `git diff --check -- src/daemon/execution/mission/agent_ability_specs.rs
  pr/20260716-agent-ability-spec-discovery-dto`: clean.
