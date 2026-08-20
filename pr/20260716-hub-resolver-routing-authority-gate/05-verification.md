# Verification

## CodeGraph

- `codegraph status .`: index up to date.
- `codegraph explore "HubResolver allow_directory_fallback directory endpoint static peer routing authority fallback"`:
  identified `HubResolver::resolve` as the owner of remote hub route source
  precedence, with `RouteResolver::resolve_delegation` consuming the typed
  outcome.

## Commands

- `bash -n tools/scripts/check-architecture-convergence.sh && bash -n tests/scripts/test_check_architecture_convergence.sh`:
  passed.
- `tools/scripts/check-architecture-convergence.sh`: passed with
  `architecture-convergence: OK`.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed with all
  cases.
- `cargo test -q hub_resolver --lib`: passed, 6 tests.

## Outside This Slice

An earlier run failed before reaching HubResolver tests because `agent.list`
fixture wiring still expected a registry-only provider. The current tree fixes
that through the separate Agent aggregate snapshot slice.
