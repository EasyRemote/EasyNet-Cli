# Verification

## CodeGraph

- `codegraph status .`: index up to date.
- `codegraph explore "principal_command PrincipalLifecycle actor fallback actor_ura principal CLI command proof idempotency"`:
  identified `principal_command` as the single command JSON construction
  helper with 22 callers in `src/cli/commands/groups/principal.rs`.

## Commands

- `rustfmt src/cli/commands/groups/principal.rs`: passed.
- `bash -n tools/scripts/check-architecture-convergence.sh && bash -n tests/scripts/test_check_architecture_convergence.sh`:
  passed.
- `tools/scripts/check-architecture-convergence.sh`: passed with
  `architecture-convergence: OK`.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed with all
  cases.
- `cargo test -q principal_command --lib`: compiled successfully but matched
  no tests.
- `cargo test -q principal --lib`: passed, 48 tests.
- `git diff --check -- src/cli/commands/groups/principal.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-principal-command-actor-boundary`:
  passed.

## Notes

The working tree contains unrelated dirty documentation and existing
agent-purge/formatting changes. This slice does not revert or depend on those
changes for the PrincipalLifecycle actor-source refactor.
