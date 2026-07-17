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
