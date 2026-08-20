# Decisions

- Add the scenario at the daemon provider boundary first because the final
  daemon/CLI/URA join E2E must not hide state-machine defects behind process
  orchestration noise.
- Exercise enrollment, grants, key add/rotate/revoke, recovery,
  suspend/reactivate/delete and persisted reload in one test so the
  backend-free lifecycle is proven as a coherent aggregate, not only as isolated
  transitions.
- Keep SPEC status explicit: this is provider evidence, not standalone-Hub
  cutover.
- Add focused recovery boundary tests alongside the happy-path aggregate
  scenario. Replayed recovery proof, suspended-principal recovery and
  deleted-principal terminality are state-machine facts and should fail at the
  provider before CLI or Backend UX is involved.
