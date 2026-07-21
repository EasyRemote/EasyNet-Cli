# Decisions Log

## 2026-07-17

- Preserve the existing plan-pack history and add the skill-required file set
  instead of deleting or renaming prior audit files.
- Use the current working tree as an in-flight baseline. Pre-existing edits are
  not reverted.
- Use URA terminology for semantic runtime identity even where older tooling or
  dependencies expose HTTP/gRPC `Uri` APIs.
- Treat completion as gate-backed closure of root forks, not as presence of
  newer code paths.
- Continue implementation by re-running the convergence gates first, then
  deleting or refactoring any old public path that remains live.
- Treat `AgentSkillLayout` as the active skill directory abstraction; do not
  regress validation gates to retired `AgentType` naming.
- Treat committed adapter reports as coverage manifests. Live parity requires
  generated `language.json` results from the conformance runner.
- Treat Go result fixtures emitting `receipt` as retired alias regressions
  unless the fixture is explicitly testing rejection.
- Treat Go SDK plain canonical invocation encoding as obsolete after Axon
  removed the plain encoder. The public Go facade keeps the existing function
  name but now delegates only to descriptor-bound canonical encoding.
- Treat RF-4 deadline parity as a vector-backed matrix property: Go and Python
  `unary_invoke` gain `deadline` evidence only through concrete provider
  selectors, not through timeout option shape alone.
- Keep V2 completion open. The deadline vector reduces the RF-4 matrix gap but
  does not close `child_dispatch`, `restart_recover`, or cross-language
  cutover readiness.
- Treat `environment/process_root` as native runtime `start` evidence only when
  Go and Python provider proofs bind concrete SDK-environment selectors. This
  reduces the RF-4 start gap without claiming dispatch, stream, bidi, recovery,
  or cutover readiness.
- Treat bidi frame0 as a direct runtime session-entry invariant, not an API
  shape assumption. Go and Python only become `provider-backed` for `bidi`
  after the provider rejects missing or non-EnvelopeOpen frame0 before opening
  a runtime session; other languages keep the explicit unproven requirement.
- Treat stream and bidi cancel as request lifecycle evidence, not synthetic
  terminal completion. Go and Python receive `cancel` vector evidence only
  through selectors that prove the cancel acknowledgement is non-terminal and
  that terminal cancel acks are rejected by the facade.
- Treat direct runtime provider registration as the owner of generic runtime
  environment, connection, and lifecycle provider-backed status when the same
  concrete selectors already prove process-root ownership and control-only
  runtime behavior. This does not make any runtime core capability
  cutover-ready; it only removes the seam-only modelling defect for the
  generic runtime concepts that wrap the provider-backed native runtime.
- Treat stream and bidi deadline ownership as provider-backed lifecycle
  evidence only when direct runtime selectors prove typed timeout projection,
  cleanup, and safe retry for both Go and Python. The evidence closes the
  `deadline` action for those capabilities but keeps the cells below
  cutover-ready while dispatch, child-dispatch, restart recovery, and complete
  transition coverage remain open.
- Treat stream and bidi dispatch as provider-entry evidence only when direct
  runtime selectors prove the complete draft reached the provider, the first
  provider output is non-terminal, and a terminal receipt follows through the
  same session. This closes the `dispatch` action for Go and Python stream/bidi
  cells without claiming child dispatch, restart recovery, runtime start, or
  cutover readiness.
- Treat ability facade start as a borrowed view over the Native Runtime
  provider graph. The handle owns Runtime, Addressing, and Health; the ability
  facade is created only after that provider graph exists and fails closed
  after provider close. This closes the `start` action for Go and Python
  `ability_invocation_facade` without adding a product-specific ability
  lifecycle root or claiming restart recovery.
- Treat ability child dispatch as a provider-backed generic runtime facade
  vector only when the selector proves all three facts: the parent terminal
  receipt is required, the child draft uses Axon scalar causal context derived
  from that receipt, and the child terminal receipt records the parent receipt
  link. Presence of a `causal_context` field alone is not lifecycle proof.
- Treat ability provider lifecycle methods as generic facade delegation, not a
  second lifecycle owner. Go and Python close ability-facade `dispatch`,
  `stream_open`, `bidi_open`, `cancel`, and `terminal_receipt` only when the
  selector proves descriptor-bound lowering followed by Runtime Core provider
  entry or handle control.
- Treat ability deadline parity as composition over the Runtime Core provider
  deadline owner. The ability facade must not grow an independent timeout field
  or language-specific deadline state machine; it closes `deadline` only when
  the selector proves provider timeout projection and retry after cleanup.

## 2026-07-22

- Treat Directory as a canonical runtime projection, not an EasyNet Hub
  directory schema. The SDK may expose generic resolver records, cursor state,
  raw runtime event facts, and subscription lifecycle, but product DTOs such as
  `DirectoryEntry`, `DirectoryAgentSummary`, and `DirectorySigningAuthority`
  belong downstream of the SDK.
- Keep `DirectoryEvent` as a generic `{type, raw}` runtime event so consumers
  can observe provider facts without the SDK validating or owning product event
  variants.
- Strengthen `check-sdk-product-neutrality.sh` to reject future product-owned
  Directory wire DTOs/files in production SDK sources. The rule intentionally
  ignores Python build caches so local mypy/pycache artifacts cannot become
  false architecture failures.
- Update the released edge-adapter frozen surface digest after intentional
  removal of product DTO members from Go/Python non-canonical inventory.
- Treat daemon boot progress as daemon-owned lifecycle state. `easynet start`
  no longer parses `EASYNET_PAGES_PORT` to reconstruct a missing
  `BootEvent::PortChosen.start`; if the daemon omits the start candidate, the
  CLI renders only the declared chosen port instead of preserving old-daemon
  compatibility semantics.
- Treat `easynet mcp install` as a host-local front-door writer for the live
  `easynet mcp serve` contract only. Retired `--endpoint`, `--bound-node`, and
  `--allow-node-override` inputs are removed rather than accepted and dropped,
  because silent no-op install options produce false product success.
- Treat `federation.advertise_abilities` as a write-side owner projection
  publication, not as an optional read-model convenience. Production dispatch
  already owns the ability catalog, so a missing catalog is a daemon
  construction defect rather than a successful no-op acknowledgement.
- Treat the runtime system descriptor catalog as local-owner construction
  evidence only. Remote descriptor resolution must not rebind static daemon
  catalog rows for an arbitrary target owner because that bypasses owner
  liveness, caller signer custody, and route/read-model authority.
