# 设计裁决书：InvocationLifecycle 状态机下沉至 Axon Rust SDK（工业级彻底重构）

> 状态：**已裁决（RATIFIED 2026-06-25 by CTO Silan.Hu）— 写码前阻塞已全部关闭，见 §6 裁定**
> 作者：Silan.Hu（CTO 凉冰审）
> 日期：2026-06-25
> 范围：`EasyNet-Axon/sdk/rust` + `EasyNet-Cli/src/services/invocation_transport`
> 原则：彻底（无 fallback / 无 backward-compat shim / 无 legacy 残渣 / 无 dead code）。真阻塞上交，不绕过。

本文所有 file:line 已对盘磁盘（2026-06-25，分支 `seven-axes-p0-landing-v1`），非凭记忆。

---

## 0. 一句话

`invocation_transport/` 当前 **30,957 LOC / 26 个 `.rs` 源文件**（含 2 个 test 文件即 28 个）把同一条调用生命周期（admission → route → dispatch → project → settle）按 4 种传输几何（unary / stream / bidi / local-session）重写了 4 遍。每个语义修复都要打 N 次补丁、互相 drift，**结构上不可能收敛**。本裁决把这条生命周期作为**平台原语**下沉进 Axon SDK 的一个新文件 `src/invocation/lifecycle.rs`，CLI 只剩 4 个 `Transport` adapter。Go 后端将来 mirror 同一状态机。

---

## 1. 根因（为什么不收敛）

**根因：生命周期编排逻辑没有归宿（no home），被复制进了 4 个传输适配器，因为 SDK 此前只下沉了"原语"（admission gate、dispatch_shim、receipt、ledger_sink）却从未下沉"编排这些原语的状态机"。**

证据（同一条生命周期的 5 个阶段在 4 个文件里各写一遍）：

| 生命周期阶段 | SDK 已有原语 | CLI 重复实现点（file:line） |
|---|---|---|
| Admission 4-step gate | `admission.rs::run_descriptor_bound_admission`（SDK 已有，mod.rs:41 导出） | `admission_facade.rs:506 verify_invoke` / `:522 verify_invoke_stream` / `:539 verify_envelope_for_bidi` — 三个几何各调一次 `run_transport_policy_gate` |
| Route resolve | `SelectedInvokeRoute`（CLI 内，**proto-coupled**，见 §5 阻塞 B1） | `route_resolver.rs:595 resolve_route` 被 `unary_dispatcher.rs` / `stream_dispatcher.rs:397` / `bidi_dispatcher.rs:604` 三处相同调用 |
| Dispatch-ack | `dispatch_shim.rs:338-572 dispatch_rpc_*/open_stream_*/open_bidi_*`（已是 Axon 原语） | 三个 dispatcher 各自按几何挑入口、各自 await |
| Pending + backpressure 分类 | `backpressure.rs::BackpressurePolicy`（SDK 已有） | `unary_dispatcher.rs:1509-1625` / `bidi_dispatcher.rs:304-339` / `local_session_dispatcher.rs spawn_*_forwarder` 各写一遍 Full=retryable vs Closed=offline |
| Terminal detect + receipt project | `handle.rs:83 is_terminal()` + `TERMINAL_STATES`（SDK 已有）+ `RpcDispatchOutcome{state,terminal_receipt}`（dispatch_shim.rs:230-241） | `unary_dispatcher.rs:1723-1753` / `bidi_dispatcher.rs:378-403` / `stream_dispatcher.rs:359-371` / `local_session_dispatcher.rs:1596-1610` 各写一遍终态判定 |
| Ledger project | `ledger_sink.rs::LedgerSink`（SDK 已有 seam） | `ledger_projection.rs:37 build_unary_ledger_record` / `:514 ledger_record_from_remote_receipt` + `bidi_dispatcher.rs:1178-1204` |
| 失败分类 | （SDK `AxonErrorKind` 仅 7 个通用 gRPC 变体，见 §5 阻塞 B2） | `invoke_remote_initiator.rs:354 RequestOutcome` / `:367 SessionRequestError{TargetOffline\|PermissionDenied\|UpstreamFailure\|UpstreamTimeout}` — CLI 本地枚举 |

**直接后果**：`busy != offline` 修复、carrier 版本 stamping、non-blocking drain 这类语义修复，必须在 `unary_dispatcher`(2282) / `bidi_dispatcher`(3369) / `stream_dispatcher`(472) / `local_session_dispatcher`(3670) 四处分别打，任一处漏打就 drift。这不是代码风格问题，是**缺一个 owner**。

**澄清一个误导信号**：`admission_facade.rs` 与 `federation_wrappers.rs` **字节数完全相同（各 95394 bytes）但 MD5 不同**（`47085e1d…` vs `d55e2254…`）。已对盘：二者非克隆，仅 439 行共享且全是 `assert!`/`#[test]`/`None,` 之类样板，承载逻辑零重叠。`admission_facade` 是"每-RPC 策略 gate"（100% 共享、0% adapter，应折叠进 driver 的 admit() 边），`federation_wrappers` 是 14 个 `federation.*` ability 的 handler 业务体（gate 之后运行，**不是生命周期、不进 SDK**）。字节相同是编辑器巧合，**零架构含义**。真正的重复在 4 个 dispatcher，不在这两个文件之间。

---

## 2. 终局架构

### 2.1 归宿（谁住哪）

```
EasyNet-Axon/sdk/rust/src/invocation/
  lifecycle.rs        ← 新建。状态机 driver（admit→route→dispatch→project→settle）
  transport.rs        ← 新建。Transport trait + TransportFrame enum（proto-free）
  route.rs            ← 新建。RouteResolver trait + ResolvedRoute（proto-free 投影，见 §5-B1）
  admission.rs        ← 已有。run_descriptor_bound_admission（driver 的 admit() 调它）
  handle.rs           ← 已有。InvocationState / TERMINAL_STATES / is_terminal()
  backpressure.rs     ← 已有。BackpressurePolicy
  ledger_sink.rs      ← 已有。LedgerSink（driver 在 settle 写）
  audit.rs            ← 已有。InvocationReceipt
  axiom.rs            ← 已有。DescriptorBoundEnvelope

EasyNet-Cli/src/services/invocation_transport/
  transport/unary.rs    ← Transport impl（一次性 collapse 成 InvokeResponse）
  transport/stream.rs   ← Transport impl（mpsc→tonic server-stream pump）
  transport/bidi.rs     ← Transport impl（sequence stamping / up-seq 校验 / heartbeat）
  transport/session.rs  ← Transport impl（carrier-v0 JSON / carrier-v1 DispatchResult 双读）
  route_adapter.rs      ← RouteResolver impl（包 resolve_route，proto→ResolvedRoute 投影）
  federation_wrappers.rs← 保留（ability handler 业务体，非生命周期）
  daemon_invocation_service.rs ← 瘦身：3 个 RPC 方法各建对应 Transport adapter，交给 SDK driver
```

> **命名冲突告警**：SDK 内已存在 `src/presets/ability_dispatch/lifecycle.rs`（别的关注点）。新文件**必须**是 `src/invocation/lifecycle.rs`，不得复用 presets 路径。

### 2.2 状态机（states + transitions）

```
                    ┌─────────────┐
   inbound Envelope │  Admitting  │
   ─────────────────►             │
                    └──────┬──────┘
        admit() Ok         │        admit() Err
   (4-step verify +        │   (ENVELOPE_INCOMPLETE / SIG_INVALID /
    delegation + quota)    │    NONCE_REPLAY / membership miss /
                           │    quota exceeded → 无 admission receipt)
                    ┌──────▼──────┐                    │
                    │   Routing   │                    │
                    └──────┬──────┘                    │
        route() Ok         │        route() Err        │
   (resolve_route +        │   (no route / ownership    │
    locality decision)     │    mismatch / cross-realm  │
                           │    unresolved)             │
                    ┌──────▼──────┐                    │
              ┌────►│ Dispatching │                    │
              │     └──────┬──────┘                    │
   非终态 frame│            │                            │
  (self-loop: │   ┌────────┼─────────┐                 │
   stream/bidi│   │ Ok     │  Err    │                 │
   pump,      │   │        │ (offline/closed/timeout/  │
   recv/send) │   │        │  try_send err/escalation   │
              └───┘        │  Err{TargetOffline...})    │
                    ┌──────▼──────┐                    │
                    │ Projecting  │                    │
                    └──┬───────┬──┘                    │
       Completed       │       │  Failed/TimedOut/     │
   (terminal_receipt   │       │  Cancelled            │
    present, ledger    │       │  (outcome.error →     │
    record)            │       │   TerminalReceiptFailure)│
              ┌────────▼──┐ ┌──▼──────────┐◄───────────┘
              │  Settled  │ │   Failed    │
              │ [TERMINAL]│ │  [TERMINAL] │
              └───────────┘ └─────────────┘
        emit terminal+admission     emit Failed terminal receipt
        receipts via send_frame;    (或纯 tonic::Status 若在 Admitting/
        close()/Drain best-effort   Routing 即拒，runtime 前)；close()
```

**Canonical states**（driver 内部枚举，与 SDK `InvocationState` 的关系见下）：
`Admitting → Routing → Dispatching → Projecting → Settled | Failed`

注意：driver 的 5 阶段状态 ≠ SDK 已有的 `InvocationState`（`Admitted/Running/Completed/Failed/TimedOut/Cancelled`，handle.rs:27）。后者是**调用结果状态**（写进 receipt/ledger），前者是**编排阶段**。`Projecting` 边读 `outcome_state: InvocationState` 决定走 Settled 还是 Failed（`is_terminal() && !Completed → Failed`）。两者**不合并**——一个是 driver 私有控制流，一个是 wire-visible 结果契约。

**转移守卫**（逐边，证据见 §1 表与转移列表，此处只列裁决要点）：

- `Admitting→Routing`：`run_descriptor_bound_admission` Ok（4-step：validate_envelope → validate_signature_structure → verify_signature → `NonceReplayStore.check_and_record`）；`envelope.subject != caller` 时加 `verify_delegation_metadata`；loopback URA 绕过 crypto **但不绕过 membership**。quota（`check_quota_for_ability`）作为 Policy 子门在此边跑（admission_facade.rs:374, daemon_invocation_service.rs:737-752）。
- `Admitting→Failed`：任一拒因 → **不发 admission receipt**（runtime 前拒，纯 `tonic::Status`）。
- `Routing→Dispatching`：`resolve_route` Ok 且 `matches_self_target_ura == true → LOCAL`；`== false → REMOTE`（sub-target 由 Transport adapter 选：presence sender / session call_id / peer hub endpoint）。`is_authoritative_local_or_better()` 决定 local vs better-route。
- `Routing→Failed`：`ResolveRouteFailure`（无路由 / ability_ura ownership 不符 / dispatch_key 不符）| 跨 realm delegation 未解析。
- `Dispatching→Projecting`：LOCAL → `dispatch_shim` 返回 `RpcDispatchOutcome`/handle；REMOTE → `Transport.recv_frame` 在 `PRESENCE_DISPATCH_REPLY_TIMEOUT` 内 yield 终态 frame。
- `Dispatching→Dispatching`（self-loop）：非终态 frame，`recv_frame`/`send_frame` 继续；bidi 校验 up-sequence。**这条 self-loop 是 stream pump 的归宿**。
- `Dispatching→Failed`：target offline/closed（**busy != offline**：`Full`=可重试背压保持 session；`Closed`=`OfflineReason::StreamClosed` 驱逐）| timeout（`UpstreamTimeout`）| `sender.try_send` err | escalation `Err{TargetOffline|PermissionDenied|UpstreamFailure|UpstreamTimeout}`。
- `Projecting→Settled`：`outcome.state ∈ {Completed}`，carrier-v1 **要求**终态 receipt；`axon_took_it==false` 时写 ledger。
- `Projecting→Failed`：`outcome.state ∈ {Failed,TimedOut,Cancelled}`，`outcome.error → TerminalReceiptFailure`。
- `Settled/Failed → [TERMINAL]`：发终态(+admission) receipt，`close()`/`Drain(timeout)` best-effort flush，rate_limit 附在响应上。无出边。

### 2.3 Transport trait（proto-free 接缝）

```rust
// EasyNet-Axon/sdk/rust/src/invocation/transport.rs
// 零 tokio / 零 tonic / 零 prost。async-trait 已是 SDK 硬依赖（Cargo.toml:26）。
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// 拉下一个入站 wire 单元，归一化成几何无关的 frame。
    /// Unary: 一次 Envelope 后 ChannelClosed。Stream: 一次 Envelope（header）。
    /// Bidi: Envelope(frame-0) 后跟 Progress/Message up-frame 序列。
    /// Session: SessionDispatch::{Dispatch|BidiOpen|BidiInput|RequestResult} demux 成 frame。
    async fn recv_frame(&mut self) -> Result<TransportFrame, AxonError>;

    /// 推一个出站 wire 单元。driver 在每个生命周期边界调用：
    ///   Admitting→Routing:  send_frame(Receipt(admission))
    ///   Dispatching self-loop: send_frame(Progress{..})  // stream/bidi；unary 缓冲
    ///   Settled/Failed:     send_frame(Receipt(terminal))
    /// adapter 拥有几何：unary 收成单个 InvokeResponse；stream 发 InvokeStreamChunk；
    /// bidi 盖 InvokeBidiDown.sequence；session 编码 carrier-v0 JSON 或 carrier-v1 DispatchResult。
    async fn send_frame(&mut self, frame: TransportFrame) -> Result<(), AxonError>;

    /// 优雅收尾。Drain(timeout_ms) 是 best-effort 提示；unary/session 可 noop。
    async fn close(&mut self) -> Result<(), AxonError>;
}

/// 几何无关 frame 词汇（覆盖全部 4 种几何）。全部成员 proto-free。
pub enum TransportFrame {
    Envelope(DescriptorBoundEnvelope),  // admission/open frame（axiom.rs，core 类型）
    Receipt(InvocationReceipt),         // admission + terminal 边界 receipt（audit.rs，core）
    Progress { payload: Vec<u8>, content_type: String, terminal: bool, sequence: u64 },
    Message(Vec<u8>),                   // bidi/session 入站 up-frame（stdin/resize/chunk）
    Cancel { reason: String },          // 外部取消 → Cancelled
    Drain { timeout_ms: u64 },          // settled flush 提示
}
```

**背压裁决**：`Full`(可重试) vs `Closed`(offline) 的分类由 **adapter 在 send_frame 内**做，表面化为 `Err(AxonError)`，driver 映射成 `Dispatching→Failed`。**driver 永不解释 tokio/tonic 类型**。这是 `busy != offline` 修复唯一一份实现的归宿。

### 2.4 Context struct（按 move 穿过 5 个阶段）

`InvocationLifecycleContext` 是 4 个 dispatcher 各自 `state_carried` 的并集，按 move 穿过 `admit()→route()→dispatch()→project()→settle()`。每个阶段消费完的字段在下一阶段死亡（admission 输入在 Routing 后死，route 在 Dispatching 后死）。完整字段表见附录 A（采纳收敛设计原文，逐字段标注 SDK/CLI 归属）。

**唯一 proto 污染点**：`selected_route: Option<SelectedInvokeRoute>` —— 当前 `SelectedInvokeRoute`（route_resolver.rs:137-151）持有 `axon_pb::ResolverReleaseProfile`(:146) / `axon_pb::RouteReason`(:147) / `serde_json::Value`×3。**这阻断了它原样进 proto-free SDK core**。裁决见 §5 阻塞 B1。

### 2.5 Go 将来如何 mirror

Axon proto 已是 Envelope/URA/Invocation/Receipt 的唯一定义（跨 repo 协议重复≈0，已对盘）。`lifecycle.rs` 是纯 Rust 编排，不进 proto。Go 后端将来：(1) 复用同一 proto 投影出的 Go facade（Envelope/Receipt）；(2) 用 Go 重写**同一张状态机**（states + transitions 是语言无关的本文 §2.2），driver 逻辑照抄；(3) 实现 Go 侧 `Transport` 接口（Go 用 `interface{ RecvFrame; SendFrame; Close }`，无 async-trait 负担）。**状态机的 transition 表是跨语言契约**，Rust 与 Go 各自实现但必须逐边一致——本文 §2.2 即该契约。

---

## 3. 落地序列（每步独立可编译 = 干净的 1-逻辑-改动 commit）

> 纪律：旧 dispatcher 是 **DELETE，不是 deprecate**。不留 `#[deprecated]`、不留 `_v2` 并存、不留 feature flag 切换。每步 `cargo build -p easynet-cli` + `cargo build -p easynet_axon --features proto` + `--features axon-pb`（CLI 侧）必须绿。

**先决条件（CTO 裁决，见 §5）必须先关闭 B1、B2、B3，否则 Step 2 无法编译。**

### Phase A — SDK 建原语（CLI 不动，SDK 单独可编译）

- **Step 1**：SDK 新建 `src/invocation/transport.rs`：`Transport` trait + `TransportFrame` enum。`mod.rs` 加 `pub mod transport;` + 导出。
  验证：`cargo build -p easynet_axon`（default，proto-off）绿 → 证明 TransportFrame 全 proto-free。
- **Step 2**：SDK 新建 `src/invocation/route.rs`：`RouteResolver` trait + `ResolvedRoute`（proto-free 投影，B1 裁决产物）。
  验证：`cargo build -p easynet_axon` 绿。
- **Step 3**：SDK 新建 `src/invocation/lifecycle.rs`：`InvocationLifecycleContext` + 5 阶段 driver（`admit/route/dispatch/project/settle`）+ 状态枚举 + 失败分类（B2 裁决产物）。driver 调用已有 `run_descriptor_bound_admission` / `dispatch_shim`-等价原语 / `is_terminal` / `LedgerSink`。
  验证：`cargo build -p easynet_axon` 绿 + 为 driver 写单元测试（用 in-memory 假 Transport 跑通 5 阶段 happy path + 每条 Failed 边）。SDK 此时已能独立驱动一条调用。

### Phase B — CLI 建 adapter（逐几何替换，每替换一个删一个旧 dispatcher）

- **Step 4**：CLI 新建 `transport/unary.rs`（impl `Transport`）+ `route_adapter.rs`（impl `RouteResolver`，包 `resolve_route`，做 proto→`ResolvedRoute` 投影）。把 `daemon_invocation_service.rs` 的 `invoke`（unary RPC）改为：建 unary adapter → 交 SDK driver。**DELETE `unary_dispatcher.rs`(2282)**。`admission_facade.rs` 的 `verify_invoke` 调用点折叠进 driver 的 admit()。
  验证：`cargo build` + unary 相关 e2e（`cross_device_invoke_remote_e2e`、`resolve_before_invoke_e2e`）绿。
- **Step 5**：CLI 新建 `transport/stream.rs`。`invoke_server_stream` 改走 driver。**DELETE `stream_dispatcher.rs`(472)**。
  验证：`cargo build` + stream e2e（`cross_realm_directory_streaming_e2e`）绿。
- **Step 6**：CLI 新建 `transport/bidi.rs`（sequence stamping / up-seq 校验 / 5s heartbeat / carrier 协商 / claimant-nonce 全留在 adapter）。`invoke_bidi` 改走 driver。**DELETE `bidi_dispatcher.rs`(3369)**。
  验证：`cargo build` + bidi e2e（`cross_device_chat`、`session_negotiation`、`session_dispatch_fixture`）绿。
- **Step 7**：CLI 新建 `transport/session.rs`（carrier-v0/v1 双读、JSON↔BinaryChunk codec、escalation RequestResult）。**DELETE `local_session_dispatcher.rs`(3670)**。escalation 失败映射并入 driver 失败分类（B2）。
  验证：`cargo build` + session e2e（`cross_hub_two_daemon_real_tls_e2e`）绿。

### Phase C — 折叠剩余共享逻辑 + 清残渣

- **Step 8**：`admission_facade.rs`：`verify_invoke*` 三入口已无调用方 → **DELETE 这三个方法**，保留 `check_quota_for_ability` 及其被 driver 调的策略件（迁入 driver 的 admit 边或保留为 CLI policy provider，按 B2 裁决）。
  验证：`cargo build` + `admission_delegation_metadata`、`cross_realm_signed_admission_e2e` 绿。
- **Step 9**：`ledger_projection.rs`：`build_unary_ledger_record` / `ledger_record_from_remote_receipt` 的逻辑由 driver 经 `LedgerSink` 接管 → **DELETE `ledger_projection.rs`(641)**（若 daemon ledger row 映射有 CLI-only 字段，保留最小 mapper，但不得保留旧整文件）。
  验证：`cargo build` + ledger 断言相关 e2e 绿。
- **Step 10**：`invoke_remote_initiator.rs`：`RequestOutcome`/`SessionRequestError` 已被 driver 失败分类取代 → **DELETE 该枚举及映射(354-378)**；剩余 remote send/recv 机制并入 `transport/session.rs` 或 `transport/bidi.rs` 的 adapter。
  验证：`cargo build` + 全 e2e 套件绿。
- **Step 11**：`daemon_invocation_service.rs`：删 ability-name routing 决策树（760-827 if/else over `ABILITY_FEDERATION_*`）中属于 dispatcher 选择的部分，federation ability 仍 route 到 `federation_wrappers` handler。`mod.rs` 删所有被删文件的 `pub mod`/`pub use`。
  验证：`cargo build`（含 `--features axon-pb`）+ `cargo clippy` 零 warning（证明无 dead code）+ 全 e2e 绿。

每步结束 `cargo build` 必绿 = 每个 commit 是一个独立可编译的逻辑单元。

---

## 4. 被删除清单（无残渣）

> 以下文件**整文件删除**（DELETE，非 deprecate）。LOC 来自磁盘 `wc -l`。

| 文件 | LOC | 删除原因 | 删除步 |
|---|---:|---|---|
| `unary_dispatcher.rs` | 2282 | 生命周期编排下沉 driver；unary 几何 → `transport/unary.rs` | Step 4 |
| `stream_dispatcher.rs` | 472 | 同上；stream 几何 → `transport/stream.rs` | Step 5 |
| `bidi_dispatcher.rs` | 3369 | 同上；bidi 几何 → `transport/bidi.rs` | Step 6 |
| `local_session_dispatcher.rs` | 3670 | 同上；session 几何 → `transport/session.rs` | Step 7 |
| `ledger_projection.rs` | 641 | 经 `LedgerSink` 由 driver 接管 | Step 9 |

**整文件删除合计：10,434 LOC。**

部分删除（方法/枚举级，非整文件）：
- `admission_facade.rs`：`verify_invoke`/`verify_invoke_stream`/`verify_envelope_for_bidi`（506-560 区）折叠进 driver admit()。文件其余（quota meter、policy provider）保留。
- `invoke_remote_initiator.rs`：`RequestOutcome`/`SessionRequestError`(354-378) 删除，逻辑并入 driver 失败分类 + adapter。
- `daemon_invocation_service.rs`：dispatcher-选择决策树(760-827) 删除。
- `route_resolver.rs`：保留 `resolve_route` 逻辑，但 `SelectedInvokeRoute` 按 B1 裁决拆分（proto 部分留 CLI，投影部分进 SDK `ResolvedRoute`）。

**不删除**（明确保留，非生命周期）：
- `federation_wrappers.rs`(2398)：14 个 `federation.*` ability handler 业务体（gate 之后运行）。
- `session_initiator.rs` / `session_escalation.rs` / `peer_envelope_signer.rs` / `federated_key_resolver.rs` / `register_device_pubkey.rs` 等：identity/federation 子系统，非调用生命周期。
- `route_resolver.rs` 主体 / `boot.rs` / `invocation_wire.rs` / `target_gate.rs` / `quota_meter.rs`。

**残渣清零验证**：Step 11 末 `cargo clippy -p easynet-cli --features axon-pb -- -D warnings` 必须零 warning。任何 unused import / unused fn = 残渣，CI 红。

---

## 5. 风险与阻塞（**需 CTO 在写码前裁决**）

### B1（硬阻塞 / 必须先裁）：`SelectedInvokeRoute` 是 proto-coupled，不能原样进 proto-free SDK core

`SelectedInvokeRoute`（route_resolver.rs:137-151）持有 `axon_pb::ResolverReleaseProfile`(:146)、`axon_pb::RouteReason`(:147)、`serde_json::Value`×3(:148-150)。SDK 的 `default = []` 不变量（Cargo.toml:58）要求 core 保持 proto-agnostic——`is_authoritative_local_or_better()`(:155) 直接 match `axon_pb::ResolverReleaseProfile`。

context struct 的 `selected_route` 字段因此无法直接放进 `lifecycle.rs`（proto-free）。

**待裁三选一：**
- **(a) 推荐**：SDK `route.rs` 定义 proto-free 的 `ResolvedRoute`（用 SDK 自有枚举 `RouteReleaseProfile`/`RouteReason` 镜像，而非 `axon_pb`）；CLI `route_adapter.rs` 做 `axon_pb::* → ResolvedRoute` 投影。locality 决策（`is_authoritative_local_or_better`）的语义随之进 SDK。代价：一次性投影映射 + 两个小枚举镜像。**符合"proto 定义一次、facade 投影"的终局**。
- (b) `lifecycle.rs` 仅在 `proto` feature 下编译。代价：破坏 `default=[]` 不变量，污染整个 core 的 proto 洁净度，且 Go 无法 mirror 一个 proto-gated 的 Rust 文件。**不推荐**。
- (c) `ResolvedRoute` 用 `String`/`enum` 退化承载 release_profile。代价：丢类型安全。**不推荐**。

> 不裁 B1，Step 2/3 无法编译。

### B2（硬阻塞 / 必须先裁）：失败分类 taxonomy —— SDK `AxonErrorKind` 只有 7 个通用 gRPC 变体，没有 `TargetOffline`

SDK `AxonErrorKind`（error.rs:176）= `Cancelled / DeadlineExceeded / Unavailable / InvalidArgument / ResourceExhausted / PermissionDenied / Internal`。CLI 的 `SessionRequestError`（invoke_remote_initiator.rs:367）= `TargetOffline / PermissionDenied / UpstreamFailure / UpstreamTimeout`，这四个**正是** `Dispatching→Failed` 的边守卫。driver 的失败分类需要表达这四种语义，但 SDK 没有 `TargetOffline` 这个 kind。

**待裁二选一：**
- **(a) 推荐**：映射进现有 7 变体 + 用 `reason`/`code`/`stage` 携带细分语义（`TargetOffline→Unavailable+reason`、`UpstreamTimeout→DeadlineExceeded`、`UpstreamFailure→Internal`、`PermissionDenied→PermissionDenied`）。背压 `Full` 用 `ResourceExhausted+retry_after_ms`。`busy!=offline` 靠 `retriable()`(error.rs:331) 区分。代价：失败语义靠 reason 字符串而非 kind 枚举，需约定 reason 常量（与 admission REASON_* 同纪律）。
- (b) SDK `AxonErrorKind` 扩展加 `TargetOffline`/`UpstreamTimeout` 等。代价：改 SDK wire-visible 错误契约 + `map_proto_code`(error.rs:213)，跨 repo（Go/Python）连锁。**仅当 (a) 表达力不足时才选**。

> 不裁 B2，driver 失败边无法落地。

### B3（软阻塞 / 建议裁）：driver 的 async 与 tokio 解耦边界

`Transport` trait 用 `async-trait`（SDK 已有硬依赖，Cargo.toml:26），trait 本身零 tokio/tonic ✅。但 driver 的 `Dispatching` self-loop（stream pump）和 REMOTE await（`PRESENCE_DISPATCH_REPLY_TIMEOUT`）涉及超时——SDK core 不应依赖 `tokio::time`。

**待裁**：超时由 **adapter 在 `recv_frame` 内实现**（adapter 持 tokio，超时到则返回 `Err(AxonError::deadline_exceeded)`），driver 只看 `Result`。**推荐采纳**——与背压同纪律（adapter 拥有运行时，driver 拥有状态机）。低风险，但需明确写进契约以防 driver 误引 tokio。

### B4（非阻塞 / 知会）：proto-only 双导入

`TransportFrame::Envelope(DescriptorBoundEnvelope)` 用 axiom.rs:478 的 core 类型（proto-free，已验）。CLI adapter 把 wire 的 `EnvelopeOpen`（proto）转成 `DescriptorBoundEnvelope` 一次，避免双导入。无需裁决，实现纪律即可。

### B5（非阻塞 / 知会）：Go reuse 约束

状态机 transition 表（§2.2）是跨语言契约。Rust 落地后，Go mirror 必须逐边一致。建议 Step 3 完成后把 §2.2 的转移表抽成 `document/concepts/INVOCATION_STATE_MACHINE.md` 的权威版本（该文件已被 mod.rs:7 引用），作为 Rust/Go 共同的 source of truth。无需写码前裁决，但需进 endgame 文档纪律。

---

## 附录 A — InvocationLifecycleContext 字段归属表

（采纳收敛设计原文，逐字段标 SDK core / CLI adapter 归属与阶段生死）

```
pub struct InvocationLifecycleContext {
  // ── 不可变请求（Admitting 携入，全程存活）── 全部 proto-free，进 SDK ──
  envelope: DescriptorBoundEnvelope,        // axiom.rs core ✅
  ability: String,
  args: Vec<u8>,
  metadata: Option<HashMap<String,String>>, // delegation/session-authority proof
  started_unix_ms: i64,
  content_envelope: SessionContentEnvelope, // 编码/加密 —— 若含 proto 需 B1 同类投影
  origin_caller: Option<OriginCallerClaim>, // device-mode 身份抬升

  // ── Admission/Policy（Admitting 变更，Routing 后死）──
  replay_store: SharedNonceReplayStore,     // admission.rs::NonceReplayStore ✅ SDK
  quota_status: Option<RateLimitInfo>,      // CLI policy provider 注入，附终态响应

  // ── Route（Routing 填，Dispatching 消费后死）── 见 B1 ──
  selected_route: Option<ResolvedRoute>,    // ⚠ B1：proto→ResolvedRoute 投影
  is_self_target: bool,                      // matches_self_target_ura → local/remote
  target_realm: Option<String>,
  peer_hub_endpoint: Option<String>,

  // ── Dispatch correlation（Dispatching 填）──
  call_id: CallId,                           // u64(presence/session) | [u8;16](invoke_remote)
  call_mode: CallMode,                       // call_mode.rs ✅ SDK：Rpc|Stream|Bidi
  expected_up_sequence: u64,                 // bidi/session up-frame 序
  terminal_seen: bool,                       // exactly-one-terminal 不变量

  // ── Outcome（Projecting 填）── 全部 SDK core 类型 ──
  outcome_state: InvocationState,            // handle.rs ✅
  payload_bytes: Vec<u8>,
  error: Option<AxonError>,                  // error.rs ✅（B2 决定细分语义载体）
  invocation_id: Option<String>,
  admission_receipt: Option<InvocationReceipt>,   // audit.rs ✅
  terminal_receipt: Option<InvocationReceipt>,    // carrier-v1 终态必需
}
```

---

## 附录 B — 留在 CLI 的 adapter 关注点（明确不进 SDK）

- **UNARY**（unary.rs）：一次性 collapse 成单个 `InvokeResponse` + 可选 `tonic::Status`。`recv_frame` 一次 yield Envelope。无 frame loop/sequence/Progress。
- **STREAM**（stream.rs）：mpsc→`tonic Response<BoxedDownStream<InvokeStreamChunk>>` pump，"drain 到 terminal，逐 frame forward"。`frame.terminal` 作 bool flag。这是 Dispatching self-loop 的发送侧。
- **BIDI**（bidi.rs）：`InvokeBidiDown.sequence` stamping（旧 bidi_dispatcher.rs:1930-1933）、up-seq 校验、EOF 注入安全、5s `SESSION_DOWN_HEARTBEAT`、carrier 协商 `min(device,hub)`、claimant-nonce 碰撞/displacement、up/down 双 loop 并发、Pty/FileTransfer/JsonFrames wire-kind 映射。
- **LOCAL SESSION**（session.rs）：carrier-v0(JSON BinaryChunk)/carrier-v1(proto DispatchCall) 双读、JSON↔BinaryChunk codec、remote_bidi/stream session 注册表、escalation `RequestResult` 完成（device-mode）。
- **FEDERATION**（federation_wrappers.rs，**不是 transport**）：14 个 `federation.*` ability handler 业务体（gate 之后），保留。
- **device-mode escalation handle**（`SessionEscalationHandle`，session_escalation.rs）+ **cross-realm peer delegation**（`client.forward_invoke` over `FederationClient`，unary_dispatcher.rs:1370-1433 区域）：Dispatching 边背后的 remote send/recv 机制，wire 成 Transport impl。
```

---

## 6. 裁定（RATIFIED 2026-06-25 — CTO Silan.Hu）

§5 的三个写码前阻塞已全部裁决。落地序列（§3）的先决条件**就此关闭**，Step 1 起可开工。

### B1 → 裁定 (a)：SDK 定义 proto-free `ResolvedRoute` + CLI 投影

SDK `src/invocation/route.rs` 定义 proto-free 的 `ResolvedRoute`，用 SDK 自有枚举 `RouteReleaseProfile` / `RouteReason` **镜像** `axon_pb::ResolverReleaseProfile` / `axon_pb::RouteReason`（不引 `axon_pb`）。CLI `route_adapter.rs` 做一次性 `axon_pb::* → ResolvedRoute` 投影。`is_authoritative_local_or_better()` 的 locality 决策语义随 `ResolvedRoute` 进 SDK。
**理由**：唯一符合"proto 定义一次、facade 投影"终局的选项；保住 SDK `default=[]` proto-agnostic 不变量（Cargo.toml:58）；Go 将来可 mirror（(b) 的 proto-gated 文件 Go 无法 mirror）。
**代价已接受**：一次性投影映射 + 两个小枚举镜像。

### B2 → 裁定 (a)：映进现有 7 变体 + `reason` / `retriable()` 承载细分语义

不扩 SDK `AxonErrorKind`（保 wire-visible 错误契约不变、不触发 Go/Python 跨 repo 连锁）。`Dispatching→Failed` 的四种失败语义映射如下：
- `TargetOffline` → `Unavailable` + `reason`（offline 细分）
- `UpstreamTimeout` → `DeadlineExceeded`
- `UpstreamFailure` → `Internal`
- `PermissionDenied` → `PermissionDenied`
- 背压 `Full`（可重试） → `ResourceExhausted` + `retry_after_ms`

`busy != offline` 的区分靠 `retriable()`（error.rs:331）：`Full`/`ResourceExhausted` 可重试保 session，`Closed`/`Unavailable+StreamClosed` 驱逐。
**代价已接受**：失败细分语义靠 `reason` 常量约定承载（与 admission `REASON_*` 同纪律）——须为这四个 reason 立常量，不得裸字符串散落。

### B3 → 裁定：adapter 拥有 tokio，driver 只看 `Result`

超时由 adapter 在 `recv_frame` 内实现（adapter 持 `tokio::time`，到期返回 `Err(AxonError::deadline_exceeded)`），driver 永不引 `tokio`/`tonic`。与背压同纪律（adapter 拥有运行时，driver 拥有状态机）。
**契约硬约束**：`src/invocation/lifecycle.rs` 与 `route.rs` 不得出现 `tokio::` / `tonic::` / `axon_pb::` 任一路径；Step 3 验证须包含 `! grep -E 'tokio::|tonic::|axon_pb::' src/invocation/lifecycle.rs src/invocation/route.rs`（命中即失败）。这是 Go 可 mirror 纯状态机的前提。

### 裁定后状态

- 先决条件关闭，§3 落地序列 Step 1 可开工。
- **但开工时机受共享检出约束**：本分支 `seven-axes-p0-landing-v1` 当前有并发会话改动 `src/runtime/ability_dispatch.rs`（未提交）且在跑 `cargo test --features axon-pb`。开工前须 (1) 确认无并发 build 占用 `target/`，(2) 优先在 GitHub-兄弟目录的隔离 worktree 实施（`../EasyNet-Axon` 相对依赖要求 worktree 为兄弟位），(3) 每步 `cargo build` + Step 11 末 `clippy -D warnings` 验残渣清零。
- B2 的四个 `reason` 常量、B1 的两个镜像枚举，须在对应 Step 的同一 commit 内落地，不得后补。
