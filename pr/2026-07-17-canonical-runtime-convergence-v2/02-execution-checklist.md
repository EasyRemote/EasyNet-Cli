# Execution Checklist

## RF-5 / RF-3 Signer Custody and Descriptor-Bound Proof

- [x] Inventory CLI and SDK public plain admission/signature helpers.
- [x] Confirm daemon code uses descriptor-bound requests for legitimate
      invocation callers.
- [ ] Remove or privatize upstream legacy admission paths after callers move.
- [x] Add gates that reject new plain admission/fallback signer call sites in
      this repository.

## RF-8 / RF-7 Tuple Ingress and LocalRuntime Routes

- [x] Inventory unary, stream, bidi, exact-route, and loopback ability paths.
- [x] Ensure public ingress requires or exposes derivation for all seven tuple
      fields.
- [x] Ensure route handlers enter LocalRuntime through descriptor-bound
      requests.
- [ ] Delete obsolete direct envelope/response synthesis for ability calls.

## RF-4 Lifecycle Parity

- [x] Ensure the shared capability matrix uses the SPEC states exactly:
      `Unsupported`, `Seam`, `ProviderBacked`, `CutoverReady`.
- [x] Ensure transition vectors cover start, dispatch, stream_open, bidi_open,
      child_dispatch, cancel, deadline, terminal_receipt, restart_recover.
- [x] Add validation that no language is `CutoverReady` without vector evidence.

## RF-6 Receipt Proof Facts

- [x] Inventory receipt constructors and fixtures.
- [x] Gate omitted authority/proof facts.
- [x] Delete compatibility constructors that synthesize proof facts.

## RF-1 / RF-2 Product and Mission Boundary

- [x] Scan canonical SDK packages for product feature families.
- [ ] Keep Mission/EAL state in daemon-owned packages only.
- [ ] Gate Axon/core copied schemas against Mission state reintroduction where
      visible in this repository.

## RF-9 URA and Schema Ownership

- [x] Add active-source terminology gate.
- [ ] Classify transport-library `uri()` usages outside runtime identity.
- [x] Add deterministic schema-copy check by delegating to Axon's
      `scripts/proto/sync_axon_v1.sh --check` from the canonical runtime
      convergence gate.

## Verification

- [x] `cargo fmt --check`
- [x] Targeted Rust tests for changed runtime/daemon modules.
- [x] SDK conformance policy/gate scripts touched by this work.
- [ ] Full relevant conformance runner if feasible in the local environment.
