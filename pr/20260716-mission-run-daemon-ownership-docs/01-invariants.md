# Invariants

1. Mission/EAL remains an implementation strategy for composite ability logic,
   not a second Invocation primitive.
2. Mission-run local persistence is EasyNet daemon product-runtime state, not
   Axon protocol state.
3. `MissionRunStatus` is exactly `Running -> {Ok, Partial, Error, Cancelled}`
   in the current persisted `meta.json` model.
4. Liveness is heartbeat freshness, not pid-file presence.
5. CLI mission commands project or trigger daemon-owned orchestration; they do
   not own the lifecycle state machine.
