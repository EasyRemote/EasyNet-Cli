# API Contract

No exported CLI or FFI public API changes.

Internal Rust callers migrate from:

```rust
target.default_subject_ura()
```

to issuer-level helpers such as:

```rust
LocalDaemonSystemAbilityIssuer::invoke_target_root_timeout(...)
LocalSystemInvocationIssuer::root_context_for_target(...)
SystemInvocationTargetIssuer::local_root(...)
```
