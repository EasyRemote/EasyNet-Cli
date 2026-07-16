# Execution Checklist

- [x] Inspect cutover readiness output for repeated compile warnings.
- [x] Confirm `publish.rs` is not dirty before editing.
- [x] Use `rg` and CodeGraph-style lookup to confirm `agents` is test-only.
- [x] Move the registry alias into the test module.
- [x] Run focused publish tests.
- [x] Run focused compile/warning check.
- [x] Record verification results.
