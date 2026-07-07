# Node Mission Seam Intent

Add a Node/TypeScript Mission profile seam that matches
`docs/spec/daemon-sdk-requirements-v1.md` while keeping Mission execution,
EAL planning policy, scheduling, retry, and child Invocation semantics outside
the Node SDK facade.

## Scope

- Expose Node Mission carriers for run EAL, run file, track, cancel, and events.
- Delegate Invocation carrier construction and mission result projection to an
  injected Mission transport.
- Project daemon-authored mission status and mission event pages into stable
  DTOs.
- Expose a bounded mission event stream adapter over the generic runtime stream
  handle shape when a provider supplies one.
- Declare Node for `mission/carrier_status` only with direct Node test evidence.

## Out Of Scope

- No EAL parser, planner, scheduler, retry policy, or daemon execution loop.
- No fabricated child receipts or receipt URAs.
- No MissionPlan child Invocation conformance claim for Node in this slice.
