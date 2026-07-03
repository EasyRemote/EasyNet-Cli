# Architecture

## Layering

- `src/core`: pure value/domain layer.
- `src/daemon`: product/device daemon policy and long-lived services.
- `src/daemon/invocation`: daemon admission, routing, dispatch, receipts,
  streams, and bidi lifecycle.
- `src/daemon/ability`: names, descriptors, authority, implementation bindings,
  catalog, wire projection, and built-in handlers.
- `src/cli`: command facade, presentation, daemon-client adapters, and MCP
  command integration.
- `src/ffi`: stable C ABI projection over daemon lifecycle, client handles,
  generic Invocation submission, typed errors, and string ownership.
- `src/eal`: parser, interpreter, runtime support, and diagnostics.
- `src/support`: low-level async, shellguard, and platform helpers.

## Boundary Proof

The convergence is structural. It does not change Invocation tuple fields,
Receipt binding, Axon admission, or Ability wire names. The daemon still
delegates canonical protocol semantics to Axon-owned types while EasyNet-Cli
retains daemon-local product policy.

## Planning Root Decision

The EasyNet engineering contract asks for a `pr/<date>-<task>/` pack, but the
active project-structure spec forbids permanent roots outside the final layout.
This pack is therefore placed under `docs/pr/pr/` to satisfy auditability
without reintroducing a forbidden top-level ownership root.
