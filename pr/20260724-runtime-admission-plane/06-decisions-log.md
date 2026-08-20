# Decisions Log

## 2026-07-24

- The service should own an admission plane, not a field documented as a legacy transport facade.
- Keeping the old raw field as an alias would preserve the seam, so callers must migrate.
- `RuntimeAdmissionPlane` is intentionally private to `DaemonInvocationService`; it is a service ownership value object, not a new public admission abstraction.
- Test-only accessors remain `#[cfg(test)]` so production code cannot grow a second raw facade access path.
