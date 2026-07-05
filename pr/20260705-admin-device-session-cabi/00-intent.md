# Admin Device Session C ABI Runtime Slice

## Intent

Implement the Admin + Gateway device-session mutation path across the Rust
contract, C ABI, Go C ABI transport, and Python C ABI transport without changing
the daemon SDK requirements spec.

The slice closes the ABI gap for `session.create` and `session.delete` carriers
and projections while keeping hub lifecycle, pairing, and credential
verification explicitly out of scope because their complete daemon-owned
contracts are not yet present.

## Scope

- Add Rust Admin + Gateway carrier builders for `session.create` and
  `session.delete`.
- Add Rust projection for a single created device-session result.
- Export the new functions through `libeasynet_cli` C ABI and the public header.
- Wire Go and Python C ABI transports to invoke and project the new session
  mutation path through Runtime Core.
- Update parity notes and verification evidence.
