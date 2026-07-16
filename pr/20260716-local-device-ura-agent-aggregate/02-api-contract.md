# API Contract

Existing aggregate API reused:

```rust
AgentAggregateRepository::load_hosted_identity_status()
AgentHostedIdentityStatus::host_device_agent_ura()
```

No new public API is introduced.

Validation contract:

- `local_invocation::local_device_ura()` keeps its existing Device-kind guard.
  The aggregate provides the persisted hosted-identity projection; the caller
  still decides whether a non-Device persisted value is acceptable for local
  invocation fallback.
- `clipboard_tracker::spawn()` keeps its existing permissive behavior and uses
  the projected value when available, otherwise an empty string.

No compatibility layer or fallback path is added beyond existing public
fallback behavior.
