# RuntimeAdmin Route Binding Convergence

## Intent

Move RuntimeAdmin ability route names out of independently maintained Go,
Python, and daemon constants into one checked provider route manifest.

## Expected Effect

- Architecture convergence: one source of truth for the RuntimeAdmin
  provider route names consumed by both SDK implementations and daemon
  conformance.
- Product acceleration: future route additions update one manifest and
  regenerate bindings instead of manually syncing three language surfaces.
- Public behavior preservation: existing Go/Python private constant names
  and daemon alias names keep their values; no caller-visible API expands.

## Scope

- Add `provider_routes/easynet-runtime-admin-routes.v1.json`.
- Generate Go, Python, and daemon Rust route binding files from the manifest.
- Repoint existing RuntimeAdmin facade and daemon baseline constants to the
  generated bindings.

## Non-Goals

- No changes to runtime lifecycle, session listing payload semantics, device
  revoke ack semantics, descriptor resolution, or invocation transport.
- No compatibility fallback layer; the migrated names remain the canonical
  route names.
