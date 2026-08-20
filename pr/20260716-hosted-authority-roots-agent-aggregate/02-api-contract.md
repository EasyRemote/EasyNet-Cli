# API Contract

Existing public facade preserved:

```rust
pub fn hosted_agent_authority_roots() -> anyhow::Result<Vec<String>>
```

New internal projection:

```rust
AgentHostedIdentitySnapshot::hosted_agent_authority_roots() -> Vec<String>
```

Behavior:

- Preserve hosted Agent order from the persisted identity projection.
- Preserve URA strings exactly as stored; downstream authority validation still
  owns canonicality checks.
- Do not add fallback paths.
