# Verification

Passed:

- `cargo test daemon::invocation::admission::admission_facade::tests::device_role_uses_strict_path_when_signature_is_present --lib -- --exact --nocapture`
- `cargo test session_open --lib -- --nocapture`
- `cargo test --lib` (`4057 passed`, `0 failed`, `4 ignored`)
