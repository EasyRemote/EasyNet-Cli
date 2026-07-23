# API Contract

- No public API or wire format changes.
- Internal Rust variant names and tests change only inside the daemon routing
  adapter and FFI descriptor resolver.
- Existing remote descriptor probe behavior remains fail-closed and signer
  backed.
