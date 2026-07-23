# Architecture

Root abstraction problem:

`registry_with_temp_home()` creates an isolated but unjoined HOME. Several
real-invoke tests used it while exercising local device operations that require
`local_invocation::local_device_ura()` or device-management credentials. That
made tests depend on legacy permissive behavior and caused failures once local
identity became mandatory.

Refactoring:

- Keep `registry_with_temp_home()` as the empty HOME fixture for tests whose
  semantics are intentionally unjoined.
- Rename and reuse the joined fixture for tests that require local Device
  identity.
- Move `fs_ref()` callers into a fixture scope where credentials are already
  seeded.
- Do not add fallback identity paths in production code.
