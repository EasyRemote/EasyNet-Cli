## 2026-07-27

- Decision: Replace clone-failure dispatch fallback with explicit dispatcher concurrency policy.
- Reason: Lifecycle capabilities must be modeled as state, not inferred from failed operations.
- Scope: Internal EAL interpreter only; no public SDK/API shape changes.
- Verification note: Full `cargo test eal::interpreter` reaches unrelated integration-test compile failures around `PresenceRegistry::insert`; focused `cargo test --lib eal::interpreter` proves the modified module.
