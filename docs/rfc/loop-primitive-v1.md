# Loop Primitive v1 — Local Control, Not Planner

> Plan v10.1. Pins the boundary so external readers don't mis-read
> EasyNet's loop wrapper as "we shipped a planning system".

## 1. What a Loop is in v1

A **local control primitive** — a bounded "worker + verify + retry"
closure. One worker agent runs a body step; one verifier checks the
result; a step either advances or retries; `max_iters` caps the
runtime.

Concrete v1 shape (PR-LOOP):

```rust
system.loop.create(worker_agent, verify_expr, max_iters, body_prompt)
    → { loop_id }
system.loop.status(loop_id)
    → { state, current_iter, last_body_output, last_verify_output }
system.loop.subscribe(loop_id)
    → Stream<LoopStatus>   // IterStarted | BodyFrame | VerifyFrame | Terminal
system.loop.cancel(loop_id)
    → Receipt
```

Internally the loop controller emits one Invocation per body
iteration and one per verify step (plan v10.3 C* unity constraint).

## 2. What a Loop is **not** in v1

- Not a planner. It does not choose which worker agent to run
  or which ability to call; the Client specifies.
- Not an agent team. No negotiation, no role assignment, no
  coordination between multiple loops.
- Not cost-aware. It does not route based on tokens / latency
  budgets; `max_iters` is the only safety valve.
- Not cross-loop coordinated. Two loops running concurrently do
  not share state or priority.

If a Client wants any of the above, that is a planner. Planners
live *above* KernelApi, alongside the Control layer. The loop is a
primitive the planner composes with.

## 3. Reviewer checklist when "loop" appears in a PR

- [ ] Does the PR treat Loop as a single worker+verify closure?
      OK.
- [ ] Does the PR add multi-agent coordination / agent teaming
      inside the loop? Reject — that belongs to a future planner
      plan, not here.
- [ ] Does the PR claim the loop "decides what to do next"? Reject
      — the loop runs what the Client (or planner) specified.
- [ ] Does the loop controller bypass `Kernel::invoke` for body /
      verify steps? Reject — plan v10.3 C* requires one
      Invocation per step.

## 4. v2 evolution

When a planner lands, it will consume `LoopInstance` as one
primitive among several and compose multi-step strategies over
them. The loop's v1 surface does not change when the planner
arrives; only new consumers appear. That is the load-bearing
property.
