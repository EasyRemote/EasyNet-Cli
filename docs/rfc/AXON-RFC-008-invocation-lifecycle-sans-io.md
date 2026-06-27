# AXON-RFC-008：Invocation 相位架构 — 统一语义内核 + 多 transport surface（v3，2026-06-27）

> 状态：**descriptive / in-progress**（描述 CTO 在 Axon runtime-rs 进行中的实现，未提交）。
> v3 取代 v1/v2：v1/v2 提出的「sans-IO `InvocationLifecycle` 状态机 + `LifecycleDriver` + Transport
> trait」是**错误方向**——它抽的是「公共 RPC loop」（一个泵吞四个 geometry），而正确的抽象是
> 「公共 phase contract」：生命周期**语义**统一，wire/stream/frame/session **机制**各自保留。
> 对应的 SDK 侧 S0-S3 代码已 revert（`23b629e1`/`6ad2173b`/`c7a527ff`/`20362164` + CLI `9eff9780`）。
> 真实实现落在 `EasyNet-Axon/core/runtime-rs/src/services/invocation/`（server 侧），不在 SDK，也不在
> CLI dispatcher。全部 file:line 已对磁盘亲核（2026-06-27，in-progress 工作树）。

---

## 0. 核心判断

**理想架构不是「一个 `LifecycleDriver` 整吞四个 dispatcher」，而是：一个统一 Invocation 语义内核，多条 transport-specific surface。**

要收敛的是**生命周期语义**，不是**文件形状**。`unary` / `server-stream` / `bidi` / `hub-forward` 的共同点只有这些（统一）：

- admission 必须先于 dispatch
- replay / idempotency 必须可查询、可复放
- route / policy / delegation 决策必须在执行前定型
- terminal 必须唯一、单调、可审计
- receipt / ledger / watch / idempotency terminal mapping 必须在**同一个地方**落地

它们**不**共享的（各自保留）：

- unary 的 response shaping
- server-stream 的 chunk / transcript / terminal chunk 规则
- bidi 的 frame-chain / HMAC / session attach
- hub-forward 的 peer transport / resolver delegation

因此抽象的是**公共 phase contract**，不是公共 RPC loop。

---

## 1. 真实架构（磁盘亲核，runtime-rs/services/invocation/）

CTO 进行中的实现把巨型 handler（`rpc_handlers.rs` / `bidi_handler.rs` / `invoke_stream_pipeline.rs`，净 −3461 行）拆成 ~30 个聚焦的 phase 模块。

### 1.1 统一语义内核（所有 geometry 共享）

| 相位 | 模块 | 强类型产物 |
|---|---|---|
| Admission | `admission_phase.rs` / `admission_flow.rs` | `InvocationAdmissionStart{candidate_invocation_id, InvokeAdmissionState::Unsigned\|Verified\|Admitted}`；拒绝 `AdmissionStartRejection` |
| Authorization | `authorization_phase.rs` | `InvocationAuthorization{optional VerifiedDelegation}`；拒绝 `AuthorizationRejection` |
| Scheduling | `invocation_scheduling_phase.rs` | `InvocationSchedulingDecision::{Scheduled(InvocationScheduledDispatch)\|Terminal(InvocationSchedulingTerminal)}` |
| Terminal | `terminal_finalization.rs` | `FinalizedTerminal{outcome, admission_receipt, terminal_receipt, elapsed_ms}` |

强类型纪律：`InvokeAdmissionState`（`admission_flow.rs`）是 `Unsigned → Verified → Admitted` 状态机，承载 `invocation_id/envelope/signed_ability/arguments/proof_binding/admitted_at`——**不再让 handler 到处传 loose strings**。`invocation_id` 在 idempotency claim 成功后经 `commit_verified_with()` 锁定为 `AdmittedInvocation`。

### 1.2 唯一 terminal owner

**`TerminalFinalizationService::finalize`（`terminal_finalization.rs:44`，commit 落 `:138-316`）是唯一 terminal owner。** unary / server-stream / bidi 三 geometry **全部 delegate 到它**，传入 `TerminalOutcome` + optional `TerminalReceiptContext`，它独占：

1. invocation record upsert（`execution.invocations`）
2. request→invocation 反向映射
3. idempotency terminal mapping commit/delete
4. topology node inflight decrement
5. circuit breaker metrics
6. terminal event broadcast（`invocation_tx`）
7. audit trail append

**没有 per-geometry terminal side effect**——response/frame shaping 一律发生在 finalization **之后**（unary `finalize_invoke`、stream chunk projection、bidi frame encoding）。任何 handler 直接写 terminal receipt 或 terminal state 都是架构分叉，本架构在类型层杜绝。

### 1.3 per-geometry surface（机制各自保留）

| Geometry | surface 相位 | 各自拥有的机制 |
|---|---|---|
| unary | `unary_{idempotency,route,scheduling,dispatch,execution,early_response,inline_response,inline_terminal}_phase` | idempotency claim（RAII `UnaryIdempotencyClaim`：`inflight_guard` + rollback fingerprint）、`InvokeResponse` shaping、dispatch opening（RAII 持 semaphore permit 至 terminal） |
| server-stream | `server_stream_{opening,route,execution,loop,hub,terminal}_phase` + `server_stream_terminal_facts.rs` | `StreamGate`（admission 一次、后续相位复用）、transcript digest、chunk loop、terminal chunk 规则 |
| bidi | `bidi_{frame_zero,opening,up_frame,down_frame,payload,loop,session,terminal,phase}_phase` + `bidi_error_codes.rs` | frame-chain / HMAC（up/down frame auth）、session attach、frame-zero 校验、payload routing |
| hub-forward | `hub_forward.rs` / `hub_profile/{forward,resolve,streaming}.rs` | peer transport、resolver delegation |

### 1.4 两层状态（v1/v2 混淆，v3 纠正）

v1/v2 把 `Idle/Admitting/Routing/.../Settled` 当 protocol-visible state 还宣称「Go 1:1 复用」——**错**。两层必须分清：

| internal phase | protocol-visible event |
|---|---|
| request structurally accepted | `accepted` |
| admission + replay commit complete | `admitted` |
| execution target selected + notified | `dispatched` |
| callee/session ack | `running` |
| terminal outcome finalized | `completed`/`failed`/`timed_out`/`cancelled` |

runtime / SDK / conformance 不再各讲一套状态语言。

---

## 2. 相位链（按 geometry）

**统一前缀（所有 geometry）**：
`AdmissionPhase → AuthorizationPhase → InvocationSchedulingPhase`

**unary**：
`UnaryInvokeOpening → UnaryIdempotencyPhase(RAII claim) → start_admission → commit_verified_with → authorize → UnaryRouteFacts(CanonicalInvokeRoute) → UnarySchedulingPhase → UnaryDispatchRecordPhase(RAII opening) → execution → TerminalFinalizationService::finalize → finalize_invoke(shape response)`

**server-stream**：
`ServerStreamOpeningPhase::open(admission+authorization inline) → commit_verified_with(invocation_id 锁入 StreamGate) → route/hub/execution(复用 StreamGate，不重跑 admission) → ServerStreamTerminalFacts → TerminalFinalizationService::finalize → chunk projection`

**bidi**：
`BidiAdmissionOpening.admit(run_admission_gate, seal invocation_id) → frame_zero 校验 → opening → session attach → up/down frame auth → payload loop → bidi_terminal_phase → TerminalFinalizationService::finalize → frame encoding`

---

## 3. 理想模块边界（映射真实实现）

| 理想模块（架构信） | 真实落点 | 职责 / 不该做什么 |
|---|---|---|
| `InvocationSurface` | `rpc_handlers.rs`(瘦后) / `invoke_stream_pipeline.rs` / `bidi_handler.rs` | 只做 wire 适配 + 调用 phase；**不**拥有 receipt commit 细节 |
| `AdmissionPipeline` | `admission_phase` + `admission_flow` + `admission_gate` | raw request → `AdmittedInvocation` 或 terminal rejection（强类型事实） |
| `RouteDecisionPipeline` | `unary_route_phase` / `server_stream_route_phase` + `resolver_consume` | 解释 resolver 结果不执行；产 `CanonicalInvokeRoute`（route_only / descriptor_bound）。**注**：架构信的 sealed `RouteDecision{LocalDispatch\|ForwardDelegation\|TerminalReject}` 当前**未物化**，路由/委派隐含在 `CanonicalInvokeRoute + VerifiedDelegation` 中——是否显式化为 sealed enum 待 CTO 裁决（见 §5）。 |
| `ExecutionDispatch` | `unary_{scheduling,dispatch,execution}` / stream execution | 只送执行端，返回 `TerminalOutcome`，**不** emit terminal side effect |
| `TerminalFinalizationService` | `terminal_finalization.rs` ✅ | 唯一 terminal owner（§1.2）。已是最接近理想的模块 |
| `ReceiptProjection` | （SDK 侧纯映射，待定位） | terminal-kind → receipt-kind/source 纯语义；**不**当 runtime receipt signer。runtime signer/ledger 仍归 runtime |

---

## 4. 落地序列（架构信的 7 步：Extract Invocation Phase Services Behind Existing Surfaces）

> CTO 进行中已覆盖前几步（admission/authorization/scheduling/terminal phase + per-geometry surface 相位已落 runtime-rs）。本节记录目标序列，剩余项为收尾。

1. ✅ 从 `rpc_handlers::invoke` 抽 `AdmissionPipeline`（`admission_phase`/`admission_flow`），不改行为。
2. ⏳ 抽 `RouteDecisionPipeline`，让 resolver/delegation 只有一个解释出口（当前 `CanonicalInvokeRoute`，sealed enum 待裁）。
3. ✅ 抽 `ExecutionDispatch` 返回统一 outcome（`TerminalOutcome`，无 side effect）。
4. ✅ 收紧 `TerminalFinalizationService` 为唯一 terminal owner，禁止 handler 直接 terminal side effect。
5. ✅ `InvokeStreamPipeline` 复用 admission/route/terminal phase，保留 stream loop（`StreamGate` + server_stream 族）。
6. ✅ `bidi_handler` 复用 admission/terminal phase，保留 frame-chain/session（bidi 族）。
7. ⏳ 决定 SDK `InvocationLifecycle` 归宿：保留为纯 conformance algebra，还是抽到 shared protocol crate（见 §5）。

---

## 5. 待裁决（CTO）

- **(a) RouteDecision sealed enum**：当前路由/委派隐含在 `CanonicalInvokeRoute + VerifiedDelegation`。是否物化为架构信的 sealed `RouteDecision{LocalDispatch\|ForwardDelegation\|TerminalReject}`？显式化收益（单一解释出口、穷尽匹配）vs 当前隐式的成本。
- **(b) SDK lifecycle 归宿**：S0-S3 已 revert。SDK 侧是否需要一个**纯 conformance algebra**（internal-phase 的可执行规格，供六语言一致性测试），还是 runtime-rs 的 phase 模块即真相源、SDK 不需要镜像？
- **(c) ReceiptProjection 定位**：terminal-kind → receipt-kind/source 的纯 vocabulary 映射是否值得作为 SDK 类型（CTO 曾认可方向），还是留在 runtime 内。

---

## 6. 一句话结论

理想架构是「统一生命周期事实和 terminal closure」，不是「统一所有 dispatcher 控制流」。真正应被消灭的是重复的 admission/route/terminal **语义**，不是 unary/stream/bidi/hub-forward 各自必要的协议**形态**。本架构（runtime-rs phase 模块 + 唯一 `TerminalFinalizationService`）正是其落地。

---

*v3 描述磁盘真实实现（in-progress, runtime-rs/services/invocation/），每处 file:line 可 `grep` 验证。v1/v2 的 sans-IO LifecycleDriver 方向已作废、对应 SDK 代码已 revert。*
