# Project structure source truth

## Intent

Converge the project-structure specification and executable gate with the
current repository ownership model:

- `src/bin` contains process entry points and maintainer verifier binaries.
- `tools/sdk-conformance-runner` is the SDK conformance runner workspace crate,
  not a `src/bin` file.
- `provider_routes` is a route-manifest source root consumed by generated
  Rust/Go/Python constants and tests.
- grouped federation AbilityDescriptor TOMLs are valid system descriptors.
- local Python `__pycache__` directories are generated output and must not
  influence source-structure verification.

## Expected effect

- Architecture convergence: one checked source tree contract.
- Product acceleration: structure gate failures point at real architecture
  drift instead of stale allowlists.
- Public behavior: no runtime behavior or public API changes.
