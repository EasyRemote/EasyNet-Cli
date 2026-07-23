# Execution checklist

- [x] Replace FFI-local descriptor-ref formatting with `descriptor.descriptor_ref()`.
- [x] Keep descriptor hash validation for catalog serialization.
- [x] Add/update convergence gate to reject FFI-local `canonical_ability_descriptor_ref(&format!(...))`.
- [x] Run targeted FFI descriptor resolver tests.
- [x] Run SPEC v2, architecture, format, and whitespace checks.
