# Invariants — RemoteApp Capability Gate Hardening

1. Product capability projection must use runtime backend readiness, not broad
   package descriptors.
2. `production_target_subjects` must be gated by `production_ready`.
3. `diagnostic_target_subjects` must remain display-only.
4. A closed production gate must expose a non-empty blocked reason.
5. Boundary tests must fail if any of those fields are removed or reduced to
   old raw-descriptor behavior.
