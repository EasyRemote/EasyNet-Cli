# Agent Purge Platform Deletion Decisions Log

## 2026-07-16

- Decision: keep `agent.purge` public descriptor behavior stable in this slice.
- Reason: `capability_state` is currently a static descriptor/conformance contract, while live publication uses separate catalog filtering. Changing the descriptor state first would alter the public capability surface before there is a platform-qualified capability model.
- Decision: introduce one platform deletion owner before capability-state modeling.
- Reason: future capability-state truth needs a named implementation fact; scattered `cfg` helper functions force callers and gates to infer platform support indirectly.
