## Goal

Remove the EAL interpreter's implicit "parallel clone failure means sequential execution" compatibility path.

## Non-goals

- Do not change EAL source syntax.
- Do not change Mission child Invocation construction, receipt binding, or public CLI behavior.
- Do not introduce product-specific SDK/runtime abstractions.

## Acceptance criteria

- Dispatch concurrency is declared as explicit dispatcher capability state.
- Sequential dispatch is selected from declared policy, not from a failed parallel clone probe.
- Production `AgentAwareDispatcher` remains parallel-capable.
- Single-thread test dispatchers remain valid by declaring sequential policy.
- Focused EAL interpreter tests and architecture gates pass.
