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
