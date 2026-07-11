# SDK Runtime Environment Projection

## Intent

Expose the local runtime state root and paired runtime identity projection from
the canonical SDK model so downstream products do not parse EasyNet daemon
credentials independently.

EasyRemote currently derives `LocalIdentity` from `credentials.json` directly.
This keeps a second product-owned interpretation of runtime identity fields.
The SDK should own the projection once for both Go and Python.

## Scope

- Add Go and Python SDK runtime-environment projection helpers.
- Keep the projection generic: state root, control path, credentials path,
  realm, device id, optional username and hub endpoint.
- Migrate EasyRemote to consume the Python SDK projection while preserving its
  public `LocalIdentity` and `read_credentials()` behavior.
- Extend downstream gates to require the SDK projection.

## Out of scope

- Starting or stopping a daemon.
- Backend account/OAuth semantics.
- Private-key custody or key-service access.
