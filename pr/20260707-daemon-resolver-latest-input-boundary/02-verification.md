# Verification

Completed checks:

- `cargo test daemon::invocation::routing::route_resolver::tests::resolve_query_json_ignores_retired_snake_case_input_aliases --lib`
- `cargo test daemon::invocation::dispatch::daemon_invocation_service_tests::unary::invoke_rejects_namespace_proxy_resolve_legacy_input_aliases --lib`
- `bash tools/scripts/check-daemon-latest-input-boundary.sh`
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
- `cargo test ffi::directory::tests:: --lib`
- `cargo test protocol::directory_contract::tests::project_resolved_ref --lib`
- `cd sdk/go && go test . -run 'TestDirectoryRuntimeTransport|TestPublicationRuntimeTransport' -count=1`
- `bash tools/scripts/check-sdk-completion-audit.sh`

Notes:

- The second Cargo command was run as `cargo test invoke_rejects_namespace_proxy_resolve_legacy_input_aliases` because the test lives under the dispatch service module path exposed by the full test target, not the initially guessed `--lib` filter path.
- `git diff --check` passed.
- The aggregate completion audit passed through scaffold, parity, conformance
  reports, section 27 coverage, ABI/header checks, URA naming, daemon latest
  input boundary, daemon Invocation migration, EasyRemote/backend boundaries,
  product smokes, Python SDK live smoke, Go SDK live smoke, cutover readiness,
  and completion matrix.
