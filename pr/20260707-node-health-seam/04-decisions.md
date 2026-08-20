# Decisions

## Health Transport Boundary

`HealthClient` requires only `runtimeHealth` and treats `runtimeDiagnostics` as
an optional transport capability. This matches Go/Python behavior while keeping
the Node seam provider-neutral.

## Typed DTOs Over Plain Objects

Runtime health, diagnostics reports, and diagnostic checks are explicit classes
with `toJSON` methods. This keeps API liveness, runtime readiness, diagnostics,
and transport errors as separate typed concepts instead of procedural payload
handling.
