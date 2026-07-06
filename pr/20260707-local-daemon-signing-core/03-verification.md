# Verification

Verified:

- `cargo test sdk_local_daemon_signer --lib`: passed.
- `cargo test sdk_signed_invocation --lib`: passed.
- `cargo test invocation_prepare_and_sign_prepared_allocate_state_handles --lib`: passed.

Not complete:

- C ABI does not yet expose a local-daemon keyring signing function.
- Go/Python live daemon keyring transport cutover remains incomplete.
