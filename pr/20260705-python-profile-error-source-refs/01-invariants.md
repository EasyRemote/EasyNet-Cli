# Invariants

1. Error schema remains stable.
   - No wire-schema or fixture changes.
   - `SDKError.details` carries source refs; no new top-level DTO field is
     required.

2. Profile ownership is explicit.
   - Each Python profile error has a `profile` detail matching the owning
     SDK profile.
   - Each Python profile error has a stable `source_ref` detail that can be
     used by product facades and conformance diagnostics.

3. Existing semantics are preserved.
   - Existing `code`, `stage`, retry hint, retryability, and human messages
     do not change.
   - Existing details such as `reason`, `profile_method`, and transport facts
     are preserved and must not be overwritten.

4. No lower-layer coupling.
   - Profile error source refs must not import Axon proto, C ABI symbols, or
     daemon transport internals.
   - The change is Python SDK facade metadata only.
