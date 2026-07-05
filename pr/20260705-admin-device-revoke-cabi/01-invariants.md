# Invariants

- `federation.revoke` is a daemon ability. SDKs only build its Invocation
  carrier and project daemon output.
- URA validation stays in the Rust daemon SDK contract or Axon-delegated
  helpers; language bindings must not parse or fabricate URA ownership facts.
- Hub join/leave, pairing lifecycle, credential verification, and device
  session create/delete remain explicit daemon/ABI contract gaps.
- C ABI outputs are caller-owned strings released through `easynet_string_free`.
- Go and Python C ABI transports must go through Runtime Core invoke before
  projecting revoke results.
