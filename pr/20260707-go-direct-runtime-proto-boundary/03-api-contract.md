# API Contract

## Default Build

- `easynet.run/cli/sdk/go` exposes the public Daemon SDK DTOs, clients,
  profile runtime adapters, C ABI adapters under their existing tags, and
  conformance helpers.
- The default build must not expose `DirectDaemonRuntimeConnector` or
  `DirectDaemonRuntimeTransport`.

## Direct Runtime Build

- Enabling `-tags easynet_direct_runtime` exposes:
  - `DirectDaemonRuntimeConnector`
  - `DirectDaemonRuntimeConnectorOptions`
  - `DirectDaemonRuntimeTransport`
  - `DirectRuntimeOptions`
  - `OpenDirectDaemonRuntimeTransport`
- The direct transport still delegates prepare/submit/handle operations to a
  configured `RuntimeTransport`; it does not generate canonical signing material
  itself.

## Error Contract

- Missing handle transport remains `ErrNotImplemented` with retry hint
  `RetryNever`.
- Dial failures remain `ErrDaemonOffline` with retry hint `RetrySafe`.
- The build tag must not change runtime error semantics when the direct
  transport is compiled.
