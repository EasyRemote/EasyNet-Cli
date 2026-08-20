# Intent

Implement the Admin + Gateway device-revoke SDK facade as a daemon-owned
carrier path across Rust/C ABI and P0 language bindings.

The SDK must not own hub trust or pairing state. This slice only covers the
existing daemon `federation.revoke` ability carrier and result projection, so
Go/Python C ABI transports can execute device revoke through Runtime Core
instead of reporting an artificial profile gap.
