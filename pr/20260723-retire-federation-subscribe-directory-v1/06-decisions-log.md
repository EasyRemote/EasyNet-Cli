# Decisions Log

## 2026-07-23

- Decision: remove the v1 descriptor instead of redirecting it to v2.
- Reason: an alias would preserve the legacy protocol name as an active public
  surface and hide caller migration. The runtime already has one typed v2 stream
  path; convergence requires deleting the second descriptor-only surface.
- Decision: gate descriptor inventory, not only Rust source/tests.
- Reason: the retired v1 surface survived as a TOML descriptor even though the
  production dispatcher only exposes v2. Source-only scans are insufficient for
  active discovery convergence.
- Decision: update the cross-realm e2e harness to provide the invocation
  attempt audit ledger.
- Reason: `DaemonInvocationService` is now correctly fail-closed without
  pre-runtime failure observability. Tests that instantiate it directly must
  wire the same dependency as daemon boot rather than expecting an implicit
  compatibility ledger.
