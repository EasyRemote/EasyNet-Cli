# Execution Checklist

- [x] Inspect repository instructions, sibling worktrees, branch, and dirty-state
      ownership before editing.
- [x] Confirm the authoritative readiness matrix still marks all eight product
      requirements partial.
- [x] Run focused static and unit gates for target binding, input, media,
      lifecycle, network, frontend, and cross-device evidence integrity.
- [x] Compare gate coverage to each readiness requirement and identify the
      highest-priority untested or incorrectly implemented path.
- [x] Implement the ABI v9 root-cause fixes in the owning layer with bounded failure-path
      tests.
- [x] Make ABI v9 additive in feature discovery, preserve v7/v8 compatibility,
      and publish the executable native library in release archives.
- [x] Enforce callback quiescence, one shared queued-and-leased byte budget,
      and bounded per-handle/global stream registries.
- [x] Complete and verify safe leased-buffer consumption in Go and Python
      without exposing borrowed payload memory past release; expose Rust-owned
      bytes directly without routing its same-language transport through C FFI.
- [x] Replace the migration-specific public Addressing reason with the
      product-neutral `ABILITY_OWNER_NOT_PUBLISHER`, classify the new public
      stream/Ability types, and regenerate canonical API/parity evidence.
- [x] Execute ABI v9 against a hermetic live daemon in Go and Python through
      `RuntimeAbilityClient`, canonical descriptor-bound User subjects, and
      daemon-custodied authority signing; prove one raw JSON data payload,
      explicit lease release, and one receipt-backed terminal frame.
- [x] Add the missing Go managed-signing selector so Go and Python both select
      the daemon-canonical lexicographically first active key for an exact
      subject and purpose instead of duplicating key-inventory rules in product
      code.
- [ ] Verify exact exported symbols on Linux and Windows release runners; local
      macOS evidence alone is not cross-platform release proof.
- [ ] Run current-host macOS capture/input/media checks with real permission and
      process identities where available.
- [ ] Run locally provisionable Linux/container network and provider scenarios.
- [ ] Record external Windows/cross-device requirements as unresolved unless
      fresh runner evidence is actually produced.
- [ ] Form explicit semantic commit manifests only after concurrent diffs can be
      separated safely.
- [ ] Require the signed aggregate gate to emit
      `product_complete_claim=true` before declaring completion.
- [x] Replace duplicated Windows FILETIME and Linux XRes/process-instance logic
      with one private native-platform provider (`7a59c2926`).
- [x] Migrate discovery, observer, media-host, focus, and input to the provider;
      delete PID fallback authority (`7a59c2926`).
- [x] Apply one capture-eligibility predicate to macOS, Windows, and Linux
      discovery/capture enumeration (`7a59c2926`).
- [x] Fence deferred frontend creation and compensate stale successful creates
      (`666244ea3cc815b78e27d4d24343d411687df6b0`).
- [x] Preserve/reconcile ambiguous closing sessions and replay event watches
      from the last committed sequence with bounded retry (`666244e`).
- [x] Keep lease supervision alive during permission verification and preserve
      the bound session subject across inventory omission (`666244e`).
- [ ] Restore complete Linux application membership/stacking revalidation after
      native-platform cutover; committed-ID presence alone does not prove that
      the process window set is unchanged.
- [ ] Migrate the Windows process-scoped application observer fixture to the
      canonical process-instance invariant so the authoritative platform branch
      passes the full main-crate filter matrix.
