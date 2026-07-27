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

## Iteration 10 — Owner projection cursor URA binding cutover

- [x] Replace non-empty-only owner projection cursor validation with canonical owner/host URA binding validation.
- [x] Apply the same binding validation to owner projection publication integrity.
- [x] Remove tests that model cursor ordering with non-URA owner placeholders.
- [x] Add failure-path coverage for malformed owner URA, malformed host URA, wrong host kind, and contradictory owner/host bindings.
- [x] Update convergence gates and run focused owner projection tests.

## Iteration 11 — Builtin plugin provider entrypoint binding cutover

- [x] Move provider/manifest entrypoint identity validation into the provider registry projection.
- [x] Reuse the manifest-layer `validate_builtin_entrypoint` rule instead of duplicating entrypoint comparison logic.
- [x] Add provider-registry failure coverage for mismatched compiled provider entrypoints.
- [x] Update convergence gates so provider registry cannot project mismatched manifests into package bindings.
- [x] Run focused plugin provider tests and canonical convergence gates.

## Iteration 12 — Desktop companion daemon lifecycle fail-closed audit

- [x] Replace silent post-ready companion plan/state skips with typed reconcile failures.
- [x] Replace silent runtime-stop companion plan skips with explicit warnings.
- [x] Preserve daemon-start public behavior by warning/reporting companion failures rather than blocking runtime readiness.
- [x] Add failure-path tests for corrupt desired-state store and malformed companion plan.
- [x] Update convergence gates so daemon lifecycle reconciliation cannot reintroduce `continue`-on-error compatibility skips.
- [x] Run focused companion tests and canonical convergence gates.

## Iteration 13 — Go runtime-host detach provider fail-closed parity

- [x] Re-run codegraph and targeted SDK lifecycle search for silent no-op/default provider paths.
- [x] Identify Go `RuntimeLifecycleTransportFunc.Detach` as a cross-language lifecycle divergence from Python's required transport contract.
- [x] Replace the nil detach-provider no-op with the same neutral invalid-runtime-client diagnostic pattern used by the other lifecycle provider functions.
- [x] Add regression coverage for direct adapter detach and `ConnectLocalRuntimeHost` after-open detach failure.
- [x] Update convergence gates so Go SDK lifecycle neutrality includes the required detach provider diagnostic.

## Iteration 14 — Session handler error-frame fact cutover

- [x] Re-run codegraph and targeted session/remote-bidi search for legacy/default terminal failure projections.
- [x] Identify local session dispatcher handler error frames as allowing missing `code` or `message` to become synthesized terminal failure facts.
- [x] Introduce a shared `HandlerErrorFrame` value object that requires non-empty `code` and `message` before failure projection.
- [x] Reuse the value object for file-transfer and JSON-frame bidi output mapping.
- [x] Add regression coverage for incomplete handler error frames and update convergence gates to reject default error projection helpers.

## Iteration 15 — Node receipt provider cutover

- [x] Clear local `~/.easynet` through `easynet device reset --purge-local-state --force --yes`.
- [x] Rebuild the CLI and reproduce from a clean isolated Hub/device topology.
- [x] Verify remote `meta.list_abilities` and `meta.list_resources` succeed through descriptor-bound public ingress.
- [x] Verify direct remote `invocation.history.list` is rejected as a public action and local canonical history read succeeds with verified receipt chains.
- [x] Add Node `RuntimeAbilityClient` and `RuntimeReceiptProvider` so products have the same provider-backed history path as Go/Python.
- [x] Add Node regression tests proving receipt provider uses governance descriptor provider and generic public invoke rejects history.
- [x] Run Node tests, SDK conformance/gates, fmt/diff checks, then commit if stable.

## Iteration 16 — Runtime response state projection fail-closed cutover

- [x] Re-run codegraph and targeted source search for runtime response state fallback/default projections.
- [x] Identify `InvocationAttemptHandle::finalize_response` projecting undecodable `InvokeResponse.state` to `unknown` and then `runtime_started`.
- [x] Extract runtime response state projection into an explicit value object.
- [x] Project undecodable wire states as terminal `RuntimeFailed` attempt rows with `PROTOCOL_MISMATCH`.
- [x] Add regression coverage proving invalid runtime state cannot remain a non-terminal attempt.
- [x] Update convergence gates and run focused tests.
- [x] Commit if stable.

## Iteration 17 — Presence resolve-only slot cutover

- [x] Re-run codegraph and targeted source search for bidi presence default/legacy session contract paths.
- [x] Identify device-mode self presence seeding as a resolve-only directory visibility path masquerading as a canonical dispatch session with an empty claimant nonce.
- [x] Split presence slots into negotiated dispatch sessions and resolve-only visibility rows.
- [x] Require dispatch-session contracts to carry canonical carrier version and a 16-byte claimant fingerprint.
- [x] Migrate device-mode self presence seed to the explicit resolve-only registration path.
- [x] Update convergence gates and run focused tests.
- [x] Commit if stable.

## Iteration 18 — FFI native session caller-authority cutover

- [x] Re-run codegraph and targeted signer/descriptor search for `keyring entry not found` and caller-signer divergence.
- [x] Identify FFI native session binding as the remaining runtime-owner-only signer path.
- [x] Extract explicit session caller authority from daemon discovery and Ready paired-user signer facts.
- [x] Route native unsigned invocation signing through `load_runtime_caller_signer` for both runtime-owner and managed User callers.
- [x] Preserve fail-closed rejection for unsigned foreign callers and foreign cancellation authority.
- [x] Add regression coverage for Ready-proven paired User signer binding.

## Iteration 19 — Local runtime stream chunk projection cutover

- [x] Re-run codegraph and targeted stream/default/fallback search.
- [x] Identify local runtime stream projection as a duplicated procedural
      state/receipt/error assembly path.
- [x] Extract local stream progress, successful terminal, and failed terminal
      chunk mapping into one projection object.
- [x] Update the canonical convergence gate so the projection object remains
      the only local stream chunk assembly authority.
- [x] Run focused stream checks, fmt, architecture/SPEC gates, and codegraph
      sync.

## Iteration 20 — Driver invocation trace URA cutover

- [x] Re-run codegraph and targeted trace/subject/URA search.
- [x] Identify driver invocation trace metadata as product observability that
      accepted legacy `/invocation/...` address strings.
- [x] Refactor trace parsing so optional URA fields fail closed through the
      canonical runtime identity value object.
- [x] Update driver tests to use canonical resource invocation URAs and add a
      negative legacy trace vector.
- [x] Update the canonical convergence gate.
- [x] Run focused tests/gates.

## Iteration 21 — Plugin template helper profile cutover

- [x] Re-run codegraph and targeted plugin template/provider helper search.
- [x] Identify `plugin init` template generation as carrying a language-only
      generation path separate from the SDK provider helper matrix.
- [x] Introduce a `PluginTemplateProfile` generation authority that binds
      language, exec call mode, helper capability state, and SDK helper package.
- [x] Route `HelloPluginTemplate` through the profile instead of storing a raw
      language field.
- [x] Add regression coverage that generated templates and README output expose
      the matrix-declared SDK helper package.
- [x] Update the canonical convergence gate so template generation cannot bypass
      the profile.
- [x] Run focused tests/gates.

## Iteration 22 — FFI bidi frame-chain MAC cutover

- [x] Re-run codegraph and targeted bidi/FFI frame-chain search.
- [x] Identify FFI bidi send and daemon client convenience send paths as
      accepting or emitting N≥1 up frames with empty MAC defaults.
- [x] Record the frame-chain invariant before code changes.
- [x] Introduce a canonical FFI up-frame MAC value object requiring 32-byte
      frame-chain tags.
- [x] Make `runtime_invocation_bidi_close_send` fail closed because it cannot
      provide a canonical frame-chain tag.
- [x] Update tests and SPEC v2 gate to reject empty-MAC default paths.
- [x] Run focused tests/gates.
