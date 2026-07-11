# Planner-Facing KernelApi Subset — v1 Skeleton

> Plan v10.1. Freezes the minimum method signatures that a future
> `easynet-planner` will consume. v1 does not implement a planner;
> this doc is the contract a successor PR honours.

## 1. Why freeze this now

The plan promotes the architecture from "left a space" to "can
grow". Concretely: every feature PR (`PR-ATTACH` / `PR-PERM` / …)
must declare its KernelApi method using domain-object signatures
from `src/core/domain/`. If a later planner wants to ask
"what abilities can I call, what do they cost, when am I allowed",
it should find the answers on the same surface the Control layer
already uses — no parallel API surface, no second-pass refactor.

## 2. Minimum surface

```rust
trait PlannerFacingKernelApi {
    // Ability discovery — enumerate callable primitives
    fn discover_abilities(&self, filter: AbilityFilter) -> Vec<AbilityDescriptor>;
    fn get_ability_schema(&self, ability: AbilityUra) -> AbilitySchema;

    // Cost estimation — the signal the planner uses to decide "run
    // locally or push to peer B?". v1 returns best-effort (one
    // latency number + locality hint); v2 extends with a real cost
    // model (tokens, CPU, cached vs fresh).
    fn estimate_invocation_cost(&self, target: InvocationTarget) -> CostEstimate;

    // Execution — the RPC and the stream variant
    fn invoke_ability(&self, target: InvocationTarget) -> Result<Value>;
    fn subscribe_ability(&self, target: InvocationTarget) -> Subscription<Frame>;

    // Observation — domain objects the planner audits
    fn subscribe_session(&self, session: SessionId) -> Subscription<TimelineEvent>;
    fn list_active_sessions(&self) -> Vec<Session>;

    // Approval — the planner may decide to wait for a human
    fn pending_permission_requests(&self) -> Vec<PermissionRequest>;
    fn decide_permission(&self, id: PermissionId, decision: Decision);

    // Loops and schedules as first-class primitives the planner
    // composes with other Invocations
    fn manage_loop(&self, op: LoopOp) -> LoopInstance;
    fn manage_schedule(&self, op: ScheduleOp) -> ScheduleEntry;
}
```

Each method returns a domain object declared in
`src/core/domain/`, not a `serde_json::Value` / `args_json`
tuple. That is the load-bearing property: the planner can write
its strategy layer without inventing a parallel type system.

## 3. What is deliberately not here

- No batch primitives (`invoke_many` / `subscribe_many`). The
  planner composes over the single-shot entries explicitly.
- No transaction semantics. The planner is an optimiser, not a
  distributed transaction coordinator.
- No scheduler-policy surface. That lands inside
  `daemon::boot::kernel::Kernel::invoke`'s admission phase as a v2 hook; see
  `docs/design/invocation-unity-v1.md` §6.
- No capability grant. v1 permission is an approval broker, not
  capability security (see `docs/rfc/permission-broker-v1.md`).

## 4. How a successor planner PR lands

1. Create `easynet-planner` as a module or bin alongside the
   Control layer (never beneath Execution).
2. Register itself as one or more `ReceiptSubscriber`
   implementations when the S-path upgrade ships.
3. Consume the surface above. The KernelApi trait does not
   change; the planner is a peer of Control, not a layer below.

## 5. Binding point for Client

A Client that wants a planner-driven experience calls a
planner-owned canonical ability (for example, a planner agent's
`execute_goal` ability URA) through the normal daemon Invocation path;
the planner consumes the call and produces downstream Invocations
itself. Client does not speak to the planner directly — it speaks to
the daemon boundary, which routes.
