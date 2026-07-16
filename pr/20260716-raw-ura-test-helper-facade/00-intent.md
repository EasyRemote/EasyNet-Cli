# Raw URA test helper facade cleanup

## Intent

Remove the remaining raw `format!("easynet:///r/...")` constructions detected
by the URA construction valve and route inline test helpers through the
Axon-backed `crate::core::ura` facade.

## Expected effect

- Architecture convergence: URA construction remains centralized in the facade.
- Proof quality: inline tests use the same canonical builders as production
  code instead of encoding parallel string shapes.
- Public compatibility: fixture values remain semantically identical.
