# Execution Checklist

- [x] Read architecture skills.
- [x] Run codegraph and targeted source search.
- [x] Select active runtime/SDK legacy surface.
- [x] Refactor canonical owner and migrate tests/callers.
- [x] Run focused tests and gates.
- [x] Commit if stable.

## Iteration 2 — Rejected local read issuer split

- [x] Re-read architecture skills.
- [x] Run codegraph and targeted issuer search.
- [x] Select active subject-owner conflation in `LocalRuntimeStateReadIssuer`.
- [x] Prototype explicit Device-owned read issuer.
- [x] Reject the prototype because the SPEC v2 runtime-state read boundary gate requires the selected CLI read paths to enter through `LocalRuntimeStateReadIssuer`.
- [x] Remove the prototype and restore the previous production source state.
- [x] Preserve the finding as an architecture decision instead of committing a gate-divergent implementation.

## Iteration 3 — Node receipt-history governance subject parity

- [x] Re-run codegraph and targeted source search for `invocation.history.list`, `meta.list_abilities`, descriptor resolution, and authority subject mismatch paths.
- [x] Identify Node SDK history admission divergence from Go/Python: Node accepted only user runtime-state subjects, while Go/Python also accept an exact callee runtime-owner subject.
- [x] Split Node history subject admission into explicit user runtime-state and runtime-owner predicates.
- [x] Add Node tests for exact device runtime-owner subject with delegation authority and non-callee runtime-owner rejection.
- [x] Clear local EasyNet state through the product purge path after user authorization.
- [x] Run focused tests and gates.
- [x] Commit if stable.

## Iteration 4 — Swift receipt canonicalizer fail-closed parity

- [x] Re-run codegraph/search for URI terminology, receipt canonicalizer defaults, and cross-language governance subject parity.
- [x] Identify Swift `RuntimeReceipt.canonicalReceiptType` as a fail-open helper returning an empty string for unknown canonical lifecycle states.
- [x] Refactor Swift receipt type binding to throw on unknown canonical lifecycle state.
- [x] Add direct Swift regression coverage for unknown canonical lifecycle state.
- [x] Run focused Swift tests and repository gates.
- [x] Commit if stable.

## Iteration 5 — Java receipt canonicalizer fail-closed parity

- [x] Re-run codegraph/search for Java receipt proof-fact bypass and canonicalizer fail-open behavior.
- [x] Identify Java `RuntimeReceipt.canonicalReceiptType` as the same fail-open empty-string sentinel as the retired Swift helper.
- [x] Refactor Java receipt type binding helper to throw on unknown canonical lifecycle state.
- [x] Add Java regression coverage for the private helper's unknown lifecycle-state branch without exposing new public API.
- [x] Run focused Java tests and repository gates.
- [x] Commit if stable.

## Iteration 6 — Device directory user-binding state machine

- [x] Reproduce clean hub + clean federation-native device join after user authorized clearing old runtime state.
- [x] Verify `meta.list_abilities` and `invocation.history.list` succeed on the clean paired device with descriptor refs and verified receipt chains.
- [x] Identify `device list` as a product boundary that treated unbound federation-native credentials as a missing legacy user id.
- [x] Refactor device directory read selection into explicit bound-user, unbound-federation-native, and local-authority states.
- [x] Fail closed at the CLI boundary for unbound federation-native device directory reads instead of sending an unauthorized operator/audit request to the daemon.
- [x] Add regression coverage for bound and unbound credential states.
- [x] Run focused tests and clean-state command verification.

## Iteration 7 — Hosted-Agent target projection fail-closed cutover

- [x] Replace silent hosted-Agent target projection drops with an explicit projection result.
- [x] Make malformed/non-Agent hosted identities move the daemon self-target index into `Unavailable`.
- [x] Update self-target tests to describe aggregate projection states, not fallback/slow-tier behavior.
- [x] Add a corruption regression proving registry-only matching is disabled when hosted identity projection is malformed.
- [x] Run focused daemon target-gate tests and canonical convergence gates.

## Iteration 8 — Hosted-Agent route placement projection fail-closed cutover

- [x] Reuse the hosted-Agent identity projection error for both target locality and route placement projections.
- [x] Replace silent hosted-Agent placement drops with a typed projection result.
- [x] Make route resolver placement state unavailable when `local-agents.json` carries malformed hosted identity rows.
- [x] Add aggregate regression coverage for malformed placement projection.
- [x] Run focused placement/route tests and canonical convergence gates.

## Iteration 9 — Registered-Agent registry projection fail-closed cutover

- [x] Replace silent registered-Agent registry key drops with an explicit aggregate projection result.
- [x] Route admission, Mission target conflict detection, and skill ownership listing through the same checked registry-name projection.
- [x] Add aggregate regression coverage for malformed registry keys in target projection and registered-name projection.
- [x] Update convergence gates so the checked projection remains part of the architecture contract.
- [x] Run focused registry projection tests and canonical convergence gates.
