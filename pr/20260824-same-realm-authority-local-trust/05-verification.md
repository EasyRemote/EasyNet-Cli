# Verification

Passed:

```text
cargo test --lib daemon::invocation::admission::device_trust_sync::tests
20 passed; 0 failed
```

The rebuilt daemon changed the live failure from same-session
`CALLER_KEY_NOT_FOUND: hub resolve_key timed out after 10s` to the independent
Hub/Device build-skew failure `resolved_descriptor_hash_mismatch`.

After rebuilding Hub and Device from the same source, the live catalogue call
passed with `resolve_unavailable=null` and returned the executable Remote
Desktop routes. This proves the same-realm Authority path no longer enters
session key resolution.
