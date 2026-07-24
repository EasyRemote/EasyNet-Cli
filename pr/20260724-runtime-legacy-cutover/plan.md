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

## Iteration 3 candidate policy

- Focus on SDK receipt/proof-fact parity because public product flows rely on
  SDK-side validation before and after daemon submission.
- Prefer removal of language-local canonicalizers that can accept weaker receipt
  or authority facts than the canonical runtime model.
- The first audit target is Java receipt/proof-fact handling. If Java accepts a
  receipt or authority shape that Go/Python/Rust reject, that is a cross-language
  compatibility fork, not a product feature.
- Verification must include the touched language tests, API inventory gates when
  SDK public surface changes, and the canonical runtime convergence gate.

## Iteration 3 decision log

- The Java receipt proof-fact constructor already calls
  `RuntimeReceiptProofFacts.validate(raw)`, but codegraph exposed a separate
  authority subject predicate inside `InvocationAuthorityBindingValidator`.
- That predicate parsed the invocation subject by substring search for
  `/resource/`, which can admit path-shaped strings without proving a canonical
  URA resource owner projection. Go and Python already route this through
  structured URA parsing.
- The selected cutover is to move Java session-authority subject admission into
  `AuthoritySupport` and remove the validator-local parser. The helper now
  admits exact subject equality or canonical resource URAs owned by the session
  user / that user's agent, and rejects path-substring impostors.
- This keeps the public Java API unchanged while converging Java onto the same
  SDK authority model as Go/Python.

## Iteration 4 candidate policy

- Focus on public ingress tuple construction and route facades.
- Prefer removal of CLI/product-facing code that still:
  1. accepts partial invocation tuples and repairs them through defaults,
  2. preserves non-canonical "not wired" / feature-disabled invoke paths,
  3. translates legacy target fields into descriptor-bound invocation state,
  4. keeps product-specific route discovery outside LocalRuntime.
- The first audit target is `src/cli/commands/invoke.rs` plus
  `src/cli/commands/invocation_tuple.rs`, because these are direct public
  ingress surfaces for product smoke tests and operator reproduction.
- Verification must prove the public CLI can only construct descriptor-bound
  canonical tuples, and axon-pb disabled builds fail with canonical unsupported
  errors rather than legacy invoke wording.

## Iteration 4 decision log

- codegraph and targeted search found a production-facing compatibility remnant
  in `ability invoke`: the axon-pb-disabled branch and its tests still described
  the public remote path as a legacy/not-wired message.
- `ability invoke`, `ability stream`, and `ability bidi` also duplicated their
  remote-transport disabled messages. That duplication makes it easy for one
  public ingress surface to drift back into product-specific or legacy wording.
- The refactor adds a shared public-ingress helper,
  `remote_invocation_transport_unsupported`, in `invocation_tuple.rs` and routes
  all three CLI surfaces through it.
- The invoke test now asserts that retired `not wired` / `legacy` wording is not
  accepted, and the SPEC v2 gate rejects reintroducing the old text or bypassing
  the shared helper.

## Iteration 5 candidate policy

- Focus on feature/cfg ownership. A canonical read model must not disappear just
  because one transport carrier is disabled.
- `cargo check --no-default-features --lib` currently fails because
  `invocation_history.rs` imports `dispatch::attempt_audit`, while
  `attempt_audit` is entirely behind `feature = "axon-pb"`.
- The root abstraction problem is that the attempt audit ledger mixes two
  responsibilities:
  1. transport-specific protobuf request projection, and
  2. transport-independent JSONL attempt read/write records consumed by
     invocation history.
- The intended cutover is to make the ledger record/store available independent
  of axon-pb and cfg-gate only protobuf request adapters.
- Verification must include `cargo check --no-default-features --lib` so this
  does not regress back into a hidden feature compatibility fork.

## Iteration 5 decision log

- codegraph confirmed `attempt_audit` is consumed by both daemon transport and
  the `invocation.history.list` governance read model.
- The selected cutover keeps the attempt ledger module always available and
  gates only protobuf/tonic adapters behind `axon-pb`.
- `InvocationAttemptHandle::reject_diagnostic` is now the transport-neutral
  terminal-state API; `reject_status` is only a tonic adapter.
- The default real invocation history test was also migrated off the retired
  provider filter field `agent_ura`. Runtime history providers accept canonical
  `callee_ura` / `subject_ura`; CLI-only `--agent` lowering must not enter the
  provider schema.
- SPEC v2 now includes an explicit feature-boundary guard to prevent the ledger
  module from being transport-gated again.

## Iteration 6 candidate policy

- Focus on SDK provider-output projection. A product-neutral SDK must not
  silently turn malformed provider directory rows into empty records.
- codegraph located the shared Directory projection surfaces in Go and Python.
  Both already reject malformed container types, but record projection still
  accepts a mapping without canonical `kind` or any canonical URA fact by
  projecting empty strings.
- The root abstraction problem is not legacy alias promotion; that was already
  blocked. The remaining compatibility behavior is weaker: malformed provider
  rows survive as empty canonical records and defer failure to product code.
- The intended cutover is to make Directory record projection fallible in both
  SDKs and require canonical record identity facts at the SDK boundary.
- Verification must cover Go/Python directory tests and the canonical runtime
  convergence gate so both SDK implementations remain aligned.

## Iteration 6 decision log

- The selected cutover was SDK Directory record projection, not product device
  rendering. Products may still decide how to display directory rows, but SDK
  provider output must either carry canonical record facts or fail closed.
- Go now uses an internal `projectDirectoryRecord` strict projector inside
  `ProjectDirectoryResolution`. The exported `ProjectDirectoryRecord` signature
  remains unchanged for public API compatibility, but it is no longer the
  provider-output authority path.
- Python's private `_project_record` now requires `kind` and at least one
  canonical URA fact (`ura`, `owner_ura`, `ability_ura`, or `route_ura`).
- Both SDKs now reject alias-only rows such as `{type, canonical_name}` at the
  provider boundary instead of producing empty canonical records.
- SPEC v2 now checks the strict record projector, canonical record fact guards,
  and Go/Python tests for missing record identity facts.
- Verification passed:
  - `go test ./... -run Directory` in `sdk/go`
  - `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python3 -m pytest -q sdk/python/tests/test_directory.py`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`
