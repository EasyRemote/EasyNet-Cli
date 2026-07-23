# Verification

Passed:

- `cargo fmt --check`
- `cargo test local_target_root_issues_target_bound_tuple_facts --features axon-pb`
- `cargo test principal_ability_target_uses_hub_owner_from_principal_realm --features axon-pb`
- `cargo test pages_ability_targets_pages_agent_callee --features axon-pb`
- `cargo test pages_ability_projects_to_local_registry_key --features axon-pb`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph index .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "invoke_target_root_derived_subject_timeout" --limit 20`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "LocalTargetRootInvocation local_target_root invoke_issued_target_root_timeout" --limit 20`
- `rg -n "invoke_target_root_derived_subject_timeout" src tools/scripts -S`
- `rg -n "LocalTargetRootInvocation|local_target_root\\(|invoke_issued_target_root_timeout" src tools/scripts -S`

Notes:

- `cargo test pages --features axon-pb` currently triggers
  `pages_management_is_user_owned_and_runs_on_the_declared_pages_agent`, which
  requires local daemon credentials and failed with `no credentials found`.
  This is an environment-dependent broad filter, not a regression in this
  slice. The exact Pages tests above passed.
- `codegraph query "invoke_target_root_derived_subject_timeout"` returned no
  semantic results.
- The residual `rg` search found the retired helper only in gate negative
  fixtures / rejection rules.
