# AXON-RFC-008：InvocationLifecycle sans-IO 协议语义下沉 Axon SDK（v2，2026-06-25）

> 状态：**ratified-design / pre-code**（CTO Silan.Hu 修正版裁决，编码前无遗留决策）。
> 本版 v2 取代 v1：v1 的两点被 CTO 否决——「route facade 整体下沉 SDK」改为「只下沉纯
> 类型 + CLI 留 resolver」；「Timeout/Cancelled 并进 Failed」改为「独立终态不可混淆」。
> 关系：本 RFC 是 A1（会话平面隐式状态机，F-008/F-009）债的**第二根支柱**——
> T1.1 的 `DeviceSessionState` 治"连接层"状态机（Dialing→Live→Backoff），本 RFC
> 治"调用层"协议语义状态机（Admitting→…→Settled）。两者正交互补，见 §0.3。
> **依赖**：A2（dispatch-frame-carrier-unification mini-RFC，已批准在落）必须先完成——
> 本设计的 `InvocationReceived` 需要 carrier 保证的帧七元组完整性 + `DispatchResult.receipt`
> 才能解码出干净 envelope 并在终态投影真 receipt。carrier 是地基，本 RFC 是上层。
> 全部 file:line 已对照磁盘亲核（分支 `seven-axes-p0-landing-v1`，2026-06-25）。

---

## 0. 核心裁决与边界

### 0.1 核心裁决（CTO 修正版）

**`InvocationLifecycle` 的协议语义下沉到 Axon SDK；`DaemonRouteResolver`、presence、session、plugin/local runtime facts 仍留在 EasyNet-Cli daemon。**

Axon 统一"生命周期状态、终态、receipt 投影规则、stream/bidi/unary 语义"；CLI 只做 wire decode、daemon route facts、transport send/recv。这是一条**语义/事实分离**线：Axon 拥有**协议如何演进**（与 I/O、运行时、daemon 存储无关的纯语义），CLI 拥有**本地事实从哪来**（live runtime/session/presence/plugin lookup）。

### 0.2 边界裁定（谁拥有什么）

**Axon SDK 拥有：**

- sans-IO lifecycle state machine：**无 tokio、无 tonic、无 mpsc、无 daemon store**。
- 纯 Rust `RouteQuery` / `ResolvedRoute` / `RouteError` / `RouteProfile` / `DispatchTarget` 类型，**但不实现 daemon route lookup**。
- `TransportFrame` / `LifecycleFrame` 这种 wire-agnostic frame union。
- terminal mapping：`Completed`、`Failed`、`TimedOut`、`Cancelled` **不可混淆**。
- receipt projection API：复用现有 `InvocationReceipt`、`ReceiptProofFacts`、`LedgerSink`、`receipt_to_wire`。

**EasyNet-Cli 保留：**

- `route_resolver.rs` 的 live runtime/session/hosted-agent placement lookup（`DaemonRouteResolver` 不下沉）。
- `PresenceRegistry`、pending maps、session up/down sender、plugin/ability registration。
- tonic/proto ↔ Axon pure frame conversion。
- federation wrapper handlers。**`federation_wrappers.rs` 不是 lifecycle 复制体，不是删除目标。**

### 0.3 与 A1（T1.1）/ A2（carrier）的层级关系

| 层 | 状态机 | 管什么 | 归属 |
|---|---|---|---|
| 连接层 | `DeviceSessionState{Idle,Dialing,Preluding,Live,Backoff}` + `CloseClass`（T1.1 / F-008） | 一条 device↔hub **会话连接**的生死 | CLI device 侧 |
| wire 层 | DispatchCall/DispatchResult proto 帧（A2 / F-004） | dispatch **帧载体**归一（七元组完整、receipt 闭环） | Axon proto + CLI |
| **调用层** | **`InvocationLifecycle{Idle,Admitting,Routing,Authorizing,Dispatching,Open,Settled(TerminalKind)}`（本 RFC）** | **一次调用的协议语义生命周期**，跨全部 4 geometry，Go 可复用 | **Axon SDK 纯核心** |

三者正交且层叠：carrier（wire）保证帧干净 → 本 RFC（调用层语义）在干净帧上跑单一生命周期 → T1.1（连接层）管承载这些调用的会话连接。**本 RFC 不取代任何一个，是 A1 终态的调用层支柱。**

### 0.4 根因（为何不收敛）

调用生命周期的**协议语义**（admission → route → authorize → dispatch → terminal/receipt projection）被复制实现了四遍，唯一差异是 transport geometry，四份各自演化、彼此漂移：

| Geometry | 文件 | 总 LOC | 其中 lifecycle | 漂移锚点 |
|---|---|---|---|---|
| unary | `unary_dispatcher.rs` | 2224 | ~800 | busy/offline 分类 `:1524` 区——**只有此处区分 Full→retryable 与 Closed→offline** |
| bidi | `bidi_dispatcher.rs` | 3348 | ~1060 | `settle_terminal_result`:2484；session drain |
| stream | `stream_dispatcher.rs` | 469 | ~106 | terminal 空 payload 直接 break `:356`——**terminal chunk 不发、receipt 不附（现存 bug）** |
| local_session | `local_session_dispatcher.rs` | 3829 | ~400 | carrier-v1 dispatch/result 双解码 |

> **修正（2026-06-26 逐方法解剖 12,268 行）**：上表"总 LOC"大部分**不是** lifecycle——四文件合计仅 ~2,400 行是真·调用生命周期，其余 ~5,000 行是 RPC handler（多数字面调 `federation_wrappers::handle_*`）+ ~4,800 行共享辅助（frame builders/mappers/stream types/ctor）。漂移只发生在那 ~2,400 行 lifecycle 上。因此本 RFC 的目标是**迁出那 ~2,400 行 lifecycle，保留其余作 handler-router**，不是整删文件。

**为何永远不收敛**：每个语义修复必须在 N 个 dispatcher 各打一遍补丁，N 份不可能同步。活样本：busy≠offline 只在 unary 落实；stream 根本不投影 terminal receipt。没有一个对象拥有"协议状态"——它散在四个 `async fn` 的 lifecycle 路径里，无法集中演进、无法被 Go 复用、无法独立单测。

---

## 1. Axon 核心表面（sans-IO）

落 `EasyNet-Axon/sdk/rust/src/invocation/lifecycle.rs`（greenfield，已确认 SDK 下无 `lifecycle.rs`）。零 tokio、零 await、零 tonic、零 mpsc、零 daemon store。可独立单测，可被 Go 1:1 镜像。

```rust
pub struct InvocationLifecycle { state: State, ctx: Context }

pub enum State {
    Idle,
    Admitting,
    Routing,
    Authorizing,
    Dispatching,
    Open,
    Settled(TerminalKind),
}

pub enum TerminalKind {
    Completed,
    Failed { code: ErrorCode },
    TimedOut,
    Cancelled,
}

impl InvocationLifecycle {
    pub fn on_event(&mut self, ev: Event) -> Vec<Action>;
    pub fn state(&self) -> State;
}
```

**终态不可混淆（CTO 硬约束）**：`TimedOut` 与 `Cancelled` 是 `TerminalKind` 的**独立变体**，绝不并进 `Failed`。timeout → `TimedOut` receipt；caller cancel → `Cancelled` receipt。

**`Open` 取代长期 `Projecting`（CTO 修正）**：receipt 投影**不是**一个长期可见状态。它是 terminal transition 上的**原子 action 序列**：先 `ProjectReceipt`，再 `SendFrame(terminal)`，同一个 `on_event` 后直接进入 `Settled(...)`。流式调用在出结果块期间停在 `Open`（每块只发 `SendFrame(chunk)`），直到 terminal 事件一次性收口。**若 terminal frame 发送失败，invocation 仍已终态**——那是 transport diagnostic，**不回滚 receipt**。

`Idle` 是初态；`Open` 是"已 dispatch、正在出块或等终态"的持续态。

### 1.1 Event 与 Action

**Event：**

- `InvocationReceived`
- `AdmissionGranted` / `AdmissionDenied`
- `RouteResolved` / `RouteRejected`
- `AuthorizationGranted` / `AuthorizationDenied`
- `DispatchAccepted`
- `DispatchChunk`
- `DispatchTerminal`
- `DispatchFailed`
- `CallerCancelled`
- `DeadlineExceeded`
- `TargetPeerClosed`

**Action：**

- `RunAdmission`
- `ResolveRoute`
- `DispatchTo`
- `SendFrame`
- `ProjectReceipt`
- `PersistLedgerBestEffort`
- `EmitError`
- `CloseTransport`

**v1 authorization 空透传**：`RouteResolved` 后进入 `Authorizing`，同 tick 自发 `AuthorizationGranted`，返回 `DispatchTo`。**不引入 `RunPolicy`，但保留 `AuthorizationDenied` 边**（Policy frame 落地时只在进 Authorizing 发 RunPolicy + executor 回喂决策，state/transition/ctx 不动）。

### 1.2 关键转移表

```text
Idle + InvocationReceived
  -> Admitting [RunAdmission]

Admitting + AdmissionGranted
  -> Routing [ResolveRoute]

Admitting + AdmissionDenied
  -> Settled(Failed) [ProjectReceipt(admission_failed), EmitError]

Routing + RouteResolved
  -> Authorizing -> Dispatching [DispatchTo]    ← 同 tick 自发 AuthorizationGranted

Routing + RouteRejected
  -> Settled(Failed) [ProjectReceipt(route_rejected), EmitError]

Dispatching/Open + DispatchChunk
  -> Open [SendFrame(chunk)]

Dispatching/Open + DispatchTerminal(Completed)
  -> Settled(Completed) [ProjectReceipt(completed), SendFrame(terminal)]

Dispatching/Open + DispatchFailed
  -> Settled(Failed) [ProjectReceipt(failed), EmitError/SendFrame(error)]

Dispatching/Open + DeadlineExceeded
  -> Settled(TimedOut) [ProjectReceipt(timed_out), EmitError]

Dispatching/Open + CallerCancelled
  -> Settled(Cancelled) [ProjectReceipt(cancelled), CloseTransport]

Dispatching/Open + TargetPeerClosed
  -> Settled(Failed{PEER_CLOSED}) [ProjectReceipt(failed), EmitError]

Settled + *
  -> Settled [no-op]
```

**五主干脊柱落点**：`Admitting`=Admission（Identity 内嵌于 envelope 校验）；`Routing`=Discovery；`Authorizing`=Policy（v1 空透传，State 变体 + 两条边 + `ctx.policy_decision` 字段三重常驻）；terminal transition 的 `ProjectReceipt`=Receipt 主干唯一 proof_facts sink。`trust` 是 attribute 不入 State。

---

## 2. Route 修正（CTO：不整体下沉）

**S0 不是"整体下沉 route facade"。** 拆成：

- Axon 新增**纯类型**：`RouteQuery`、`ResolvedRoute`、`RouteProfile`、`DispatchTarget`（无 daemon lookup 实现）。
- CLI 的 `SelectedInvokeRoute`（`route_resolver.rs:146`）实现 `TryFrom<SelectedInvokeRoute> for axon::ResolvedRoute`。
- **`DaemonRouteResolver` 留在 CLI**；它执行 `ResolveRoute` action 并回喂 `RouteResolved` event。
- `DispatchTarget` 中 daemon 专属字段（local session / presence / plugin 路由信息）用 **opaque key** 承载，**Axon 不解释其含义**——core 只知"有个目标键",不知它指向 presence 槽位还是 plugin handler。

这条保证 Axon core 真正 sans-IO + 无 daemon 概念泄漏，同时 CLI 保留所有 live fact lookup。

---

## 3. Receipt 修正（CTO：CLI 不许手搓 canonical receipt）

| 情形 | receipt 来源 | 禁止 |
|---|---|---|
| local Axon runtime 执行的 call | Axon `LocalRuntime` 现有 receipt chain 产生 terminal receipt | CLI 自造 |
| remote carrier-v1 回来的 callee-signed receipt | CLI 只把它投到本地 hub ledger | **重签成 callee receipt** |
| admission/route 失败、未进 callee execution | runtime/admission 层失败 receipt 或 ledger diagnostic | **伪造成 callee execution receipt** |
| stream terminal | 必须携带 receipt（proto 已有 `InvokeStreamChunk.admission_receipt` + `terminal_receipt` 字段） | 改 proto（不需要） |

`ProjectReceipt` action 在 Axon 侧据上表规则选择 receipt 来源；CLI 的 executor 只搬运不铸造。

---

## 4. 落地序列（每步独立可编译，1 commit = 1 逻辑变更；lifecycle 方法迁出后旧体删除不弃用，handler 方法保留，无 fallback）

| 步 | 内容 | 落点 |
|---|---|---|
| **S-1** | 建计划包 `pr/2026-06-25-invocation-lifecycle-sans-io/`，写 invariants、边界证明、caller inventory | CLI `pr/` |
| **S0** | Axon SDK 增加纯 route/frame/lifecycle 类型（`RouteQuery/ResolvedRoute/RouteProfile/DispatchTarget` + `TransportFrame`），**不接 CLI resolver** | Axon `invocation/{route,frame}.rs` |
| **S1** | 实现 `InvocationLifecycle` 转移表，单测覆盖每条边、终态吸收、timeout/cancel 独立终态、authorization 空透传 | Axon `invocation/lifecycle.rs` |
| **S2** | 实现 receipt projection API，复用现有 `InvocationReceipt`/`ReceiptProofFacts`/`LedgerSink` | Axon `invocation/` |
| **S3** | CLI 新增 `lifecycle_driver.rs`，mock transport 单测：decode frame → `on_event` → 执行动作 → 回喂 event | CLI `lifecycle_driver.rs` |
| **S4-unary** | 切 invoke arm 的 catch-all `_other`（`daemon_invocation_service.rs:822` = `dispatch_local_rpc_selected_route`）到 driver；替换 unary busy/offline 分类；确认无 double ledger row（复用 `axon_took_it` 路径）。**具名 federation/namespace/identity arm 全是 handler，保持直调不动。** forward_invoke（hybrid）留待后续 pass | CLI `transport_impls/unary.rs`（新建 adapter，借用幸存 helper）；`unary_dispatcher.rs` **降为 handler-router**（lifecycle ~800/2224 行迁出，handler+helper ~1400 行留） |
| **S4-stream** | 切 `dispatch_local_selected_route`（`stream_dispatcher.rs:287`）到 driver；**修** terminal 空 payload break（driver 的 Open→Settled 总投 `[ProjectReceipt, SendFrame(terminal)]`） | CLI `transport_impls/stream.rs`；`stream_dispatcher.rs` **降为 directory-subscription handler 模块**（`dispatch_subscribe_directory_*` 是 presence pump，留；lifecycle ~106 行迁出） |
| **S4-bidi** | 切 lifecycle arm（`dispatch_remote_bidi:283`/`dispatch_local_bidi_selected_route:627`/`dispatch_invoke_remote:856`/`dispatch_self_session_accept:1266`）到 driver；`settle_terminal_result` 终态化进 driver | CLI `transport_impls/bidi.rs`；`bidi_dispatcher.rs` **保留 frame-0 router + session-plane handler**（`drain_session_up_stream`/`dispatch_session_request_named` 留；frame builders/mappers/stream types 留或抽 util；lifecycle ~1060 行迁出） |
| **S4-session** | 切 carrier-v1/Axon-dispatch lifecycle 入口（`handle_carrier_v1_dispatch:221`/`handle_carrier_v1_stream_open:381`/`try_dispatch_via_axon:783`/`open_stream_via_axon:913`）到 driver；**保留 `SessionUpSender` 作为 transport ordering primitive** | CLI `transport_impls/local_session.rs`；`local_session_dispatcher.rs` **保留 `handle_down` frame router + bidi/forwarding handlers**（lifecycle ~400 行迁出，handler+plumbing 留） |
| **S5** | 每个 geometry 切完同 commit 删除对应旧 dispatcher 残壳；**`federation_wrappers.rs` 不删** | CLI |
| **S6** | 收口 `admission_facade.rs`：把 Axon admission/canonical crypto 辅助迁到 Axon，CLI 只留 keyring/session/delegation policy glue | Axon + CLI |
| **S7** | `cargo udeps` 或等价死代码检查，删未引用壳层 | CLI |

---

## 5. 验收门槛

每步必须跑（六命令全绿）：

```bash
cargo build
cargo build --features axon-pb
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features axon-pb -- -D warnings
cargo test
cargo test --features axon-pb
```

**新增必测（七条）：**

- unary busy 是 retryable backpressure，**不移除 presence**；closed 才 offline。
- stream terminal 空 payload **也必须发 terminal chunk + receipt**。
- bidi terminal 缺 receipt 要 **fail closed 或记录协议违约，不能挂 pending**。
- timeout → `TimedOut` receipt，**不是 `Failed`**。
- caller cancel → `Cancelled` receipt，**不是 `Failed`**。
- terminal 后重复 frame/event **全 no-op**。
- remote receipt projection **不重签、不伪造 callee execution receipt**。

---

## 6. 落点速查（编码定位）

- 核心落点：`EasyNet-Axon/sdk/rust/src/invocation/lifecycle.rs`（新建）+ `mod.rs` `pub mod lifecycle;`
- 纯路由/帧类型：`EasyNet-Axon/sdk/rust/src/invocation/{route,frame}.rs`（新建，纯类型无 lookup）
- SDK 既有依赖：`audit.rs:383`(new_receipt)、`axiom.rs:343`(ReceiptProofFacts)、`call_mode.rs:107`(CallMode)、`admission.rs`(run_descriptor_bound_admission)
- CLI driver：`EasyNet-Cli/src/services/invocation_transport/lifecycle_driver.rs`（新建）+ `transport_impls/{unary,stream,bidi,local_session}.rs`（新建）
- CLI 保留：`route_resolver.rs`（`DaemonRouteResolver` live lookup）、`PresenceRegistry`、session up/down sender、plugin/ability registration
- 迁出目标（**非整删**，2026-06-26 逐方法解剖修正）：四个 dispatcher 各自的 lifecycle 方法（unary `dispatch_local_rpc_selected_route` / stream `dispatch_local_selected_route` / bidi `dispatch_remote_bidi`等4个 / session `handle_carrier_v1_dispatch`等4个）迁入 driver+executor；**四文件保留为 handler-router**（federation/namespace/identity/directory-subscription/session-plane handler + frame builders/mappers/stream types，与 federation_wrappers 同类不删，每文件留 ~75-85%）。S5 删的是迁出后变死的 lifecycle 方法体，**不是删文件**。`admission_facade.rs` 的 lifecycle 编排部分（S6 收口）。
- ⚠️ 跨文件耦合：`dispatch_self_targeted_forward_invoke`/`dispatch_self_targeted_invoke_remote`（unary 内）被 `bidi_dispatcher.dispatch_invoke_remote` 调用——是共享 lifecycle 入口，不可内联删除，须保持可调或移入共享模块。
- ⚠️ forward_invoke hybrid（unary `:1041` / bidi `:856`）：handler 外壳包 contained lifecycle。**S4 只迁纯本地 owner-routed arm，forward_invoke 内层 lifecycle 留后续 pass**（爆炸半径最小）。
- **不删**：`federation_wrappers.rs`（非 lifecycle 复制体，CTO 确认）

---

*本 RFC v2 描述磁盘真实代码，每处 file:line 可经 `grep` 验证。v2 取代 v1（route facade 整体下沉、Timeout/Cancelled 并进 Failed 两点已被 CTO 否决）。依赖 A2（carrier-unification）先行；编码前 CTO 修正版裁决已拍板，无遗留决策。*
