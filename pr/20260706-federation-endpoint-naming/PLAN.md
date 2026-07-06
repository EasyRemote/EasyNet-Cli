# Federation Endpoint Naming Convergence Plan

## Objective

Remove the active daemon federation `HubUri` abstraction because it represents
a dialable TCP/TLS endpoint, not an EasyNet/Axon URA. Converge naming with the
runtime-boundary rule that identity is URA and transport location is endpoint.

## Invariants

- The daemon SDK requirements SPEC remains unchanged.
- No compatibility alias from `HubUri` to the new name is kept.
- URA remains reserved for routable EasyNet/Axon identities.
- Hub endpoint remains a transport locator such as `https://host:port`.
- Federation admission, trust-anchor lookup, channel cache, and breaker state
  behavior remain unchanged.
- The rename must be semantic and complete across trait surfaces, tests, error
  types, and comments in active code.

## Boundary Proof

The cross-hub dialer does not carry caller/callee/subject identity. It dials a
peer daemon transport endpoint selected from trust-anchor policy. Therefore the
canonical abstraction is `HubEndpoint`; using `HubUri` conflates transport and
identity and violates the URA-only naming constraint.

## Implementation Steps

1. Rename `HubUri` to `HubEndpoint` in the federation client module.
2. Migrate `FederationClient` method signatures, error variants, channel cache
   keys, breaker keys, mocks, and call sites.
3. Rename local `target_hub` parameters where they denote endpoints.
4. Clean URI-era comments in active federation code.
5. Rename the surface scope helper from URI to URA because it matches URAs.
6. Run targeted Rust tests, format checks, and SPEC drift checks.

## Verification Plan

- `cargo test federation --lib`
- `cargo test --test cross_realm_directory_poll_e2e`
- `cargo test --test cross_realm_directory_streaming_e2e`
- `cargo test --test cross_realm_signed_admission_e2e`
- `cargo test daemon::invocation::dispatch::daemon_invocation_service::tests::forward --lib`
- `cargo fmt --check`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `cargo test federation --lib`
- PASS: `cargo test --test cross_realm_directory_poll_e2e`
- PASS: `cargo test --test cross_realm_directory_streaming_e2e`
- PASS: `cargo test --test cross_realm_signed_admission_e2e`
- PASS: `cargo test daemon::invocation::dispatch::daemon_invocation_service::tests::forward --lib`
- PASS: `cargo fmt --check`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Completed Scope

- Replaced the active federation client `HubUri` type with `HubEndpoint`.
- Migrated federation client trait signatures, error payload fields, breaker
  keys, channel cache keys, mocks, and caller tests.
- Renamed endpoint-valued `target_hub` variables in active federation dial
  paths.
- Renamed the surface scope boundary helper from URI to URA terminology.
- Removed active hub-URI comments from daemon federation code without changing
  trust-anchor behavior or wire protocol data.
