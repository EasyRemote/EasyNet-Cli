# Runtime Legacy Cutover Plan

## Goal restatement

Continue converging EasyNet-Cli toward the canonical runtime model by removing
legacy/compat layers that preserve pre-SPEC behavior. The change must improve
architecture rather than add feature surface.

## Scope for this iteration

- Inspect current runtime, SDK facade, descriptor, admission, and catalog paths.
- Select one root legacy/compat layer whose removal improves canonical runtime
  convergence without changing public product interfaces.
- Refactor from the owned lower layer upward.
- Add or update deterministic tests that prove the removed path cannot re-enter.

## Non-goals

- Do not add EasyNet/EasyRemote-specific SDK abstractions.
- Do not add fallback paths for old storage, old descriptor inference, or old
  route discovery.
- Do not loosen daemon admission to hide product ingress defects.

## Invariants

1. Runtime invocation tuples remain explicit: caller, callee, subject, ability,
   action, descriptor, and signer authority must be bound before dispatch.
2. Descriptor resolution must not synthesize a route from product naming or hidden
   discovery probes.
3. Session authority may admit only the canonical subject family encoded in its
   authority metadata.
4. Legacy data shape migration is not a runtime authority path. If old data is
   unsupported, loaders must fail closed or start from an empty canonical store.
5. Tests must prove the obsolete path is absent, not merely unused by one caller.

## Boundary proof

- Core runtime owns admission, descriptor resolution, signer binding, and receipt
  finalization.
- CLI/product surfaces may construct user intent but must not repair canonical
  tuple state through defaults.
- SDKs expose generic runtime concepts only; product lifecycle and product naming
  stay outside the SDK.

## Verification plan

- Run targeted Rust tests for the touched module.
- Run formatting checks for touched Rust code.
- Run architecture/SPEC gates when the change reaches a stable boundary.
- Re-check repository searches for the removed legacy pattern.

## Iteration 1 decision log

- `principal_lifecycle` was inspected first because it had legacy-store test
  fixtures. The production loader is already canonical and fail-closed via
  `deny_unknown_fields`; changing it would only rename tests, not remove a
  runtime compatibility path.
- The selected cutover target is daemon admission naming and receipt proof
  provenance. Runtime admission still used `ProductAdmission*`,
  `product_policy`, and the proof issuer string
  `easynet-cli.product_admission.v1`. That branded the canonical admission
  proof as an EasyNet product policy even though the object coordinates generic
  runtime admission around descriptor-bound envelopes, quota reservation, and
  authority proof facts.
- The refactor renames the state machine to `RuntimeAdmission*` /
  `DaemonRuntimeAdmission*`, updates call sites, and changes the proof issuer to
  `runtime.admission.v1`.
- Targeted verification exposed a non-hermetic plugin host test: the test
  catalog used `AxonAbilityCatalog::new_with_runtime`, which derives test device
  authority from local credentials. The helper now injects an explicit test
  device URA so runtime-admission tests do not depend on developer machine
  pairing state.

## Iteration 2 candidate policy

- Continue scanning for production legacy/compat paths after the runtime
  admission terminology cutover.
- Prefer paths that:
  1. preserve product-specific naming inside runtime core,
  2. retain fallback route/authority/defaulting behavior,
  3. weaken receipt/admission/descriptor proof provenance,
  4. make tests depend on local developer state.
- Reject candidates that are only negative tests, generated compatibility
  comments, or product-boundary features intentionally outside canonical SDK.

## Iteration 2 decision log

- codegraph and targeted searches found a production descriptor compatibility
  path in `AbilityDescriptor::with_hints`: advisory
  `hints.streaming_only` / `hints.bidi_only` still selected canonical
  `call_mode`.
- This violates the canonical runtime model because routing, descriptor hashing,
  and descriptor refs are governed by `call_mode`; presentation hints must not
  become a second transport authority.
- The cutover target is therefore the descriptor construction state machine:
  `call_mode` remains explicit and synchronizes presentation hints outward,
  while `with_hints` only records advisory UI facts and rejects contradictory
  transport hints at the wire boundary.
- Verification must prove that hints cannot change call mode or descriptor refs,
  and that wire payloads with contradictory hints fail closed instead of being
  interpreted through a legacy transport selector.
