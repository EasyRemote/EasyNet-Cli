# Verification

Passed:

- `cargo test daemon::ability::builtins::real_invoke_tests::real_device_session_list_returns_empty_under_temp_home --lib -- --exact`
- `cargo test daemon::ability::builtins::real_invoke_tests::every_published_ability_has_a_real_invoke_test --lib -- --exact --nocapture`

The coverage gate reports `135 / 135 published abilities exercised`.
